// SPDX-License-Identifier: MPL-2.0

//! Reachability walk over the crate's own source, so a test can ask
//! "which types can this deserializer actually be handed?" and get an
//! answer that stays true as the model grows.
//!
//! The problem this exists to solve: `serde(deny_unknown_fields)` is
//! the thing that stops a hand-authored `.mindmap.json` key from
//! being dropped on load and destroyed on the next save. It is a
//! per-type opt-in. A hand-maintained list of "types that must carry
//! it" is exactly the sort of twin surface that drifts the moment
//! somebody adds a field — the same failure mode
//! `lib/baumhard/CONVENTIONS.md` §B4 records for the field-tag enums.
//!
//! So nothing is listed. [`TypeGraph::build`] parses every non-test
//! `.rs` file under baumhard's `src/` with `syn`, indexes every
//! struct / enum / type alias, and [`TypeGraph::reachable_from`]
//! walks the field graph outward from a root type name. Adding a
//! field of a new type extends the covered set automatically; the
//! test that consumes this fails until the new type opts in.
//!
//! **`src/` is the whole of what it can see**, and that is a real
//! limit rather than a phrasing detail: a type emitted by
//! `lib/baumhard/build.rs` into `$OUT_DIR` is reachable from a load
//! and invisible here. [`TypeGraph::unresolved_from`] reports every
//! name the walk gave up on so a test can hold that gap to an
//! explicit list instead of letting it grow in silence.
//!
//! Test-only and native-only: it reads `.rs` files off the
//! filesystem, which is also why it is `#[cfg(test)]` — `syn` is a
//! dev-dependency and nothing in a shipped build should carry a
//! parser for its own source.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use syn::{Attribute, Fields, GenericArgument, Item, PathArguments, Type};

/// Whether a walk follows `#[serde(into = "...")]` proxies. Read
/// questions must not (a serialize-only proxy is never handed to a
/// deserializer); write questions must.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowSerializeProxy {
    /// Follow `into` proxies — the walk is about the save path.
    Yes,
    /// Ignore them — the walk is about the load path.
    No,
}

/// Whether an indexed item is a struct, an enum, or a type alias.
/// Aliases carry no attributes of their own; they exist in the index
/// so `type DeltaGlyphArea = Delta<GlyphAreaField>` forwards the walk
/// to both `Delta` and `GlyphAreaField`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    /// A `struct` item.
    Struct,
    /// An `enum` item.
    Enum,
    /// A `type X = ...;` alias — a pass-through node in the walk.
    Alias,
}

/// One indexed type: where it lives, what serde does with it, and
/// which other type names its fields mention.
#[derive(Debug, Clone)]
pub struct TypeInfo {
    /// The item's identifier, as written.
    pub name: String,
    /// Repo-relative-ish path of the file that defines it, for error
    /// messages that a reader can act on.
    pub file: PathBuf,
    /// Struct, enum, or alias.
    pub kind: TypeKind,
    /// `true` when the item's derive list includes `Deserialize`.
    pub derives_deserialize: bool,
    /// `true` when the item carries `#[serde(deny_unknown_fields)]`.
    pub denies_unknown_fields: bool,
    /// The proxy named by `#[serde(from = "...")]` /
    /// `#[serde(try_from = "...")]`, if any. Such a type never
    /// deserializes its own shape — the proxy does — so the
    /// requirement moves to the proxy.
    pub deserialize_proxy: Option<String>,
    /// The proxy named by `#[serde(into = "...")]`, if any. Such a
    /// type never serializes its own shape — it converts first — so
    /// the *write* path's contract lives on the proxy. Followed only
    /// by [`TypeGraph::omit_predicates_from`], never by
    /// [`TypeGraph::reachable_from`]: a serialize-only proxy is never
    /// handed to a deserializer and has no business in the
    /// `deny_unknown_fields` coverage set.
    pub serialize_proxy: Option<String>,
    /// `true` when the item carries `#[serde(untagged)]`. Untagged
    /// enums decide a variant by trial deserialization, so denying
    /// unknown fields on them changes which variant matches rather
    /// than merely tightening a check.
    pub untagged: bool,
    /// `true` when some part of the item can consume named JSON
    /// object keys: a struct with named fields, or an enum with at
    /// least one struct variant. Tuple and unit shapes have no field
    /// names for a stray key to hide in.
    pub has_named_fields: bool,
    /// Every type name mentioned by a deserializable field, in
    /// source order and de-duplicated by the walk rather than here.
    pub referenced: Vec<String>,
    /// Every predicate named by a field's
    /// `#[serde(skip_serializing_if = "...")]`, in source order.
    ///
    /// These are the only sanctioned reason a key present in an
    /// authored file may be absent from the saved one, so a
    /// round-trip test needs the set to tell "the saver dropped a
    /// key" from "the key held its own default and serde left it
    /// out". Collected from *every* field, including ones
    /// `#[serde(skip)]` keeps out of [`Self::referenced`] — this is a
    /// question about the write path, not the read path.
    pub omit_predicates: Vec<String>,
}

/// Index of every struct, enum, and type alias defined under one
/// source root.
///
/// Cost: one full read and `syn` parse of every non-test `.rs` file
/// under the root — tens of milliseconds for this crate, paid once
/// per test that builds a graph.
#[derive(Debug, Clone, Default)]
pub struct TypeGraph {
    items: BTreeMap<String, TypeInfo>,
    duplicates: BTreeSet<String>,
}

impl TypeGraph {
    /// Parse every `.rs` file under `src_root` and index the items it
    /// declares, descending into inline `mod { … }` blocks.
    ///
    /// Test sources are skipped — a `struct StubCtx` inside a test
    /// module is not part of any on-disk contract, and letting one
    /// shadow a real model type would silently move the walk. Files
    /// named `tests.rs` / `*_tests.rs` / `*_test.rs`, directories
    /// named `tests`, and `#[cfg(test)]` modules are all excluded.
    ///
    /// Panics if a file cannot be read or parsed: this backs a test
    /// whose entire job is to notice that the source moved, so a
    /// silent skip would defeat it.
    pub fn build(src_root: &Path) -> Self {
        let mut graph = TypeGraph::default();
        let mut files = Vec::new();
        collect_source_files(src_root, &mut files);
        files.sort();
        for file in files {
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
            let parsed = syn::parse_file(&text)
                .unwrap_or_else(|e| panic!("{} must parse as Rust: {e}", file.display()));
            graph.index_items(&parsed.items, &file);
        }
        graph
    }

    /// The indexed entry for `name`, or `None` when the crate does
    /// not declare a type by that name.
    pub fn get(&self, name: &str) -> Option<&TypeInfo> {
        self.items.get(name)
    }

    /// Type names declared more than once across the indexed sources.
    /// The walk resolves by bare name, so a collision would make its
    /// answer ambiguous; callers assert this is empty rather than
    /// silently trusting whichever definition landed last.
    pub fn duplicate_names(&self) -> &BTreeSet<String> {
        &self.duplicates
    }

    /// Every distinct `#[serde(skip_serializing_if = "...")]`
    /// predicate named by a type reachable from `root`.
    ///
    /// This is the complete set of reasons the saver is allowed to
    /// write out fewer keys than it read in. A round-trip test pins
    /// it against a hand-modeled allowlist, so a new predicate cannot
    /// quietly widen what "no authored key is lost" tolerates.
    ///
    /// Walks the **write** graph, which is not the read graph:
    /// `#[serde(into = "...")]` proxies are followed here and nowhere
    /// else. `CustomMutation` is the live case — its keys are written
    /// by `CustomMutationOut`, and one of that type's fields is the
    /// only `String::is_empty` omission a saved map can produce.
    ///
    /// Cost: one BFS over the reached types; no I/O.
    pub fn omit_predicates_from(&self, root: &str) -> BTreeSet<String> {
        self.walk(root, FollowSerializeProxy::Yes)
            .iter()
            .flat_map(|info| info.omit_predicates.iter().cloned())
            .collect()
    }

    /// Every name a reachable type mentions that this index does not
    /// resolve — the walk's **terminators**.
    ///
    /// [`reachable_from`](Self::reachable_from) drops an unresolved
    /// name on the floor, which is right for `String` and `f64` and
    /// wrong for a type that really is part of the on-disk contract
    /// but was not indexed: anything `build.rs` emits into `$OUT_DIR`
    /// (`AppFont`), a derive-generated tag (strum's
    /// `*Discriminant`), a type from another crate. The walk cannot
    /// tell those two cases apart — both are simply "a name I do not
    /// have" — so it reports them and
    /// `tests::test_every_walk_terminator_is_an_expected_one` decides,
    /// against [`EXPECTED_TERMINATORS`]. A newly unresolved name goes
    /// red instead of silently shrinking the covered set.
    ///
    /// Cost: same walk as `reachable_from` plus one set insert per
    /// referenced name; no I/O.
    pub fn unresolved_from(&self, root: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for info in self.reachable_from(root) {
            let mentioned = info.referenced.iter().chain(info.deserialize_proxy.as_ref());
            for name in mentioned {
                if !self.items.contains_key(name.as_str()) {
                    out.insert(name.clone());
                }
            }
        }
        out
    }

    /// Every indexed type reachable from `root` by following
    /// deserializable fields, including `root` itself.
    ///
    /// Names that are not indexed (`String`, `f64`,
    /// `serde_json::Value`, generic parameters) terminate the walk —
    /// they carry no fields of ours. Aliases forward to the types
    /// their right-hand side mentions.
    ///
    /// Cost: O(reached types × their field count); no I/O.
    pub fn reachable_from(&self, root: &str) -> Vec<&TypeInfo> {
        self.walk(root, FollowSerializeProxy::No)
    }

    /// The BFS behind [`reachable_from`] and
    /// [`omit_predicates_from`]. Field references and
    /// `#[serde(from)]` proxies are always followed;
    /// `#[serde(into)]` proxies only when the caller is asking a
    /// question about the write path.
    fn walk(&self, root: &str, serialize: FollowSerializeProxy) -> Vec<&TypeInfo> {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        let mut out: Vec<&TypeInfo> = Vec::new();

        queue.push_back(root);
        seen.insert(root);
        while let Some(name) = queue.pop_front() {
            let Some(info) = self.items.get(name) else { continue };
            out.push(info);
            let write_proxy = match serialize {
                FollowSerializeProxy::Yes => info.serialize_proxy.as_deref(),
                FollowSerializeProxy::No => None,
            };
            let successors = info
                .referenced
                .iter()
                .map(String::as_str)
                .chain(info.deserialize_proxy.as_deref())
                .chain(write_proxy);
            for next in successors {
                if let Some(entry) = self.items.get_key_value(next) {
                    if seen.insert(entry.0.as_str()) {
                        queue.push_back(entry.0.as_str());
                    }
                }
            }
        }
        out
    }

    fn index_items(&mut self, items: &[Item], file: &Path) {
        for item in items {
            match item {
                Item::Struct(item) => {
                    let serde = SerdeAttrs::read(&item.attrs);
                    self.insert(TypeInfo {
                        name: item.ident.to_string(),
                        file: file.to_path_buf(),
                        kind: TypeKind::Struct,
                        derives_deserialize: derives_deserialize(&item.attrs),
                        denies_unknown_fields: serde.deny_unknown_fields,
                        deserialize_proxy: serde.proxy,
                        serialize_proxy: serde.into_proxy,
                        untagged: serde.untagged,
                        has_named_fields: matches!(item.fields, Fields::Named(_)),
                        referenced: referenced_types(&item.fields),
                        omit_predicates: omit_predicates(&item.fields),
                    });
                }
                Item::Enum(item) => {
                    let serde = SerdeAttrs::read(&item.attrs);
                    let mut referenced = Vec::new();
                    let mut predicates = Vec::new();
                    let mut has_named_fields = false;
                    for variant in &item.variants {
                        has_named_fields |= matches!(variant.fields, Fields::Named(_));
                        referenced.extend(referenced_types(&variant.fields));
                        predicates.extend(omit_predicates(&variant.fields));
                    }
                    self.insert(TypeInfo {
                        name: item.ident.to_string(),
                        file: file.to_path_buf(),
                        kind: TypeKind::Enum,
                        derives_deserialize: derives_deserialize(&item.attrs),
                        denies_unknown_fields: serde.deny_unknown_fields,
                        deserialize_proxy: serde.proxy,
                        serialize_proxy: serde.into_proxy,
                        untagged: serde.untagged,
                        has_named_fields,
                        referenced,
                        omit_predicates: predicates,
                    });
                }
                Item::Type(item) => {
                    let mut referenced = Vec::new();
                    collect_type_names(&item.ty, &mut referenced);
                    self.insert(TypeInfo {
                        name: item.ident.to_string(),
                        file: file.to_path_buf(),
                        kind: TypeKind::Alias,
                        derives_deserialize: false,
                        denies_unknown_fields: false,
                        deserialize_proxy: None,
                        serialize_proxy: None,
                        untagged: false,
                        has_named_fields: false,
                        referenced,
                        omit_predicates: Vec::new(),
                    });
                }
                Item::Mod(item) => {
                    if is_test_gated(&item.attrs) {
                        continue;
                    }
                    if let Some((_, inner)) = &item.content {
                        self.index_items(inner, file);
                    }
                }
                _ => {}
            }
        }
    }

    fn insert(&mut self, info: TypeInfo) {
        if self.items.contains_key(&info.name) {
            self.duplicates.insert(info.name.clone());
            return;
        }
        self.items.insert(info.name.clone(), info);
    }
}

/// Recursively gather `.rs` files under `dir`, skipping test sources.
fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            if name != "tests" {
                collect_source_files(&path, out);
            }
        } else if name.ends_with(".rs") && !is_test_file_name(&name) {
            out.push(path);
        }
    }
}

fn is_test_file_name(name: &str) -> bool {
    name == "tests.rs" || name.ends_with("_tests.rs") || name.ends_with("_test.rs")
}

fn is_test_gated(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        let mut gated = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("test") {
                gated = true;
            }
            Ok(())
        });
        gated
    })
}

fn derives_deserialize(attrs: &[Attribute]) -> bool {
    let mut found = false;
    for attr in attrs {
        if !attr.path().is_ident("derive") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta
                .path
                .segments
                .last()
                .is_some_and(|seg| seg.ident == "Deserialize")
            {
                found = true;
            }
            Ok(())
        });
    }
    found
}

/// The subset of `#[serde(...)]` container options this walk cares
/// about.
#[derive(Debug, Default)]
struct SerdeAttrs {
    deny_unknown_fields: bool,
    untagged: bool,
    /// `#[serde(from = "…")]` / `#[serde(try_from = "…")]`.
    proxy: Option<String>,
    /// `#[serde(into = "…")]`.
    into_proxy: Option<String>,
}

impl SerdeAttrs {
    fn read(attrs: &[Attribute]) -> Self {
        let mut out = SerdeAttrs::default();
        for attr in attrs {
            if !attr.path().is_ident("serde") {
                continue;
            }
            // Parse errors are ignored on purpose: an option this
            // walk does not model (a nested `bound(...)`, say) must
            // not take the whole file down. The flags it does model
            // are plain idents or `= "literal"` pairs, both handled
            // below.
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("deny_unknown_fields") {
                    out.deny_unknown_fields = true;
                } else if meta.path.is_ident("untagged") {
                    out.untagged = true;
                } else if meta.path.is_ident("from") || meta.path.is_ident("try_from") {
                    let literal: syn::LitStr = meta.value()?.parse()?;
                    out.proxy = Some(last_path_segment(&literal.value()));
                } else if meta.path.is_ident("into") {
                    let literal: syn::LitStr = meta.value()?.parse()?;
                    out.into_proxy = Some(last_path_segment(&literal.value()));
                } else if meta.input.peek(syn::Token![=]) {
                    // Consume the value of an option we do not model
                    // so the remaining options in the same attribute
                    // still get seen.
                    let value = meta.value()?;
                    value.parse::<syn::Expr>()?;
                }
                Ok(())
            });
        }
        out
    }
}

/// `"serialized::CustomMutationIn"` → `"CustomMutationIn"`.
fn last_path_segment(path: &str) -> String {
    path.rsplit("::").next().unwrap_or(path).trim().to_string()
}

/// Type names mentioned by fields that serde can populate. A field
/// marked `#[serde(skip)]` / `#[serde(skip_deserializing)]` never
/// receives on-disk data, so it does not extend the contract.
fn referenced_types(fields: &Fields) -> Vec<String> {
    let mut out = Vec::new();
    for field in fields {
        if is_skipped(&field.attrs) {
            continue;
        }
        collect_type_names(&field.ty, &mut out);
    }
    out
}

/// The `#[serde(skip_serializing_if = "...")]` predicate of every
/// field that carries one, as written (`"Option::is_none"`,
/// `"is_default_position"`).
///
/// Unlike [`referenced_types`] this looks at every field, skipped or
/// not: it answers a question about what the *saver* may leave out.
fn omit_predicates(fields: &Fields) -> Vec<String> {
    let mut out = Vec::new();
    for field in fields {
        for attr in &field.attrs {
            if !attr.path().is_ident("serde") {
                continue;
            }
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip_serializing_if") {
                    let literal: syn::LitStr = meta.value()?.parse()?;
                    out.push(literal.value());
                } else if meta.input.peek(syn::Token![=]) {
                    let value = meta.value()?;
                    value.parse::<syn::Expr>()?;
                }
                Ok(())
            });
        }
    }
    out
}

fn is_skipped(attrs: &[Attribute]) -> bool {
    let mut skipped = false;
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") || meta.path.is_ident("skip_deserializing") {
                skipped = true;
            } else if meta.input.peek(syn::Token![=]) {
                let value = meta.value()?;
                value.parse::<syn::Expr>()?;
            }
            Ok(())
        });
    }
    skipped
}

/// Every identifier appearing in `ty`, including generic arguments.
/// Module qualifiers come along harmlessly — a name only matters when
/// the index holds a type by that name.
fn collect_type_names(ty: &Type, out: &mut Vec<String>) {
    match ty {
        Type::Path(path) => {
            if let Some(qself) = &path.qself {
                collect_type_names(&qself.ty, out);
            }
            for segment in &path.path.segments {
                out.push(segment.ident.to_string());
                match &segment.arguments {
                    PathArguments::AngleBracketed(args) => {
                        for arg in &args.args {
                            if let GenericArgument::Type(inner) = arg {
                                collect_type_names(inner, out);
                            }
                        }
                    }
                    PathArguments::Parenthesized(args) => {
                        for inner in &args.inputs {
                            collect_type_names(inner, out);
                        }
                    }
                    PathArguments::None => {}
                }
            }
        }
        Type::Reference(inner) => collect_type_names(&inner.elem, out),
        Type::Slice(inner) => collect_type_names(&inner.elem, out),
        Type::Array(inner) => collect_type_names(&inner.elem, out),
        Type::Paren(inner) => collect_type_names(&inner.elem, out),
        Type::Group(inner) => collect_type_names(&inner.elem, out),
        Type::Ptr(inner) => collect_type_names(&inner.elem, out),
        Type::Tuple(tuple) => {
            for inner in &tuple.elems {
                collect_type_names(inner, out);
            }
        }
        _ => {}
    }
}

/// Absolute path to baumhard's own `src/` directory, resolved from
/// the crate's `CARGO_MANIFEST_DIR` so the walk finds the sources no
/// matter which crate's test invoked it.
pub fn crate_src_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name the walk from `MindMap` is allowed to stop at.
    ///
    /// Four kinds of entry, and the distinction is the whole point of
    /// the list:
    ///
    /// 1. **Primitives and containers** — `String`, `f32`, `Option`,
    ///    `Vec`, `HashMap`, `Value`. No fields of ours; stopping is
    ///    correct and always will be.
    /// 2. **Module path qualifiers.** `collect_type_names` pushes
    ///    every segment of a path, so
    ///    `crate::mindmap::animation::AnimationTiming` contributes
    ///    three names that are not types alongside the one that is.
    /// 3. **Foreign types, traits, and generic parameters** — `Vec2`
    ///    (glam), `OrderedFloat` (ordered-float), the `Discriminated`
    ///    trait named by a `<F as Discriminated>::Discriminant`
    ///    qself, and `F` itself. Their shape is not ours to annotate;
    ///    if one ever grows named JSON keys a map can write, that is
    ///    a review decision, not a silent one.
    /// 4. **Types this walk genuinely cannot see** — `AppFont`, which
    ///    `lib/baumhard/build.rs` emits into `$OUT_DIR`, and strum's
    ///    generated `Discriminant` tags. Both are unit enums today,
    ///    so they carry no JSON keys and cost nothing. They are on
    ///    the list because the module's promise — a new field of a
    ///    new type extends the covered set on its own — is *false*
    ///    for anything a build script emits, and a comment saying so
    ///    drifts where a checked list does not.
    const EXPECTED_TERMINATORS: &[&str] = &[
        // 1. primitives and containers
        "BTreeSet",
        "Box",
        "FxHashMap",
        "HashMap",
        "Option",
        "String",
        "Value",
        "Vec",
        "bool",
        "f32",
        "f64",
        "i16",
        "i32",
        "u32",
        "u8",
        "usize",
        // 2. module path qualifiers, not types
        "animation",
        "crate",
        "mindmap",
        "serde_json",
        // 3. foreign types, traits, and generic parameters
        "Discriminated",
        "F",
        "OrderedFloat",
        "Vec2",
        // 4. real types this walk cannot see
        "AppFont",
        "Discriminant",
    ];

    /// **The walker's blind spot, made visible.** The walk stops at
    /// any name it did not index, and it cannot tell "known-terminal
    /// primitive" from "type I failed to resolve" — `AppFont` lives
    /// in `$OUT_DIR` and is reachable from a real map, and nothing
    /// about the walk announces that.
    ///
    /// So the list is here rather than in a comment: every terminator
    /// must be one somebody looked at. When a field of a new
    /// unindexed type lands, this fails naming it, and the reader
    /// decides whether it belongs on the list or whether the walk has
    /// to learn to reach it. That is the same drift discipline
    /// `test_every_loadable_type_rejects_unknown_keys` applies to the
    /// types the walk *does* see.
    #[test]
    fn test_every_walk_terminator_is_an_expected_one() {
        let graph = TypeGraph::build(&crate_src_root());
        let unexpected: Vec<String> = graph
            .unresolved_from("MindMap")
            .into_iter()
            .filter(|name| !EXPECTED_TERMINATORS.contains(&name.as_str()))
            .collect();
        assert!(
            unexpected.is_empty(),
            "the reachability walk stops at {} name(s) nobody has vetted. Each is \
             either a primitive (add it to EXPECTED_TERMINATORS) or a type that is \
             part of the on-disk contract and is silently missing from the \
             deny_unknown_fields coverage — the case `AppFont` is on the list for:\n  {}",
            unexpected.len(),
            unexpected.join("\n  ")
        );
    }

    /// The allowlist itself must not rot: an entry for a name the
    /// walk no longer reaches is a claim about the model that stopped
    /// being true, and it would keep covering for a *different* name
    /// that happened to reappear later.
    #[test]
    fn test_no_expected_terminator_is_stale() {
        let graph = TypeGraph::build(&crate_src_root());
        let actual = graph.unresolved_from("MindMap");
        let stale: Vec<&&str> = EXPECTED_TERMINATORS
            .iter()
            .filter(|name| !actual.contains(**name))
            .collect();
        assert!(
            stale.is_empty(),
            "EXPECTED_TERMINATORS lists name(s) the walk from MindMap no longer \
             stops at — delete them: {stale:?}"
        );
    }

    /// The walk is only as good as its index: a name declared twice
    /// would make "which `Position`?" unanswerable, and the coverage
    /// assertion built on top would be checking an arbitrary one.
    #[test]
    fn test_type_graph_has_no_ambiguous_names() {
        let graph = TypeGraph::build(&crate_src_root());
        assert!(
            graph.duplicate_names().is_empty(),
            "type names declared more than once — the reachability walk \
             resolves by bare name and cannot tell them apart: {:?}",
            graph.duplicate_names()
        );
    }

    /// A spot check that the walk actually descends rather than
    /// returning its root: `MindMap` must reach a type three field
    /// hops away (`MindMap` → `MindNode` → `MindSection` →
    /// `TextRun`).
    #[test]
    fn test_reachable_from_follows_nested_fields() {
        let graph = TypeGraph::build(&crate_src_root());
        let reached: Vec<&str> = graph
            .reachable_from("MindMap")
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        for expected in ["MindMap", "MindNode", "MindSection", "TextRun"] {
            assert!(
                reached.contains(&expected),
                "{expected} must be reachable from MindMap"
            );
        }
    }

    /// `#[serde(from = "...")]` moves the on-disk shape onto the
    /// proxy, so the walk has to follow it — `CustomMutationIn` is
    /// named only in that attribute, never as a field type.
    #[test]
    fn test_reachable_from_follows_a_serde_proxy() {
        let graph = TypeGraph::build(&crate_src_root());
        let reached: Vec<&str> = graph
            .reachable_from("MindMap")
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        assert!(
            reached.contains(&"CustomMutationIn"),
            "the deserialize proxy behind CustomMutation must be reachable"
        );
    }

    /// The write graph is not the read graph. `CustomMutation`'s keys
    /// are written by `CustomMutationOut` via
    /// `#[serde(into = "...")]`, a type no deserializer ever sees —
    /// so `reachable_from` must not reach it while
    /// `omit_predicates_from` must. `String::is_empty` appears on
    /// exactly one field in the whole crate, `CustomMutationOut`'s,
    /// which makes it the witness.
    #[test]
    fn test_omit_predicates_follow_the_serialize_proxy() {
        let graph = TypeGraph::build(&crate_src_root());
        let read_side: Vec<&str> = graph
            .reachable_from("MindMap")
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        assert!(
            !read_side.contains(&"CustomMutationOut"),
            "a serialize-only proxy must stay out of the deny_unknown_fields coverage set"
        );
        assert!(
            graph.omit_predicates_from("MindMap").contains("String::is_empty"),
            "the write walk must reach CustomMutationOut and see its \
             skip_serializing_if predicates"
        );
    }

    /// Test-only declarations must stay out of the index, or a stub
    /// struct in a test module could shadow a real model type.
    #[test]
    fn test_type_graph_excludes_test_sources() {
        let graph = TypeGraph::build(&crate_src_root());
        assert!(
            graph.get("StubCtx").is_none(),
            "a struct declared inside a test module must not be indexed"
        );
    }
}
