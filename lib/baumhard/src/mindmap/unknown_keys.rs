// SPDX-License-Identifier: MPL-2.0

//! Keys a `.mindmap.json` carries that this build's model has no field
//! for — found at load, reported once each, and written back untouched
//! at save.
//!
//! **The problem this solves is forward compatibility across
//! versions.** A map authored by a newer build carries keys an older
//! build does not know. Refusing the load leaves the reader with
//! nothing; ignoring the keys is worse, because the editor resaves the
//! whole model and the next save deletes them. So the load keeps them:
//! every unrecognized key is captured with the route that leads to it,
//! warned about once, carried on
//! [`MindMap`](crate::mindmap::model::MindMap),
//! and spliced back into the serialized document before it reaches
//! disk.
//!
//! **Why the capture is not a per-type `#[serde(flatten)]` catch-all.**
//! A flattened `HashMap<String, Value>` field cannot go on a type that
//! derives `Copy`, `Eq`, or `Hash` — `serde_json::Value` implements
//! none of the three — and the graph reachable from a load is full of
//! such types (`Color`, `OrderedVec2`, `Position`, `Anchor`, `Range`,
//! `GlyphMatrix`, …). A catch-all on only the types that can hold one
//! would preserve keys in some places and silently drop them in
//! others, which is the failure the policy exists to remove. Capturing
//! at the deserializer instead covers **every** type in the graph, and
//! types the source walk cannot even see (anything `build.rs` emits
//! into `$OUT_DIR`), with no per-type opt-in and therefore no drift
//! surface.
//!
//! What holds the mechanism honest is the other direction: a reachable
//! type must not be able to *swallow* a key before the capture sees
//! it. `#[serde(deny_unknown_fields)]` would abort the load, and
//! `#[serde(flatten)]`, `#[serde(untagged)]` and `#[serde(tag = "…")]`
//! would absorb the key without ever reaching
//! `deserialize_ignored_any`. `mindmap::loader`'s
//! `test_no_loadable_type_can_swallow_an_unknown_key` walks the model's
//! own source and fails if any of them appears.
//!
//! # The model's own serialization is the oracle
//!
//! Two of the three hard problems here are solved by the same thing: a
//! **probe**, the `serde_json::Value` the freshly loaded model
//! serializes to. It is taken once per load, and only for a document
//! that carried something unrecognized, so a map this build authored
//! never pays for it.
//!
//! - **Levels the JSON has that serde's ignored-key path does not.**
//!   An externally tagged enum writes `{"Void": {…}}`, and the variant
//!   name never travels through a `MapAccess`, so serde's path is one
//!   step short. Deciding "is this one-member object a variant wrapper
//!   or the thing I am addressing?" from its member count alone is a
//!   guess, and it guesses wrong the moment a payload carries a key
//!   spelled like the variant. The probe answers it without guessing:
//!   **a key the model itself writes at a level cannot be the key
//!   serde ignored there**, so the walk descends past it. See
//!   `expand_route`.
//! - **Containers the saver omits.** `#[serde(skip_serializing_if)]`
//!   means a container that holds its own default is simply absent
//!   from the saved document, and a route through it would die with
//!   nothing to hold on to. Comparing the authored document against
//!   the probe finds those levels at load time, so the save can put
//!   the container back before writing the key into it. See
//!   `Scaffold`.
//! - **Array elements that move.** A route below an array is
//!   positional, and deleting an earlier element silently slides the
//!   route onto a different one. The probe's element fingerprints are
//!   captured alongside the index, so the save re-finds the element it
//!   was actually attached to — and refuses, loudly, when it cannot.
//!   See `IndexAnchor`.

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// One step of the route from the document root to a captured key.
///
/// A route is what a JSON pointer would be, kept structured rather
/// than joined: a Dewey-decimal node id contains `.` and `/`-joining
/// it would make the route ambiguous to read back.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Step {
    /// An object member, named by its key.
    Key(String),
    /// An array element, by zero-based index.
    Index(usize),
}

/// A content hash of one JSON value, used to recognize an array
/// element again after the model has been edited around it.
///
/// `Value`'s object member order is `BTreeMap` order, so
/// `Value::to_string` is canonical and two structurally equal values
/// always hash alike. The hash is compared only between two values
/// produced inside **one process run** (the load-time probe and the
/// save-time document), which is what makes `DefaultHasher` — whose
/// output is explicitly not stable across releases — sound here.
///
/// Cost: one `String` render of `value` plus one hash of it.
fn fingerprint(value: &Value) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    hasher.finish()
}

/// The identity of one array along a captured key's route, so the
/// save can tell "the element I was attached to" from "whatever is at
/// that index now".
///
/// Without this, deleting an earlier element of the same array
/// silently reattaches the key to its neighbor — the route still
/// resolves, just to the wrong element, which is worse than losing the
/// key because nothing about it looks wrong.
#[derive(Debug, Clone)]
struct IndexAnchor {
    /// Position in the route of the [`Step::Index`] this describes.
    step: usize,
    /// [`fingerprint`] of every element of the array as the model
    /// serialized it at load time, in order.
    ///
    /// **Shared, not copied.** Every key captured under the same array
    /// needs the same vector, and building one per key made the capture
    /// quadratic: `K` keys inside an `N`-element array cost `K × N`
    /// fingerprints to compute and `K × N` `u64`s to retain, each
    /// fingerprint rendering its element with `to_string()` first. At
    /// a hundred thousand keys that is 10^10 fingerprints. The ceiling
    /// then in force bounded the route vector while the anchors behind
    /// it stayed unbounded, which is why bounding the input was the
    /// wrong instrument and the cost itself had to be fixed. Measured before the fix, one unknown key per
    /// element: 2 000 keys / 404 KB reached 46 MiB, 6 000 keys /
    /// 1.2 MB reached 311 MiB, and 12 000 keys / 2.4 MB did not finish
    /// in two minutes. `Arc` makes it `O(N)` per array instead.
    siblings: Arc<Vec<u64>>,
}

/// A container the saver leaves out, and the value that puts it back.
///
/// `#[serde(skip_serializing_if = "…")]` on a non-`Option` field means
/// a container holding its own default never reaches the saved
/// document at all. A captured key nested inside one has nowhere to
/// land, and would be lost on a **zero-edit** load → save — no user
/// action, no warning that means anything. So the load records the
/// authored container (with every captured key already lifted out of
/// it) and the save re-creates it on the way past.
///
/// Recorded only when the container is absent from the load-time probe
/// — that is, when a save of the map *as loaded* would already have
/// omitted it. That condition is what makes re-inserting the authored
/// container faithful rather than a resurrection: the model's value
/// for it satisfied the omission predicate at load and satisfies it
/// again at save, so the authored bytes still describe the model's
/// value. A container the model wrote at load and dropped later was
/// dropped **by an edit**, and that is a deliberate deletion the save
/// reports rather than undoes.
#[derive(Debug, Clone)]
struct Scaffold {
    /// Route of the object that has to receive [`Self::member`].
    owner: Vec<Step>,
    /// Name of the omitted container.
    member: String,
    /// The container as it was authored, minus every key this capture
    /// took out of it — those are spliced back at their own routes.
    value: Value,
}

/// One key the model has no field for, with the route that leads to
/// it, the value it held, and what the save needs in order to put it
/// back where it came from.
///
/// Plain data. Cost: the route's `String`s plus the captured value,
/// both moved out of the parsed document rather than cloned, plus one
/// `u64` per element of each array the route passes through.
#[derive(Debug, Clone)]
pub struct UnknownKey {
    /// Full route from the document root, the last step being the key
    /// itself. Literal: every level the JSON has is a step, including
    /// the ones serde's own path elides (see [`expand_route`]).
    route: Vec<Step>,
    /// The value the key held, taken out of the parsed document.
    value: Value,
    /// The container the saver omits and the splice has to rebuild,
    /// when there is one.
    scaffold: Option<Scaffold>,
    /// Element identities for every positional step of the route.
    anchors: Vec<IndexAnchor>,
}

impl UnknownKey {
    /// The key as the author wrote it. O(1).
    pub fn key(&self) -> &str {
        match self.route.last() {
            Some(Step::Key(key)) => key,
            // Unreachable in practice: serde only ignores *named*
            // members, so a captured route always ends in a key.
            // Degrading to the empty name keeps a malformed route out
            // of a panic on an interactive path (CODE_CONVENTIONS §9).
            _ => "",
        }
    }

    /// The addressable part of the document the key sits in, stamped
    /// the way `maptool verify` and [`MindMap::edge_locations`] stamp
    /// it — `node "1.2"`, `edge[3]`, `palette "coral"`,
    /// `custom_mutations[0]`, `canvas`, or `map` for a key at the top
    /// level.
    ///
    /// [`MindMap::edge_locations`]: crate::mindmap::model::MindMap::edge_locations
    ///
    /// Cost: one `String` allocation.
    pub fn location(&self) -> String {
        location_of(&self.route).0
    }

    /// Where inside [`Self::location`] the key sits, rendered as a
    /// field path (`style.shpe`, `sections[0].txet`, or just the key
    /// when it hangs directly off the part).
    ///
    /// Cost: one `String` allocation.
    pub fn path_within_location(&self) -> String {
        render_path(location_of(&self.route).1)
    }

    /// The value the key held in the authored file — exactly what a
    /// save writes back. O(1) borrow.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The single line the loader logs for this key, and the single
    /// place its wording lives.
    ///
    /// Carries the `"<area>: message"` prefix CODE_CONVENTIONS §9
    /// requires, so the `log::warn!` call site formats nothing of its
    /// own and a test can assert the exact text.
    ///
    /// **One wording, because there is one outcome.** A captured key
    /// is carried by the next save, with the value it was authored
    /// with; only an *edit* to the surroundings can cost it, and the
    /// three shapes that can are refused loudly at save time by
    /// [`UnknownKeys::splice_into`] rather than guessed at here. This
    /// line used to have a second wording for a key the load could
    /// tell in advance the save would not carry, and nothing could
    /// reach it — see `plan_write_back` for the shape that would have
    /// to exist first.
    ///
    /// Cost: one `String` allocation.
    pub fn warning(&self) -> String {
        format!(
            "loader: {}: unrecognized key `{}` — this build has no field for it, so it is \
             kept as written and saved back with the value it was authored with. Check the \
             spelling if you meant an existing key; see format/schema.md.",
            self.location(),
            self.path_within_location()
        )
    }
}

/// Split a route into the location stamp of the addressable part it
/// falls in and the remaining steps inside that part.
fn location_of(route: &[Step]) -> (String, &[Step]) {
    match route {
        [Step::Key(head), Step::Key(id), rest @ ..] if head == "nodes" => (format!("node {id:?}"), rest),
        [Step::Key(head), Step::Key(name), rest @ ..] if head == "palettes" => {
            (format!("palette {name:?}"), rest)
        }
        [Step::Key(head), Step::Index(i), rest @ ..] if head == "edges" => (format!("edge[{i}]"), rest),
        [Step::Key(head), Step::Index(i), rest @ ..] if head == "custom_mutations" => {
            (format!("custom_mutations[{i}]"), rest)
        }
        [Step::Key(head), rest @ ..] if head == "canvas" => ("canvas".to_string(), rest),
        rest => ("map".to_string(), rest),
    }
}

/// One construct this build cannot name, lifted out of the document
/// so the rest of the map can load, and written back untouched at
/// save.
///
/// **Why a whole construct rather than the part that failed.** An
/// unrecognized *key* has no meaning to this build, so ignoring it
/// changes nothing about what the map does. An unrecognized
/// **variant** is the opposite: it is the thing that was supposed to
/// happen. Dropping one `Mutation` out of a macro leaves a mutation
/// that still appears in `mutation list`, still fires, and now does
/// two of the three things it says it does — a silent partial
/// behavior the user has no way to see. So the unit is the nearest
/// container whose absence reads *as absence*: the whole custom
/// mutation, or the whole trigger binding. What this build cannot
/// carry out, it does not half carry out; it says it skipped it, and
/// keeps the bytes so a build that understands them still has them.
#[derive(Debug, Clone)]
pub struct SkippedConstruct {
    /// Route of the array element that was lifted out. The last step
    /// is the index it had **in the authored document**, which is
    /// where the save puts it back.
    route: Vec<Step>,
    /// The element exactly as it was authored.
    value: Value,
    /// What the typed read said when it refused it — the variant name
    /// and the ones this build knows, straight from serde.
    reason: String,
}

impl SkippedConstruct {
    /// Where the construct sat, stamped the way `maptool verify`
    /// stamps a location. Cost: one `String` allocation.
    pub fn location(&self) -> String {
        let (part, rest) = location_of(&self.route);
        let within = render_path(rest);
        if within.is_empty() {
            part
        } else {
            format!("{part}: {within}")
        }
    }

    /// serde's own account of why this build could not read it.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The construct as authored — exactly what a save writes back.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The single line the loader logs for this construct.
    ///
    /// Says the three things the reader needs and nothing else: what
    /// was skipped, **what the consequence is** — it will not run —
    /// and that the file still has it.
    ///
    /// Cost: one `String` allocation.
    pub fn warning(&self) -> String {
        format!(
            "loader: {}: this build cannot read this construct ({}), so it is skipped — \
             it does not appear in the model and nothing it describes will run. It is \
             written back to the file unchanged, so a build that understands it still \
             has it. See format/schema.md.",
            self.location(),
            self.reason
        )
    }
}

/// Every construct one load had to skip, in document order.
///
/// Lives on [`MindMap`](crate::mindmap::model::MindMap) beside
/// [`UnknownKeys`], for the same reason and with the same
/// `#[serde(skip)]`: the constructs go back at their own routes, not
/// into a side object.
#[derive(Debug, Clone, Default)]
pub struct SkippedConstructs {
    entries: Vec<SkippedConstruct>,
}

impl SkippedConstructs {
    /// Whether the load understood the whole document. O(1).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many constructs were skipped. O(1).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The skipped constructs, in document order. O(1).
    pub fn iter(&self) -> std::slice::Iter<'_, SkippedConstruct> {
        self.entries.iter()
    }

    /// Record one construct the typed read refused.
    ///
    /// `route` must end in the element's authored index.
    pub fn push(&mut self, route: Vec<Step>, value: Value, reason: String) {
        self.entries.push(SkippedConstruct { route, value, reason });
    }

    /// Put every skipped construct back into a freshly serialized
    /// document, at the index it was authored at.
    ///
    /// **Runs after [`UnknownKeys::splice_into`], not before.** The
    /// captured keys' routes were resolved against a document the
    /// constructs had already been lifted out of, so their indices
    /// are the shortened ones; re-inserting first would slide every
    /// one of them onto the wrong element. Restoring afterwards puts
    /// the array back to its authored length with both sets of data
    /// already where they belong.
    ///
    /// A construct whose array is gone — the node it hung off was
    /// deleted — is reported rather than dropped in silence. An index
    /// past the end of a shortened array lands at the end, which is
    /// the only honest answer once the elements it used to sit
    /// between are gone.
    ///
    /// Cost: O(constructs × route length), plus one `Value` clone
    /// each. Nothing at all for a map this build understood.
    pub fn splice_into(&self, document: &mut Value) {
        for entry in &self.entries {
            let Some((Step::Index(index), parent)) = entry.route.split_last() else {
                continue;
            };
            // The array itself may be gone from the saved document:
            // every one of these lists is `skip_serializing_if =
            // "Vec::is_empty"`, and skipping the only element a map
            // had leaves the model with an empty one. Re-create it —
            // without this the acute case of the whole feature (a map
            // whose *single* custom mutation is the one from the
            // future) loses the construct on the very next save.
            restore_empty_list(document, parent);
            let Some(array) = array_at_mut(document, parent) else {
                log::warn!(
                    "loader: {}: the construct this build could not read has nowhere to go \
                     back to — the part of the document it sat in is gone. It is dropped \
                     from the saved file.",
                    entry.location()
                );
                continue;
            };
            let at = (*index).min(array.len());
            array.insert(at, entry.value.clone());
        }
    }
}

/// Put an empty array back at `route` when the saver omitted it.
///
/// Every list a construct can be skipped out of carries
/// `#[serde(skip_serializing_if = "Vec::is_empty")]`, so a map whose
/// only custom mutation was the unreadable one serializes with no
/// `custom_mutations` member at all. Adding the empty list back costs
/// nothing when the model has entries of its own (the member is
/// already there and this does nothing) and is the difference between
/// preserving the construct and losing it when it does not.
fn restore_empty_list(document: &mut Value, route: &[Step]) {
    let Some((Step::Key(name), owner)) = route.split_last() else {
        return;
    };
    let Some(owner) = value_at_mut(document, owner).and_then(Value::as_object_mut) else {
        return;
    };
    if !owner.contains_key(name) {
        owner.insert(name.clone(), Value::Array(Vec::new()));
    }
}

/// [`value_at`], mutably.
fn value_at_mut<'v>(root: &'v mut Value, route: &[Step]) -> Option<&'v mut Value> {
    let mut current = root;
    for step in route {
        current = match (current, step) {
            (Value::Object(members), Step::Key(key)) => members.get_mut(key)?,
            (Value::Array(items), Step::Index(index)) => items.get_mut(*index)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Follow a literal route and hand back the array at the end of it.
fn array_at_mut<'v>(document: &'v mut Value, route: &[Step]) -> Option<&'v mut Vec<Value>> {
    let mut current = document;
    for step in route {
        current = match (current, step) {
            (Value::Object(members), Step::Key(key)) => members.get_mut(key)?,
            (Value::Array(items), Step::Index(index)) => items.get_mut(*index)?,
            _ => return None,
        };
    }
    current.as_array_mut()
}

/// Render route steps as a field path — `sections[0].trigger_bindings[1]`.
fn render_path(steps: &[Step]) -> String {
    let mut out = String::new();
    for step in steps {
        match step {
            Step::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Step::Index(index) => out.push_str(&format!("[{index}]")),
        }
    }
    out
}

/// Every unrecognized key one load found, in the order the
/// deserializer met them.
///
/// Lives on [`MindMap`](crate::mindmap::model::MindMap) so the save path can
/// put the keys back. `#[serde(skip)]` on that field is what keeps
/// this out of the on-disk shape — the keys are written at their own
/// routes, not collected into a side object.
#[derive(Debug, Clone, Default)]
pub struct UnknownKeys {
    entries: Vec<UnknownKey>,
}

impl UnknownKeys {
    /// Whether the load found nothing it did not understand — true for
    /// every map this build authored itself. O(1).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many keys were captured. O(1).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The captured keys, in the order the deserializer met them. O(1).
    pub fn iter(&self) -> std::slice::Iter<'_, UnknownKey> {
        self.entries.iter()
    }

    /// Put every captured key back into a freshly serialized document.
    ///
    /// # What it reconstructs before writing
    ///
    /// A route does not have to survive serialization intact for the
    /// key to. Two things are rebuilt on the way past, and both exist
    /// because without them a key is lost with no user action at all:
    ///
    /// - **A container the saver omitted.** A `skip_serializing_if`
    ///   field holding its own default is absent from the saved
    ///   document, so a route through it has nothing to walk. The load
    ///   recorded the authored container (`Scaffold`); it is put back
    ///   before the key is written into it. This is what makes a
    ///   **zero-edit load → save key-set-preserving**, which is the
    ///   whole point of the mechanism.
    /// - **An array element that moved.** Every positional step
    ///   carries the load-time fingerprints of the array's elements
    ///   (`IndexAnchor`), so deleting or reordering earlier elements
    ///   re-finds the right one instead of silently writing the key
    ///   onto its neighbor.
    ///
    /// # What it refuses, and says so
    ///
    /// - The route no longer resolves — the node, edge, or section the
    ///   key hung off was deleted. The key goes with it.
    /// - The element the key was attached to cannot be identified any
    ///   more: it was edited *and* its siblings changed, so neither its
    ///   fingerprint nor its position is evidence. Refusing is the
    ///   point — writing the key onto a different element would be
    ///   wrong data rather than missing data.
    /// - This build has since grown a field of that name at that
    ///   place. The model owns the name now and the captured value is
    ///   stale.
    ///
    /// Each of the three is a distinct `warn!`, because each asks the
    /// reader to do something different.
    ///
    /// # What is still not preserved, and one thing that is preserved
    /// oddly
    ///
    /// The three refusals above are the whole residue, and every one
    /// of them needs an **edit** to reach: a zero-edit load → save
    /// keeps every authored key, in every position.
    /// `format/schema.md` §"Where a preserved key can still be lost"
    /// states it for the reader and
    /// `loader::tests::test_a_zero_edit_round_trip_keeps_every_authored_key`
    /// is what keeps it honest — table-driven over every position a
    /// key can sit in, because four of them were lost while a
    /// hand-picked case passed.
    ///
    /// The odd one is a key below a `#[serde(from = "…")]` /
    /// `#[serde(into = "…")]` proxy, where what the load reads is not
    /// what the save writes: `CustomMutationIn` accepts a legacy
    /// `mutations` list, `CustomMutationOut` only ever writes the
    /// upgraded `mutator`. Recording the route against the proxy's
    /// own shape would not help — the legacy pair is *folded* into a
    /// synthesized `MutatorNode` and the fold is not invertible, so
    /// there is no position in the written shape that corresponds to
    /// `mutations[i]`. What happens instead is that the omitted
    /// `mutations` list is rebuilt as an ordinary `Scaffold`, so the
    /// saved entry carries the legacy list *alongside* its upgraded
    /// form. `mutator` takes precedence on reload, so the model is
    /// unaffected and the key survives; the cost is that resaving such
    /// an entry no longer erases the legacy spelling. That is the
    /// trade, and it is only paid by an entry that had a preserved key
    /// inside its legacy list.
    ///
    /// Cost: O(captured keys × route length), plus one fingerprint of
    /// each array the routes pass through, plus one `Value` clone per
    /// captured key. Nothing at all when the capture is empty, which
    /// is every map this build wrote.
    pub fn splice_into(&self, document: &mut Value) {
        for entry in &self.entries {
            // Rebuild the omitted container first: until it is there,
            // the key's own route has no owner to land in.
            if let Some(scaffold) = &entry.scaffold {
                if let Some(owner) = resolve_owner(document, &scaffold.owner, &entry.anchors) {
                    if !owner.contains_key(&scaffold.member) {
                        owner.insert(scaffold.member.clone(), scaffold.value.clone());
                    }
                }
            }
            let Some((Step::Key(key), parent)) = entry.route.split_last() else {
                continue;
            };
            let Some(owner) = resolve_owner(document, parent, &entry.anchors) else {
                log::warn!(
                    "loader: {}: unrecognized key `{}` could not be written back — the place \
                     it was loaded from is no longer in the document, or the element it hung \
                     off can no longer be identified. It is dropped from the saved file.",
                    entry.location(),
                    entry.path_within_location()
                );
                continue;
            };
            if owner.contains_key(key) {
                log::warn!(
                    "loader: {}: unrecognized key `{}` was not written back — this build now \
                     writes a key of that name there, and the value it writes wins. The \
                     value read from the file is dropped.",
                    entry.location(),
                    entry.path_within_location()
                );
                continue;
            }
            owner.insert(key.clone(), entry.value.clone());
        }
    }
}

/// Follow a literal route (as [`expand_route`] produced) and hand back
/// the object at the end of it, resolving each positional step against
/// the element identities the load recorded.
///
/// No wrapper skipping and no guessing about object levels: each
/// `Key` step must name a member that is really there. That
/// strictness is the point on the save path — a route that no longer
/// resolves means the document changed shape, and the caller reports
/// that rather than placing the key somewhere plausible.
///
/// Cost: O(route length), plus one fingerprint pass over each array a
/// positional step lands on.
fn resolve_owner<'v>(
    document: &'v mut Value,
    route: &[Step],
    anchors: &[IndexAnchor],
) -> Option<&'v mut Map<String, Value>> {
    let mut current = document;
    for (position, step) in route.iter().enumerate() {
        current = match (current, step) {
            (Value::Object(members), Step::Key(key)) => members.get_mut(key)?,
            (Value::Array(items), Step::Index(index)) => {
                let anchor = anchors.iter().find(|anchor| anchor.step == position);
                let resolved = resolve_index(items, *index, anchor)?;
                items.get_mut(resolved)?
            }
            _ => return None,
        };
    }
    current.as_object_mut()
}

/// Which element of `items` the route's `index` means *now*.
///
/// The recorded index alone is not an answer: deleting an earlier
/// element slides every later one down, and the index then names a
/// different element with no sign that anything went wrong. So the
/// load recorded what the array looked like, and this asks three
/// questions in order, stopping at the first that identifies exactly
/// one element:
///
/// 1. **Is the element still at that index?** The overwhelmingly
///    common answer, and free.
/// 2. **Is it somewhere else in the array?** Reordering
///    (`sections.reverse()`) and deleting a *different* element both
///    land here, and both re-find the right element. Only a unique
///    match counts; two identical elements are not evidence.
/// 3. **Is this array the same array with only that one element
///    edited?** Same length, and every other element byte-identical to
///    what the load saw. Then the element at the index is the same
///    element, changed — editing a section's text must not cost it the
///    keys the section carried.
///
/// Anything else — the element is gone, or it changed *and* its
/// siblings did — is `None`. The caller warns and drops the key.
/// Writing it onto whatever now sits at the index is the one outcome
/// worth avoiding: wrong data reads as authored, missing data does
/// not.
///
/// Without an anchor (a route below a container the model does not
/// serialize, where there was nothing to fingerprint) the index is
/// taken literally, which is all there is to go on.
fn resolve_index(items: &[Value], index: usize, anchor: Option<&IndexAnchor>) -> Option<usize> {
    let Some(anchor) = anchor else {
        return (index < items.len()).then_some(index);
    };
    let want = *anchor.siblings.get(index)?;
    // **The common answer costs one fingerprint, not `items.len()`.**
    // Nothing moved is the overwhelmingly likely case — the array is
    // the one the load saw and the element is still at its index — and
    // deciding it needs only that element. Rendering every sibling
    // first made `splice_into` O(keys x array length).
    //
    // The load side had the identical defect and was fixed by sharing
    // one fingerprint vector per array. That cannot work here:
    // `splice_into` writes keys back as it goes, so a cached
    // fingerprint is stale the moment an element is spliced. Asking a
    // cheaper question first is what works on this side.
    //
    // The full pass below still happens when the fast check fails,
    // which is what the two fallbacks need. That is bounded by how many
    // elements actually moved, not by how many keys the document has.
    if items.get(index).map(fingerprint) == Some(want) {
        return Some(index);
    }
    let now: Vec<u64> = items.iter().map(fingerprint).collect();
    let mut matches = now.iter().enumerate().filter(|(_, seen)| **seen == want);
    if let (Some((only, _)), None) = (matches.next(), matches.next()) {
        return Some(only);
    }
    let only_this_one_changed = now.len() == anchor.siblings.len()
        && now
            .iter()
            .zip(anchor.siblings.iter())
            .enumerate()
            .all(|(position, (seen, was))| position == index || seen == was);
    only_this_one_changed.then_some(index)
}

/// Deserialize `T` from `json`, collecting the route of every key the
/// model had no field for.
///
/// The typed value is produced by the same one-pass parse a load
/// already paid for; the routes come from wrapping that parse so an
/// ignored member reports where it was. A map with nothing unknown in
/// it costs the empty `Vec` and no more — in particular, no second
/// pass over the document.
///
/// Errors are `serde_json`'s own, unchanged, so a caller can still
/// diagnose them.
///
/// Cost: one parse of `json`, plus one `Vec<Step>` per unrecognized
/// key — bounded by [`MAX_UNKNOWN_KEYS`], which is what keeps that
/// second term from being chosen by the document.
pub fn deserialize_capturing<'de, T: Deserialize<'de>>(
    json: &'de str,
) -> Result<(T, Vec<Vec<Step>>), serde_json::Error> {
    let mut routes: Vec<Vec<Step>> = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let value: T =
        serde_ignored::deserialize(&mut deserializer, |path| push_route_capped(&mut routes, &path))?;
    // `serde_json::from_str` does this for us; a hand-driven
    // `Deserializer` has to, or trailing garbage after the closing
    // brace parses clean. `loader::tests::test_trailing_content_after_the_document_is_rejected`
    // is what notices when it goes away.
    deserializer.end()?;
    Ok((value, routes))
}

/// Ceiling on how many unrecognized keys one document may carry, or
/// `None` where the platform imposes none.
///
/// **`None` on native, because the cost is now proportional.** This
/// began at 100 000 with a memory argument: every captured key cost a
/// heap-allocated route, and reaching one committed the load to two
/// full `serde_json::Value` trees — but the real problem was that both
/// the capture and the write-back were *superlinear*. `plan_write_back`
/// built one fingerprint vector per key rather than per array, and
/// `resolve_index` rendered every sibling before asking whether the
/// element had moved. At 6 000 keys the save alone took 67 seconds.
/// A ceiling was the wrong instrument for that; it bounded the input
/// instead of fixing the cost, and it did so at a number chosen against
/// a 252-node fixture.
///
/// Both halves are linear now, so the cost of preserving keys tracks
/// the document that carries them — which is the property a user can
/// reason about, and the one a ceiling never provided. A map with ten
/// million unrecognized keys is a large map, and Mandala opens large
/// maps.
///
/// **`Some` on wasm32**, for the same reason [`MAX_MAP_BYTES`] is: a
/// 32-bit address space is physics. The number is deliberately
/// generous — it is a backstop against exhausting the browser's 4 GiB,
/// not a judgment about how many keys a document should have.
///
/// [`MAX_MAP_BYTES`]: crate::mindmap::loader::MAX_MAP_BYTES
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_UNKNOWN_KEYS: Option<usize> = None;

/// See the native definition above. Sized so the capture cannot by
/// itself exhaust wasm32's 4 GiB address space before the loader can
/// report anything.
#[cfg(target_arch = "wasm32")]
pub const MAX_UNKNOWN_KEYS: Option<usize> = Some(2_000_000);

/// The ceiling in force for this call.
///
/// Reads [`MAX_UNKNOWN_KEYS`] in a shipped build. In a test build it
/// reads a thread-local that defaults to the same value, so a test can
/// drive the **real doors** — `load_from_str`, `parse_for_inspection`
/// and the tolerant path — against a ceiling it can afford.
///
/// The seam exists because the alternative was worse. `MAX_UNKNOWN_KEYS`
/// is `None` on every target the suite runs on, so a test written
/// against the constant asserts nothing and passes, and the arm it
/// stops covering is the browser's. Testing the predicate alone would
/// keep that honest but would no longer catch a door that forgot to
/// call it — which is exactly the defect a review round found on the
/// tolerant path.
fn active_key_ceiling() -> Option<usize> {
    #[cfg(test)]
    {
        TEST_KEY_CEILING.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        MAX_UNKNOWN_KEYS
    }
}

#[cfg(test)]
thread_local! {
    static TEST_KEY_CEILING: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(MAX_UNKNOWN_KEYS) };
}

/// Run `body` with the unknown-key ceiling set to `ceiling`, restoring
/// the previous value afterwards even if `body` panics.
#[cfg(test)]
pub fn with_key_ceiling<T>(ceiling: Option<usize>, body: impl FnOnce() -> T) -> T {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_KEY_CEILING.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(TEST_KEY_CEILING.with(|c| c.get()));
    TEST_KEY_CEILING.with(|c| c.set(ceiling));
    body()
}

/// Push one route unless the active ceiling is already reached.
///
/// Stopping *here* rather than after the walk is the point: the
/// allocation this bounds is `routes` itself, so a check applied to the
/// finished vector would be a check applied after the memory was
/// already spent. One route past the ceiling is kept, and it is what
/// tells the caller the document overflowed —
/// `unknown_key_count_violation` reads `routes.len() > MAX_UNKNOWN_KEYS`
/// and the loader refuses.
fn push_route_capped(routes: &mut Vec<Vec<Step>>, path: &serde_ignored::Path<'_>) {
    if active_key_ceiling().is_none_or(|cap| routes.len() <= cap) {
        routes.push(route_of(path));
    }
}

/// The refusal message for a document over [`MAX_UNKNOWN_KEYS`], or
/// `None` when it is inside the ceiling.
///
/// The count is reported as "more than" rather than exactly, because
/// the capture stops counting at the ceiling — deliberately, since
/// counting them all is the cost being avoided.
pub fn unknown_key_count_violation(routes: &[Vec<Step>]) -> Option<String> {
    count_violation_against(routes.len(), active_key_ceiling())
}

/// [`unknown_key_count_violation`] against a caller-supplied ceiling.
///
/// Split out for the reason `loader::check_text_cap_against` is:
/// `MAX_UNKNOWN_KEYS` is `None` on every target the suite runs on, so a
/// test written against the constant asserts nothing and passes, and
/// the arm it stops covering is the browser's.
pub fn count_violation_against(found: usize, ceiling: Option<usize>) -> Option<String> {
    let cap = ceiling.filter(|cap| found > *cap)?;
    Some({
        format!(
            "map carries more than {cap} unrecognized keys — refusing to load \
             it. Keys this build has no field for are normally kept and saved back \
             untouched, but capturing them costs memory per key, and a document with this \
             many cannot be held in this target's address space."
        )
    })
}

/// Deserialize `T` out of an already-parsed document, collecting the
/// same routes [`deserialize_capturing`] collects.
///
/// The tolerant load path needs this: it excises the constructs this
/// build cannot name from a `serde_json::Value` and then has to read
/// the remainder, which is no longer the text on disk. `&Value` is
/// itself a `Deserializer` and reports ignored members exactly as the
/// streaming one does, so the capture is the same capture — no
/// buffering through serde's `Content`, and therefore no second way
/// for a key to go missing.
///
/// Cost: one walk of `document`, plus one `Vec<Step>` per
/// unrecognized key, under the same [`MAX_UNKNOWN_KEYS`] ceiling
/// [`deserialize_capturing`] applies. Both doors, or the tolerant path
/// would be the way around the bound.
pub fn deserialize_value_capturing<T: serde::de::DeserializeOwned>(
    document: &Value,
) -> Result<(T, Vec<Vec<Step>>), serde_json::Error> {
    let mut routes: Vec<Vec<Step>> = Vec::new();
    let value: T = serde_ignored::deserialize(document, |path| push_route_capped(&mut routes, &path))?;
    Ok((value, routes))
}

/// Lift routes collected by [`deserialize_capturing`] into
/// [`UnknownKeys`], moving each key's value out of `document` and
/// working out what a save will need in order to put it back.
///
/// `document` is the same JSON parsed as a `serde_json::Value`.
/// `probe` is the model's own serialization of what that JSON loaded
/// as — the oracle described in the module header, and `None` only
/// when serializing the model failed, in which case the capture
/// degrades to the routes alone.
///
/// The values are **taken**, not copied, so the captured keys are gone
/// from `document` afterwards — the caller is expected to be done with
/// it. That also matters for correctness: a rebuilt container is
/// snapshotted from `document` *after* every capture has been lifted
/// out, so it cannot write a captured key a second time.
///
/// A route that does not resolve is skipped. That is not reachable
/// from a route this crate produced; it is what keeps a caller
/// pairing a route list with the wrong document out of a panic.
///
/// Cost: O(routes × route length), plus one fingerprint of each array
/// the routes pass through, plus one clone of each container the saver
/// omits.
pub fn take_from(document: &mut Value, probe: Option<&Value>, routes: Vec<Vec<Step>>) -> UnknownKeys {
    // Pass 1 lifts every captured value out. Pass 2 has to run
    // afterwards rather than alongside: a container it snapshots for
    // rebuilding must not still contain a key that will be spliced
    // back separately, or the save writes that key twice — once
    // inside the rebuilt container, once at its own route.
    let mut taken: Vec<(Vec<Step>, Value)> = Vec::with_capacity(routes.len());
    for route in routes {
        let Some(route) = expand_route(document, probe, &route) else {
            continue;
        };
        let Some((Step::Key(key), parent)) = route.split_last() else {
            continue;
        };
        let (key, parent) = (key.clone(), parent.to_vec());
        let Some(value) = resolve_owner(document, &parent, &[]).and_then(|owner| owner.remove(&key)) else {
            continue;
        };
        taken.push((route, value));
    }
    // One cache for the whole pass — the point of it is sharing
    // *across* routes, so it cannot live inside `plan_write_back`.
    let mut arrays: HashMap<Vec<Step>, Arc<Vec<u64>>> = HashMap::new();
    let entries = taken
        .into_iter()
        .map(|(route, value)| {
            let plan = plan_write_back(document, probe, &route, &mut arrays);
            UnknownKey {
                route,
                value,
                scaffold: plan.scaffold,
                anchors: plan.anchors,
            }
        })
        .collect();
    UnknownKeys { entries }
}

/// What [`UnknownKeys::splice_into`] will need in order to write one
/// captured key back where it came from.
struct WriteBackPlan {
    anchors: Vec<IndexAnchor>,
    scaffold: Option<Scaffold>,
}

/// Walk the probe alongside a captured key's route and record what the
/// save will be missing.
///
/// Two things come out of the walk. Every positional step gets an
/// [`IndexAnchor`], so the save can recognize the element rather than
/// trust the index. And the **first** level the probe does not have is
/// a container `skip_serializing_if` leaves out of the saved document
/// — the load records it as a [`Scaffold`] so the save can put it
/// back. Recording only the first is enough: re-inserting the authored
/// container restores everything below it in one move.
///
/// Cost: O(route length). The fingerprint pass over each array on the
/// way is paid **once per array**, not once per key: `siblings` is
/// memoized in `arrays`, keyed by the route prefix that reaches it.
/// Every key under one array produces the identical vector, so before
/// the cache a document with `K` keys inside an `N`-element array paid
/// `K × N` fingerprints — see [`IndexAnchor::siblings`].
fn plan_write_back(
    document: &Value,
    probe: Option<&Value>,
    route: &[Step],
    arrays: &mut HashMap<Vec<Step>, Arc<Vec<u64>>>,
) -> WriteBackPlan {
    let unanchored = WriteBackPlan {
        anchors: Vec::new(),
        scaffold: None,
    };
    let (Some(probe), Some((_, parent))) = (probe, route.split_last()) else {
        return unanchored;
    };
    let mut anchors: Vec<IndexAnchor> = Vec::new();
    let mut current = probe;
    for (position, step) in parent.iter().enumerate() {
        let next = match (current, step) {
            (Value::Object(members), Step::Key(key)) => members.get(key),
            (Value::Array(items), Step::Index(index)) => {
                // Keyed by the prefix that reaches this array, which is
                // what makes two keys under the same array share one
                // vector. The prefix is walked from `probe`, which is
                // immutable for the whole of `take_from`, so the same
                // prefix always names the same array.
                let siblings = match arrays.get(&parent[..position]) {
                    Some(cached) => Arc::clone(cached),
                    None => {
                        let built: Arc<Vec<u64>> = Arc::new(items.iter().map(fingerprint).collect());
                        arrays.insert(parent[..position].to_vec(), Arc::clone(&built));
                        built
                    }
                };
                anchors.push(IndexAnchor {
                    step: position,
                    siblings,
                });
                items.get(*index)
            }
            _ => None,
        };
        let Some(next) = next else {
            // The saver omits this level. Rebuilding it needs a name
            // to hang the authored container off; a positional step
            // has none, and inventing an array element the model does
            // not hold would put a value into the document that
            // nothing in the model stands behind.
            //
            // **Nothing reaches the positional case today.** It needs
            // the probe's array to be shorter than the authored one
            // at the first level the two differ, and no type on the
            // load graph changes a sequence's length across the round
            // trip: a `Vec` field writes one element per element it
            // read, and the constructs the tolerant path excises come
            // out of the authored document too, so both sides shorten
            // together. A `from`/`into` proxy that folds or drops
            // elements, or a hand-written `Serialize` that does, is
            // what would make it live. Until then the key is simply
            // left with no scaffold, and `splice_into`'s "could not
            // be written back" refusal reports it at save — which is
            // where the fact becomes observable at all. There is no
            // load-time flag for it, because a flag no load can set
            // is a promise nobody is keeping.
            let (Step::Key(member), Some(value)) = (step, value_at(document, &route[..=position])) else {
                return WriteBackPlan {
                    anchors,
                    scaffold: None,
                };
            };
            return WriteBackPlan {
                anchors,
                scaffold: Some(Scaffold {
                    owner: route[..position].to_vec(),
                    member: member.clone(),
                    value: value.clone(),
                }),
            };
        };
        current = next;
    }
    WriteBackPlan {
        anchors,
        scaffold: None,
    }
}

/// Follow a literal route through a document without modifying it.
fn value_at<'v>(root: &'v Value, route: &[Step]) -> Option<&'v Value> {
    let mut current = root;
    for step in route {
        current = match (current, step) {
            (Value::Object(members), Step::Key(key)) => members.get(key)?,
            (Value::Array(items), Step::Index(index)) => items.get(*index)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Flatten one `serde_ignored::Path` into a route.
///
/// `Some` / `NewtypeStruct` / `NewtypeVariant` are dropped: serde adds
/// them to mark a layer of the *Rust* type that the JSON does not have
/// a level for.
fn route_of(path: &serde_ignored::Path) -> Vec<Step> {
    fn walk(path: &serde_ignored::Path, out: &mut Vec<Step>) {
        use serde_ignored::Path;
        match path {
            Path::Root => {}
            Path::Seq { parent, index } => {
                walk(parent, out);
                out.push(Step::Index(*index));
            }
            Path::Map { parent, key } => {
                walk(parent, out);
                out.push(Step::Key(key.clone()));
            }
            Path::Some { parent } | Path::NewtypeStruct { parent } | Path::NewtypeVariant { parent } => {
                walk(parent, out)
            }
        }
    }
    let mut out = Vec::new();
    walk(path, &mut out);
    out
}

/// What the next step of a route addresses at the level the walk is
/// standing on.
enum Descend {
    /// The step names a member (or an element) of this very value.
    Member,
    /// This value is a level serde's path elides — an externally
    /// tagged enum's variant wrapper — named by the tag inside.
    Wrapper(String),
    /// Neither; the route does not lead anywhere from here.
    Nowhere,
}

/// Rewrite a route from serde's ignored-key path into one that names
/// every JSON level literally.
///
/// **What has to be put back: externally tagged enum levels.**
/// `MutatorNode::Void` is written `{"Void": { … }}`, and the variant
/// name never travels through a `MapAccess`, so serde's path has one
/// fewer step than the JSON has levels.
///
/// **How the missing level is identified.** Not by counting members —
/// that is a guess, and it guesses wrong on
/// `{"Void": {"channel": 0, "Void": 99}}`, where the payload carries a
/// key spelled like the variant. It is identified by asking the model
/// what *it* writes at that level, via `probe`:
///
/// - **A key the model writes here cannot be the ignored key.** serde
///   only reports members no field claimed, so if the probe has the
///   key at this level, the one serde meant is deeper and this level
///   is a wrapper. That single rule is what makes the collision case
///   come out right.
/// - **A key the model does not write, that the author did, is the
///   member.** Either the unrecognized key itself or a field the saver
///   omits.
/// - Otherwise the level is a wrapper, and the probe has to agree on
///   the tag's spelling for the walk to take it.
///
/// **Below a container the model does not serialize there is no
/// probe**, and the walk falls back to the shape: a one-member object
/// that cannot answer the step is taken as a wrapper. That is the old
/// guess, kept only where nothing better exists, and it is bounded —
/// the containers the saver omits (`format/schema.md`
/// §"Where a preserved key can still be lost") hold no externally
/// tagged enum today, so the fallback resolves plain objects.
///
/// Doing the expansion once, here, is what lets
/// [`UnknownKeys::splice_into`] walk a saved document with no
/// heuristic at all: the stored route is a literal path.
///
/// Recursive, bounded by `route.len()`, which is bounded by the parse
/// that produced it — `serde_json` caps recursion at 128 levels.
///
/// Cost: O(route length), plus one `String` clone per step of the
/// rewritten route.
fn expand_route(document: &Value, probe: Option<&Value>, route: &[Step]) -> Option<Vec<Step>> {
    fn walk<'v>(
        authored: &'v Value,
        model: Option<&Value>,
        route: &[Step],
        out: &mut Vec<Step>,
    ) -> Option<&'v Value> {
        let Some((step, rest)) = route.split_first() else {
            return Some(authored);
        };
        let mut authored = authored;
        let mut model = model;
        loop {
            match descend_kind(authored, model, step, rest.is_empty()) {
                Descend::Member => {
                    let next = child(authored, step)?;
                    out.push(step.clone());
                    return walk(next, model.and_then(|model| child(model, step)), rest, out);
                }
                Descend::Wrapper(tag) => {
                    model = model.and_then(|model| model.get(&tag));
                    authored = authored.get(&tag)?;
                    out.push(Step::Key(tag));
                }
                Descend::Nowhere => return None,
            }
        }
    }
    let mut out = Vec::with_capacity(route.len());
    walk(document, probe, route, &mut out).map(|_| out)
}

/// The child of `value` that `step` addresses, if it is there.
fn child<'v>(value: &'v Value, step: &Step) -> Option<&'v Value> {
    match (value, step) {
        (Value::Object(members), Step::Key(key)) => members.get(key),
        (Value::Array(items), Step::Index(index)) => items.get(*index),
        _ => None,
    }
}

/// Whether `step` addresses something in `authored` itself, or
/// `authored` is a variant wrapper the walk has to descend through
/// first. See [`expand_route`] for why the model decides this rather
/// than the member count.
fn descend_kind(authored: &Value, model: Option<&Value>, step: &Step, last: bool) -> Descend {
    let model_writes_it = match (model, step) {
        (Some(Value::Object(members)), Step::Key(key)) => members.contains_key(key),
        _ => false,
    };
    // A key the model writes is a real member — except at the last
    // step, where it cannot be: serde does not ignore a member a field
    // claimed, so the key it meant is one level down.
    if model_writes_it && !last {
        return Descend::Member;
    }
    if !model_writes_it && child(authored, step).is_some() {
        return Descend::Member;
    }
    wrapper_of(authored, model)
}

/// Read the variant tag off a level serde's path elides.
///
/// The authored value has to be a one-member object — that is what an
/// externally tagged variant is written as. When the model still has
/// this level it must name the same single member, which is what turns
/// the answer from a guess into a check; the walk gives up rather than
/// descend past a level the model disagrees about.
fn wrapper_of(authored: &Value, model: Option<&Value>) -> Descend {
    let Some(members) = authored.as_object() else {
        return Descend::Nowhere;
    };
    if members.len() != 1 {
        return Descend::Nowhere;
    }
    let Some((tag, _)) = members.iter().next() else {
        return Descend::Nowhere;
    };
    match model.and_then(Value::as_object) {
        Some(model_members) if model_members.len() == 1 && model_members.contains_key(tag) => {
            Descend::Wrapper(tag.clone())
        }
        // The model has this level and does not agree that it is a
        // one-member wrapper spelled that way.
        Some(_) => Descend::Nowhere,
        // No model here at all — below a container the saver omits.
        None => Descend::Wrapper(tag.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_logger;

    /// Build a route out of a JSON array literal, so a table of
    /// cases reads as paths rather than as constructor calls.
    fn route(steps: serde_json::Value) -> Vec<Step> {
        steps
            .as_array()
            .expect("route literal")
            .iter()
            .map(|step| match step {
                serde_json::Value::String(key) => Step::Key(key.clone()),
                serde_json::Value::Number(index) => Step::Index(index.as_u64().expect("index") as usize),
                other => panic!("unsupported route step {other}"),
            })
            .collect()
    }

    /// An entry with nothing for the save to rebuild — the shape every
    /// capture had before scaffolds and anchors existed.
    fn plain(steps: serde_json::Value, value: serde_json::Value) -> UnknownKey {
        UnknownKey {
            route: route(steps),
            value,
            scaffold: None,
            anchors: Vec::new(),
        }
    }

    /// A route through an externally tagged enum has one fewer step
    /// than the JSON has levels — `{"Void": {…}}` costs a level that
    /// never travels through a `MapAccess`. The expansion has to put
    /// it back, or every key captured inside a mutator lands nowhere.
    #[test]
    fn test_expand_route_restores_an_external_enum_tag() {
        let document = serde_json::json!({
            "custom_mutations": [{"mutator": {"Void": {"channel": 0, "surprise": 7}}}]
        });
        let probe = serde_json::json!({
            "custom_mutations": [{"mutator": {"Void": {"channel": 0, "children": []}}}]
        });
        let expanded = expand_route(
            &document,
            Some(&probe),
            &route(serde_json::json!(["custom_mutations", 0, "mutator", "surprise"])),
        )
        .expect("the enum payload must be reachable");
        assert_eq!(
            expanded,
            route(serde_json::json!([
                "custom_mutations",
                0,
                "mutator",
                "Void",
                "surprise"
            ]))
        );
    }

    /// **The case the member-count guess got wrong.** An unknown key
    /// spelled like the variant it sits inside made the old test
    /// (`one member, and it isn't the thing I'm looking for`) decide
    /// the wrapper was not a wrapper: it reported a key named after a
    /// variant this build *does* know, at a location that does not
    /// exist, holding the whole payload — and then refused to write it
    /// back. Asking the model settles it: the model writes `Void` at
    /// the wrapper level, so the ignored key cannot be the one there.
    #[test]
    fn test_expand_route_resolves_a_key_spelled_like_its_own_variant() {
        let document = serde_json::json!({
            "custom_mutations": [{"mutator": {"Void": {"channel": 0, "Void": 99}}}]
        });
        let probe = serde_json::json!({
            "custom_mutations": [{"mutator": {"Void": {"channel": 0, "children": []}}}]
        });
        let expanded = expand_route(
            &document,
            Some(&probe),
            &route(serde_json::json!(["custom_mutations", 0, "mutator", "Void"])),
        )
        .expect("the collided key must be reachable");
        assert_eq!(
            expanded,
            route(serde_json::json!([
                "custom_mutations",
                0,
                "mutator",
                "Void",
                "Void"
            ])),
            "the key inside the payload is the unrecognized one, not the variant tag"
        );
    }

    /// **Every variant of every externally tagged enum a load can
    /// reach, not the two somebody tried.**
    ///
    /// The collision that broke the old member-count guess —
    /// a payload key spelled like the enclosing variant — is a
    /// property of the *name*, so the only honest coverage is all the
    /// names. They come from the same source walk the
    /// swallow-a-key drift test uses, so a new variant is covered the
    /// day it is declared and a renamed one cannot leave a stale case
    /// behind.
    ///
    /// Both directions per variant: the ordinary key inside a payload
    /// (which needs the elided tag level put back) and the key spelled
    /// like the variant (which needs the walk to keep going past a
    /// level the model writes).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_every_reachable_variant_name_resolves_both_ways() {
        use crate::util::serde_coverage::{crate_src_root, TypeGraph, TypeKind};

        let graph = TypeGraph::build(&crate_src_root());
        let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for info in graph.reachable_from("MindMap") {
            if info.kind != TypeKind::Enum || !info.derives_deserialize {
                continue;
            }
            names.extend(info.variants.iter().cloned());
        }
        assert!(
            names.len() > 40,
            "the walk found only {} variant names reachable from MindMap, which is \
             too few to be the real model — the table stopped testing anything",
            names.len()
        );

        let mut failures: Vec<String> = Vec::new();
        for variant in &names {
            let probe = serde_json::json!({"field": {variant.clone(): {"known": 0}}});

            let ordinary = serde_json::json!({"field": {variant.clone(): {"known": 0, "zz": 1}}});
            let expanded = expand_route(
                &ordinary,
                Some(&probe),
                &route(serde_json::json!(["field", "zz"])),
            );
            let want = Some(route(serde_json::json!(["field", variant.clone(), "zz"])));
            if expanded != want {
                failures.push(format!(
                    "{variant}: a key inside the payload resolved to {expanded:?}"
                ));
            }

            let collided = serde_json::json!({"field": {variant.clone(): {"known": 0, variant.clone(): 99}}});
            let expanded = expand_route(
                &collided,
                Some(&probe),
                &route(serde_json::json!(["field", variant.clone()])),
            );
            let want = Some(route(serde_json::json!([
                "field",
                variant.clone(),
                variant.clone()
            ])));
            if expanded != want {
                failures.push(format!(
                    "{variant}: a payload key spelled like the variant resolved to {expanded:?}"
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "the route expansion got these variant names wrong — a key reported at \
             one of them would be named, located, valued and written back \
             incorrectly:\n  {}",
            failures.join("\n  ")
        );
    }

    /// The tag skip must not fire on a one-member object that *is*
    /// the addressed thing — a node whose only authored key is the
    /// one being captured.
    #[test]
    fn test_expand_route_prefers_a_real_member_over_a_tag_skip() {
        let document = serde_json::json!({"nodes": {"0": {"mystery": 1}}});
        let steps = route(serde_json::json!(["nodes", "0", "mystery"]));
        assert_eq!(expand_route(&document, None, &steps), Some(steps.clone()));
    }

    /// A route that leads nowhere resolves to nothing rather than to
    /// something near it.
    #[test]
    fn test_expand_route_returns_none_for_a_route_that_is_gone() {
        let document = serde_json::json!({"nodes": {"0": {}}});
        assert!(expand_route(&document, None, &route(serde_json::json!(["nodes", "9", "x"]))).is_none());
    }

    /// The expanded route is literal, so the save-side walk needs no
    /// heuristic — and must not apply one. A one-member object on the
    /// way back is just a one-member object.
    #[test]
    fn test_resolve_owner_takes_a_route_literally() {
        let mut document = serde_json::json!({"canvas": {"background_color": "#000"}});
        let owner = resolve_owner(&mut document, &route(serde_json::json!(["canvas"])), &[])
            .expect("the canvas must be reachable");
        owner.insert("grid_snap".to_string(), serde_json::json!(8));
        assert_eq!(document["canvas"]["grid_snap"], serde_json::json!(8));
    }

    /// Capture takes the key out of the document it was read from, so
    /// nothing downstream sees a half-owned value.
    #[test]
    fn test_take_from_moves_the_value_out_of_the_document() {
        let mut document = serde_json::json!({"nodes": {"0": {"mystery": [1, 2]}}});
        let captured = take_from(
            &mut document,
            None,
            vec![route(serde_json::json!(["nodes", "0", "mystery"]))],
        );
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured.iter().next().expect("one entry").value(),
            &serde_json::json!([1, 2])
        );
        assert_eq!(document, serde_json::json!({"nodes": {"0": {}}}));
    }

    /// **The zero-edit loss, at the unit that causes it.** The saver
    /// omits a container holding its own default, so the route through
    /// it dies with nothing to hold on to. The capture records the
    /// authored container and the splice puts it back.
    #[test]
    fn test_take_from_records_the_container_the_saver_omits() {
        let mut document = serde_json::json!({"nodes": {"0": {"offset": {"x": 0, "grid_hint": 7}}}});
        // The model writes no `offset` at all — it held its default.
        let probe = serde_json::json!({"nodes": {"0": {}}});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["nodes", "0", "offset", "grid_hint"]))],
        );
        let mut saved = serde_json::json!({"nodes": {"0": {}}});
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"nodes": {"0": {"offset": {"x": 0, "grid_hint": 7}}}}),
            "the omitted container has to come back, or the key has nowhere to go"
        );
    }

    /// The rebuilt container is snapshotted *after* the capture lifted
    /// its keys out, so two unknown keys in one omitted container are
    /// each written once rather than one of them twice.
    #[test]
    fn test_two_keys_in_one_omitted_container_are_each_written_once() {
        let mut document = serde_json::json!({"nodes": {"0": {"offset": {"x": 0, "one": 1, "two": 2}}}});
        let probe = serde_json::json!({"nodes": {"0": {}}});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![
                route(serde_json::json!(["nodes", "0", "offset", "one"])),
                route(serde_json::json!(["nodes", "0", "offset", "two"])),
            ],
        );
        let mut saved = serde_json::json!({"nodes": {"0": {}}});
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"nodes": {"0": {"offset": {"x": 0, "one": 1, "two": 2}}}})
        );
    }

    /// A container the model *did* write at load and does not write at
    /// save was emptied by an edit. Rebuilding it from what the author
    /// wrote would resurrect the value the edit removed, so the key is
    /// dropped and said so instead.
    #[test]
    fn test_a_container_an_edit_emptied_is_not_resurrected() {
        let mut document = serde_json::json!({"nodes": {"zz-edit": {"offset": {"x": 5, "hint": 7}}}});
        let probe = serde_json::json!({"nodes": {"zz-edit": {"offset": {"x": 5.0}}}});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["nodes", "zz-edit", "offset", "hint"]))],
        );
        // The user moved the offset back to its default; the saver
        // leaves it out entirely.
        let mut saved = serde_json::json!({"nodes": {"zz-edit": {}}});
        test_logger::install();
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"nodes": {"zz-edit": {}}}),
            "the edit deleted the container; the save must not put the old one back"
        );
        assert_eq!(
            test_logger::lines_containing("zz-edit").len(),
            1,
            "the drop has to be reported"
        );
    }

    /// A key whose place in the document is gone is dropped, and says
    /// so. Silence here would be the one hole in the preservation
    /// guarantee, so it is a `warn!` and a test that the surviving
    /// document is not corrupted around it.
    #[test]
    fn test_splice_skips_a_route_the_document_no_longer_has() {
        let keys = UnknownKeys {
            entries: vec![plain(
                serde_json::json!(["nodes", "zz-gone", "x"]),
                serde_json::json!(1),
            )],
        };
        let mut document = serde_json::json!({"nodes": {"0": {}}});
        test_logger::install();
        keys.splice_into(&mut document);
        assert_eq!(document, serde_json::json!({"nodes": {"0": {}}}));
        let reported = test_logger::lines_containing("zz-gone");
        assert_eq!(reported.len(), 1, "expected one warning, got {reported:?}");
        assert!(
            reported[0].contains("no longer in the document"),
            "the reader has to be told the place is gone: {}",
            reported[0]
        );
    }

    /// A build that grew a field of the captured name owns that name
    /// now; the stale capture must not overwrite what the model just
    /// wrote — and the line that reports it must not claim the place
    /// is missing, because the place is right there.
    #[test]
    fn test_splice_does_not_overwrite_a_key_the_model_now_writes() {
        let keys = UnknownKeys {
            entries: vec![plain(
                serde_json::json!(["canvas", "zz-grid-snap"]),
                serde_json::json!(8),
            )],
        };
        let mut document = serde_json::json!({"canvas": {"zz-grid-snap": 16}});
        test_logger::install();
        keys.splice_into(&mut document);
        assert_eq!(document["canvas"]["zz-grid-snap"], serde_json::json!(16));
        let reported = test_logger::lines_containing("zz-grid-snap");
        assert_eq!(reported.len(), 1, "expected one warning, got {reported:?}");
        assert!(
            reported[0].contains("this build now writes a key of that name"),
            "the two reasons a key is not written back must not share one message: {}",
            reported[0]
        );
    }

    /// **Deleting an edge must not hand its neighbor a key it never
    /// had.** The route is positional, so `edges.remove(0)` slides
    /// every later edge down one; the fingerprints recorded at load
    /// are what re-find the edge the key was actually attached to.
    #[test]
    fn test_a_positional_route_follows_its_element_when_an_earlier_one_is_deleted() {
        let mut document = serde_json::json!({"edges": [
            {"from_id": "0", "to_id": "1"},
            {"from_id": "0", "to_id": "2", "authored_note": "keep me"},
            {"from_id": "0", "to_id": "3"}
        ]});
        let probe = serde_json::json!({"edges": [
            {"from_id": "0", "to_id": "1"},
            {"from_id": "0", "to_id": "2"},
            {"from_id": "0", "to_id": "3"}
        ]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["edges", 1, "authored_note"]))],
        );
        let mut saved = serde_json::json!({"edges": [
            {"from_id": "0", "to_id": "2"},
            {"from_id": "0", "to_id": "3"}
        ]});
        captured.splice_into(&mut saved);
        assert_eq!(
            saved["edges"][0]["authored_note"],
            serde_json::json!("keep me"),
            "the note belongs to edge 0→2 wherever that edge now sits"
        );
        assert_eq!(saved["edges"][1].get("authored_note"), None);
    }

    /// Reordering moves the element without changing it, so the
    /// fingerprint finds it again.
    #[test]
    fn test_a_positional_route_follows_its_element_through_a_reversal() {
        let mut document = serde_json::json!({"sections": [
            {"text": "a", "note": 1},
            {"text": "b"}
        ]});
        let probe = serde_json::json!({"sections": [{"text": "a"}, {"text": "b"}]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["sections", 0, "note"]))],
        );
        let mut saved = serde_json::json!({"sections": [{"text": "b"}, {"text": "a"}]});
        captured.splice_into(&mut saved);
        assert_eq!(saved["sections"][1]["note"], serde_json::json!(1));
        assert_eq!(saved["sections"][0].get("note"), None);
    }

    /// Editing the element the key hangs off must not cost it the key
    /// — that is an ordinary edit, and the element is still the same
    /// element. Recognized by everything around it being untouched.
    #[test]
    fn test_a_positional_route_survives_an_edit_to_its_own_element() {
        let mut document = serde_json::json!({"sections": [
            {"text": "a", "note": 1},
            {"text": "b"}
        ]});
        let probe = serde_json::json!({"sections": [{"text": "a"}, {"text": "b"}]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["sections", 0, "note"]))],
        );
        let mut saved = serde_json::json!({"sections": [{"text": "A"}, {"text": "b"}]});
        captured.splice_into(&mut saved);
        assert_eq!(saved["sections"][0]["note"], serde_json::json!(1));
    }

    /// When the element cannot be identified any more — it changed
    /// *and* the array around it changed — the key is dropped with a
    /// warning rather than written onto whatever now sits at the
    /// index. Wrong data reads as authored; missing data does not.
    #[test]
    fn test_an_unidentifiable_element_is_a_warned_drop_not_a_wrong_write() {
        let mut document = serde_json::json!({"edges": [
            {"from_id": "0", "to_id": "1"},
            {"from_id": "0", "to_id": "2", "zz-note": "keep me"}
        ]});
        let probe = serde_json::json!({"edges": [
            {"from_id": "0", "to_id": "1"},
            {"from_id": "0", "to_id": "2"}
        ]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["edges", 1, "zz-note"]))],
        );
        // Edge 0 deleted and edge 0→2 re-pointed: neither the
        // fingerprint nor the position identifies it.
        let mut saved = serde_json::json!({"edges": [{"from_id": "9", "to_id": "2"}]});
        test_logger::install();
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"edges": [{"from_id": "9", "to_id": "2"}]}),
            "no key may be attached to an element that cannot be identified"
        );
        assert_eq!(test_logger::lines_containing("zz-note").len(), 1);
    }

    /// Two elements that look alike are not evidence. When the
    /// fingerprint matches in more than one place the array cannot say
    /// which one the key came from, and guessing the first is how a
    /// key ends up on the wrong element.
    #[test]
    fn test_two_matching_elements_are_not_an_identification() {
        let mut document = serde_json::json!({"sections": [
            {"text": "a", "zz-twin": 1},
            {"text": "a"},
            {"text": "b"}
        ]});
        let probe = serde_json::json!({"sections": [{"text": "a"}, {"text": "a"}, {"text": "b"}]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["sections", 0, "zz-twin"]))],
        );
        // Element 0 replaced, and the `b` at the end swapped for
        // another `a`: the fingerprint now matches twice and matches
        // nothing at index 0.
        let mut saved = serde_json::json!({"sections": [{"text": "x"}, {"text": "a"}, {"text": "a"}]});
        test_logger::install();
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"sections": [{"text": "x"}, {"text": "a"}, {"text": "a"}]}),
            "an ambiguous match must place nothing"
        );
        assert_eq!(test_logger::lines_containing("zz-twin").len(), 1);
    }

    /// The index is trusted only when everything around it is
    /// untouched, which is what proves the element at it is the same
    /// element. Once a sibling changed too, position is no longer
    /// evidence of identity — an element could have been removed and
    /// another appended — so the key is dropped rather than written
    /// onto whatever is there.
    #[test]
    fn test_the_index_is_not_trusted_once_a_sibling_changed_too() {
        let mut document = serde_json::json!({"sections": [
            {"text": "a", "zz-shifted": 1},
            {"text": "b"}
        ]});
        let probe = serde_json::json!({"sections": [{"text": "a"}, {"text": "b"}]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["sections", 0, "zz-shifted"]))],
        );
        let mut saved = serde_json::json!({"sections": [{"text": "x"}, {"text": "y"}]});
        test_logger::install();
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"sections": [{"text": "x"}, {"text": "y"}]}),
            "neither element is identifiable, so neither may receive the key"
        );
        assert_eq!(test_logger::lines_containing("zz-shifted").len(), 1);
    }

    /// The model has to *agree* that a level is a one-member wrapper
    /// spelled that way. When it does not, the document and the model
    /// have stopped corresponding and the walk stops — descending on
    /// the author's word alone is the guess this replaced.
    #[test]
    fn test_the_walk_stops_where_the_model_disagrees_about_the_wrapper() {
        let document = serde_json::json!({"mutator": {"Void": {"surprise": 1}}});
        let probe = serde_json::json!({"mutator": {"Macro": {"channel": 0}}});
        assert!(
            expand_route(
                &document,
                Some(&probe),
                &route(serde_json::json!(["mutator", "surprise"]))
            )
            .is_none(),
            "the model names a different variant here; the walk has no business \
             descending past a level it cannot corroborate"
        );
    }

    /// The location stamp is what the reader has to open, and each
    /// addressable part is stamped the way `maptool verify` stamps
    /// it.
    #[test]
    fn test_location_names_each_addressable_part() {
        let cases: &[(serde_json::Value, &str, &str)] = &[
            (
                serde_json::json!(["nodes", "1.2", "style", "shpe"]),
                "node \"1.2\"",
                "style.shpe",
            ),
            (
                serde_json::json!(["edges", 3, "arrowhead"]),
                "edge[3]",
                "arrowhead",
            ),
            (
                serde_json::json!(["palettes", "coral", "hue"]),
                "palette \"coral\"",
                "hue",
            ),
            (
                serde_json::json!(["custom_mutations", 0, "flavor"]),
                "custom_mutations[0]",
                "flavor",
            ),
            (serde_json::json!(["canvas", "grid_snap"]), "canvas", "grid_snap"),
            (serde_json::json!(["authors"]), "map", "authors"),
            (
                serde_json::json!(["nodes", "0", "sections", 0, "txet"]),
                "node \"0\"",
                "sections[0].txet",
            ),
        ];
        for (steps, location, within) in cases {
            let entry = plain(steps.clone(), serde_json::Value::Null);
            assert_eq!(&entry.location(), location);
            assert_eq!(&entry.path_within_location(), within);
        }
    }

    /// The load-time line must not promise what the save cannot
    /// deliver, and what it can deliver is the *value* — key order and
    /// number spelling are normalized for the whole document, the
    /// captured key included, because the save renders every member
    /// through the same `serde_json::Value`.
    ///
    /// This test used to have a second half, over an entry the load
    /// had marked unpreservable. That state existed but nothing could
    /// produce it — it was constructed here by hand — so the line it
    /// selected was a wording no reader could ever be shown. Both are
    /// gone; `plan_write_back` records what would have to change for
    /// the case to become real.
    #[test]
    fn test_the_warning_promises_the_value_and_not_the_bytes() {
        let kept = plain(serde_json::json!(["canvas", "grid_snap"]), serde_json::json!(1));
        assert!(
            kept.warning()
                .contains("saved back with the value it was authored with"),
            "got: {}",
            kept.warning()
        );
        assert!(
            !kept.warning().contains("unchanged"),
            "the save re-renders the whole document, so it cannot promise the bytes: {}",
            kept.warning()
        );
    }

    /// **The derivation itself, held to a second one that shares
    /// nothing with it.**
    ///
    /// `test_the_published_positional_arrays_are_the_ones_the_model_has`
    /// below compares the document against the source walk, which
    /// catches a document nobody updated and cannot catch a *walk*
    /// that is wrong — the document was written by reading the same
    /// model the same way, so the two agree on being wrong and the
    /// comparison certifies the agreement. Three of the sixteen names
    /// in that block were missing for exactly that reason:
    /// `ColorFontRegions::regions` is a `BTreeSet` and the walk
    /// matched the token `Vec`; `MutationListSrc::Literal` and
    /// `GlyphModelField::GlyphLines` are newtype-variant payloads and
    /// the walk skipped fields with no name. All three are live
    /// arrays in today's model.
    ///
    /// So the walk needs something that can disagree with it, and it
    /// has to get its answer from somewhere else. `serde_derive`
    /// already wrote the answer down: the impl it generates for a
    /// struct hands `deserialize_struct` the exact list of JSON
    /// member names, and a growable array asks for `deserialize_seq`
    /// whatever it is spelled as. [`crate::util::serde_probe`] drives
    /// those generated impls with a `Deserializer` that records what
    /// they ask for. No source text, no `syn`, no list — the two
    /// derivations share only the model they are both about.
    ///
    /// A disagreement here is not automatically a bug in the walk.
    /// It is a shape one reader can see and the other cannot, and
    /// which one is right is a decision — which is the whole
    /// improvement over a silence nobody can question.
    #[test]
    fn test_the_derived_positional_arrays_survive_an_independent_derivation() {
        use crate::util::serde_coverage::{crate_src_root, TypeGraph};
        use crate::util::serde_probe::derived_shape;

        let graph = TypeGraph::build(&crate_src_root());
        let scanned = graph.key_bearing_sequences_from("MindMap");
        let shape = derived_shape::<crate::mindmap::model::MindMap>();
        let derived = shape.key_bearing_sequences();

        assert!(
            scanned.len() > 10,
            "the source walk found {} array(s), which is too few for this model to have \
             lost honestly — the derivation broke rather than the model: {scanned:?}",
            scanned.len()
        );
        let unseen_by_the_probe: Vec<&String> = scanned.difference(&derived).collect();
        let unseen_by_the_walk: Vec<&String> = derived.difference(&scanned).collect();
        assert!(
            unseen_by_the_probe.is_empty() && unseen_by_the_walk.is_empty(),
            "the two derivations of the positional-array set disagree.\n  \
             the source walk has, the generated impls do not: {unseen_by_the_probe:?}\n  \
             the generated impls have, the source walk does not: {unseen_by_the_walk:?}\n\n\
             The generated impls are the on-disk truth about *shape* — including every \
             `#[serde(alias = \"…\")]` spelling, which the derive writes into the same \
             `FIELDS` and `VARIANTS` lists as the name it aliases. A name the walk has \
             and the probe does not is therefore a shape \
             `util::serde_coverage::TypeGraph::sequence_element` has classified wrongly. \
             A name the probe has and the walk does not is the walk failing to see an \
             array, which is the defect issue #122 exists for."
        );

        assert!(
            graph.unnamed_sequences_from("MindMap").is_empty() && shape.unnamed_sequences().is_empty(),
            "the model has grown an array at a place with no JSON member name — a tuple \
             position, the value of a map whose keys are data, or an array nested \
             directly inside another one, as `Vec<Vec<T>>` is. Only the probe can see \
             that last shape: the walk classifies *fields*, and a nested array is one \
             field, so an empty answer from it is not evidence. A captured key's route \
             crosses whichever it is positionally and `format/schema.md`'s \
             one-name-per-line block has no way to name it, so what the document should \
             say is a decision somebody has to make:\n  walk: {:?}\n  generated impls: \
             {:?}",
            graph.unnamed_sequences_from("MindMap"),
            shape.unnamed_sequences()
        );

        // Every member below which the probe stops seeing, held to
        // the two the model means opaque. `MindMap::macros` and
        // `MindNode::inline_macros` are the crate's only
        // `Vec<serde_json::Value>` fields, deliberately so — nothing
        // inside one is ever captured — and a self-describing value
        // is the only thing here that asks a deserializer to decide
        // for itself. An `#[serde(untagged)]` enum asks the same way
        // and would otherwise vanish in silence, which is the one
        // blind spot in this derivation that raises no error of its
        // own.
        let expected: std::collections::BTreeSet<String> = ["MindMap.macros[0]", "MindNode.inline_macros[0]"]
            .iter()
            .map(|site| (*site).to_string())
            .collect();
        assert_eq!(
            shape.opaque_sites(),
            &expected,
            "the probe answered a `deserialize_any` somewhere new. Either a member became \
             deliberately opaque — a `serde_json::Value`, in which case add it here — or \
             a type on the load path grew `#[serde(untagged)]`, which decides its variant \
             by replaying a buffered `Content` through a deserializer the loader's capture \
             is not part of, and drops every key no variant claimed. \
             `loader::tests::test_no_loadable_type_can_swallow_an_unknown_key` refuses that \
             attribute; this is the same refusal arrived at from the compiler's side."
        );
    }

    /// **What the model's members are called on disk, from the
    /// compiler.**
    ///
    /// The array list is published under member names, and the walk
    /// that produces them reads `#[serde(rename …)]` out of source
    /// text. It read exactly one of the four spellings serde accepts
    /// until #122 — `rename = "…"` — so a field carrying
    /// `rename(deserialize = "points")` published the Rust
    /// identifier, and a container `rename_all` published every
    /// field's identifier. Neither is exercised by the model today,
    /// which is why nothing failed and why the rule is checked here
    /// rather than left to the first field that uses one.
    ///
    /// `serde_derive`'s `FIELDS` list is the same question already
    /// answered by the code that will actually do the reading, so
    /// this holds every struct the walk reaches against it. The
    /// witnesses are named because a comparison over an empty
    /// intersection passes just as loudly as one that agrees.
    #[test]
    fn test_the_derived_member_names_are_the_ones_the_compiler_generated() {
        use crate::util::serde_coverage::{crate_src_root, TypeGraph};
        use crate::util::serde_probe::derived_shape;

        let scanned = TypeGraph::build(&crate_src_root()).member_names_from("MindMap");
        let derived = derived_shape::<crate::mindmap::model::MindMap>();

        let mut compared: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let mut disagreements = Vec::new();
        for (container, walked) in &scanned {
            // Struct variants are recorded by the probe under
            // `Enum::Variant` and by the walk under the enum, so an
            // enum's members are pooled before they are compared.
            let generated: std::collections::BTreeSet<String> = derived
                .containers()
                .filter(|name| {
                    *name == container || name.split_once("::").is_some_and(|(owner, _)| owner == container)
                })
                .filter_map(|name| derived.members_of(name))
                .flatten()
                .cloned()
                .collect();
            // A type no value reaches — a serialize-only proxy, or one
            // behind a variant the sweep proved unreachable — has no
            // generated answer to compare against.
            if generated.is_empty() {
                continue;
            }
            compared.insert(container.as_str());
            if walked != &generated {
                disagreements.push(format!("{container}: walk {walked:?} vs generated {generated:?}"));
            }
        }

        assert!(
            disagreements.is_empty(),
            "the source walk and the generated `Deserialize` impls disagree about what \
             {} type(s) call their members on disk. The generated list is what the \
             loader will actually match against, so the walk is wrong wherever they \
             differ — teach `util::serde_coverage`'s name reader the spelling rather \
             than adjusting the expectation:\n  {}",
            disagreements.len(),
            disagreements.join("\n  ")
        );
        assert!(
            compared.len() > 20,
            "only {} type(s) were compared, which is too few for this model — the walk \
             and the probe have stopped meeting, and a check with nothing in its \
             intersection passes for the wrong reason",
            compared.len()
        );
        // Named rather than counted, because the count above is
        // satisfied by any twenty types: a comparison that quietly
        // stopped covering `MindNode` would still pass it. Each of
        // these carries members on both sides today, so a witness
        // that drops out of `compared` means one of the two
        // derivations stopped reaching it.
        for witness in ["MindNode", "MindEdge", "MindSection", "CustomMutationIn"] {
            assert!(
                compared.contains(witness),
                "`{witness}` was not compared — either the walk stopped reaching it or \
                 no value of it reaches the probe, and it is one of the types this \
                 check exists to cover: compared {compared:?}"
            );
        }
        assert!(
            scanned
                .get("MindEdge")
                .is_some_and(|names| names.contains("type") && !names.contains("edge_type")),
            "`MindEdge::edge_type` is written `type`, and the true positive that proves \
             this comparison is about the *file's* names rather than the model's"
        );
    }

    /// **A variant name is a JSON member name too**, and the walk was
    /// reading it off the identifier.
    ///
    /// An externally tagged enum writes its variant as the single
    /// member of a wrapper object, and
    /// `test_every_reachable_variant_name_resolves_both_ways` drives
    /// `expand_route` over every one of them — so a variant that
    /// renamed itself, or an enum carrying `rename_all`, would send
    /// that test through a JSON level no document contains, and it
    /// would pass, because `expand_route` would be asked about the
    /// same wrong name it was handed. Nothing in the model renames a
    /// variant today, which is why the identifier and the member name
    /// coincide and why this has to be checked rather than observed.
    ///
    /// `serde_derive` hands `deserialize_enum` its `VARIANTS` list
    /// with every rename already applied, which is the same answer
    /// arrived at without reading a line of source.
    #[test]
    fn test_the_derived_variant_names_are_the_ones_the_compiler_generated() {
        use crate::util::serde_coverage::{crate_src_root, TypeGraph, TypeKind};
        use crate::util::serde_probe::derived_shape;

        let graph = TypeGraph::build(&crate_src_root());
        let derived = derived_shape::<crate::mindmap::model::MindMap>();

        let mut compared = 0usize;
        let mut disagreements = Vec::new();
        for info in graph.reachable_from("MindMap") {
            if info.kind != TypeKind::Enum {
                continue;
            }
            let Some(generated) = derived.variants_of(&info.name) else {
                continue;
            };
            compared += 1;
            if info.variants != generated {
                disagreements.push(format!(
                    "{}: walk {:?} vs generated {:?}",
                    info.name, info.variants, generated
                ));
            }
        }

        assert!(
            disagreements.is_empty(),
            "the source walk and the generated `Deserialize` impls disagree about what \
             {} enum(s) call their variants on disk, including the order — a variant is \
             the wrapper member an externally tagged payload sits under, so the \
             generated list is what a document actually spells:\n  {}",
            disagreements.len(),
            disagreements.join("\n  ")
        );
        assert!(
            compared > 10,
            "only {compared} enum(s) were compared, which is too few for this model — a \
             comparison with nothing in its intersection passes for the wrong reason"
        );
    }

    /// **The one list of arrays, held to the model in both
    /// directions.** `format/schema.md` publishes where a captured
    /// key's route turns positional, because that is where an edit
    /// can move it and where the three save-time refusals live. It
    /// published a hand-written version of that list first, and the
    /// list had already drifted: `edges[i].control_points` and
    /// `palettes.<name>.groups` both hold objects, both carry
    /// positional routes, and neither was on it — the twin surface
    /// `lib/baumhard/CONVENTIONS.md` §B4 is about, in a doc rather
    /// than in code.
    ///
    /// So the set comes off the model's own source now, the doc
    /// publishes it, and this compares the two. Both directions
    /// matter: an array the doc omits is a place a reader does not
    /// know to be careful about, and an array the doc names that the
    /// model no longer has is a claim that stopped being true.
    ///
    /// It cannot, on its own, catch a *derivation* that is wrong —
    /// the document is written by reading the walk, so the two agree
    /// however wrong the walk is. That is
    /// `test_the_derived_positional_arrays_survive_an_independent_derivation`'s
    /// job, above.
    ///
    /// The discriminator is **"can a key be captured at or below an
    /// element"**, not "is a `Vec`": `macros` and `inline_macros` are
    /// `Vec<Value>`, deliberately opaque, so no member of one is ever
    /// unrecognized and no route crosses their indexes. The doc says
    /// that in prose next to the list.
    #[test]
    fn test_the_published_positional_arrays_are_the_ones_the_model_has() {
        use crate::util::doc_fixtures::{documented_plain_block, format_doc_path};
        use crate::util::serde_coverage::{crate_src_root, TypeGraph};

        let derived = TypeGraph::build(&crate_src_root()).key_bearing_sequences_from("MindMap");
        assert!(
            !derived.is_empty(),
            "the walk found no positional array at all — the model cannot have lost \
             every one of them, so the derivation broke rather than the model"
        );

        let doc = format_doc_path("schema.md");
        let published: std::collections::BTreeSet<String> =
            documented_plain_block(&doc, "## Unknown keys are kept", 1)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();

        let missing: Vec<&String> = derived.difference(&published).collect();
        let stale: Vec<&String> = published.difference(&derived).collect();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "format/schema.md §\"Unknown keys are kept\" publishes the arrays a captured \
             key's route can cross, and it no longer matches the model.\n  \
             the model has, the doc does not list: {missing:?}\n  \
             the doc lists, the model does not have: {stale:?}\n\n\
             The block is one name per line, sorted. An array whose elements cannot hold \
             an unrecognized key — `Vec<Value>`, `Vec<String>` — is correctly absent; if \
             one of those is what showed up here, the derivation is what to look at."
        );
    }

    /// **The anchor vector is shared, and that is a memory bound, not
    /// a tidiness preference.**
    ///
    /// `plan_write_back` records an `IndexAnchor` per array on a
    /// captured key's route, and its `siblings` fingerprints *every*
    /// element of that array. Built per key, `K` keys inside an
    /// `N`-element array cost `K x N` fingerprints to compute and
    /// `K x N` `u64`s to keep — and `fingerprint` renders each element
    /// with `to_string()` first, so the CPU cost is quadratic too.
    /// `MAX_UNKNOWN_KEYS` bounds `K` and does nothing about the
    /// product, which is why the ceiling alone was not the bound its
    /// commit claimed. Measured through `maptool grep` with one unknown
    /// key per element, before the fix: 2 000 keys / 404 KB reached
    /// 46 MiB, 6 000 / 1.2 MB reached 311 MiB, 12 000 / 2.4 MB did not
    /// finish in two minutes. After: 15, 36 and 68 MiB.
    ///
    /// Asserting on `Arc::ptr_eq` rather than on equal contents is the
    /// point — the contents were always equal; it is the *allocation*
    /// that used to be per key.
    #[test]
    fn test_one_fingerprint_vector_is_shared_by_every_key_under_an_array() {
        const ELEMENTS: usize = 12;
        let items: Vec<Value> = (0..ELEMENTS)
            .map(|i| serde_json::json!({ "known": i, format!("u{i}"): i }))
            .collect();
        let mut document = serde_json::json!({ "list": items });
        // The probe is what the walk reads: same shape, minus the keys
        // the model had no field for.
        let probe = serde_json::json!({
            "list": (0..ELEMENTS).map(|i| serde_json::json!({ "known": i })).collect::<Vec<_>>()
        });
        let routes: Vec<Vec<Step>> = (0..ELEMENTS)
            .map(|i| {
                vec![
                    Step::Key("list".into()),
                    Step::Index(i),
                    Step::Key(format!("u{i}")),
                ]
            })
            .collect();

        let captured = take_from(&mut document, Some(&probe), routes);
        assert_eq!(captured.entries.len(), ELEMENTS, "every key must be captured");

        let first = captured.entries[0]
            .anchors
            .first()
            .expect("a key inside an array carries an anchor")
            .siblings
            .clone();
        assert_eq!(first.len(), ELEMENTS, "the anchor fingerprints every sibling");
        for (i, entry) in captured.entries.iter().enumerate() {
            let anchor = entry.anchors.first().expect("each key sits under the same array");
            assert!(
                Arc::ptr_eq(&anchor.siblings, &first),
                "entry {i} built its own fingerprint vector — the capture is quadratic again"
            );
        }
        // Guards the assertions above against passing on a single entry.
        // A compile-time check: shrinking `ELEMENTS` to 1 is a build error
        // rather than a test that quietly stops testing anything.
        const { assert!(ELEMENTS > 1) };
    }

    /// **Saving must stay linear in the number of preserved keys.**
    ///
    /// `resolve_index` answers "which element does this route mean
    /// now?", and its first question — is the element still where it
    /// was — needs one fingerprint. Rendering every sibling before
    /// asking made `splice_into` O(keys x array length). Measured
    /// through `to_json_value` on a map with one unknown key per array
    /// element: 2 000 keys took 5.39 s, 6 000 took 67.4 s, and 12 000
    /// did not finish in two minutes. After: 6.4 ms, 28.2 ms, 62.2 ms.
    ///
    /// The load side had the identical defect and was fixed by sharing
    /// one fingerprint vector per array. That cannot work here:
    /// `splice_into` writes keys back as it goes, so a cached
    /// fingerprint is stale the moment an element is spliced. Asking a
    /// cheaper question first is what works on this side.
    ///
    /// This asserts a duration, which every other pin on this branch
    /// deliberately avoids. It is justified by the margin rather than
    /// by the number: the linear form is ~25 ms and the quadratic one
    /// ~20 s, so the bound below sits two orders of magnitude above the
    /// good case and two below the bad. A source pin would be steadier
    /// but would only say "the fast check is present", not "the cost is
    /// linear", and the cost is the property.
    #[test]
    fn test_splicing_keys_back_is_linear_in_their_count() {
        const N: usize = 4000;
        let items: Vec<Value> = (0..N)
            .map(|i| serde_json::json!({ "known": i, format!("u{i}"): i }))
            .collect();
        let mut document = serde_json::json!({ "list": items });
        let probe = serde_json::json!({
            "list": (0..N).map(|i| serde_json::json!({ "known": i })).collect::<Vec<_>>()
        });
        let routes: Vec<Vec<Step>> = (0..N)
            .map(|i| {
                vec![
                    Step::Key("list".into()),
                    Step::Index(i),
                    Step::Key(format!("u{i}")),
                ]
            })
            .collect();
        let captured = take_from(&mut document, Some(&probe), routes);
        assert_eq!(captured.entries.len(), N, "every key must be captured");

        let started = std::time::Instant::now();
        captured.splice_into(&mut document);
        let elapsed = started.elapsed();

        // Every key really went back — otherwise this times an early exit.
        for i in 0..N {
            assert!(
                document["list"][i].get(format!("u{i}")).is_some(),
                "key u{i} was not spliced back"
            );
        }
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "splicing {N} keys took {elapsed:?}; linear is tens of milliseconds and the \
             quadratic form this guards against is tens of seconds"
        );
    }
}
