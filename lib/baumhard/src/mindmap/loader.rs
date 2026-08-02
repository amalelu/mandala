// SPDX-License-Identifier: MPL-2.0

//! `.mindmap.json` loader + saver.
//!
//! **The loader never drops what it does not understand.** A key the
//! model has no field for is a load error, not a shrug: the editor
//! resaves the map it loaded, so a key ignored at load is a key
//! deleted at save, and the file the author hand-wrote was the only
//! copy. Every type reachable from a load therefore carries
//! `#[serde(deny_unknown_fields)]` — enforced against the model that
//! exists, not a list of it, by
//! `tests::test_every_loadable_type_rejects_unknown_keys`.
//!
//! That posture is what the pre-refactor rejections were always an
//! instance of: a top-level `portals[]` array or per-node `text` /
//! `text_runs` is a shape serde would otherwise ignore, and those get
//! a concrete `maptool convert ...` pointer instead of the generic
//! message.
//!
//! Everything expensive lives on the failure path. A successful load
//! parses the document exactly once; the raw JSON is re-examined only
//! to explain a parse that already failed.

use crate::mindmap::custom_mutation::CustomMutation;
use crate::mindmap::model::{validate, Canvas, MindEdge, MindMap, MindNode, Palette};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Load a `MindMap` from a file path. Reads the entire file into
/// memory via `std::fs::read_to_string`, then delegates to
/// [`load_from_str`]. Native-only (synchronous I/O). Returns a
/// `String` error describing the path + underlying cause.
///
/// Cost: one filesystem read (latency-bound, plus an allocation
/// sized to the file's UTF-8 length) followed by [`load_from_str`]'s
/// JSON parse — O(file_size) overall. Felt every map load.
pub fn load_from_file(path: &Path) -> Result<MindMap, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read file {}: {}", path.display(), e))?;
    load_from_str(&content)
}

/// Parse a `MindMap` from a JSON string.
///
/// A key the model does not know is rejected rather than dropped —
/// see the module header for why that is the only policy that keeps
/// a hand-authored file intact across a load / save cycle. Legacy
/// shapes (a top-level `portals[]`, per-node `text` / `text_runs`)
/// get a concrete `maptool convert ...` pointer instead of serde's
/// generic complaint.
///
/// Cost on the happy path: **exactly one parse of `json`**, and
/// nothing that walks the text again. The raw string is not even in
/// scope below the parse — every remaining invariant is expressed
/// against the typed map, which is what keeps that property true as
/// the invariant list grows.
///
/// Cost on the failure path: roughly **2× the happy path**, and
/// worth naming because it is more than the extra `Value` parse it
/// looks like. `diagnose_rejected_json` pays that parse *plus*
/// `locate_typed_failure`, which re-deserializes the canvas, then
/// every palette, then every node, then every edge, then every
/// custom mutation, stopping at the first that fails. When the
/// offending key is in the last node, that second stage has typed the
/// whole document a second time. The trade is deliberate: the load is
/// not going to complete, and a node id beats a byte offset into a
/// 545 KB file.
pub fn load_from_str(json: &str) -> Result<MindMap, String> {
    let map = match serde_json::from_str::<MindMap>(json) {
        Ok(map) => map,
        Err(e) => return Err(diagnose_rejected_json(json, &e)),
    };
    // `json` deliberately ends here. `check_invariants` is not given
    // it, so no future invariant can quietly reintroduce a second
    // pass over the document text.
    check_invariants(map)
}

/// Post-parse invariants that live in the typed model rather than in
/// the JSON: they are checked against `MindMap` values, never against
/// the source text.
///
/// Cost: O(nodes + edges) — one sorted pass for the zero-section and
/// section-cap checks, one memoized parent walk, one edge-tuple scan.
fn check_invariants(map: MindMap) -> Result<MindMap, String> {
    if let Some(err) = detect_zero_section_node(&map) {
        return Err(err);
    }
    if let Some(err) = detect_parent_cycle(&map) {
        return Err(err);
    }
    if let Some(err) = detect_section_count_cap(&map) {
        return Err(err);
    }
    warn_on_duplicate_edges(&map);
    Ok(map)
}

/// Reject a map where any node ships zero sections. `sections` is
/// `#[serde(default)]` so serde accepts the omission — it has to, or
/// a node without the key would fail with a confusing "missing
/// field" — which makes this the loader's invariant to hold. Nodes
/// are visited in sorted-id order so the reported node is
/// deterministic across `HashMap` iteration order.
fn detect_zero_section_node(map: &MindMap) -> Option<String> {
    let mut ids: Vec<&String> = map.nodes.keys().collect();
    ids.sort();
    let id = ids.into_iter().find(|id| map.nodes[*id].sections.is_empty())?;
    Some(format!(
        "node {:?} ships zero sections — every renderable node \
         needs at least one. Run `maptool convert --sections <file>` \
         to migrate, or add an explicit `sections` array.",
        id
    ))
}

/// Turn a failed typed parse into a message that names *what* the
/// loader choked on and *where*.
///
/// serde reports the first thing it could not accept, as a byte
/// offset into a file that can be thousands of lines long. That is
/// enough to fix a typo and not much else. So the failure path parses
/// the document once more as a `serde_json::Value` and asks three
/// questions in order:
///
/// 1. is this a pre-refactor shape we have a migration verb for?
/// 2. which *part* of the document fails on its own — the canvas, one
///    named palette, one node, one edge, one custom mutation? Each
///    part is re-deserialized against its own typed shape, so the
///    answer carries the node id or the edge index that the raw serde
///    error does not.
/// 3. neither — the failure is at the top level, where serde's own
///    message already names the key.
///
/// Cost: the `Value` parse (O(file_size), one allocation per JSON
/// node) plus, in the worst case, a full typed re-deserialization of
/// every addressable part — the `Value` is borrowed, not cloned, but
/// step 2 stops only at the first part that fails, so a failure in
/// the last node types the whole document again. Call it 2× a
/// successful load. Only reached when the load has already failed, so
/// it costs nothing anybody gets to keep.
fn diagnose_rejected_json(json: &str, error: &serde_json::Error) -> String {
    let Ok(raw) = serde_json::from_str::<Value>(json) else {
        // Not valid JSON at all. serde's message carries the line and
        // column, which is the entire diagnosis for a syntax error.
        return format!("Failed to parse mindmap JSON: {error}");
    };
    if let Some(message) = detect_legacy_shape(&raw) {
        return message;
    }
    if let Some(message) = locate_typed_failure(&raw) {
        return message;
    }
    format!("Failed to parse mindmap JSON: {}", explain(error.to_string()))
}

/// Legacy field shapes that predate the current format, each with the
/// `maptool convert` verb that migrates it. These win over the
/// generic per-part diagnosis because "unknown field `text`" is a
/// true but useless thing to tell someone holding a pre-section map.
fn detect_legacy_shape(raw: &Value) -> Option<String> {
    // Pre-refactor maps stored portals in a separate `portals[]`
    // array. Post-refactor portals are edges with
    // `display_mode = "portal"`. The key's presence is the signal —
    // an empty array still has to come out of the file, and
    // `convert --portals` is what removes it.
    if raw.get("portals").is_some() {
        return Some(
            "legacy `portals` field present; run `maptool convert --portals <file>` \
             to migrate to portal-mode edges"
                .to_string(),
        );
    }
    // Pre-section-refactor maps put `text` and `text_runs` directly
    // on each node. Post-refactor those live on
    // `MindNode.sections[].{text, text_runs}`.
    let nodes = raw.get("nodes").and_then(|n| n.as_object())?;
    if let Some((id, _)) = nodes
        .iter()
        .find(|(_, v)| v.get("text").is_some() || v.get("text_runs").is_some())
    {
        return Some(format!(
            "legacy `text` / `text_runs` on node {:?}; run \
             `maptool convert --sections <file>` to migrate node \
             text into `sections[]`",
            id
        ));
    }
    if let Some((id, _)) = nodes
        .iter()
        .find(|(_, v)| v.get("sections").map(|s| !s.is_array()).unwrap_or(false))
    {
        return Some(format!(
            "node {:?} has `sections` but it is not an array — \
             see format/sections.md",
            id
        ));
    }
    None
}

/// Re-deserialize each addressable part of the document against its
/// own typed shape and report the first that fails, prefixed with the
/// part's location.
///
/// Borrows the `Value` rather than cloning it: `&Value` is itself a
/// `Deserializer`, so a part is checked in place.
fn locate_typed_failure(raw: &Value) -> Option<String> {
    if let Some(value) = raw.get("canvas") {
        if let Some(message) = part_failure::<Canvas>("canvas", value) {
            return Some(message);
        }
    }
    if let Some(palettes) = raw.get("palettes").and_then(Value::as_object) {
        for (name, value) in palettes {
            if let Some(message) = part_failure::<Palette>(&format!("palette {name:?}"), value) {
                return Some(message);
            }
        }
    }
    if let Some(nodes) = raw.get("nodes").and_then(Value::as_object) {
        for (id, value) in nodes {
            if let Some(message) = part_failure::<MindNode>(&format!("node {id:?}"), value) {
                return Some(message);
            }
        }
    }
    if let Some(edges) = raw.get("edges").and_then(Value::as_array) {
        for (i, value) in edges.iter().enumerate() {
            if let Some(message) = part_failure::<MindEdge>(&format!("edge[{i}]"), value) {
                return Some(message);
            }
        }
    }
    if let Some(mutations) = raw.get("custom_mutations").and_then(Value::as_array) {
        for (i, value) in mutations.iter().enumerate() {
            let label = format!("custom_mutations[{i}]");
            if let Some(message) = part_failure::<CustomMutation>(&label, value) {
                return Some(message);
            }
        }
    }
    None
}

/// `Some(message)` when `value` does not deserialize as `T`, naming
/// `label` so the reader knows which part of the map it is.
fn part_failure<'de, T: Deserialize<'de>>(label: &str, value: &'de Value) -> Option<String> {
    let error = T::deserialize(value).err()?;
    Some(format!("{label}: {}", explain(error.to_string())))
}

/// Spell out the unknown-key policy on the one message where a reader
/// would otherwise assume the friendlier behavior.
///
/// serde words a `deny_unknown_fields` rejection as ``unknown field
/// `x`, expected one of ...``, which reads like pedantry until you
/// know what the alternative was: the key silently gone from the file
/// after the next save. Matching on serde's wording is a message
/// nicety, not a control-flow decision — a reworded serde would cost
/// the extra sentence and nothing else, and
/// `test_unknown_node_key_is_rejected_with_the_policy` fails loudly
/// if that day comes.
fn explain(message: String) -> String {
    if message.starts_with("unknown field") {
        format!(
            "{message} — unknown keys are rejected, not dropped: a load that ignored \
             this key would erase it from the file on the next save. \
             See format/schema.md."
        )
    } else {
        message
    }
}

/// Reject a `MindMap` whose `parent_id` links form a cycle. A cycle
/// is worse than the legacy-shape rejections above: every model
/// walker that follows `parent_id` (`is_hidden_by_fold`,
/// `all_descendants`, `is_ancestor_or_self`) assumes the chain
/// terminates at a root, and a hand-edited or hostile file is under
/// no such obligation. Left unchecked, the first scene build after
/// load walks straight into an infinite loop or an uncatchable stack
/// overflow (see `format/macros.md`'s "opening any `.mindmap.json`
/// from an untrusted source IS a privilege event"). The three
/// walkers also carry their own iteration caps as defense in depth,
/// but rejecting the cycle here means the map never loads at all
/// instead of silently degrading.
///
/// Cost: O(n) overall, not O(n) per node — nodes proven acyclic are
/// memoized so a later walk that reaches one stops immediately
/// rather than re-walking the same suffix. Nodes are visited in
/// sorted-id order so the reported node is deterministic across
/// `HashMap` iteration order. A dangling `parent_id` (referencing a
/// node absent from the map — a separate, pre-existing invariant
/// this function doesn't police) is treated like a root.
fn detect_parent_cycle(map: &MindMap) -> Option<String> {
    let mut resolved: HashSet<&str> = HashSet::new();
    let mut ids: Vec<&String> = map.nodes.keys().collect();
    ids.sort();

    for start_id in ids {
        if resolved.contains(start_id.as_str()) {
            continue;
        }
        let mut chain: Vec<&str> = Vec::new();
        let mut on_chain: HashSet<&str> = HashSet::new();
        let mut current_id: &str = start_id.as_str();
        loop {
            if resolved.contains(current_id) {
                for id in &chain {
                    resolved.insert(id);
                }
                break;
            }
            if on_chain.contains(current_id) {
                let cycle_start = chain.iter().position(|&id| id == current_id).unwrap();
                let path = chain[cycle_start..]
                    .iter()
                    .chain(std::iter::once(&current_id))
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" → ");
                return Some(format!(
                    "node {:?}: parent chain contains a cycle ({}); fix parent_id",
                    chain[cycle_start], path
                ));
            }
            chain.push(current_id);
            on_chain.insert(current_id);
            match map.nodes.get(current_id).and_then(|n| n.parent_id.as_deref()) {
                Some(parent_id) => current_id = parent_id,
                None => {
                    for id in &chain {
                        resolved.insert(id);
                    }
                    break;
                }
            }
        }
    }
    None
}

/// Warn at load time when the same `(from_id, to_id, edge_type)`
/// tuple appears more than once. Duplicates render deterministically
/// as the last edge in the array, but every `EdgeRef` lookup and
/// scene-cache key is ambiguous, so hand-edited maps should degrade
/// loudly rather than silently overwrite geometry.
fn warn_on_duplicate_edges(map: &MindMap) {
    use std::collections::HashMap;
    let mut seen: HashMap<(&str, &str, &str), usize> = HashMap::new();
    for (i, edge) in map.edges.iter().enumerate() {
        let key = (
            edge.from_id.as_str(),
            edge.to_id.as_str(),
            edge.edge_type.as_str(),
        );
        if let Some(&first) = seen.get(&key) {
            log::warn!(
                "duplicate edge tuple (from_id={:?}, to_id={:?}, edge_type={:?}) \
                 at edges[{}] and edges[{}]; EdgeRef / scene-cache lookups are unstable",
                edge.from_id,
                edge.to_id,
                edge.edge_type,
                first,
                i
            );
        } else {
            seen.insert(key, i);
        }
    }
}

/// Reject maps whose typed `sections` vectors exceed the shared
/// per-node cap. The JSON has already been parsed by this point,
/// so this is not a parser-level allocation limit; it is the
/// loader's honest model-entry invariant, matching the document
/// mutator cap and `maptool verify`.
fn detect_section_count_cap(map: &MindMap) -> Option<String> {
    let mut nodes: Vec<&crate::mindmap::model::MindNode> = map.nodes.values().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    for node in nodes {
        if let Err(message) = validate::section_count(node) {
            return Some(format!("node {:?}: {}", node.id, message));
        }
    }
    None
}

/// Serialize a `MindMap` to pretty-printed JSON and write it to disk
/// atomically and deterministically.
///
/// **Determinism**: routes through `serde_json::Value` (which uses
/// `BTreeMap` for object keys) so two saves of the same `MindMap` produce
/// byte-identical output regardless of `HashMap` iteration order. Costs
/// one extra heap copy of the JSON tree; acceptable for the editor's
/// save cadence (post-mutation, not per-frame).
///
/// **Atomicity**: writes to a sibling `.<name>.<pid>.tmp` file then
/// renames over `path`. A reader (another process, or the editor
/// reloading after an external edit) never observes a torn-write
/// half-written file. The temp file is removed on rename failure.
///
/// Native-only (synchronous I/O via `std::fs`). Returns a `String`
/// error describing the path + underlying cause.
pub fn save_to_file(path: &Path, map: &MindMap) -> Result<(), String> {
    let value = serde_json::to_value(map).map_err(|e| format!("failed to serialize map: {e}"))?;
    let json = serde_json::to_string_pretty(&value).map_err(|e| format!("failed to render map JSON: {e}"))?;
    write_atomic(path, &json)
}

/// Write `contents` to `path` via `<dir>/.<name>.<pid>.tmp` + rename.
/// Cleans up the temp file on rename failure so a partially-written
/// staging file is never left behind. Used by [`save_to_file`] for the
/// typed-`MindMap` save path; also exposed for legacy-migration tools
/// — every `maptool convert` verb routes through it — that ship raw
/// `serde_json::Value` to disk without a `MindMap` round-trip.
///
/// The existing file at `path` is never opened for writing, which is
/// what makes an in-place migration (input path == output path) safe:
/// the old bytes survive untouched until the rename swaps the
/// finished file in.
///
/// **The saved file is a new inode.** That is the mechanism, not an
/// incidental detail, and it has consequences a caller must know:
/// hard links to the old file keep the old content, and a symlink at
/// `path` is replaced by a regular file rather than followed. What
/// does *not* change is the mode — when `path` already exists its
/// permissions are carried onto the staging file, so a map the user
/// deliberately `chmod 600`'d does not come back world-readable at
/// the process umask. A new file takes the umask default, as it
/// would from any other writer.
///
/// The mode is applied **at creation, before any content lands** —
/// see `write_staging_file` (private; `cargo doc
/// --document-private-items` renders it). Nothing ever exists on disk
/// holding the caller's bytes at a wider mode than the target had.
///
/// Cost: one `stat` of the target, one create + one write of
/// `contents`, and one rename. Reading the target's mode is
/// best-effort: a target whose metadata cannot be read falls through
/// to umask defaults rather than failing a save the user asked for.
pub fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("invalid path: {}", path.display()))?
        .to_string_lossy();
    let tmp_path = dir.join(format!(".{}.{}.tmp", file_name, std::process::id()));

    // The mode of the file being replaced, if there is one to
    // inherit. Read before the staging file is created, because
    // that is when it has to be applied.
    let inherited = fs::metadata(path).ok().map(|meta| meta.permissions());
    write_staging_file(&tmp_path, contents, inherited)
        .map_err(|e| format!("failed to write {}: {e}", tmp_path.display()))?;

    fs::rename(&tmp_path, path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "failed to rename {} -> {}: {e}",
            tmp_path.display(),
            path.display()
        )
    })
}

/// Create `tmp_path` already carrying `inherited`'s permissions, then
/// write `contents` into it.
///
/// The **order is the whole point**. Creating at the process umask
/// and chmod-ing afterwards would leave a complete copy of the map
/// sitting at the wider mode for the entire duration of the write —
/// precisely the exposure the inheritance exists to prevent, and a
/// window a reader only has to be unlucky to hit. So the mode goes on
/// at `open(2)` time, when the file is still empty.
///
/// `OpenOptions::mode` applies only when the file is *created*, so a
/// stale staging file — left by a crashed run that happened to hold
/// this pid — is removed first. Without that it would keep its own
/// (possibly wide) mode straight through the truncate and inherit
/// nothing.
///
/// Permissions are advisory to the save, never fatal to it: on
/// platforms without a create-time mode the post-creation fallback
/// logs and continues rather than losing content the caller asked to
/// persist.
fn write_staging_file(
    tmp_path: &Path,
    contents: &str,
    inherited: Option<fs::Permissions>,
) -> std::io::Result<()> {
    use std::io::Write;

    let _ = fs::remove_file(tmp_path);
    let mut file = create_with_mode(tmp_path, inherited)?;
    file.write_all(contents.as_bytes())
}

/// Create an empty file at `tmp_path` with `inherited`'s mode applied
/// from the moment it exists.
#[cfg(unix)]
fn create_with_mode(tmp_path: &Path, inherited: Option<fs::Permissions>) -> std::io::Result<fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    if let Some(permissions) = inherited {
        options.mode(permissions.mode());
    }
    options.open(tmp_path)
}

/// Non-Unix fallback: no create-time mode exists, so the only
/// carryable bit (the read-only flag) is applied immediately after
/// creation, while the file is still empty. That flag is not a
/// confidentiality control, so the ordering carries no exposure here
/// the way it does on Unix.
#[cfg(not(unix))]
fn create_with_mode(tmp_path: &Path, inherited: Option<fs::Permissions>) -> std::io::Result<fs::File> {
    let file = fs::File::create(tmp_path)?;
    if let Some(permissions) = inherited {
        if let Err(e) = file.set_permissions(permissions) {
            log::warn!("could not carry permissions onto {}: {e}", tmp_path.display());
        }
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mindmap::test_helpers::testament_map_path as test_map_path;
    use crate::util::test_temp::TempDir;
    use std::path::PathBuf;

    /// `format/README.md` §"Minimum-viable example" claims to print
    /// "a complete, valid mindmap with a single root node". Nothing
    /// checked that claim, and the loader is strict enough —
    /// required keys, zero-section rejection, the legacy-shape
    /// screens — that a stale example would quietly become a broken
    /// starting point for anyone hand-authoring a map.
    ///
    /// The example is **read out of the spec**, not restated here, so
    /// the pin follows the doc when the doc moves rather than
    /// agreeing with a copy of its old self.
    #[test]
    fn test_documented_minimum_viable_example_loads() {
        let doc = crate::util::doc_fixtures::format_doc_path("README.md");
        let published =
            crate::util::doc_fixtures::documented_json_block(&doc, "## Minimum-viable example", 0);

        let map = load_from_str(&published).unwrap_or_else(|e| {
            panic!("format/README.md's minimum-viable example must load: {e}\n{published}")
        });
        assert_eq!(map.name, "hello");
        assert_eq!(map.nodes.len(), 1);
        assert_eq!(map.nodes["0"].sections[0].text, "Hello");
        assert!(map.edges.is_empty());
    }

    /// **The unknown-key policy, enforced against the model that
    /// exists rather than a list of the model as it once was.**
    ///
    /// A `.mindmap.json` key the model does not know must be a load
    /// error. Without `deny_unknown_fields` serde drops it in
    /// silence, the app resaves what it loaded, and a hand-authored
    /// field is gone — the file was the only copy.
    ///
    /// The set of types this applies to is not written down here.
    /// [`crate::util::serde_coverage`] parses baumhard's own sources
    /// and walks outward from `MindMap` through every deserializable
    /// field, so a new field of a new type extends the covered set
    /// on its own and this test fails until that type opts in. Two
    /// shapes are exempt, and only two: a container that delegates
    /// its on-disk shape to a proxy via `#[serde(from = "...")]`
    /// (the requirement follows the proxy, which the walk also
    /// reaches), and an `#[serde(untagged)]` enum, where denying
    /// unknown fields changes which variant matches rather than
    /// merely tightening a check.
    #[test]
    fn test_every_loadable_type_rejects_unknown_keys() {
        use crate::util::serde_coverage::{crate_src_root, TypeGraph, TypeKind};

        let graph = TypeGraph::build(&crate_src_root());
        let mut missing: Vec<String> = Vec::new();
        for info in graph.reachable_from("MindMap") {
            let exempt = info.deserialize_proxy.is_some() || info.untagged;
            if !info.derives_deserialize || !info.has_named_fields || exempt {
                continue;
            }
            if !info.denies_unknown_fields {
                // Naming the item kind matters for an enum: the
                // attribute goes on the enum, not on the struct
                // variant whose keys serde actually rejected.
                let kind = match info.kind {
                    TypeKind::Struct => "struct",
                    TypeKind::Enum => "enum",
                    TypeKind::Alias => "type",
                };
                missing.push(format!("{kind} {} — {}", info.name, info.file.display()));
            }
        }
        assert!(
            missing.is_empty(),
            "these types are reachable from a `.mindmap.json` load and would \
             silently drop an unknown key (add `#[serde(deny_unknown_fields)]`):\n  {}",
            missing.join("\n  ")
        );
    }

    #[test]
    fn test_load_testament_map() {
        let path = test_map_path();
        let map = load_from_file(&path).expect("Failed to load testament map");

        assert_eq!(map.version, "1.0");
        assert_eq!(map.name, "testament");
        assert_eq!(map.canvas.background_color, "#000000");
        assert_eq!(map.nodes.len(), 252);
        assert_eq!(map.edges.len(), 258);
    }

    #[test]
    fn test_root_nodes() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let roots = map.root_nodes();
        assert!(!roots.is_empty());
        for root in &roots {
            assert!(root.parent_id.is_none());
        }
        // Verify sorted by index
        for w in roots.windows(2) {
            assert!(
                crate::mindmap::model::id_sort_key(&w[0].id) <= crate::mindmap::model::id_sort_key(&w[1].id)
            );
        }
    }

    #[test]
    fn test_children_of() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Lord God node
        let children = map.children_of("0");
        assert!(!children.is_empty());
        for child in &children {
            assert_eq!(child.parent_id.as_deref(), Some("0"));
        }
        // Verify sorted by index
        for w in children.windows(2) {
            assert!(
                crate::mindmap::model::id_sort_key(&w[0].id) <= crate::mindmap::model::id_sort_key(&w[1].id)
            );
        }
    }

    /// Pre-section-refactor maps carry `text` / `text_runs` directly
    /// on each node; the loader rejects those with a concrete
    /// migration pointer (per CODE_CONVENTIONS §10 "no dual shapes")
    /// instead of silently dropping the unknown fields. Mirrors the
    /// portal-legacy rejection at the top of `load_from_str`.
    #[test]
    fn test_legacy_text_field_rejected_with_migration_pointer() {
        let raw = r##"{
            "version": "1.0",
            "name": "legacy",
            "canvas": {"background_color": "#000", "default_border": null,
                       "default_connection": null, "theme_variables": {},
                       "theme_variants": {}},
            "nodes": {"0": {
                "id": "0", "parent_id": null,
                "position": {"x": 0, "y": 0},
                "size": {"width": 100, "height": 50},
                "text": "I'm legacy",
                "text_runs": [],
                "style": {"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false},
                "layout": {"type":"map","direction":"auto","spacing":0},
                "folded": false, "notes": "",
                "color_schema": null
            }},
            "edges": []
        }"##;
        let err = load_from_str(raw).expect_err("legacy text field must be rejected");
        assert!(
            err.contains("legacy") && err.contains("maptool convert --sections"),
            "error must point at the migration tool: {err}"
        );
    }

    /// A valid post-section node parses through the typed loader.
    /// Pairs with `test_legacy_text_field_rejected_with_migration_pointer`
    /// — the rejection only fires for maps that ship the legacy
    /// shape, not for fresh ones.
    #[test]
    fn test_post_section_node_parses() {
        let raw = r##"{
            "version": "1.0",
            "name": "fresh",
            "canvas": {"background_color": "#000", "default_border": null,
                       "default_connection": null, "theme_variables": {},
                       "theme_variants": {}},
            "nodes": {"0": {
                "id": "0", "parent_id": null,
                "position": {"x": 0, "y": 0},
                "size": {"width": 100, "height": 50},
                "sections": [{"text": "ok"}],
                "style": {"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false},
                "layout": {"type":"map","direction":"auto","spacing":0},
                "folded": false, "notes": "",
                "color_schema": null
            }},
            "edges": []
        }"##;
        let map = load_from_str(raw).expect("post-section node parses");
        assert_eq!(map.nodes.len(), 1);
        let node = map.nodes.get("0").unwrap();
        assert_eq!(node.sections.len(), 1);
        assert_eq!(node.sections[0].text, "ok");
    }

    /// A node with `sections: []` is rejected — every renderable
    /// node needs at least one section, and the loader catches this
    /// at parse time so the tree builder's recursion never sees a
    /// zero-section node.
    #[test]
    fn test_zero_sections_rejected() {
        let raw = r##"{
            "version": "1.0",
            "name": "empty",
            "canvas": {"background_color": "#000", "default_border": null,
                       "default_connection": null, "theme_variables": {},
                       "theme_variants": {}},
            "nodes": {"0": {
                "id": "0", "parent_id": null,
                "position": {"x": 0, "y": 0},
                "size": {"width": 100, "height": 50},
                "sections": [],
                "style": {"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false},
                "layout": {"type":"map","direction":"auto","spacing":0},
                "folded": false, "notes": "",
                "color_schema": null
            }},
            "edges": []
        }"##;
        let err = load_from_str(raw).expect_err("empty sections must be rejected");
        assert!(
            err.contains("zero sections"),
            "error must explain the invariant: {err}"
        );
    }

    #[test]
    fn test_section_count_cap_rejected() {
        let sections = (0..=crate::mindmap::model::MAX_SECTIONS_PER_NODE)
            .map(|i| format!(r#"{{"text":"section {i}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let raw = format!(
            r##"{{
            "version": "1.0",
            "name": "too-many-sections",
            "canvas": {{"background_color": "#000", "default_border": null,
                       "default_connection": null, "theme_variables": {{}},
                       "theme_variants": {{}}}},
            "nodes": {{"0": {{
                "id": "0", "parent_id": null,
                "position": {{"x": 0, "y": 0}},
                "size": {{"width": 100, "height": 50}},
                "sections": [{sections}],
                "style": {{"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false}},
                "layout": {{"type":"map","direction":"auto","spacing":0}},
                "folded": false, "notes": "",
                "color_schema": null
            }}}},
            "edges": []
        }}"##
        );
        let err = load_from_str(&raw).expect_err("over-cap sections must be rejected");
        assert!(
            err.contains("node.sections.len()=1025 exceeds cap 1024"),
            "error must use the shared cap message: {err}"
        );
    }

    #[test]
    fn test_text_runs() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let node = map.nodes.get("0").unwrap();
        assert_eq!(node.sections.len(), 1, "post-migration: one section per node");
        let section = &node.sections[0];
        assert_eq!(section.text, "Lord God");
        assert_eq!(section.text_runs.len(), 1);
        let run = &section.text_runs[0];
        assert_eq!(run.start, 0);
        assert_eq!(run.end, 8);
        assert!(run.bold);
        assert!(run.underline);
        assert_eq!(run.font, "LiberationSans");
        assert_eq!(run.size_pt, 74);
        assert_eq!(run.color, "#ffffff");
    }

    #[test]
    fn test_color_schema() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let root_node = map.nodes.get("0").unwrap();
        let schema = root_node.color_schema.as_ref().unwrap();
        assert_eq!(schema.level, 0);
        assert!(schema.palette.starts_with("coral"));
        let palette = map.palettes.get(&schema.palette).unwrap();
        assert!(!palette.groups.is_empty());
        assert_eq!(palette.groups[0].frame, "#30b082");
    }

    #[test]
    fn test_edges() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let edge = &map.edges[0];
        assert_eq!(edge.from_id, "0");
        assert_eq!(edge.to_id, "0.0");
        assert_eq!(edge.edge_type, "parent_child");
        assert!(edge.visible);

        // Find an edge with control points
        let curved = map.edges.iter().find(|e| !e.control_points.is_empty());
        assert!(curved.is_some());
    }

    #[test]
    fn test_resolve_theme_colors() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Root schema node should resolve to level 0 group
        let root_node = map.nodes.get("0").unwrap();
        let colors = map.resolve_theme_colors(root_node).unwrap();
        assert_eq!(colors.frame, "#30b082");
    }

    #[test]
    fn test_testament_edges_produce_paths() {
        use crate::mindmap::connection;

        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let mut straight_count = 0;
        let mut bezier_count = 0;
        for edge in &map.edges {
            let from_node = map.nodes.get(&edge.from_id).expect("Missing from_node");
            let to_node = map.nodes.get(&edge.to_id).expect("Missing to_node");

            let from_pos = from_node.pos_vec2();
            let from_size = from_node.size_vec2();
            let to_pos = to_node.pos_vec2();
            let to_size = to_node.size_vec2();

            let conn_path = connection::build_connection_path(
                from_pos,
                from_size,
                &edge.anchor_from,
                to_pos,
                to_size,
                &edge.anchor_to,
                &edge.control_points,
            );
            match conn_path {
                connection::ConnectionPath::Straight { .. } => straight_count += 1,
                connection::ConnectionPath::CubicBezier { .. } => bezier_count += 1,
            }

            // Verify sampling produces non-empty result
            let samples = connection::sample_path(&conn_path, 7.2);
            assert!(
                !samples.is_empty(),
                "Edge {}→{} produced no samples",
                edge.from_id,
                edge.to_id
            );
        }
        assert_eq!(straight_count + bezier_count, 258);
        assert!(straight_count > 200, "Expected most edges to be straight");
        assert!(bezier_count > 0, "Expected some Bezier edges");
    }

    #[test]
    fn test_testament_scene_has_connections() {
        use crate::mindmap::scene_cache::SceneConnectionCache;
        use crate::mindmap::tree_builder::{build_connection_elements, node_clip_aabbs};

        let path = test_map_path();
        let map = load_from_file(&path).unwrap();
        let hidden = map.fold_hidden_set();
        let offsets = std::collections::HashMap::new();
        let aabbs = node_clip_aabbs(&map, &offsets, None, &hidden);
        let mut cache = SceneConnectionCache::new();
        let (connection_elements, _handles) =
            build_connection_elements(&map, &offsets, &aabbs, None, None, &mut cache, 1.0, &hidden);

        // All visible edges should produce connection elements
        let visible_edges = map.edges.iter().filter(|e| e.visible).count();
        assert_eq!(
            connection_elements.len(),
            visible_edges,
            "Expected {} connection elements, got {}",
            visible_edges,
            connection_elements.len()
        );

        // Each connection element should have glyph positions
        for elem in &connection_elements {
            assert!(
                !elem.glyph_positions.is_empty(),
                "Connection has no glyph positions"
            );
            assert!(!elem.body_glyph.is_empty(), "Connection has no body glyph");
            assert!(!elem.color.is_empty(), "Connection has no color");
        }
    }

    #[test]
    fn test_backward_compat_no_custom_mutations() {
        // Existing maps without custom_mutations/trigger_bindings/inline_mutations
        // should load with empty defaults
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        assert!(
            map.custom_mutations.is_empty(),
            "Existing map should have no custom_mutations"
        );

        let node = map.nodes.get("0").unwrap();
        assert!(
            node.trigger_bindings.is_empty(),
            "Existing node should have no trigger_bindings"
        );
        assert!(
            node.inline_mutations.is_empty(),
            "Existing node should have no inline_mutations"
        );
    }

    #[test]
    fn test_backward_compat_no_theme_variables() {
        // Existing maps without theme_variables/theme_variants should load
        // with empty defaults (the new fields must be opt-in via serde default).
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();
        assert!(map.canvas.theme_variables.is_empty());
        assert!(map.canvas.theme_variants.is_empty());
    }

    /// Existing fixtures pre-date the zoom-visibility feature, so
    /// every model field on every node / edge / label / portal
    /// endpoint must round-trip as `None`. Pins the
    /// `skip_serializing_if` contract so the on-disk form of
    /// unchanged maps stays byte-stable against the JSON keys
    /// — a regression in a serde attribute would surface here as
    /// a `Some(…)` drift on load, and (via the follow-up
    /// serialize roundtrip) as a newly-emitted key in the file.
    #[test]
    fn test_existing_fixtures_have_no_authored_zoom_windows() {
        let path = test_map_path();
        let map = load_from_file(&path).expect("testament loads");
        for node in map.nodes.values() {
            assert!(
                node.min_zoom_to_render.is_none(),
                "testament node {} has an unexpected min_zoom_to_render",
                node.id
            );
            assert!(
                node.max_zoom_to_render.is_none(),
                "testament node {} has an unexpected max_zoom_to_render",
                node.id
            );
        }
        for (i, edge) in map.edges.iter().enumerate() {
            assert!(
                edge.min_zoom_to_render.is_none(),
                "testament edge[{i}] has an unexpected min_zoom_to_render"
            );
            assert!(
                edge.max_zoom_to_render.is_none(),
                "testament edge[{i}] has an unexpected max_zoom_to_render"
            );
            if let Some(cfg) = edge.label_config.as_ref() {
                assert!(cfg.min_zoom_to_render.is_none());
                assert!(cfg.max_zoom_to_render.is_none());
            }
            if let Some(pf) = edge.portal_from.as_ref() {
                assert!(pf.min_zoom_to_render.is_none());
                assert!(pf.max_zoom_to_render.is_none());
                assert!(pf.perpendicular_offset.is_none());
            }
            if let Some(pt) = edge.portal_to.as_ref() {
                assert!(pt.min_zoom_to_render.is_none());
                assert!(pt.max_zoom_to_render.is_none());
                assert!(pt.perpendicular_offset.is_none());
            }
        }

        // Serialize back and confirm the raw JSON never mentions
        // the new keys — the `skip_serializing_if = "Option::is_none"`
        // attributes must suppress every default field.
        let serialized = serde_json::to_string(&map).expect("serializes");
        assert!(
            !serialized.contains("min_zoom_to_render"),
            "testament roundtrip emitted an unexpected min_zoom_to_render key"
        );
        assert!(
            !serialized.contains("max_zoom_to_render"),
            "testament roundtrip emitted an unexpected max_zoom_to_render key"
        );
        assert!(
            !serialized.contains("perpendicular_offset"),
            "testament roundtrip emitted an unexpected perpendicular_offset key"
        );
    }

    /// Roundtrip the same fixture through a second parse and
    /// confirm the structural shape is preserved — serde's
    /// default / skip_if attributes on the new fields must be
    /// symmetric so two load / save passes converge on the
    /// same model. Complements the raw-JSON check above.
    #[test]
    fn test_testament_double_roundtrip_is_stable() {
        let path = test_map_path();
        let first = load_from_file(&path).expect("first load");
        let intermediate = serde_json::to_string(&first).expect("first serialize");
        let second: MindMap = serde_json::from_str(&intermediate).expect("second load");

        // Canonical markers on the model that would drift if
        // any new serde attribute was asymmetric. Cover each of
        // the four structs that gained the zoom pair.
        assert_eq!(first.nodes.len(), second.nodes.len());
        assert_eq!(first.edges.len(), second.edges.len());
        for (id, first_node) in &first.nodes {
            let second_node = second.nodes.get(id).expect("node preserved");
            assert_eq!(first_node.min_zoom_to_render, second_node.min_zoom_to_render);
            assert_eq!(first_node.max_zoom_to_render, second_node.max_zoom_to_render);
        }
        for (first_edge, second_edge) in first.edges.iter().zip(second.edges.iter()) {
            assert_eq!(first_edge.min_zoom_to_render, second_edge.min_zoom_to_render);
            assert_eq!(first_edge.max_zoom_to_render, second_edge.max_zoom_to_render);
        }
    }

    fn theme_demo_path() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop();
        path.pop();
        path.push("maps/theme_demo.mindmap.json");
        path
    }

    #[test]
    fn test_load_theme_demo_map() {
        let path = theme_demo_path();
        let map = load_from_file(&path).expect("Failed to load theme demo map");
        assert_eq!(map.version, "1.0");
        assert_eq!(map.name, "theme_demo");
        assert_eq!(map.canvas.background_color, "var(--bg)");
        assert!(map.canvas.theme_variables.contains_key("--bg"));
        assert_eq!(map.canvas.theme_variants.len(), 3);
        assert!(map.canvas.theme_variants.contains_key("dark"));
        assert!(map.canvas.theme_variants.contains_key("light"));
        assert!(map.canvas.theme_variants.contains_key("forest"));
        assert_eq!(map.custom_mutations.len(), 3);
    }

    /// The canvas background is a theme-variable reference in the
    /// fixture; the renderer reads `Canvas.background_color` and
    /// resolves it through `theme_variables` at clear-color time.
    /// Pin the resolution here so a broken variant table shows up
    /// as a loader test rather than a black canvas.
    #[test]
    fn test_theme_demo_resolves_background_through_theme_vars() {
        let path = theme_demo_path();
        let map = load_from_file(&path).unwrap();
        assert_eq!(
            crate::util::color::resolve_var(&map.canvas.background_color, &map.canvas.theme_variables),
            "#141414"
        );
    }

    #[test]
    fn test_theme_demo_roundtrip() {
        let path = theme_demo_path();
        let map = load_from_file(&path).unwrap();
        let json = serde_json::to_string(&map).unwrap();
        let back: MindMap = serde_json::from_str(&json).unwrap();
        assert_eq!(back.canvas.theme_variants.len(), 3);
        assert_eq!(back.custom_mutations.len(), 3);
    }

    /// Two consecutive `save_to_file` calls on the same `MindMap`
    /// produce byte-identical files. `MindMap.nodes` is a `HashMap`
    /// whose iteration order is randomized per-process; routing
    /// through `serde_json::Value` (a `BTreeMap` under the hood)
    /// pins the order. Without this, every save would diff against
    /// the previous one even when nothing changed.
    #[test]
    fn test_save_to_file_is_deterministic() {
        let map = load_from_file(&test_map_path()).unwrap();
        let dir = TempDir::new("determinism");
        let path_a = dir.join("a.mindmap.json");
        let path_b = dir.join("b.mindmap.json");
        save_to_file(&path_a, &map).expect("save a failed");
        save_to_file(&path_b, &map).expect("save b failed");
        let bytes_a = std::fs::read(&path_a).unwrap();
        let bytes_b = std::fs::read(&path_b).unwrap();
        assert_eq!(bytes_a, bytes_b, "save output must be deterministic");
    }

    /// `save_to_file` writes to `<dir>/.<name>.<pid>.tmp` then renames
    /// over `path`; on a successful rename, the staging file is gone.
    /// Pins the contract that a kill mid-write leaves either the old
    /// file intact or the new file complete — never a torn partial
    /// write next to either.
    #[test]
    fn test_save_to_file_leaves_no_tmp_file_on_success() {
        let map = MindMap::new_blank("no-tmp");
        let dir = TempDir::new("no-tmp-leftover");
        let path = dir.join("map.mindmap.json");
        save_to_file(&path, &map).expect("save failed");

        let pid = std::process::id();
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let leftover = dir.join(&format!(".{file_name}.{pid}.tmp"));
        assert!(
            !leftover.exists(),
            "atomic writer left a temp file behind: {}",
            leftover.display()
        );
    }

    /// The temp-file + rename that buys atomicity replaces the
    /// target's inode, so a naive implementation hands the user's map
    /// back at whatever the umask says — a map deliberately
    /// `chmod 600`'d because it carries private notes would come back
    /// world-readable after a save or an in-place `maptool convert`.
    /// `write_atomic` copies the existing mode onto the staging file
    /// before the swap; this pins that.
    ///
    /// Unix-only: `PermissionsExt` is where a mode bit is even
    /// expressible. The behavior is not — the `set_permissions` call
    /// it guards runs on every target.
    #[cfg(unix)]
    #[test]
    fn test_write_atomic_preserves_the_targets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("atomic-permissions");
        let path = dir.join("private.mindmap.json");
        fs::write(&path, "{}").expect("seed the target");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("chmod 600");

        write_atomic(&path, "{\"replaced\":true}").expect("write_atomic failed");

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "an owner-only map must not be widened by the atomic swap"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"replaced\":true}");
    }

    /// The mode is applied when the staging file is *created*, not
    /// after the content lands, so nothing ever holds the caller's
    /// bytes at a wider mode than the target had.
    ///
    /// The window itself is not observable from a single-threaded
    /// test, but the mechanism that closes it is: `OpenOptions::mode`
    /// only takes effect on creation, which forces the writer to
    /// clear any stale staging file first. This plants one at `0666`
    /// with **no** target to inherit from — the case where the old
    /// write-then-chmod shape had no chmod to run at all, so the
    /// leftover's mode rode straight onto a brand-new map.
    #[cfg(unix)]
    #[test]
    fn test_write_atomic_does_not_inherit_a_stale_staging_files_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("atomic-stale-staging");
        let path = dir.join("fresh.mindmap.json");
        let pid = std::process::id();
        let stale = dir.join(&format!(".fresh.mindmap.json.{pid}.tmp"));
        fs::write(&stale, "leftover from a crashed run").expect("plant the stale staging file");
        fs::set_permissions(&stale, fs::Permissions::from_mode(0o666)).expect("chmod 666");

        write_atomic(&path, "{\"fresh\":true}").expect("write_atomic failed");

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_ne!(
            mode, 0o666,
            "a stale staging file's mode must not ride onto a brand-new map"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"fresh\":true}");
        assert!(!stale.exists(), "the staging file must not survive the rename");
    }

    /// A target that does not exist yet has no mode to inherit, so
    /// the new file takes the umask default like any other writer —
    /// and the write still succeeds rather than failing on the
    /// missing metadata.
    #[cfg(unix)]
    #[test]
    fn test_write_atomic_creates_a_new_file_without_a_target_to_inherit_from() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("atomic-permissions-new");
        let path = dir.join("fresh.mindmap.json");
        write_atomic(&path, "{}").expect("write_atomic failed");

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert!(
            mode & 0o600 == 0o600,
            "a fresh file must at least be owner-readable/writable, got {mode:o}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
    }

    /// `save_to_file` → `load_from_file` reproduces the same `MindMap`
    /// for both the loaded testament fixture and a freshly-blank map
    /// (the `new` console verb lands on this). Locks the on-disk
    /// format as the canonical serialization for both shapes.
    #[test]
    fn test_save_to_file_round_trip_for_loaded_and_blank_maps() {
        let testament = load_from_file(&test_map_path()).unwrap();
        let blank = MindMap::new_blank("untitled");

        for (label, original) in [("testament", testament), ("blank", blank)] {
            let dir = TempDir::new("save-round-trip");
            let tmp = dir.join(&format!("{label}.mindmap.json"));
            save_to_file(&tmp, &original).expect("save failed");
            let reloaded = load_from_file(&tmp).expect("reload failed");

            assert_eq!(reloaded.version, original.version, "{label}: version");
            assert_eq!(reloaded.name, original.name, "{label}: name");
            assert_eq!(reloaded.nodes.len(), original.nodes.len(), "{label}: nodes len");
            assert_eq!(reloaded.edges.len(), original.edges.len(), "{label}: edges len");
            assert_eq!(
                reloaded.canvas.background_color, original.canvas.background_color,
                "{label}: bg",
            );
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// `MindMap.macros` round-trips through save+load with absence
    /// preserved (skip_serializing_if = "Vec::is_empty") and
    /// non-empty content preserved exactly. Locks the on-disk
    /// contract for the `macros` field.
    #[test]
    fn test_save_to_file_macros_round_trip() {
        // Empty case: no `macros` key written, no key on reload.
        let blank = MindMap::new_blank("macro-rt");
        assert!(blank.macros.is_empty());
        let dir = TempDir::new("macros-round-trip");
        let tmp_empty = dir.join("empty.mindmap.json");
        save_to_file(&tmp_empty, &blank).expect("save failed");
        let reloaded_empty = load_from_file(&tmp_empty).expect("reload failed");
        assert!(reloaded_empty.macros.is_empty());

        // Verify the key is absent on disk (skip_serializing_if).
        let raw = std::fs::read_to_string(&tmp_empty).expect("read raw");
        assert!(!raw.contains("\"macros\""), "empty macros must not be serialized");

        // Non-empty case: round-trip preserves the JSON shape.
        let mut populated = MindMap::new_blank("macro-rt-2");
        populated.macros = vec![serde_json::json!({
            "id": "save-and-quit",
            "name": "Save and Quit",
            "description": "",
            "steps": [{"kind": "Action", "action": "SaveDocument"}]
        })];
        let tmp_full = dir.join("full.mindmap.json");
        save_to_file(&tmp_full, &populated).expect("save failed");
        let reloaded_full = load_from_file(&tmp_full).expect("reload failed");
        assert_eq!(reloaded_full.macros.len(), 1);
        assert_eq!(reloaded_full.macros, populated.macros);
    }

    /// `MindNode.inline_macros` round-trips through save+load
    /// with absence preserved (skip_serializing_if = "Vec::is_empty")
    /// and non-empty content preserved exactly. Parallel to
    /// `test_save_to_file_macros_round_trip` for the per-node
    /// field added in the Inline-tier macro work.
    #[test]
    fn test_save_to_file_inline_macros_round_trip() {
        // Build a map with one node carrying a populated
        // `inline_macros`. Empty case is implicitly covered by
        // `test_save_blank_map_round_trip` (every node has an
        // empty Vec).
        use crate::mindmap::model::{Canvas, MindNode, MindSection, NodeLayout, NodeStyle, Position, Size};
        use std::collections::HashMap;

        let node = MindNode {
            id: "0".to_string(),
            parent_id: None,
            position: Position { x: 0.0, y: 0.0 },
            size: Size {
                width: 100.0,
                height: 50.0,
            },
            sections: vec![MindSection::new_default("n".to_string(), Vec::new())],
            style: NodeStyle {
                background_color: "#000000".to_string(),
                frame_color: "#ffffff".to_string(),
                text_color: "#ffffff".to_string(),
                shape: "rectangle".to_string(),
                corner_radius_percent: 0.0,
                frame_thickness: 1.0,
                show_frame: true,
                show_shadow: false,
                border: None,
            },
            layout: NodeLayout {
                layout_type: "map".to_string(),
                direction: "auto".to_string(),
                spacing: 0.0,
            },
            folded: false,
            notes: String::new(),
            color_schema: None,
            channel: 0,
            trigger_bindings: Vec::new(),
            inline_mutations: Vec::new(),
            inline_macros: vec![serde_json::json!({
                "id": "0.tag-as-inbox",
                "steps": [{"kind": "Action", "action": "Undo"}]
            })],
            min_zoom_to_render: None,
            max_zoom_to_render: None,
        };
        let mut nodes = HashMap::new();
        nodes.insert("0".to_string(), node);
        let map = MindMap {
            version: "1.0".to_string(),
            name: "inline-rt".to_string(),
            canvas: Canvas {
                background_color: "#000000".to_string(),
                default_border: None,
                default_connection: None,
                default_section_frame_border: None,
                default_focused_section_frame_border: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            palettes: HashMap::new(),
            nodes,
            edges: Vec::new(),
            custom_mutations: Vec::new(),
            macros: Vec::new(),
        };

        let dir = TempDir::new("inline-macros-round-trip");
        let tmp = dir.join("map.mindmap.json");
        save_to_file(&tmp, &map).expect("save failed");
        let reloaded = load_from_file(&tmp).expect("reload failed");

        let n = reloaded.nodes.get("0").expect("node");
        assert_eq!(n.inline_macros.len(), 1);
        assert_eq!(n.inline_macros[0], map.nodes.get("0").unwrap().inline_macros[0]);

        // Empty-case absence: a node with no inline_macros must
        // not have the key on disk (skip_serializing_if).
        let raw = std::fs::read_to_string(&tmp).expect("read raw");
        // The serialized node has `"inline_macros":` exactly once
        // (the populated one). A second occurrence would mean
        // empty Vecs were serialized. Today the map has exactly
        // one node, so 1 match is correct; if we had a second
        // node with no inline_macros it should be absent.
        assert_eq!(
            raw.matches("\"inline_macros\"").count(),
            1,
            "non-empty inline_macros must be serialized; empty must not"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_is_hidden_by_fold() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Root node has no parent, so it should never be hidden
        let root = map.nodes.get("0").unwrap();
        assert!(!map.is_hidden_by_fold(root));

        // A child of a non-folded parent should not be hidden
        let children = map.children_of("0");
        assert!(!children.is_empty());
        // The root is not folded by default, so its children are visible
        assert!(!map.is_hidden_by_fold(children[0]));
    }

    /// Minimal single-node-per-id JSON fragment — only `id` and
    /// `parent_id` vary between call sites.
    fn node_json(id: &str, parent_id: &str) -> String {
        node_json_with(id, parent_id, "")
    }

    /// [`node_json`] plus `extra`, spliced in as trailing object
    /// members (`", \"key\": value"`). Lets a rejection test add the
    /// one key under test without restating a whole node around it.
    fn node_json_with(id: &str, parent_id: &str, extra: &str) -> String {
        format!(
            r##""{id}": {{
                "id": "{id}", "parent_id": {parent_id},
                "position": {{"x": 0, "y": 0}},
                "size": {{"width": 100, "height": 50}},
                "sections": [{{"text": "n"}}],
                "style": {{"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false}},
                "layout": {{"type":"map","direction":"auto","spacing":0}},
                "folded": false, "notes": ""{extra}
            }}"##
        )
    }

    fn map_json_with_nodes(nodes_json: &str) -> String {
        map_json(nodes_json, "")
    }

    /// A whole map around the given `nodes` and `edges` object /
    /// array bodies.
    ///
    /// Every key here is one the author had to write: nothing
    /// optional is spelled out at its default value. That is what
    /// lets a round-trip test compare the key set before and after a
    /// save without having to except the keys `skip_serializing_if`
    /// legitimately omits.
    fn map_json(nodes_json: &str, edges_json: &str) -> String {
        format!(
            r##"{{
                "version": "1.0",
                "name": "loader-edges",
                "canvas": {{"background_color": "#000"}},
                "nodes": {{{nodes_json}}},
                "edges": [{edges_json}]
            }}"##
        )
    }

    /// A single line-mode edge, with `extra` spliced in the same way
    /// [`node_json_with`] takes it.
    fn edge_json_with(extra: &str) -> String {
        format!(
            r##"{{
                "from_id": "a", "to_id": "b", "type": "parent_child",
                "color": "#fff", "width": 1, "line_style": "solid",
                "visible": true, "label": null,
                "anchor_from": "auto", "anchor_to": "auto",
                "control_points": []{extra}
            }}"##
        )
    }

    /// Every `/`-joined key path in a JSON tree, with array indices
    /// **collapsed** to `[]`.
    ///
    /// This answers "which keys exist at all". It is deliberately
    /// blind to how many elements carry a key, which makes it the
    /// wrong tool for the second question and the right tool for the
    /// first: a key that stops being written *anywhere* shows up here
    /// no matter which element used to hold it.
    ///
    /// [`indexed_key_values`] answers the other half, and the two are
    /// used together — see
    /// [`assert_load_save_loses_no_authored_key`].
    fn key_paths(value: &serde_json::Value, prefix: &str, out: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(members) => {
                for (key, child) in members {
                    let path = format!("{prefix}/{key}");
                    out.insert(path.clone());
                    key_paths(child, &path, out);
                }
            }
            serde_json::Value::Array(items) => {
                let path = format!("{prefix}[]");
                for child in items {
                    key_paths(child, &path, out);
                }
            }
            _ => {}
        }
    }

    fn key_path_set(json: &str) -> std::collections::BTreeSet<String> {
        let value: serde_json::Value = serde_json::from_str(json).expect("valid JSON");
        let mut out = std::collections::BTreeSet::new();
        key_paths(&value, "", &mut out);
        out
    }

    /// Every `/`-joined key path in a JSON tree with array indices
    /// **preserved** (`/nodes/3.7/sections[0]/offset`), mapped to the
    /// value found there.
    ///
    /// The index is the point. Under [`key_paths`], `sections[0]` and
    /// `sections[1]` are the same path, so a key dropped from one
    /// element is invisible whenever a sibling keeps it — and that is
    /// not hypothetical: `maps/testament.mindmap.json` really does
    /// lose `/nodes/3.7/sections[0]/offset` on save, while
    /// `sections[1]/offset` survives. The value comes along because
    /// knowing a key vanished is only half an answer; whether it held
    /// anything is the other half.
    fn indexed_key_values<'a>(
        value: &'a serde_json::Value,
        prefix: &str,
        out: &mut std::collections::BTreeMap<String, &'a serde_json::Value>,
    ) {
        match value {
            serde_json::Value::Object(members) => {
                for (key, child) in members {
                    let path = format!("{prefix}/{key}");
                    indexed_key_values(child, &path, out);
                    out.insert(path, child);
                }
            }
            serde_json::Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    let path = format!("{prefix}[{index}]");
                    indexed_key_values(child, &path, out);
                }
            }
            _ => {}
        }
    }

    /// One `skip_serializing_if` predicate modeled at the JSON level:
    /// the predicate path exactly as the source writes it, and a test
    /// for the values it leaves out.
    type OmissionRule = (&'static str, fn(&serde_json::Value) -> bool);

    /// **The complete set of reasons a key present on the way in may
    /// be absent on the way out**, each paired with the JSON values
    /// its predicate omits.
    ///
    /// `skip_serializing_if` is the only sanctioned omission: the
    /// field held its own default, so writing it out carries no
    /// information the reload would not recover. Anything else is
    /// data loss. Modeling the predicates by hand is what lets the
    /// indexed pass be strict — "this path disappeared *and* it held
    /// something" is a failure, full stop — and
    /// `test_every_skip_serializing_if_predicate_is_modeled` is what
    /// keeps the hand-modeling honest: it walks the source and fails
    /// the moment a predicate appears that is not on this list.
    const OMITTABLE_WHEN: &[OmissionRule] = &[
        ("HashMap::is_empty", |v| {
            v.as_object().is_some_and(serde_json::Map::is_empty)
        }),
        ("Option::is_none", serde_json::Value::is_null),
        ("String::is_empty", |v| v.as_str().is_some_and(str::is_empty)),
        ("Vec::is_empty", |v| v.as_array().is_some_and(|a| a.is_empty())),
        ("is_default_position", |v| {
            v.as_object().is_some_and(|o| {
                o.len() == 2
                    && ["x", "y"]
                        .iter()
                        .all(|k| o.get(*k).and_then(serde_json::Value::as_f64) == Some(0.0))
            })
        }),
        ("is_zero_u32", |v| v.as_u64() == Some(0)),
    ];

    /// Assert that saving `source` as `saved` lost nothing an author
    /// wrote, along **both** dimensions:
    ///
    /// 1. *which keys exist* — the collapsed pass, which catches a
    ///    key that stopped being written anywhere;
    /// 2. *which elements carry them* — the indexed pass, which
    ///    catches a key dropped from one array element while a
    ///    sibling keeps it. A path may disappear here only when the
    ///    value it held is one [`OMITTABLE_WHEN`] accounts for.
    ///
    /// Neither pass subsumes the other: the collapsed one is the only
    /// one that would notice a whole field going away when every
    /// element's value happens to be a default, and the indexed one
    /// is the only one that sees per-element loss at all.
    fn assert_load_save_loses_no_authored_key(source: &str, saved: &str) {
        let before_keys = key_path_set(source);
        let after_keys = key_path_set(saved);
        let lost: Vec<&String> = before_keys.difference(&after_keys).collect();
        assert!(lost.is_empty(), "load → save dropped authored keys: {lost:?}");

        let before_value: serde_json::Value = serde_json::from_str(source).expect("valid JSON");
        let after_value: serde_json::Value = serde_json::from_str(saved).expect("valid JSON");
        let mut before = std::collections::BTreeMap::new();
        let mut after = std::collections::BTreeMap::new();
        indexed_key_values(&before_value, "", &mut before);
        indexed_key_values(&after_value, "", &mut after);

        // A key's children go with it, so only the *outermost* lost
        // path at each site is evidence: reporting
        // `…/offset/x` under a lost `…/offset` would demand a
        // predicate for a value nobody chose to omit.
        let lost: std::collections::BTreeSet<&str> = before
            .keys()
            .filter(|path| !after.contains_key(*path))
            .map(String::as_str)
            .collect();
        let unexplained: Vec<String> = lost
            .iter()
            .filter(|path| !has_lost_ancestor(path, &lost))
            .filter(|path| {
                let value = before[**path];
                !OMITTABLE_WHEN.iter().any(|(_, omits)| omits(value))
            })
            .map(|path| format!("{path} = {}", before[*path]))
            .collect();
        assert!(
            unexplained.is_empty(),
            "load → save dropped key(s) that held something, and no \
             `skip_serializing_if` predicate accounts for the value:\n  {}",
            unexplained.join("\n  ")
        );
    }

    /// Whether some proper ancestor of `path` is itself in `lost`.
    ///
    /// Ancestors are found by trimming back to the last `/` or `[`,
    /// which is exact for these paths: [`indexed_key_values`] builds
    /// them by appending `/key` and `[index]`, and no key in the
    /// format contains either character.
    fn has_lost_ancestor(path: &str, lost: &std::collections::BTreeSet<&str>) -> bool {
        let mut rest = path;
        while let Some(cut) = rest.rfind(['/', '[']) {
            rest = &rest[..cut];
            if rest.is_empty() {
                return false;
            }
            if lost.contains(rest) {
                return true;
            }
        }
        false
    }

    /// **The policy, spelled out where authors read it.**
    /// `format/schema.md` publishes one map that must be rejected
    /// for a mistyped key, the rejection message it produces, and
    /// one map that must load once the spelling is fixed. All three
    /// are read out of the spec rather than restated here, so the
    /// pin follows the doc when the doc moves instead of agreeing
    /// with a copy of its old self.
    ///
    /// The published message is compared for **equality**, not
    /// containment: the doc presents it as verbatim loader output,
    /// and its `expected one of` clause is an enumeration of
    /// `MindNode`'s keys that a hand-author reads as the field list.
    /// A substring check would let a new field land, change the real
    /// message, and leave the spec publishing a list that is missing
    /// it — with the suite green. Only line wrapping is normalized
    /// away ([`doc_fixtures::unwrapped`]): the doc wraps to the
    /// column limit, the loader emits one line, and where the breaks
    /// fall carries no meaning.
    #[test]
    fn test_documented_unknown_key_rejection_matches_the_spec() {
        use crate::util::doc_fixtures::{
            documented_json_block, documented_plain_block, format_doc_path, unwrapped,
        };
        let doc = format_doc_path("schema.md");
        let heading = "## Unknown keys are rejected";

        let rejected = documented_json_block(&doc, heading, 0);
        let err = match load_from_str(&rejected) {
            Err(err) => err,
            Ok(map) => {
                panic!("the spec's mistyped-key example must not load:\n{rejected}\nbut it loaded: {map:?}")
            }
        };
        assert!(
            err.contains("min_zoom_to_rendr"),
            "the error must name the key the author wrote: {err}"
        );

        let published = documented_plain_block(&doc, heading, 0);
        assert_eq!(
            unwrapped(&err),
            unwrapped(&published),
            "format/schema.md §{heading} publishes this rejection as verbatim loader \
             output and it no longer is. Re-wrap the block to match, or the spec is \
             publishing a stale list of a node's keys.\n\
             \n  loader: {err}\n  spec:   {published}\n"
        );

        let accepted = documented_json_block(&doc, heading, 1);
        let map = load_from_str(&accepted)
            .unwrap_or_else(|e| panic!("the spec's corrected example must load: {e}\n{accepted}"));
        assert_eq!(map.nodes["0"].min_zoom_to_render, Some(2.0));
    }

    /// An unknown key on a node names the node, names the key, and
    /// says what the loader did about it. The last part is the point:
    /// "unknown field" reads like pedantry until you know the
    /// alternative was the key silently gone from the file after the
    /// next save.
    #[test]
    fn test_unknown_node_key_is_rejected_with_the_policy() {
        let json = map_json_with_nodes(&node_json_with("1.2", "null", r#", "portal_form": {"x": 1}"#));
        let err = load_from_str(&json).expect_err("an unknown node key must be rejected");
        assert!(err.contains("node \"1.2\""), "must name the node: {err}");
        assert!(err.contains("portal_form"), "must name the key: {err}");
        assert!(
            err.contains("rejected, not dropped"),
            "must state the policy, not just the symptom: {err}"
        );
        assert!(err.contains("format/schema.md"), "must point at the spec: {err}");
        assert!(
            err.contains("expected one of") && err.contains("`inline_mutations`"),
            "must list what the loader would have accepted: {err}"
        );
    }

    /// The same rejection reaches keys nested inside a node — a typo
    /// in `style` is as destructive as one at the node level, and
    /// the message still resolves to the node the author has to open.
    #[test]
    fn test_unknown_key_inside_node_style_names_the_node() {
        let json = map_json_with_nodes(
            &node_json("0", "null").replace(r#""show_shadow":false"#, r#""show_shadow":false,"shpe":"star""#),
        );
        let err = load_from_str(&json).expect_err("an unknown style key must be rejected");
        assert!(err.contains("node \"0\""), "must name the node: {err}");
        assert!(err.contains("shpe"), "must name the key: {err}");
    }

    /// Edges are addressed by index, matching `MindMap::edge_locations`
    /// and `maptool verify`'s `edge[<i>]` stamp, because an edge has
    /// no id to name it by.
    #[test]
    fn test_unknown_edge_key_names_the_edge_index() {
        let nodes = format!("{},{}", node_json("a", "null"), node_json("b", "\"a\""));
        let json = map_json(&nodes, &edge_json_with(r#", "arrowhead": "open""#));
        let err = load_from_str(&json).expect_err("an unknown edge key must be rejected");
        assert!(err.contains("edge[0]"), "must name the edge index: {err}");
        assert!(err.contains("arrowhead"), "must name the key: {err}");
    }

    /// The canvas is a single named part rather than a collection,
    /// so it is stamped by name.
    #[test]
    fn test_unknown_canvas_key_names_the_canvas() {
        let json = map_json_with_nodes(&node_json("0", "null")).replace(
            r##""background_color": "#000""##,
            r##""background_color": "#000", "grid_snap": 8"##,
        );
        let err = load_from_str(&json).expect_err("an unknown canvas key must be rejected");
        assert!(err.contains("canvas:"), "must name the canvas: {err}");
        assert!(err.contains("grid_snap"), "must name the key: {err}");
    }

    /// A key the model does not know at the top level has no
    /// sub-object to attribute it to, so serde's own message — which
    /// already names the key and the accepted set — carries the
    /// report, with the policy sentence appended.
    #[test]
    fn test_unknown_top_level_key_is_rejected() {
        let json = map_json_with_nodes(&node_json("0", "null"))
            .replace(r#""version": "1.0","#, r#""version": "1.0", "authors": ["me"],"#);
        let err = load_from_str(&json).expect_err("an unknown top-level key must be rejected");
        assert!(err.contains("authors"), "must name the key: {err}");
        assert!(
            err.contains("rejected, not dropped"),
            "must state the policy: {err}"
        );
    }

    /// **The data-loss scenario, end to end.** Load the canonical
    /// fixture, save it through the editor's own save path, and
    /// confirm no key present on the way in is missing on the way
    /// out. This is the failure the unknown-key policy exists to
    /// prevent, checked against the file rather than argued about:
    /// before, a key the model did not know made it through load as
    /// nothing at all and came back out of `save_to_file` deleted.
    ///
    /// Key *paths* rather than bytes, because saving is allowed to
    /// reorder keys and re-indent — but never to lose one. Both the
    /// collapsed and the index-preserving pass run; see
    /// [`assert_load_save_loses_no_authored_key`] for why one is not
    /// enough.
    #[test]
    fn test_no_authored_key_is_lost_across_load_and_save() {
        let source = std::fs::read_to_string(test_map_path()).expect("read fixture");
        let map = load_from_str(&source).expect("fixture loads");

        let dir = TempDir::new("round-trip-keys");
        let saved_path = dir.join("resaved.mindmap.json");
        save_to_file(&saved_path, &map).expect("save failed");
        let saved = std::fs::read_to_string(&saved_path).expect("read resaved");

        assert_load_save_loses_no_authored_key(&source, &saved);
    }

    /// **The hand-modeled half of the round trip, held to the
    /// source.** [`OMITTABLE_WHEN`] enumerates the predicates that
    /// excuse a missing key, and a list of predicates is exactly the
    /// twin surface `lib/baumhard/CONVENTIONS.md` §B4 warns about: a
    /// new `skip_serializing_if` would start omitting keys that the
    /// round-trip test then has no model for — and, worse, an
    /// unmodeled predicate makes the indexed pass fail on a
    /// legitimate omission, which is the kind of noise that gets a
    /// test deleted.
    ///
    /// So the set is not written down twice. `serde_coverage` walks
    /// the same reachable graph the `deny_unknown_fields` drift test
    /// uses and reports every predicate a loadable type actually
    /// names; this fails until the two agree in both directions.
    #[test]
    fn test_every_skip_serializing_if_predicate_is_modeled() {
        use crate::util::serde_coverage::{crate_src_root, TypeGraph};

        let graph = TypeGraph::build(&crate_src_root());
        let in_source = graph.omit_predicates_from("MindMap");
        let modeled: std::collections::BTreeSet<String> = OMITTABLE_WHEN
            .iter()
            .map(|(predicate, _)| (*predicate).to_string())
            .collect();

        let unmodeled: Vec<&String> = in_source.difference(&modeled).collect();
        assert!(
            unmodeled.is_empty(),
            "these `skip_serializing_if` predicates are reachable from a \
             `.mindmap.json` load and the round-trip test has no model for the \
             values they omit — add each to OMITTABLE_WHEN with the JSON values it \
             leaves out: {unmodeled:?}"
        );

        let stale: Vec<&String> = modeled.difference(&in_source).collect();
        assert!(
            stale.is_empty(),
            "OMITTABLE_WHEN models predicate(s) no loadable type names any more; \
             leaving them in silently widens what the round-trip test forgives: \
             {stale:?}"
        );
    }

    /// The same round trip for a key the model knows but that no
    /// fixture happens to carry, so the guarantee is not resting on
    /// what testament was written with.
    #[test]
    fn test_an_authored_zoom_window_survives_load_and_save() {
        let json = map_json_with_nodes(&node_json_with(
            "0",
            "null",
            r#", "min_zoom_to_render": 0.25, "max_zoom_to_render": 4.0"#,
        ));
        let map = load_from_str(&json).expect("a fully-spelled map loads");

        let dir = TempDir::new("round-trip-zoom");
        let saved_path = dir.join("zoom.mindmap.json");
        save_to_file(&saved_path, &map).expect("save failed");
        let saved = std::fs::read_to_string(&saved_path).expect("read resaved");

        assert_load_save_loses_no_authored_key(&json, &saved);
        let reloaded = load_from_str(&saved).expect("resaved map loads");
        assert_eq!(reloaded.nodes["0"].min_zoom_to_render, Some(0.25));
        assert_eq!(reloaded.nodes["0"].max_zoom_to_render, Some(4.0));
    }

    /// **The screen that used to false-positive.** `"text_runs":`
    /// appears in every styled section of every current map —
    /// testament carries hundreds — and a substring screen looking
    /// for that marker flagged all of them, buying a second full
    /// parse of the document on essentially every real load. The
    /// marker is gone; a styled map is just a map.
    #[test]
    fn test_section_text_runs_are_not_a_legacy_marker() {
        let styled = node_json("0", "null").replace(
            r#""sections": [{"text": "n"}]"#,
            r##""sections": [{"text": "n", "text_runs": [
                {"start":0,"end":1,"bold":true,"italic":false,"underline":false,
                 "font":"LiberationSans","size_pt":14,"color":"#fff","hyperlink":null}]}]"##,
        );
        let map =
            load_from_str(&map_json_with_nodes(&styled)).expect("a styled section is not a legacy shape");
        assert_eq!(map.nodes["0"].sections[0].text_runs.len(), 1);
    }

    /// A legacy `portals` key still gets its migration verb, and now
    /// does so even when the array is empty: the key itself has to
    /// come out of the file, and `convert --portals` is what removes
    /// it. Previously an empty array parsed as a map with no portals
    /// and the stale key survived every save.
    #[test]
    fn test_legacy_portals_key_is_rejected_even_when_empty() {
        for portals in ["[]", r#"[{"endpoint_a":"0","endpoint_b":"0"}]"#] {
            let json = map_json_with_nodes(&node_json("0", "null"))
                .replace(r#""edges": []"#, &format!(r#""edges": [], "portals": {portals}"#));
            let err = load_from_str(&json).expect_err("a legacy portals key must be rejected");
            assert!(
                err.contains("maptool convert --portals"),
                "must point at the migration verb: {err}"
            );
        }
    }

    /// Malformed JSON is a syntax error, not a schema one — serde's
    /// line and column is the whole diagnosis, and the loader must
    /// not bury it behind a schema-shaped guess.
    #[test]
    fn test_malformed_json_reports_line_and_column() {
        let err = load_from_str("{\n  \"version\": \"1.0\",\n  \"name\":\n}")
            .expect_err("malformed JSON must be rejected");
        assert!(
            err.starts_with("Failed to parse mindmap JSON:"),
            "must read as a parse failure: {err}"
        );
        assert!(err.contains("line 4"), "must carry the position: {err}");
    }

    /// A required field left out is attributed to the part that left
    /// it out, the same way an unknown key is — the loader's job
    /// either way is to say which node to open.
    #[test]
    fn test_missing_required_field_names_the_node() {
        let json = map_json_with_nodes(&node_json("0", "null").replace(r#", "notes": """#, ""));
        let err = load_from_str(&json).expect_err("a missing required field must be rejected");
        assert!(err.contains("node \"0\""), "must name the node: {err}");
        assert!(err.contains("notes"), "must name the field: {err}");
    }

    /// `edge_type` is an **open vocabulary**, not a closed enum:
    /// `"parent_child"` and `"cross_link"` are what the renderer
    /// knows, but the field is a `String` and an unrecognized value
    /// loads and round-trips. Closedness in this format is about
    /// keys; values with open vocabularies stay open, and
    /// `maptool verify` is where an unknown one gets flagged.
    #[test]
    fn test_unknown_edge_type_value_loads_and_round_trips() {
        let nodes = format!("{},{}", node_json("a", "null"), node_json("b", "\"a\""));
        let edge = edge_json_with("").replace(r#""type": "parent_child""#, r#""type": "annotates""#);
        let map = load_from_str(&map_json(&nodes, &edge)).expect("an open vocabulary value loads");
        assert_eq!(map.edges[0].edge_type, "annotates");

        let json = serde_json::to_string(&map).expect("serializes");
        let back: MindMap = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back.edges[0].edge_type, "annotates");
    }

    /// A 2-cycle `a → b → a` (`P0-05`) must be rejected at load time
    /// — with no check, `is_hidden_by_fold` walking this chain never
    /// terminates.
    #[test]
    fn test_two_node_parent_cycle_rejected() {
        let nodes = format!("{},{}", node_json("a", "\"b\""), node_json("b", "\"a\""));
        let json = map_json_with_nodes(&nodes);
        let err = load_from_str(&json).expect_err("2-cycle must be rejected");
        assert!(err.contains("cycle"), "error must mention cycle: {err}");
        assert!(
            err.contains("fix parent_id"),
            "error must point at parent_id: {err}"
        );
        assert!(
            err.contains('a') && err.contains('b'),
            "error must name the nodes in the cycle: {err}"
        );
    }

    /// A self-parented node `a → a` is the degenerate 1-cycle and
    /// must be rejected the same way as the general case.
    #[test]
    fn test_self_parent_cycle_rejected() {
        let nodes = node_json("a", "\"a\"");
        let json = map_json_with_nodes(&nodes);
        let err = load_from_str(&json).expect_err("self-parent cycle must be rejected");
        assert!(err.contains("cycle"), "error must mention cycle: {err}");
        assert!(err.contains('a'), "error must name node 'a': {err}");
    }

    /// A valid 3-generation chain (no cycle) must load without error
    /// — pairs with the cycle-rejection tests to confirm the checker
    /// doesn't false-positive on an ordinary tree.
    #[test]
    fn test_acyclic_chain_not_rejected() {
        let nodes = format!(
            "{},{},{}",
            node_json("0", "null"),
            node_json("0.0", "\"0\""),
            node_json("0.0.0", "\"0.0\"")
        );
        let json = map_json_with_nodes(&nodes);
        let map = load_from_str(&json).expect("acyclic chain must load");
        assert_eq!(map.nodes.len(), 3);
    }
}
