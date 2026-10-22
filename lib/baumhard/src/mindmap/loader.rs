// SPDX-License-Identifier: MPL-2.0

//! `.mindmap.json` loader + saver.
//!
//! **The loader never drops what it does not understand, and never
//! refuses a document for carrying it.** A map authored by a newer
//! build has keys this one has no field for. Refusing the load leaves
//! the reader with an empty window; ignoring the keys is worse,
//! because the editor resaves the whole model and the next save
//! deletes them. So the load keeps them: every unrecognized key is
//! captured with the route back to where it sat, warned about once,
//! carried on
//! [`MindMap::unknown_keys`](crate::mindmap::model::MindMap::unknown_keys),
//! and written back untouched at
//! save. See [`crate::mindmap::unknown_keys`] for the mechanism and
//! why it is not a per-type `#[serde(flatten)]` catch-all.
//!
//! What the model owes that mechanism is only that no type between the
//! document root and a key can absorb it first — no
//! `deny_unknown_fields`, no `flatten`, no `untagged` or `tag`. That is
//! enforced against the model that exists, not a list of it, by
//! `tests::test_no_loadable_type_can_swallow_an_unknown_key`.
//!
//! **Three shapes are still refused, and all three are pre-refactor
//! spellings with a migration verb**: a top-level `portals[]`, per-node
//! `text` / `text_runs`, and a `sections` that is not an array. Those
//! are not keys from the future — they are keys from the past that the
//! current model means something else by, so preserving them would
//! carry a contradiction forward. Each gets a concrete `maptool convert
//! ...` pointer.
//!
//! A successful load parses the document exactly once. A map that does
//! carry unknown keys pays one more pass, to lift their values out;
//! everything else expensive lives on the failure path, where the raw
//! JSON is re-examined only to explain a parse that already failed.

use crate::mindmap::custom_mutation::{CustomMutation, TriggerBinding};
use crate::mindmap::model::{validate, Canvas, MindEdge, MindMap, MindNode, Palette};
use crate::mindmap::unknown_keys::{self, Step};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Ceiling on the on-disk size of a `.mindmap.json`, in bytes, or
/// `None` where the platform imposes none.
///
/// **`None` on native, and that is the design rather than an
/// omission.** Mandala is meant to open maps as large as the machine
/// can hold — the repository's own stress generator already emits one
/// the previous 256 MiB ceiling refused — so "open this map" *is* an
/// unconditional promise to allocate what the file asks for. The user
/// picked the file; a 10 GB map costing 10 GB is the feature, and a
/// failed allocation is an honest outcome of a choice they made.
///
/// The previous ceiling was justified by "far past any authored map —
/// the canonical fixture is 545 KB with 252 nodes". That reasoning
/// rested on an assumption about how large maps get, and the
/// assumption was wrong. A limit defended by the size of today's
/// fixtures is a limit that expires.
///
/// **`Some` on wasm32, because there the limit is physics.** `usize`
/// is 32 bits and the address space is 4 GiB, shared with the runtime,
/// the GPU buffers and the browser itself. A file that cannot fit is
/// refused with a message rather than trapping mid-parse. This is a
/// sanctioned target divergence, registered in `CLAUDE.md`'s
/// dual-target section.
///
/// What is *not* bounded by any of this, and is documented at
/// [`load_from_str`] instead, is the ratio: peak memory is a multiple
/// of the file, roughly 4 KB per node and 2 KB per section on top of
/// the text. Predicting cost from file size is what a user needs, and
/// a ceiling never provided it.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_MAP_BYTES: Option<u64> = None;

/// See the native definition above for the rationale. 3 GiB leaves
/// headroom inside wasm32's 4 GiB address space for the typed model
/// the text is parsed into, the scene tree built from it, and the
/// GPU-side buffers — none of which the file length counts.
#[cfg(target_arch = "wasm32")]
pub const MAX_MAP_BYTES: Option<u64> = Some(3 * 1024 * 1024 * 1024);

/// Load a `MindMap` from a file path. Reads the entire file into
/// memory via `std::fs::read_to_string`, then delegates to
/// [`load_from_str`]. Native-only (synchronous I/O). Returns a
/// `String` error describing the path + underlying cause.
///
/// Cost: one filesystem read (latency-bound, plus an allocation
/// sized to the file's UTF-8 length) followed by [`load_from_str`]'s
/// JSON parse — O(file_size) overall. Felt every map load.
pub fn load_from_file(path: &Path) -> Result<MindMap, String> {
    load_from_str(&read_capped(path)?)
}

/// Read `path` to a `String`, refusing anything over
/// [`MAX_MAP_BYTES`].
///
/// **Stat before read.** `read_to_string` sizes its buffer from the
/// file's length, so an oversized map is an allocation the process
/// has already committed to by the time any parser gets a say — and
/// the typed model built on top costs several times the text again.
/// Checking first turns an OOM kill into a sentence, which is the
/// whole difference between "this file is broken" and "the editor
/// died".
///
/// A file whose metadata cannot be read falls through to the read
/// rather than failing here: the read reports the real error
/// (missing, unreadable) far better than a guess would.
fn read_capped(path: &Path) -> Result<String, String> {
    if let Ok(meta) = fs::metadata(path) {
        if let Some(cap) = MAX_MAP_BYTES.filter(|cap| meta.len() > *cap) {
            return Err(format!(
                "{} is {} bytes, over the {} byte map limit — refusing to read it. \
                 A map this size is either damaged or hostile.",
                path.display(),
                meta.len(),
                cap
            ));
        }
    }
    fs::read_to_string(path).map_err(|e| format!("Failed to read file {}: {}", path.display(), e))
}

/// Parse a `MindMap` from a JSON string.
///
/// A key the model does not know is **kept**, not dropped and not
/// grounds for refusing the document — see the module header for why
/// that is what lets an older build open a newer map. Each one is
/// warned about once, naming the node / edge / palette it sits in, and
/// travels on [`MindMap::unknown_keys`] to the next save. Pre-refactor
/// shapes (a top-level `portals[]`, per-node `text` / `text_runs`) are
/// the exception and still fail, with a concrete `maptool convert ...`
/// pointer.
///
/// Cost on the happy path: **exactly one parse of `json`** for a map
/// with nothing unknown in it, which is every map this build wrote.
/// The capture rides that parse rather than adding one.
///
/// Cost when the map *does* carry unknown keys: one more parse, into a
/// `serde_json::Value`, to lift the captured values out and to answer
/// the legacy-shape question — the three legacy spellings are
/// themselves unrecognized keys, so nothing looks for them until the
/// capture reports something. The extra pass is paid by the documents
/// that need it and by no others.
///
/// Cost on the failure path: roughly **2× a successful load**, and
/// worth naming because it is more than the extra `Value` parse it
/// looks like. `diagnose_rejected_json` pays that parse *plus*
/// `locate_typed_failure`, which re-deserializes the canvas, then
/// every palette, then every node, then every edge, then every custom
/// mutation, stopping at the first that fails. When the offending part
/// is the last node, that second stage has typed the whole document a
/// second time. The trade is deliberate: the load is not going to
/// complete, and a node id beats a byte offset into a 545 KB file.
pub fn load_from_str(json: &str) -> Result<MindMap, String> {
    // `json` deliberately ends at the parse. `check_invariants` is
    // not given it, so no future invariant can quietly reintroduce a
    // second pass over the document text.
    check_invariants(parse_for_inspection(json)?)
}

/// [`MAX_MAP_BYTES`], enforced against text already in memory.
///
/// The file-path loader stats before it reads, which is the cheaper
/// and better check — but it is not the only door. **The browser
/// never touches it**: the WASM build receives its map as a string
/// (`?map=`) and goes straight to [`load_from_str`], so a ceiling
/// that lived only on the filesystem path left the target with the
/// smaller memory budget entirely unguarded. The same is true of
/// any future transport — a paste, a socket, a fetch.
///
/// The text is already allocated by the time this runs, so this
/// bounds the *typed model* built on top of it rather than the read
/// itself. That is still the larger of the two costs.
fn check_text_cap(json: &str) -> Result<(), String> {
    check_text_cap_against(json, MAX_MAP_BYTES)
}

/// [`check_text_cap`] against a caller-supplied ceiling.
///
/// Split out so the refusal stays testable. `MAX_MAP_BYTES` is `None`
/// on every target the test suite runs on, so a test written against
/// the constant would assert nothing and pass — and the arm it stopped
/// covering is the one the browser depends on. This takes the ceiling
/// as a parameter, so a native test can hand it a small one and drive
/// the same code the wasm32 build reaches with a real one.
fn check_text_cap_against(json: &str, ceiling: Option<u64>) -> Result<(), String> {
    if let Some(cap) = ceiling.filter(|cap| json.len() as u64 > *cap) {
        return Err(format!(
            "map is {} bytes, over the {} byte limit — refusing to load it. \
             A map this size is either damaged or hostile.",
            json.len(),
            cap
        ));
    }
    Ok(())
}

/// Parse a `MindMap` with the *shape* checks but **without** the
/// load-time invariants, for tooling that has to inspect a map the
/// editor refuses to open.
///
/// [`load_from_str`] is the editor's front door and is deliberately
/// strict: a parent cycle, a `nodes` key that disagrees with its
/// node's `id`, a font size the text shaper asserts on — none of
/// those open, because rendering them takes the process down. A
/// *diagnostic* tool has the opposite need. The maps worth
/// inspecting are exactly the broken ones, and a verifier that
/// could only read files already passing the gate would fall silent
/// precisely when it is wanted — reporting "cannot load" where the
/// user asked *what is wrong with it*.
///
/// So this keeps everything that decides what the document *is* —
/// serde's typed parse, the closed-object rejection, the
/// legacy-shape migration pointers — and drops only the checks
/// about whether it is safe to render. The returned model may
/// therefore hold a cycle, non-finite geometry, or a zero font
/// size: **do not build a scene from it.** `maptool verify` is the
/// intended caller.
///
/// Cost: identical to a successful [`load_from_str`] minus the
/// invariant sweep — one parse of `json`.
pub fn parse_for_inspection(json: &str) -> Result<MindMap, String> {
    check_text_cap(json)?;
    let (mut map, routes): (MindMap, Vec<Vec<Step>>) = match unknown_keys::deserialize_capturing(json) {
        Ok(parsed) => parsed,
        Err(e) => {
            // A construct from a newer build must not take the
            // whole document down with it — see
            // `load_skipping_unreadable_constructs`.
            return load_skipping_unreadable_constructs(json)
                .unwrap_or_else(|| Err(diagnose_rejected_json(json, &e)));
        }
    };
    // Refused *before* `adopt_unknown_keys`, which is where the two
    // full `Value` trees are built — a count check made after them
    // would be made after the memory was already spent.
    if let Some(err) = unknown_keys::unknown_key_count_violation(&routes) {
        return Err(err);
    }
    if !routes.is_empty() {
        map.unknown_keys = adopt_unknown_keys(json, &map, routes)?;
    }
    Ok(map)
}

/// [`parse_for_inspection`] from a file path, with the same
/// [`MAX_MAP_BYTES`] ceiling [`load_from_file`] applies — an
/// inspection tool still has to read the bytes, so it inherits the
/// same commitment.
pub fn parse_file_for_inspection(path: &Path) -> Result<MindMap, String> {
    let content = read_capped(path)?;
    parse_for_inspection(&content)
}

/// Turn the routes a capturing parse collected into the
/// [`UnknownKeys`](crate::mindmap::unknown_keys::UnknownKeys) the map
/// carries, warning once per key — or refuse the document when what
/// the capture found is a pre-refactor spelling with a migration verb.
///
/// The second parse is what makes the values available: the capture
/// rides the typed parse and learns *where* each key was, not what it
/// held. Reaching this at all means the map has at least one
/// unrecognized key, so the parse is never paid by a document that
/// does not need it.
///
/// Re-parsing cannot fail — `json` has already parsed once, as a
/// stricter shape — but the failure is reported rather than
/// `expect`ed, because `load_from_str` sits on the interactive
/// `console open` path (CODE_CONVENTIONS §9).
///
/// **The model's own serialization is taken here too**, and handed to
/// the capture as its oracle. It answers two questions nothing else
/// can: which JSON levels are enum-variant wrappers serde's path
/// elides (a key the model writes at a level is not the key serde
/// ignored there), and which containers the saver will leave out, so
/// the save can rebuild them rather than lose the keys nested inside.
/// See [`crate::mindmap::unknown_keys`]. Serializing cannot fail for a
/// value that just deserialized, but the failure degrades to "no
/// oracle" rather than failing a load the user asked for
/// (CODE_CONVENTIONS §9).
///
/// Cost: one `serde_json::Value` parse of `json`, one serialization of
/// `map`, one route resolution per captured key, and one `log::warn!`
/// per captured key.
fn adopt_unknown_keys(
    json: &str,
    map: &MindMap,
    routes: Vec<Vec<Step>>,
) -> Result<crate::mindmap::unknown_keys::UnknownKeys, String> {
    let mut raw: Value = serde_json::from_str(json)
        .map_err(|e| format!("Failed to re-read mindmap JSON to preserve unknown keys: {e}"))?;
    if let Some(message) = detect_legacy_shape(&raw) {
        return Err(message);
    }
    let probe = serde_json::to_value(map).ok();
    let captured = unknown_keys::take_from(&mut raw, probe.as_ref(), routes);
    for entry in captured.iter() {
        log::warn!("{}", entry.warning());
    }
    Ok(captured)
}

/// Load the map around the constructs this build cannot read, or
/// `None` when there is nothing of the kind to skip.
///
/// **The motivating scenario for #115, in its acute form.** An
/// unrecognized *key* is inert — this build has no field for it, so
/// ignoring it changes nothing. An unrecognized **enum variant** is
/// not: `{"mutator": {"Glow": …}}` from a newer build made the whole
/// document unloadable, so opening a newer map in an older build gave
/// an empty window and the newer feature was one accidental save away
/// from being gone. The maintainer's rule is that this build can
/// *offer to load what it knows how to handle*: the map appears, the
/// construct it cannot name does not, and the file keeps it.
///
/// # What gets skipped, and why that unit
///
/// Two, and only two: a whole `CustomMutation` (map-level
/// `custom_mutations[i]` or a node's `inline_mutations[i]`) and a
/// whole `TriggerBinding` (on a node or on one of its sections).
///
/// The unit is not "the part that failed", and the reason is the
/// difference between a missing key and a missing instruction.
/// Dropping one `Mutation` from a macro's list leaves a custom
/// mutation that still shows up in `mutation list`, still fires, and
/// silently does less than it says — a behavior change with no
/// symptom. Dropping the whole entry is *visible as absence*: it is
/// not in the registry, nothing triggers it, and the load said so.
/// Neither drop violates a `format/validation.md` invariant — a
/// `trigger_bindings.mutation_id` pointing at a mutation that is not
/// there is already a documented runtime no-op — so the choice is
/// made on which failure a user can see, and partial execution is the
/// one they cannot.
///
/// Nothing else is skippable. A node, an edge, the canvas, a palette:
/// removing any of them changes the map the reader is looking at
/// rather than one thing it can do, so an unreadable one still fails
/// the load with the message it always had.
///
/// # What it does not soften
///
/// **Loading and validating are different questions.** Every skipped
/// construct is carried on
/// [`MindMap::skipped_constructs`](crate::mindmap::model::MindMap::skipped_constructs),
/// and `maptool verify` reports each as a violation and exits
/// nonzero, naming the variant serde refused. A typo is still caught
/// at the moment it is a typo; it just no longer costs the author the
/// whole document while they find it.
///
/// # Cost
///
/// Nothing on a load that succeeds — this is only reached after the
/// typed parse has already failed. When it is reached: one `Value`
/// parse, one typed re-read of each custom mutation and trigger
/// binding, and one more full deserialization of the remainder.
fn load_skipping_unreadable_constructs(json: &str) -> Option<Result<MindMap, String>> {
    let mut raw: Value = serde_json::from_str(json).ok()?;
    // A pre-refactor spelling has a migration verb, and the message
    // that names it is more use than a partial load would be.
    if detect_legacy_shape(&raw).is_some() {
        return None;
    }
    let skipped = excise_unreadable_constructs(&mut raw);
    if skipped.is_empty() {
        return None;
    }
    // The remainder still has to load on its own terms. If it does
    // not, the failure was never about a construct we could skip, and
    // the caller's ordinary diagnosis is the better answer.
    let (mut map, routes): (MindMap, Vec<Vec<Step>>) =
        unknown_keys::deserialize_value_capturing(&raw).ok()?;
    if let Some(err) = unknown_keys::unknown_key_count_violation(&routes) {
        // Same ceiling as the strict door, and reported as itself.
        // `None` here would hand the caller back to
        // `diagnose_rejected_json`, which would name the unreadable
        // construct that sent the load down this path — true, but not
        // the reason the document was refused, and not the one the
        // author can act on.
        return Some(Err(err));
    }
    for entry in skipped.iter() {
        log::warn!("{}", entry.warning());
    }
    map.skipped_constructs = skipped;
    if !routes.is_empty() {
        let probe = serde_json::to_value(&map).ok();
        let captured = unknown_keys::take_from(&mut raw, probe.as_ref(), routes);
        for entry in captured.iter() {
            log::warn!("{}", entry.warning());
        }
        map.unknown_keys = captured;
    }
    Some(Ok(map))
}

/// Lift every custom mutation and trigger binding the typed model
/// cannot read out of `document`, in document order.
///
/// Refused for **any** reason, not only an unknown variant. Sorting
/// serde's message into "a variant from the future" and "a typo"
/// would mean parsing that message, which is the twin surface
/// `lib/baumhard/CONVENTIONS.md` §B4 is about — and it would be
/// sorting on the wrong axis anyway. The load's question is "can I
/// carry this out?", and the answer is no in both cases;
/// `maptool verify`'s question is "is this file right?", and it
/// reports both. So the load skips whatever it cannot read and says
/// what serde said, and the two questions stay separate.
///
/// Cost: one typed re-read per custom mutation and per trigger
/// binding in the document. Only reached on a load that has already
/// failed.
fn excise_unreadable_constructs(document: &mut Value) -> unknown_keys::SkippedConstructs {
    let mut skipped = unknown_keys::SkippedConstructs::default();
    excise_from::<CustomMutation>(
        document,
        &[Step::Key("custom_mutations".to_string())],
        &mut skipped,
    );

    // The warnings a reader sees come out in node-id order. Nothing
    // here sorts for that: `serde_json::Map` is a `BTreeMap` unless
    // serde_json's `preserve_order` feature is on, and no member of
    // this workspace turns it on, so `keys()` already yields sorted.
    // The explicit `sort()` that used to sit here re-stated a
    // guarantee the type was already giving and no test could tell
    // apart from its absence. `tests::test_skip_warnings_come_out_in_node_id_order`
    // holds the *property* instead, which is what actually goes red
    // if some future dependency unions that feature in.
    let node_ids: Vec<String> = document
        .get("nodes")
        .and_then(Value::as_object)
        .map(|nodes| nodes.keys().cloned().collect())
        .unwrap_or_default();
    for id in node_ids {
        let node = [Step::Key("nodes".to_string()), Step::Key(id.clone())];
        let at = |field: &str| {
            let mut route = node.to_vec();
            route.push(Step::Key(field.to_string()));
            route
        };
        excise_from::<CustomMutation>(document, &at("inline_mutations"), &mut skipped);
        excise_from::<TriggerBinding>(document, &at("trigger_bindings"), &mut skipped);

        let section_count = value_at(document, &at("sections"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        for index in 0..section_count {
            let mut route = at("sections");
            route.push(Step::Index(index));
            route.push(Step::Key("trigger_bindings".to_string()));
            excise_from::<TriggerBinding>(document, &route, &mut skipped);
        }
    }
    skipped
}

/// Remove every element of the array at `route` that does not
/// deserialize as `T`, recording each with the index it was authored
/// at.
///
/// Indices are recorded **before** anything is removed, so a save
/// puts the elements back where the author had them rather than where
/// the shortened array happened to leave a gap.
fn excise_from<T: serde::de::DeserializeOwned>(
    document: &mut Value,
    route: &[Step],
    skipped: &mut unknown_keys::SkippedConstructs,
) {
    let Some(items) = value_at_mut(document, route).and_then(Value::as_array_mut) else {
        return;
    };
    let mut refused: Vec<(usize, Value, String)> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if let Err(e) = T::deserialize(item) {
            refused.push((index, item.clone(), e.to_string()));
        }
    }
    for (index, _, _) in refused.iter().rev() {
        items.remove(*index);
    }
    for (index, value, reason) in refused {
        let mut full = route.to_vec();
        full.push(Step::Index(index));
        skipped.push(full, value, reason);
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

/// Post-parse invariants that live in the typed model rather than in
/// the JSON: they are checked against `MindMap` values, never against
/// the source text.
///
/// Two kinds of invariant live here, and the order they run in is
/// the order a reader wants them. **Structure** comes first — a
/// node with no sections, a parent cycle, too many sections — because
/// those describe a map that is malformed as a *document*.
/// **Numeric domain** comes last, and is the one that keeps the
/// editor alive: a `.mindmap.json` is untrusted input, and its
/// numbers reach `assert!`s inside the text shaper, an inverted
/// `f32::clamp`, and allocations sized from authored geometry. A map
/// that would abort the process on its first frame does not open;
/// see `mindmap::model::validate` for why that is a rejection rather
/// than a repair.
///
/// Cost: O(n log n) in the node count, plus O(edges + sections +
/// runs). The log factor is five separate sorts, not one: each of
/// the four `detect_*` checks sorts the nodes itself, and
/// `map_numeric_domain` sorts them again. Every one of them
/// allocates its own `Vec` of borrows to do it.
///
/// That is deliberate rather than overlooked. Each check reports the
/// *first* offending node, and "first" has to mean the same node on
/// every run or the error message depends on `HashMap` iteration
/// order. Sharing one sorted list across all five would need the
/// order threaded through checks that are otherwise independent and
/// individually testable. This runs once per file open, against a
/// parse that already cost several times more.
fn check_invariants(map: MindMap) -> Result<MindMap, String> {
    if let Some(err) = detect_zero_section_node(&map) {
        return Err(err);
    }
    if let Some(err) = detect_id_key_mismatch(&map) {
        return Err(err);
    }
    if let Some(err) = detect_parent_cycle(&map) {
        return Err(err);
    }
    if let Some(err) = detect_section_count_cap(&map) {
        return Err(err);
    }
    if let Some(err) = validate::map_numeric_domain(&map) {
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
    // The rule itself is `validate::zero_section_node`, shared with
    // `maptool verify` so the two ends cannot drift — they did, and
    // `verify` called a map the editor refuses "valid".
    ids.into_iter()
        .find_map(|id| validate::zero_section_node(&map.nodes[id]))
}

/// Reject a map where a node's key in `nodes` differs from the
/// node's own `id`.
///
/// **This is what makes the cycle rejection below sound.** The two
/// spellings of a node's identity address *different graphs*:
/// [`detect_parent_cycle`] walks `nodes` by key, while
/// [`ChildIndex`](crate::mindmap::model::ChildIndex) — which every
/// scene build and fold walk uses — keys children by `parent_id` and
/// looks them up by `node.id`. Let the two disagree and a file can
/// describe a chain that is acyclic in the key graph and a loop in
/// the id graph: `{"k": {"id": "a", "parent_id": "a"}}` is its own
/// child under `ChildIndex` and a dangling-parent root under the
/// cycle check. The scene builder then descends that self-edge
/// forever, appending to the arena until the allocator gives up.
///
/// `maptool verify` has always called this an error
/// (`verify/ids.rs`); the loader accepting it is what left the gap.
/// Nodes are visited in sorted-key order so the reported node is
/// deterministic across `HashMap` iteration order.
fn detect_id_key_mismatch(map: &MindMap) -> Option<String> {
    let mut keys: Vec<&String> = map.nodes.keys().collect();
    keys.sort();
    let key = keys.into_iter().find(|key| &map.nodes[*key].id != *key)?;
    Some(format!(
        "node {:?}: `id` is {:?} but the key in `nodes` is {:?} — they address the same node \
         and must match. A mismatch makes the parent-cycle check and the scene builder walk \
         different graphs; see format/ids.md.",
        key, map.nodes[key].id, key
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
    format!("Failed to parse mindmap JSON: {error}")
}

/// Legacy field shapes that predate the current format, each with the
/// `maptool convert` verb that migrates it.
///
/// **Reachable from both a successful parse and a failed one**, and
/// the successful one is the case that matters. `portals` and per-node
/// `text` / `text_runs` are keys the current model has no field for,
/// so under the unknown-key policy they load clean and would be
/// preserved forever — a stale spelling carried across every save with
/// nothing ever acting on it. They are not keys from the future; they
/// are keys the model means something else by now, and the author has
/// a one-command migration. So they are asked about explicitly, before
/// the capture is adopted. On the failure path the same three
/// questions win over the generic per-part diagnosis, because "unknown
/// field `text`" is a true but useless thing to tell someone holding a
/// pre-section map.
///
/// The third shape — a `sections` that is not an array — can only
/// arrive on the failure path; the typed parse rejects it before the
/// capture is consulted. It lives here because it belongs to the same
/// migration story, and asking it twice costs nothing.
///
/// Cost: O(nodes) in the worst case, two `Value::get`s per node. Only
/// reached when a load already found something it did not recognize.
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
    Some(format!("{label}: {error}"))
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
                // `chain` and `on_chain` gain the same id on the same
                // two lines below and neither is ever popped, so the
                // position is always found. It is spelled as a
                // fallback rather than an `unwrap()` because this
                // runs on the initial load and on the console `open`
                // verb: CODE_CONVENTIONS §9 makes a bare `unwrap()`
                // outside tests a bug on the first and a panic on the
                // second unacceptable outright. Reporting from the
                // head of the chain instead of from the join point
                // still names a node that is on the cycle's walk and
                // still refuses the map, which is the whole decision
                // here; a panic would refuse the *application*.
                let cycle_start = chain.iter().position(|&id| id == current_id).unwrap_or(0);
                let path = chain[cycle_start..]
                    .iter()
                    .chain(std::iter::once(&current_id))
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" → ");
                return Some(format!(
                    "node {:?}: parent chain contains a cycle ({}); fix parent_id",
                    chain.get(cycle_start).copied().unwrap_or(current_id),
                    path
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
/// **Preservation**: the keys the load did not recognize
/// ([`MindMap::unknown_keys`]) are spliced back into the serialized
/// document before it is rendered, each at the route it came from. A
/// map this build authored carries none and the step is a no-op; a map
/// authored by a newer build comes back out with its newer features
/// intact. This is the save half of the policy in the module header,
/// and [`to_json_value`] is where it happens — anything that
/// serializes a `MindMap` for persistence must go through that rather
/// than calling `serde_json::to_value` directly.
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
    let value = to_json_value(map)?;
    let json = serde_json::to_string_pretty(&value).map_err(|e| format!("failed to render map JSON: {e}"))?;
    write_atomic(path, &json)
}

/// The JSON a `MindMap` persists as: its typed shape plus the keys
/// this build did not recognize when it loaded the map, each written
/// back where it came from.
///
/// **This, not `serde_json::to_value`, is the map's on-disk form.**
/// Serializing the typed shape alone drops whatever a newer build
/// wrote, which is the data loss the unknown-key policy exists to
/// prevent; `to_value` is only the first half of the answer. Exposed
/// so `maptool` and any future export path can produce the same bytes
/// [`save_to_file`] would.
///
/// Cost: one full `Value` copy of the model (`serde_json::to_value`),
/// plus one route walk and one `Value` clone per preserved key —
/// nothing at all for a map with none.
pub fn to_json_value(map: &MindMap) -> Result<Value, String> {
    let mut value = serde_json::to_value(map).map_err(|e| format!("failed to serialize map: {e}"))?;
    map.unknown_keys.splice_into(&mut value);
    // Keys first, constructs second, and the reason is narrower than
    // it looks. A captured key's positional route was resolved against
    // a document the skipped constructs had already been lifted out
    // of, so its index counts the *shortened* array; re-inserting the
    // constructs first would make that index name a different element.
    // What stops that being a data-loss bug in the general case is not
    // this ordering — it is the element fingerprints
    // `UnknownKeys::splice_into` carries, which re-find the element
    // whichever length the array is. The ordering is what keeps the
    // fingerprints from having to: it holds the array in the shape the
    // route was recorded against, so the common path is an exact match
    // at the recorded index rather than a search.
    //
    // It stops being merely defensive wherever the fingerprint search
    // cannot finish the job on its own, and there are two such shapes
    // — both ordinary rather than exotic:
    //
    // - **Two elements that serialize identically.** The fingerprint
    //   matches in more than one place, so a search cannot say which
    //   one the key came from.
    //   `tests::test_the_splice_order_decides_when_two_elements_are_alike`.
    // - **An element that was edited**, sharing its array with a
    //   skipped construct. Its fingerprint now matches *nowhere*, and
    //   the last rung `resolve_index` has left is "every other element
    //   is unchanged and the array is still the length the route was
    //   recorded against". Re-inserting the construct first lengthens
    //   the array and breaks the second half of that.
    //   `tests::test_the_splice_order_survives_an_edit_beside_a_skipped_construct`.
    //
    // In both, this ordering is the whole difference between writing
    // the key where it belongs and dropping it with a warning.
    map.skipped_constructs.splice_into(&mut value);
    Ok(value)
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
    /// A `.mindmap.json` key this build has no field for is captured
    /// at load and written back at save. That only works while every
    /// type between the document root and the key hands the key to
    /// `deserialize_ignored_any` — which is serde's default, and
    /// which exactly four container attributes take away:
    ///
    /// - `deny_unknown_fields` fails the load instead. This is the
    ///   policy #105 shipped and #115 reversed, and it is the one
    ///   that comes back by accident: it and `#[serde(flatten)]`
    ///   **compile together, with `deny` silently winning**, so a
    ///   type carrying both rejects where it reads as preserving.
    /// - `flatten` absorbs the key into the flattened field.
    /// - `untagged`, and `tag = "…"` with or without `content`,
    ///   buffer the object into serde's own `Content` and replay it
    ///   through a deserializer the capture is not wrapping.
    ///
    /// The four split into two classes over **two different sets of
    /// types**, and conflating them is what left a hole here once
    /// already. `deny_unknown_fields` and `flatten` act on a named
    /// member no field claimed, so they are asked only of types that
    /// have named fields — which is also all serde permits. `untagged`
    /// and `tag` act on the whole value, buffering it through
    /// `Content` regardless of whether any variant has a field name in
    /// it, so they are asked of **every** reachable Deserialize type.
    /// Guarding both behind `has_named_fields` — correct in the
    /// `deny_unknown_fields` era, never revisited when the predicate
    /// grew — excluded 24 of the 70 reachable types, and a planted
    /// `#[serde(untagged)]` on `Mutation` sailed through.
    ///
    /// The set of types this applies to is not written down here.
    /// [`crate::util::serde_coverage`] parses baumhard's own sources
    /// and walks outward from `MindMap` through every deserializable
    /// field, so a new field of a new type extends the covered set on
    /// its own. One shape is exempt: a container that delegates its
    /// on-disk shape to a proxy via `#[serde(from = "...")]` never
    /// deserializes its own fields, so the requirement follows the
    /// proxy — which the walk also reaches, and holds to the same
    /// rule there.
    ///
    /// **A fifth reason exists and is not about serde at all: an
    /// attribute the source reader could not finish.** The four above
    /// are read off the text by a hand-rolled parser, and this test
    /// asserts a list is *empty* — so a flag that parser never saw
    /// reads exactly like a flag that is not there, and the run goes
    /// green. A list-form option (`rename_all(...)`) used to end that
    /// parse where it started, which made
    /// `#[serde(rename_all(deserialize = "camelCase"),
    /// deny_unknown_fields)]` a silent restoration of reject-at-load.
    /// `TypeInfo::unread_serde` is the answer: unknown is reported as
    /// unsafe, not as safe.
    ///
    /// The per-type predicate itself is
    /// [`TypeInfo::unknown_key_hiding_reasons`], next to the
    /// attributes it reads, where a test can drive it over types this
    /// crate does not contain. What lives here is the policy and the
    /// walk it is applied to.
    #[test]
    fn test_no_loadable_type_can_swallow_an_unknown_key() {
        use crate::util::serde_coverage::{crate_src_root, TypeGraph, TypeKind};

        let graph = TypeGraph::build(&crate_src_root());
        let mut offenders: Vec<String> = Vec::new();
        for info in graph.reachable_from("MindMap") {
            let reasons = info.unknown_key_hiding_reasons();
            if reasons.is_empty() {
                continue;
            }
            // Naming the item kind matters for an enum: a container
            // attribute goes on the enum, not on the struct variant
            // whose keys serde was reading.
            let kind = match info.kind {
                TypeKind::Struct => "struct",
                TypeKind::Enum => "enum",
                TypeKind::Alias => "type",
            };
            offenders.push(format!(
                "{kind} {} — {}: {}",
                info.name,
                info.file.display(),
                reasons.join(", ")
            ));
        }
        assert!(
            offenders.is_empty(),
            "these types are reachable from a `.mindmap.json` load and can hide an \
             unrecognized key from the loader's capture, so it would neither be warned \
             about nor survive the next save:\n  {}\n\n\
             Two classes of attribute, checked over different sets, and the difference \
             is load-bearing — do not re-narrow one to match the other:\n  \
             * `deny_unknown_fields` / `flatten` decide what happens to a named member \
             no field claimed, so they are asked only of types that have named fields.\n  \
             * `untagged` / `tag = \"...\"` buffer the whole value through serde's \
             `Content` and replay it outside the capture, which has nothing to do with \
             variant shape, so they are asked of every reachable Deserialize type.\n\n\
             An entry naming an attribute rather than a flag is neither class: it means \
             the source reader could not account for that attribute, so whatever serde \
             options it carries were never looked at, and \"carries none of them\" is \
             the answer a silent pass would have produced. The entry says which of the \
             two it is — an attribute the reader stopped partway through, or one it \
             never looks inside at all — and what to do about it. Neither is a claim \
             that the type hides a key; both are a claim that nobody can tell.",
            offenders.join("\n  ")
        );

        // The `Content`-buffering half of the check is the half that
        // was silently switched off, and an empty check passes just
        // as loudly as a real one. So the walk has to prove it
        // reached a type the old guard skipped.
        let shapeless: Vec<&str> = graph
            .reachable_from("MindMap")
            .iter()
            .filter(|info| {
                info.derives_deserialize && info.deserialize_proxy.is_none() && !info.has_named_fields
            })
            .map(|info| info.name.as_str())
            .collect();
        assert!(
            !shapeless.is_empty(),
            "the `untagged` / `tag` check is supposed to cover types with no named \
             fields, and the walk reached none — either the model changed shape \
             completely or the predicate narrowed again"
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

    /// `ColorSchema.level` is a `usize`, and the loader is where
    /// that stops being a claim. It used to be an `i32` the
    /// resolver cast with `as usize`, which turned an authored
    /// `-1` into 18446744073709551615 and clamped it silently to
    /// the last group — a themed node quietly wearing the wrong
    /// color with nothing in the output to say so. Three shapes,
    /// three verdicts, and none of them was pinned by anything:
    /// the converter's floor is tested, the loader's refusal was
    /// not.
    fn map_with_level(level: &str) -> String {
        format!(
            r##"{{
            "version": "1.0",
            "name": "leveled",
            "canvas": {{"background_color": "#000", "default_border": null,
                       "default_connection": null, "theme_variables": {{}},
                       "theme_variants": {{}}}},
            "palettes": {{"p": {{"groups": [
                {{"background":"#100000","frame":"#200000","text":"#300000","title":""}},
                {{"background":"#110000","frame":"#210000","text":"#310000","title":""}}
            ]}}}},
            "nodes": {{"0": {{
                "id": "0", "parent_id": null,
                "position": {{"x": 0, "y": 0}},
                "size": {{"width": 100, "height": 50}},
                "sections": [{{"text": "ok"}}],
                "style": {{"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false}},
                "layout": {{"type":"map","direction":"auto","spacing":0}},
                "folded": false, "notes": "",
                "color_schema": {{"palette":"p","level":{level},
                                  "starts_at_root":true,"connections_colored":false}}
            }}}},
            "edges": []
        }}"##
        )
    }

    /// One themed node whose `color_schema.overrides` is spelled
    /// `overrides_json` — used to pin what the loader accepts in
    /// that slot.
    fn map_with_overrides(overrides_json: &str) -> String {
        format!(
            r##"{{
            "version": "1.0",
            "name": "overridden",
            "canvas": {{"background_color": "#000", "default_border": null,
                       "default_connection": null, "theme_variables": {{}},
                       "theme_variants": {{}}}},
            "palettes": {{"p": {{"groups": [
                {{"background":"#a9decb","frame":"#30b082","text":"#000000","title":""}}
            ]}}}},
            "nodes": {{"0": {{
                "id": "0", "parent_id": null,
                "position": {{"x": 0, "y": 0}},
                "size": {{"width": 100, "height": 50}},
                "sections": [{{"text": "ok"}}],
                "style": {{"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false}},
                "layout": {{"type":"map","direction":"auto","spacing":0}},
                "folded": false, "notes": "",
                "color_schema": {{"palette":"p","level":0,
                                  "starts_at_root":true,"connections_colored":false,
                                  "overrides":{overrides_json}}}
            }}}},
            "edges": []
        }}"##
        )
    }

    /// `"overrides": null` loads, and reads exactly as the missing
    /// key does.
    ///
    /// `#[serde(default)]` fires on an absent key and not on an
    /// explicit `null`, so the field refused a spelling every other
    /// optional object in the format accepts and
    /// `format/schema.md` calls "an optional object" — with a whole
    /// load failure (`invalid type: null, expected struct
    /// ColorOverrides`), not a degrade.
    #[test]
    fn test_null_color_overrides_reads_as_no_overrides() {
        for spelling in ["null", "{}"] {
            let map = load_from_str(&map_with_overrides(spelling))
                .unwrap_or_else(|e| panic!("`\"overrides\": {spelling}` must load: {e}"));
            let node = map.nodes.get("0").expect("node 0 loads");
            let schema = node.color_schema.as_ref().expect("themed");
            assert!(
                schema.overrides.is_empty(),
                "`{spelling}` must read as no overrides at all"
            );
            // And the node is on its palette, which is the whole
            // observable consequence of "no overrides".
            assert_eq!(map.node_background_color(node), "#a9decb");
            assert_eq!(map.node_frame_color(node), "#30b082");
        }
    }

    /// The `null` acceptance is per-channel too: a channel spelled
    /// `null` is a channel with no opinion, not a channel holding
    /// the string `"null"`. Already true by `Option`'s own
    /// deserializer — pinned because the outer `null` needed a
    /// custom one and the two must not diverge.
    #[test]
    fn test_null_color_override_channel_reads_as_no_opinion() {
        let map = load_from_str(&map_with_overrides(
            r##"{"background":"#00ff00","frame":null,"text":null,"title":null}"##,
        ))
        .expect("per-channel nulls must load");
        let node = map.nodes.get("0").expect("node 0 loads");
        let overrides = &node.color_schema.as_ref().expect("themed").overrides;
        assert_eq!(overrides.background.as_deref(), Some("#00ff00"));
        assert_eq!(overrides.frame, None);
        assert_eq!(map.node_background_color(node), "#00ff00");
        assert_eq!(map.node_frame_color(node), "#30b082");
    }

    #[test]
    fn test_negative_color_schema_level_is_refused_by_the_loader() {
        let err = load_from_str(&map_with_level("-1")).expect_err("a negative level must not load");
        assert!(
            err.contains("invalid value") && err.contains("expected usize"),
            "serde's own type message is the diagnostic; got: {err}"
        );
        assert!(
            err.contains("\"0\""),
            "the message must name the node it came from: {err}"
        );
    }

    #[test]
    fn test_fractional_color_schema_level_is_refused_by_the_loader() {
        let err = load_from_str(&map_with_level("1.5")).expect_err("a fractional level must not load");
        assert!(
            err.contains("invalid type") && err.contains("expected usize"),
            "serde's own type message is the diagnostic; got: {err}"
        );
    }

    /// The other end: a level far past the palette's last group is
    /// **legal** and clamps. Nothing about a deep subtree is
    /// malformed, so the loader has no business refusing it — the
    /// resolver degrades it to the last group instead (see
    /// `format/palettes.md`).
    #[test]
    fn test_an_enormous_color_schema_level_loads_and_clamps() {
        let map = load_from_str(&map_with_level("1000000000000000000"))
            .expect("a level past the palette's end is legal");
        let node = map.nodes.get("0").expect("node 0 loads");
        assert_eq!(
            node.color_schema.as_ref().unwrap().level,
            1_000_000_000_000_000_000
        );
        let group = map
            .resolve_theme_colors(node)
            .expect("an out-of-range level clamps rather than failing");
        assert_eq!(group.frame, "#210000", "clamped to the last group");
        assert_eq!(map.node_frame_color(node), "#210000");
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
        // The refusal now comes from `deserialize_sections_capped`
        // during the parse rather than from `detect_section_count_cap`
        // afterwards, which is the point: the sweep produced its
        // message only once the whole `Vec` existed, so a 12 MB file
        // declaring four million sections reached 1 723 MiB before
        // being told no. The cap is still named, and the node still
        // identified, so an author can act on it.
        assert!(
            err.contains("node.sections.len()") && err.contains("1024"),
            "the refusal must name the field and the cap: {err}"
        );
        assert!(
            err.contains("node \"0\""),
            "and must identify the node that carries them: {err}"
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
        assert_eq!(run.size_pt, 74.0);
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
            let samples =
                connection::sample_path(&conn_path, 7.2, crate::mindmap::connection::MAX_PATH_SAMPLES);
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
        use crate::mindmap::tree_builder::{build_connection_elements, node_clip_aabbs, EdgePathCache};

        let path = test_map_path();
        let map = load_from_file(&path).unwrap();
        let hidden = map.fold_hidden_set();
        let offsets = std::collections::HashMap::new();
        let aabbs = node_clip_aabbs(&map, &offsets, None, &hidden);
        let mut cache = SceneConnectionCache::new();
        let mut paths = EdgePathCache::new(&map, &offsets);
        let (connection_elements, _handles) = build_connection_elements(
            &map, &offsets, &aabbs, None, None, &mut cache, &mut paths, 1.0, &hidden,
        );

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
        // `test_save_to_file_round_trip_for_loaded_and_blank_maps` (every node has an
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
            canvas: Canvas::default(),
            palettes: HashMap::new(),
            nodes,
            edges: Vec::new(),
            custom_mutations: Vec::new(),
            macros: Vec::new(),
            unknown_keys: Default::default(),
            skipped_constructs: Default::default(),
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
                "visible": true,
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
        // `TextRun`'s style flags: `false` is the field's default,
        // so an authored `"bold": false` carries nothing a reload
        // would not recover.
        ("is_false", |v| v.as_bool() == Some(false)),
        // `TextRun.size_pt` at the render-neutral default — the
        // scale run-less text is measured at, and what a run that
        // omits the key deserializes to. `14` and `14.0` are the
        // same JSON number.
        ("is_default_text_run_size_pt", |v| {
            v.as_f64() == Some(f64::from(crate::mindmap::model::DEFAULT_TEXT_RUN_SIZE_PT))
        }),
        // `CustomMutation.behavior` at its `Persistent` default —
        // the reader's `#[serde(default)]` restores it (#47 part B).
        ("behavior_is_default", |v| v.as_str() == Some("Persistent")),
        // `ColorSchema.overrides`: an object whose four channels are
        // all absent or `null` carries no override, which is what
        // every node that has never been recolored by hand holds.
        // A bare `null` in the field's place says the same thing —
        // the field's custom deserializer reads it as the default,
        // so a fixture spelling it that way is dropped on save for
        // the same reason and is not authored-key loss either.
        ("ColorOverrides::is_empty", |v| {
            v.is_null()
                || v.as_object().is_some_and(|o| {
                    ["background", "frame", "text", "title"]
                        .iter()
                        .all(|k| o.get(*k).is_none_or(serde_json::Value::is_null))
                })
        }),
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

    /// The one captured key whose `path_within_location` is `path`,
    /// or a panic naming everything that *was* captured — a test that
    /// fails because the loader stamped a key differently should say
    /// so rather than report `None`.
    fn captured<'m>(map: &'m MindMap, path: &str) -> &'m crate::mindmap::unknown_keys::UnknownKey {
        map.unknown_keys
            .iter()
            .find(|entry| entry.path_within_location() == path)
            .unwrap_or_else(|| {
                let seen: Vec<String> = map
                    .unknown_keys
                    .iter()
                    .map(|entry| format!("{}: {}", entry.location(), entry.path_within_location()))
                    .collect();
                panic!("no captured key at {path:?}; captured: {seen:?}")
            })
    }

    /// Save `map` through the editor's own save path and read the
    /// bytes back. The round-trip tests go through the real writer
    /// rather than [`to_json_value`], because the guarantee is about
    /// the file on disk.
    fn saved_json(map: &MindMap, label: &str) -> String {
        let dir = TempDir::new(label);
        let path = dir.join("saved.mindmap.json");
        save_to_file(&path, map).expect("save failed");
        std::fs::read_to_string(&path).expect("read saved")
    }

    /// **The acceptance criterion, and the one that separates keeping
    /// a key from merely warning about it.** A map with a key this
    /// build has no field for loads, and after a save the key is
    /// still there, holding what it held.
    ///
    /// Warn-and-drop passes every other test in this file. It fails
    /// this one.
    #[test]
    fn test_an_unrecognized_key_survives_a_load_and_save_round_trip() {
        let json = map_json_with_nodes(&node_json_with(
            "1.2",
            "null",
            r#", "portal_form": {"glyphs": "◇◆", "spin": 3}"#,
        ));
        let map = load_from_str(&json).expect("an unrecognized key must not fail the load");
        assert_eq!(
            map.nodes["1.2"].sections[0].text, "n",
            "the rest of the node must still load"
        );

        let saved: Value = serde_json::from_str(&saved_json(&map, "round-trip-unknown")).expect("valid JSON");
        assert_eq!(
            saved["nodes"]["1.2"]["portal_form"],
            serde_json::json!({"glyphs": "◇◆", "spin": 3}),
            "the unrecognized key must come back out of the save unchanged"
        );

        // And again, so a second generation of the file does not lose
        // what the first one kept.
        let reloaded =
            load_from_str(&saved_json(&map, "round-trip-unknown-2")).expect("the saved map reloads");
        assert_eq!(
            captured(&reloaded, "portal_form").value()["spin"],
            serde_json::json!(3)
        );
    }

    /// The same guarantee at the bottom of the deepest thing a map
    /// can hold. `custom_mutations[].mutator` is an externally tagged
    /// enum, so the JSON has an object level — `{"Void": { … }}` —
    /// that serde's ignored-key path does not report. If the route
    /// did not account for that, this key would be captured and then
    /// written back in the wrong place, or nowhere.
    #[test]
    fn test_an_unrecognized_key_inside_a_mutator_survives_the_round_trip() {
        let json = map_json_with_nodes(&node_json("0", "null")).replace(
            r#""edges": []"#,
            r#""edges": [], "custom_mutations": [{
                "id": "m", "name": "m", "target_scope": "SelfOnly",
                "mutator": {"Void": {"channel": 0, "children": [], "afterglow": 0.5}}
            }]"#,
        );
        let map = load_from_str(&json).expect("an unrecognized key must not fail the load");
        let entry = captured(&map, "mutator.Void.afterglow");
        assert_eq!(entry.location(), "custom_mutations[0]");

        let saved: Value = serde_json::from_str(&saved_json(&map, "round-trip-mutator")).expect("valid JSON");
        assert_eq!(
            saved["custom_mutations"][0]["mutator"]["Void"]["afterglow"],
            serde_json::json!(0.5)
        );
    }

    /// **The policy, spelled out where authors read it.**
    /// `format/schema.md` publishes one map carrying a mistyped key,
    /// the warning it produces, and what the corrected map does. All
    /// three are read out of the spec rather than restated here, so
    /// the pin follows the doc when the doc moves instead of agreeing
    /// with a copy of its old self.
    ///
    /// The published warning is compared for **equality**, not
    /// containment: the doc presents it as verbatim loader output. A
    /// substring check would let the wording drift and leave the spec
    /// publishing something the loader no longer says, with the suite
    /// green. Only line wrapping is normalized away
    /// ([`doc_fixtures::unwrapped`]): the doc wraps to the column
    /// limit, the loader emits one line, and where the breaks fall
    /// carries no meaning.
    #[test]
    fn test_documented_unknown_key_warning_matches_the_spec() {
        use crate::util::doc_fixtures::{
            documented_json_block, documented_plain_block, format_doc_path, unwrapped,
        };
        let doc = format_doc_path("schema.md");
        let heading = "## Unknown keys are kept";

        let mistyped = documented_json_block(&doc, heading, 0);
        let map = load_from_str(&mistyped)
            .unwrap_or_else(|e| panic!("the spec's mistyped-key example must load: {e}\n{mistyped}"));
        assert!(
            map.nodes["0"].min_zoom_to_render.is_none(),
            "a mistyped key must not reach the field it was aiming at"
        );
        let entry = captured(&map, "min_zoom_to_rendr");

        let published = documented_plain_block(&doc, heading, 0);
        assert_eq!(
            unwrapped(&entry.warning()),
            unwrapped(&published),
            "format/schema.md §{heading} publishes this warning as verbatim loader \
             output and it no longer is. Re-wrap the block to match, or the spec is \
             publishing something the loader does not say.\n\
             \n  loader: {}\n  spec:   {published}\n",
            entry.warning()
        );

        let corrected = documented_json_block(&doc, heading, 1);
        let map = load_from_str(&corrected)
            .unwrap_or_else(|e| panic!("the spec's corrected example must load: {e}\n{corrected}"));
        assert_eq!(map.nodes["0"].min_zoom_to_render, Some(2.0));
        assert!(
            map.unknown_keys.is_empty(),
            "the corrected example must have nothing left to preserve"
        );
    }

    /// An unrecognized key on a node names the node, names the key,
    /// and says what the loader did about it. The last part is the
    /// point: an author who mistyped a real key needs to be told the
    /// difference between "kept" and "understood".
    #[test]
    fn test_unknown_node_key_is_kept_and_named() {
        let json = map_json_with_nodes(&node_json_with("1.2", "null", r#", "portal_form": {"x": 1}"#));
        let map = load_from_str(&json).expect("an unrecognized node key must not fail the load");
        let warning = captured(&map, "portal_form").warning();
        assert!(
            warning.starts_with("loader: "),
            "must carry the §9 area prefix: {warning}"
        );
        assert!(warning.contains("node \"1.2\""), "must name the node: {warning}");
        assert!(warning.contains("portal_form"), "must name the key: {warning}");
        assert!(
            warning.contains("kept as written and saved back with the value it was authored with"),
            "must state what happened to the key, not just that it is unknown: {warning}"
        );
        assert!(
            warning.contains("format/schema.md"),
            "must point at the spec: {warning}"
        );
    }

    /// **The warning is emitted, not merely available.** Every other
    /// test here reads `map.unknown_keys`, which stays true of a
    /// loader that captured the key and told nobody — deleting the
    /// `log::warn!` leaves them all green.
    ///
    /// So this one reads the log. `crate::util::test_logger` is a
    /// real `log::Log` sink rather than a mock (TEST_CONVENTIONS
    /// §T10): it receives exactly what a real run sends. It is
    /// process-global and the suite runs in parallel, so the key name
    /// is deliberately one nothing else in the repository could
    /// produce, and the search is for that needle rather than for
    /// "the last line logged".
    #[test]
    fn test_the_unrecognized_key_warning_is_actually_logged() {
        crate::util::test_logger::install();
        let json = map_json_with_nodes(&node_json_with("7.7", "null", r#", "gnomon_bearing_9f31": 1"#));
        let map = load_from_str(&json).expect("an unrecognized node key must not fail the load");

        let logged = crate::util::test_logger::lines_containing("gnomon_bearing_9f31");
        assert_eq!(
            logged.len(),
            1,
            "the loader must warn exactly once per unrecognized key, logged: {logged:?}"
        );
        assert!(
            logged[0].starts_with("WARN "),
            "an unrecognized key is a `warn!`, not instrumentation: {}",
            logged[0]
        );
        assert!(
            logged[0].ends_with(&captured(&map, "gnomon_bearing_9f31").warning()),
            "the logged line must be the one `UnknownKey::warning` builds, so the \
             wording lives in one place: {}",
            logged[0]
        );
    }

    /// The same report reaches keys nested inside a node — a typo in
    /// `style` is as invisible as one at the node level, and the
    /// message still resolves to the node the author has to open,
    /// with the field path inside it.
    #[test]
    fn test_unknown_key_inside_node_style_names_the_node() {
        let json = map_json_with_nodes(
            &node_json("0", "null").replace(r#""show_shadow":false"#, r#""show_shadow":false,"shpe":"star""#),
        );
        let map = load_from_str(&json).expect("an unrecognized style key must not fail the load");
        let entry = captured(&map, "style.shpe");
        assert_eq!(entry.location(), "node \"0\"");
        assert_eq!(entry.value(), &serde_json::json!("star"));
    }

    /// Edges are addressed by index, matching `MindMap::edge_locations`
    /// and `maptool verify`'s `edge[<i>]` stamp, because an edge has
    /// no id to name it by.
    #[test]
    fn test_unknown_edge_key_names_the_edge_index() {
        let nodes = format!("{},{}", node_json("a", "null"), node_json("b", "\"a\""));
        let json = map_json(&nodes, &edge_json_with(r#", "arrowhead": "open""#));
        let map = load_from_str(&json).expect("an unrecognized edge key must not fail the load");
        assert_eq!(captured(&map, "arrowhead").location(), "edge[0]");
    }

    /// The canvas is a single named part rather than a collection, so
    /// it is stamped by name.
    #[test]
    fn test_unknown_canvas_key_names_the_canvas() {
        let json = map_json_with_nodes(&node_json("0", "null")).replace(
            r##""background_color": "#000""##,
            r##""background_color": "#000", "grid_snap": 8"##,
        );
        let map = load_from_str(&json).expect("an unrecognized canvas key must not fail the load");
        assert_eq!(captured(&map, "grid_snap").location(), "canvas");
    }

    /// A key at the top level has no sub-object to attribute it to,
    /// so it is stamped against the map itself.
    #[test]
    fn test_unknown_top_level_key_is_kept_and_named() {
        let json = map_json_with_nodes(&node_json("0", "null"))
            .replace(r#""version": "1.0","#, r#""version": "1.0", "authors": ["me"],"#);
        let map = load_from_str(&json).expect("an unrecognized top-level key must not fail the load");
        let entry = captured(&map, "authors");
        assert_eq!(entry.location(), "map");
        assert_eq!(entry.value(), &serde_json::json!(["me"]));
    }

    /// Every unrecognized key is reported, and each exactly once — a
    /// map that quietly kept the second one would satisfy every
    /// single-key test above.
    #[test]
    fn test_every_unrecognized_key_is_reported_exactly_once() {
        let nodes = format!(
            "{},{}",
            node_json_with("a", "null", r#", "alpha": 1"#),
            node_json_with("b", "\"a\"", r#", "beta": 2, "gamma": 3"#)
        );
        let json =
            map_json_with_nodes(&nodes).replace(r#""version": "1.0","#, r#""version": "1.0", "delta": 4,"#);
        let map = load_from_str(&json).expect("unrecognized keys must not fail the load");

        let mut reported: Vec<String> = map
            .unknown_keys
            .iter()
            .map(|entry| format!("{} {}", entry.location(), entry.path_within_location()))
            .collect();
        reported.sort();
        assert_eq!(
            reported,
            vec![
                "map delta".to_string(),
                "node \"a\" alpha".to_string(),
                "node \"b\" beta".to_string(),
                "node \"b\" gamma".to_string(),
            ]
        );
    }

    /// A map this build authored carries nothing to preserve, and the
    /// capture must say so rather than collecting the keys serde
    /// legitimately defaulted or omitted. This is what keeps the
    /// second parse — and the warning stream — off every ordinary
    /// load.
    #[test]
    fn test_a_current_map_captures_nothing() {
        let map = load_from_file(&test_map_path()).expect("the fixture loads");
        assert!(
            map.unknown_keys.is_empty(),
            "testament must carry no unrecognized keys, found: {:?}",
            map.unknown_keys
                .iter()
                .map(|entry| format!("{}: {}", entry.location(), entry.path_within_location()))
                .collect::<Vec<_>>()
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

    /// **Every canonical fixture is in canonical form.** Loading a
    /// `maps/*.mindmap.json` fixture and rendering it back through
    /// the editor's own writer must reproduce the file byte for
    /// byte — issue #47's acceptance criterion for the serde
    /// hygiene work, and CODE_CONVENTIONS §10's "fixtures update in
    /// the same commit as the data-model shift" made mechanical: a
    /// serialization change without a fixture refresh fails here,
    /// naming the file.
    ///
    /// Byte-exact is available because the writer is deterministic
    /// (`test_save_to_file_is_deterministic`) and the
    /// fixtures are themselves written by it. This is deliberately
    /// stricter than the key-path round-trip above: that one allows
    /// re-spelling, this one pins the fixtures to the writer's
    /// current spelling.
    #[test]
    fn test_canonical_fixtures_resave_byte_identically() {
        let maps_dir = crate::util::doc_fixtures::repo_path("maps");
        let mut checked: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&maps_dir).expect("maps/ exists") {
            let path = entry.expect("dir entry").path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with(".mindmap.json") {
                continue;
            }
            let source =
                std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: unreadable fixture: {e}"));
            let map = load_from_str(&source).unwrap_or_else(|e| panic!("{name}: fixture does not load: {e}"));
            let resaved =
                serde_json::to_string_pretty(&to_json_value(&map).expect("serialize")).expect("render");
            if resaved != source {
                let at = source
                    .lines()
                    .zip(resaved.lines())
                    .position(|(a, b)| a != b)
                    .unwrap_or(0);
                panic!(
                    "{name} is no longer in the writer's canonical form (first divergence \
                     around line {}). The model's serialization changed — re-save the \
                     fixtures in the same commit (CODE_CONVENTIONS §10).\n  on disk:  {}\n  \
                     writer:   {}",
                    at + 1,
                    source.lines().nth(at).unwrap_or("<past end>"),
                    resaved.lines().nth(at).unwrap_or("<past end>"),
                );
            }

            // Control: the comparison must be able to fail — an
            // edited map renders differently, so a green run above
            // is agreement rather than a comparison that cannot
            // fire.
            let mut edited = map;
            edited.name = format!("{}-edited", edited.name);
            let edited_out =
                serde_json::to_string_pretty(&to_json_value(&edited).expect("serialize")).expect("render");
            assert_ne!(edited_out, source, "{name}: an edit must move the rendered bytes");

            checked.push(name);
        }
        assert!(
            checked.iter().any(|n| n == "testament.mindmap.json"),
            "the canonical fixture must be among the files checked, found only: {checked:?}"
        );
    }

    /// The file
    /// `test_every_mindmap_write_goes_through_the_contract` proves it
    /// can read past a test gate in: twelve `#[cfg(test)] mod
    /// tests_*;` declarations at line 35, and everything a save path
    /// would plausibly live among below them.
    const COVERAGE_WITNESS: &str = "application/document/mod.rs";

    /// Whether the production text of a file still carries code below
    /// the file's first `#[cfg(test)]` — the one measurement that
    /// tells a correct excision from a truncating one, since the
    /// truncating reader could not leave anything there by
    /// construction.
    ///
    /// The gate is located in the *source* by text, which is all this
    /// needs: the question is only "was there a gate, and did the scan
    /// keep reading past it".
    fn reads_below_the_first_test_gate(source: &str, shipped: &str) -> bool {
        let Some(at) = source.find("#[cfg(test)]") else {
            return false;
        };
        let gate_line = source[..at].lines().count();
        shipped
            .lines()
            .skip(gate_line)
            .any(|line| !line.trim().is_empty())
    }

    /// The coverage assertion in
    /// `test_every_mindmap_write_goes_through_the_contract` is only as
    /// sharp as the predicate behind it, and a predicate that always
    /// answered `true` would restore the very silence it was added to
    /// end. So it is driven over the two texts it has to tell apart:
    /// what a correct excision leaves behind, and what the truncating
    /// reader left.
    #[test]
    fn test_only_a_non_truncating_excision_reads_below_a_test_gate() {
        let source = "pub fn a() {}\n#[cfg(test)]\nmod tests;\npub fn b() {}\n";
        assert!(
            reads_below_the_first_test_gate(source, "pub fn a() {}\n\n\npub fn b() {}\n"),
            "the gated declaration is blanked and `b` survives — that is what reading \
             past the gate looks like"
        );
        assert!(
            !reads_below_the_first_test_gate(source, "pub fn a() {}\n"),
            "the truncating reader stops at the gate, so it can never witness anything \
             below one"
        );
        assert!(
            !reads_below_the_first_test_gate("pub fn a() {}\n", "pub fn a() {}\n"),
            "a file with no gate at all witnesses nothing either way"
        );
    }

    /// The serialization verbs that put bytes somewhere — a file, a
    /// socket, a string that is about to become one.
    const PERSISTING_VERBS: &[&str] = &[
        "to_string_pretty",
        "to_string",
        "to_vec_pretty",
        "to_vec",
        "to_writer_pretty",
        "to_writer",
        "to_value",
    ];

    /// Whether a name reads as holding a map: `map`, `mind_map`,
    /// `stress_map`.
    fn reads_as_a_map(name: &str) -> bool {
        name.to_ascii_lowercase().ends_with("map")
    }

    /// Names bound by a `let` whose right-hand side is a map and whose
    /// right-hand side is not the contract.
    ///
    /// One hop, because one hop is what a bypass looks like when it is
    /// written by hand: `let value = map.clone();` followed by
    /// `serde_json::to_string_pretty(&value)` puts a raw map on disk
    /// while naming nothing map-shaped at the call site. A scan that
    /// only looked at the argument would miss it, and did — it was a
    /// surviving mutant before this existed.
    fn map_aliases(shipped: &str) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        for line in shipped.lines() {
            let Some(binding) = line.trim().strip_prefix("let ") else {
                continue;
            };
            let Some((name, value)) = binding.split_once('=') else {
                continue;
            };
            let name = name.trim().trim_start_matches("mut ").trim();
            if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            // *Is* the map, not merely reaches through one:
            // `let value = map.clone()` renames it, `let node =
            // map.nodes[id]` does not. Anything more elaborate than a
            // borrow and a clone is out of scope for a source scan and
            // says so rather than guessing.
            let renamed = value
                .trim()
                .trim_end_matches(';')
                .trim()
                .trim_start_matches(['&', '*'])
                .trim()
                .trim_end_matches(".to_owned()")
                .trim_end_matches(".clone()");
            if reads_as_a_map(renamed) && renamed.chars().all(|c| c.is_alphanumeric() || c == '_') {
                out.insert(name.to_string());
            }
        }
        out
    }

    /// Whether `line` serializes something map-shaped with one of
    /// [`PERSISTING_VERBS`] — `serde_json::to_string_pretty(&map)` and
    /// its neighbors, matched on the argument's *name* rather than its
    /// type, which a source scan cannot see.
    fn serializes_a_map(line: &str, aliases: &std::collections::BTreeSet<String>) -> bool {
        let mut rest = line;
        while let Some(at) = rest.find("serde_json::to_") {
            rest = &rest[at + "serde_json::".len()..];
            let Some(open) = rest.find('(') else { break };
            let verb = &rest[..open];
            let argument = rest[open + 1..].trim_start().trim_start_matches('&').trim_start();
            let name: String = argument
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if PERSISTING_VERBS.contains(&verb) && (reads_as_a_map(&name) || aliases.contains(&name)) {
                return true;
            }
        }
        false
    }

    /// **The one place a `MindMap` may be turned into bytes.**
    ///
    /// [`to_json_value`] says so in prose, and prose does not fail a
    /// build: `bin/generate_stress_map.rs` wrote a `.mindmap.json`
    /// with a bare `serde_json::to_string_pretty(&map)` for as long as
    /// the sentence has existed. It happened to be harmless — the map
    /// is built in memory, so it carries no preserved keys to lose —
    /// but a contract with a known exception is exactly the twin
    /// surface `lib/baumhard/CONVENTIONS.md` §B4 is about, and the
    /// next writer would not have been harmless.
    ///
    /// So the exception is gone and this is what keeps it gone. A
    /// source scan, because the question is "does another call site
    /// exist", which no runtime test can answer.
    ///
    /// Deliberately blunt in two directions, and both are the safe
    /// direction. It matches on the argument's *name* — a scan cannot
    /// see types — so a `MindMap` in a variable called something else
    /// slips through; and it flags any map-shaped name, so a
    /// `HashMap` called `map` would be a false positive, which is a
    /// reader deciding rather than a silent gap. Test sources are
    /// excluded: a test that serializes a map to compare it against
    /// something is not a persistence path.
    ///
    /// **How they are excluded is the part that was wrong.** This used
    /// to split each file at the first `#[cfg(test)]` and keep the
    /// text before it, on the premise that test modules hang off the
    /// end of a file. 140 of the 321 files it collects carry a gate
    /// before the end, and in 28 of them shipped code goes on below it
    /// — `#[cfg(test)] mod tests_delete;` and its eleven neighbors sit
    /// in the *module list* of `src/application/document/mod.rs`, the
    /// most plausible home for a save path, which was therefore read
    /// to line 35 of 757. A `serde_json::to_string_pretty(&map)`
    /// planted at line 79 of that file compiled clean and passed this
    /// test; the same function above line 35 failed it.
    /// [`source_scan::production_source`] excises the gated items and
    /// leaves everything else where it was, so the scan sees shipped
    /// code wherever it sits.
    #[test]
    fn test_every_mindmap_write_goes_through_the_contract() {
        use crate::util::source_scan::production_source;
        let workspace = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let mut offenders: Vec<String> = Vec::new();
        let mut scanned = 0usize;
        let mut read_below_a_gate = 0usize;
        let mut witness_read_below_its_gate = false;
        for root in ["lib/baumhard/src", "src", "crates"] {
            let mut files = Vec::new();
            collect_rust_sources(&workspace.join(root), &mut files);
            files.sort();
            for file in files {
                // `loader.rs` is the contract; it is allowed to be the
                // one place that calls `serde_json::to_value` on a map.
                if file.ends_with("mindmap/loader.rs") {
                    continue;
                }
                scanned += 1;
                let text = std::fs::read_to_string(&file).expect("readable source");
                let shipped = production_source(&file, &text);
                if reads_below_the_first_test_gate(&text, &shipped) {
                    read_below_a_gate += 1;
                    witness_read_below_its_gate |= file.ends_with(COVERAGE_WITNESS);
                }
                let aliases = map_aliases(&shipped);
                for (number, line) in shipped.lines().enumerate() {
                    // A line that *begins* with `//` is prose, and
                    // prose about this contract is exactly what the
                    // modules around it are full of — the sentence
                    // naming the forbidden call is not the call. A
                    // trailing comment on a line of code is left in,
                    // since dropping it would need the scan to know
                    // where a string literal ends.
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    if serializes_a_map(line, &aliases) {
                        offenders.push(format!("{}:{}: {}", file.display(), number + 1, line.trim()));
                    }
                }
            }
        }
        assert!(
            scanned > 100,
            "the scan found only {scanned} source files — it is looking in the wrong \
             place and would pass no matter what the code did"
        );
        // **A file count cannot notice that most of a *file* went
        // unread**, and that is the failure it presided over: 321
        // files collected, 321 counted, and 31k lines below a gate
        // never looked at.
        //
        // A line total is no better a discriminator than it sounds.
        // The truncating reader still saw 71% of the non-blank source
        // against 77% for the correct excision, so any floor
        // separating the two would sit inside the noise of how much
        // inline test code the workspace happens to carry, and would
        // need moving every time that changed.
        //
        // What separates them absolutely is *where* the unread lines
        // were. Truncation blanks a suffix, so **no** file can have
        // shipped code below its first `#[cfg(test)]`; a correct
        // excision blanks interiors, and 28 of these files do. So the
        // scan holds itself to reading past a gate in the one file the
        // planted bypass actually exploited — a named witness rather
        // than a threshold, because the count is a property of how the
        // workspace is laid out and the witness is a property of the
        // reader.
        assert!(
            witness_read_below_its_gate,
            "the scan never read below the first `#[cfg(test)]` in {COVERAGE_WITNESS}, \
             where twelve gated `mod` declarations sit above 700 lines of shipped \
             re-exports and setters. Either the excision in \
             `source_scan::production_source` is discarding the rest of a file rather \
             than the gated item — which is a scan that passes no matter what is written \
             below the gate, and did — or that file was reorganized and this witness \
             needs repointing at one of the {read_below_a_gate} others like it."
        );
        // And the witness has to *discriminate*, or it is satisfied by
        // a predicate that answers yes to everything. Most of these
        // files carry no test gate at all, so a count equal to the
        // whole set is not a better answer than the one above — it is
        // the same silence wearing a tick.
        assert!(
            read_below_a_gate < scanned,
            "every one of the {scanned} scanned files reports shipped code below a \
             `#[cfg(test)]`, including the ones that contain no gate. The measurement \
             is not looking at anything."
        );
        assert!(
            offenders.is_empty(),
            "these serialize a map outside `loader::to_json_value`, which is the only \
             thing that puts the keys a load preserved back into the document — \
             anything else writes a file with a newer build's data deleted from it. \
             Route it through `to_json_value`:\n  {}",
            offenders.join("\n  ")
        );
    }

    /// Every non-test `.rs` file under `dir`, recursively.
    fn collect_rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                if name != "tests" && name != "target" {
                    collect_rust_sources(&path, out);
                }
            } else if name.ends_with(".rs")
                && !name.starts_with("test")
                && !name.ends_with("_tests.rs")
                && !name.ends_with("_test.rs")
            {
                out.push(path);
            }
        }
    }

    /// **The capture drives the deserializer by hand, and a
    /// hand-driven `serde_json::Deserializer` does not check that the
    /// document ended.** `serde_json::from_str` calls `end()` for you;
    /// `unknown_keys::deserialize_capturing` has to call it itself,
    /// and without that one line a `.mindmap.json` with a second
    /// document glued onto the back of it loads clean — `maptool
    /// verify` says `valid` and exits 0 where `main` exits 1. Nothing
    /// in the suite noticed the line was load-bearing until this.
    #[test]
    fn test_trailing_content_after_the_document_is_rejected() {
        let source = map_json_with_nodes(&node_json("0", "null"));
        let doubled = format!("{source}\n{{\"and\": \"another document\"}}\n");
        let error =
            load_from_str(&doubled).expect_err("a file with a second document after the map must not load");
        assert!(
            error.contains("trailing characters"),
            "the reader has to be told what is wrong with the file, got: {error}"
        );
    }

    /// A whole map with one node, built so that every slot a
    /// [`unknown_key_shapes`] case needs is a parameter rather than a
    /// string edit.
    fn shape_map(canvas_extra: &str, node_json: &str, edges_json: &str, tail: &str) -> String {
        format!(
            r##"{{
                "version": "1.0",
                "name": "unknown-key-shapes",
                "canvas": {{"background_color": "#000"{canvas_extra}}},
                "nodes": {{{node_json}}},
                "edges": [{edges_json}]{tail}
            }}"##
        )
    }

    /// A node whose `sections` array is spelled out, so a case can put
    /// an unrecognized key inside a section rather than only on the
    /// node.
    fn node_with_sections(sections_json: &str) -> String {
        format!(
            r##""0": {{
                "id": "0", "parent_id": null,
                "position": {{"x": 0, "y": 0}},
                "size": {{"width": 100, "height": 50}},
                "sections": [{sections_json}],
                "style": {{"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false}},
                "layout": {{"type":"map","direction":"auto","spacing":0}},
                "folded": false, "notes": ""
            }}"##
        )
    }

    /// One `custom_mutations` entry around whatever payload the case
    /// is about.
    fn custom_mutation_tail(body: &str) -> String {
        format!(
            r##", "custom_mutations": [
                {{"id": "m", "name": "m", "target_scope": "SelfOnly"{body}}}
            ]"##
        )
    }

    /// **Every place an unrecognized key can sit, and the reason each
    /// one is here.**
    ///
    /// The shapes are not a sample. Each row is a distinct way the
    /// route from the document root to a captured key can fail to
    /// survive serialization, and the table exists because the first
    /// implementation of this mechanism survived a fixture round trip
    /// while losing keys in four of these positions:
    ///
    /// - a container the saver omits because it holds its own default
    ///   (`sections[0].offset`) — the route simply is not there any
    ///   more, on a save with **no edit at all**;
    /// - an externally tagged enum, whose variant level serde's
    ///   ignored-key path does not report;
    /// - the same, where the payload carries a key spelled like the
    ///   variant, which a member-count guess resolves backwards;
    /// - a key below a `#[serde(from = "…")]` proxy, where what the
    ///   load reads (`mutations`) is not what the save writes
    ///   (`mutator`).
    ///
    /// The rest pin the ordinary positions so a fix for the four
    /// cannot quietly cost one of them.
    fn unknown_key_shapes() -> Vec<(&'static str, String)> {
        let plain_node = node_json("0", "null");
        vec![
            (
                "a key at the top level",
                shape_map("", &plain_node, "", r#", "authors": ["ada"]"#),
            ),
            (
                "a key on the canvas",
                shape_map(r#", "grid_snap": 8"#, &plain_node, "", ""),
            ),
            (
                "a key on a node",
                shape_map("", &node_json_with("0", "null", r#", "hologram": true"#), "", ""),
            ),
            (
                "a key inside a node's style",
                shape_map(
                    "",
                    &node_json("0", "null")
                        .replace(r#""show_shadow":false"#, r#""show_shadow":false,"glow":3"#),
                    "",
                    "",
                ),
            ),
            (
                "a key on a section",
                shape_map(
                    "",
                    &node_with_sections(r#"{"text": "n", "txet": "typo"}"#),
                    "",
                    "",
                ),
            ),
            (
                "a key inside a section offset that holds its default",
                shape_map(
                    "",
                    &node_with_sections(r#"{"text": "n", "offset": {"x": 0.0, "y": 0.0, "grid_hint": 7}}"#),
                    "",
                    "",
                ),
            ),
            (
                // Every styling field deliberately holds a
                // non-default value: the collapsed key-path pass
                // has no omission model, and a run field sitting
                // at its default (`"bold": false`, `"hyperlink":
                // null`) is legitimately dropped by the saver per
                // the `skip_serializing_if` policy — which would
                // read as loss here while testing nothing about
                // the unknown key this shape exists for.
                "a key inside a section's text run",
                shape_map(
                    "",
                    &node_with_sections(
                        r##"{"text": "n", "text_runs": [{"start":0,"end":1,"bold":true,
                            "italic":true,"underline":true,"font":"LiberationSans","size_pt":12,
                            "color":"#fff","hyperlink":"https://example.com","tracking":2}]}"##,
                    ),
                    "",
                    "",
                ),
            ),
            (
                "a key on an edge",
                shape_map(
                    "",
                    &plain_node,
                    &edge_json_with(r#", "authored_note": "keep me""#)
                        .replace(r#""to_id": "b""#, r#""to_id": "0""#)
                        .replace(r#""from_id": "a""#, r#""from_id": "0""#),
                    "",
                ),
            ),
            (
                "a key on a palette",
                shape_map(
                    "",
                    &plain_node,
                    "",
                    r#", "palettes": {"coral": {"groups": [], "hue": 12}}"#,
                ),
            ),
            (
                "a key on a custom mutation",
                shape_map(
                    "",
                    &plain_node,
                    "",
                    &custom_mutation_tail(r#", "flavor": "sour""#),
                ),
            ),
            (
                "a key inside an externally tagged variant payload",
                shape_map(
                    "",
                    &plain_node,
                    "",
                    &custom_mutation_tail(r#", "mutator": {"Void": {"channel": 0, "surprise": 7}}"#),
                ),
            ),
            (
                "a key inside a variant payload spelled like the variant",
                shape_map(
                    "",
                    &plain_node,
                    "",
                    &custom_mutation_tail(r#", "mutator": {"Void": {"channel": 0, "Void": 99}}"#),
                ),
            ),
            (
                "a key below a serde `from` proxy, in the legacy mutation list",
                shape_map(
                    "",
                    &plain_node,
                    "",
                    &custom_mutation_tail(
                        r#", "mutations": [{"AreaDelta": {"fields": {}, "legacy_note": 1}}]"#,
                    ),
                ),
            ),
        ]
    }

    /// **The scenario #115 was opened for.** A map authored by a newer
    /// build names a mutator variant this one has never heard of. It
    /// used to take the whole document down — an empty window, and the
    /// newer feature one accidental save away from gone. Now the map
    /// opens, the construct is named in a warning that says it will not
    /// run, and the bytes are still there.
    #[test]
    fn test_a_map_with_an_unknown_variant_loads_around_it() {
        let json = shape_map(
            "",
            &node_json("0", "null"),
            "",
            &custom_mutation_tail(r#", "mutator": {"zzWarnGlow": {"channel": 0, "intensity": 3}}"#),
        );
        crate::util::test_logger::install();
        let map = load_from_str(&json).expect("the rest of the map must still load");

        assert_eq!(map.nodes.len(), 1, "the document around it is untouched");
        assert_eq!(map.nodes["0"].sections[0].text, "n");
        assert!(
            map.custom_mutations.is_empty(),
            "a construct this build cannot carry out must not appear in the model"
        );
        assert_eq!(map.skipped_constructs.len(), 1);

        let reported = crate::util::test_logger::lines_containing("zzWarnGlow");
        assert_eq!(reported.len(), 1, "expected one warning, got {reported:?}");
        let line = &reported[0];
        assert!(line.contains("custom_mutations[0]"), "{line}");
        assert!(
            line.contains("nothing it describes will run"),
            "the consequence is the part the reader needs: {line}"
        );
    }

    /// The construct comes back out of a save byte-for-byte, including
    /// when the model has **no** custom mutations left to write —
    /// `custom_mutations` carries `skip_serializing_if =
    /// "Vec::is_empty"`, so the list the construct belongs in is not
    /// even in the saved document until the splice puts it back. That
    /// is the acute case of the whole feature: a map whose single
    /// custom mutation is the one from the future.
    #[test]
    fn test_a_skipped_construct_survives_a_save_that_writes_no_others() {
        let authored = r#"{"id": "glow-it", "name": "Glow it", "target_scope": "SelfOnly",
             "mutator": {"zzSaveGlow": {"channel": 0, "intensity": 3}}}"#;
        let json = shape_map(
            "",
            &node_json("0", "null"),
            "",
            &format!(r#", "custom_mutations": [{authored}]"#),
        );
        let map = load_from_str(&json).expect("loads around the construct");
        let saved = to_json_value(&map).expect("serializes");

        assert_eq!(
            saved["custom_mutations"],
            serde_json::from_str::<serde_json::Value>(&format!("[{authored}]")).expect("valid"),
            "the construct has to come back exactly as it was authored"
        );
        let reloaded = load_from_str(&serde_json::to_string(&saved).expect("render"))
            .expect("the saved document still loads");
        assert_eq!(reloaded.skipped_constructs.len(), 1, "and still carries it");
    }

    /// **The tolerant door carries the ceiling too, and nothing used
    /// to check that.**
    ///
    /// `load_from_str` has two ways in. The strict one refuses over
    /// `MAX_UNKNOWN_KEYS` before `adopt_unknown_keys`; the tolerant one
    /// — reached when the typed parse fails on a construct this build
    /// cannot name — collects its own routes in
    /// `deserialize_value_capturing` and has its own refusal. The
    /// sibling test above drives `load_from_str` and
    /// `parse_for_inspection`, which is the *same* door twice, so
    /// deleting the tolerant guard left the whole workspace green. A
    /// review round found it by deleting it.
    ///
    /// Reaching this door needs a fixture that satisfies four things at
    /// once: the typed parse must fail, the document must not be a
    /// legacy shape, at least one construct must be excisable, and the
    /// remainder must carry more than the ceiling in unknown keys.
    #[test]
    fn test_the_tolerant_door_carries_the_unknown_key_ceiling() {
        // The ceiling is `None` on every target this suite runs on, so
        // the test supplies one and drives the real door with it.
        const CAP: usize = 64;

        // An unreadable mutator variant is what sends the load down the
        // tolerant path at all.
        let unreadable =
            r#"{"id": "mid", "name": "mid", "target_scope": "SelfOnly", "mutator": {"zzOrderGlow": {}}}"#;
        let mut extra = String::new();
        for i in 0..CAP + 8 {
            extra.push_str(&format!(r#", "k{i}": 0"#));
        }
        let json = shape_map(
            "",
            &node_json_with("0", "null", &extra),
            "",
            &format!(r#", "custom_mutations": [{unreadable}]"#),
        );

        let err = crate::mindmap::unknown_keys::with_key_ceiling(Some(CAP), || load_from_str(&json))
            .expect_err("the tolerant door must refuse past the ceiling, not load partially");
        assert!(
            err.contains("unrecognized keys") && !err.contains("zzOrderGlow"),
            "must refuse on the ceiling and say so, not fall through to the construct \
             diagnosis that merely sent it down this path: {err}"
        );

        // The control that makes the fixture honest: the SAME document
        // with a handful of unknown keys instead of too many really does
        // take the tolerant path and load. Without this, the assertion
        // above would pass on a fixture that never reached this door.
        let mut few = String::new();
        for i in 0..8 {
            few.push_str(&format!(r#", "k{i}": 0"#));
        }
        let json = shape_map(
            "",
            &node_json_with("0", "null", &few),
            "",
            &format!(r#", "custom_mutations": [{unreadable}]"#),
        );
        let map = crate::mindmap::unknown_keys::with_key_ceiling(Some(CAP), || load_from_str(&json))
            .expect("the tolerant path loads around the construct");
        assert_eq!(
            map.skipped_constructs.len(),
            1,
            "and it really was the tolerant path"
        );
        assert_eq!(map.unknown_keys.len(), 8, "with the keys captured");
    }

    /// A skipped construct goes back at the index it was authored at,
    /// between the entries the model does write, rather than at
    /// whatever position the shortened list left behind.
    #[test]
    fn test_a_skipped_construct_goes_back_where_it_was_authored() {
        let readable = |id: &str| format!(r#"{{"id": "{id}", "name": "{id}", "target_scope": "SelfOnly"}}"#);
        let unreadable =
            r#"{"id": "mid", "name": "mid", "target_scope": "SelfOnly", "mutator": {"zzOrderGlow": {}}}"#;
        let json = shape_map(
            "",
            &node_json("0", "null"),
            "",
            &format!(
                r#", "custom_mutations": [{}, {unreadable}, {}]"#,
                readable("first"),
                readable("last")
            ),
        );
        let map = load_from_str(&json).expect("loads around the construct");
        assert_eq!(map.custom_mutations.len(), 2);

        let saved = to_json_value(&map).expect("serializes");
        let ids: Vec<&str> = saved["custom_mutations"]
            .as_array()
            .expect("array")
            .iter()
            .map(|entry| entry["id"].as_str().expect("id"))
            .collect();
        assert_eq!(ids, ["first", "mid", "last"]);
    }

    /// **Where the splice order stops being defensive and decides the
    /// outcome.**
    ///
    /// `to_json_value` writes captured keys before skipped constructs
    /// because a key's positional route counts the array *after* the
    /// constructs were lifted out. For almost every map that ordering
    /// is belt-and-braces: the element fingerprints
    /// [`UnknownKeys::splice_into`](crate::mindmap::unknown_keys::UnknownKeys::splice_into)
    /// carries re-find the element whichever length the array is, so
    /// swapping the two lines changes nothing observable. That was a
    /// surviving mutant, and the comment on those two lines used to
    /// claim a guarantee the ordering was not the thing providing.
    ///
    /// This is the shape where it *is* the thing providing it. Two
    /// custom mutations serialize identically — same id, same name,
    /// same scope — so the fingerprint of the one that carried the key
    /// matches in two places and a search cannot say which. With the
    /// array at the length the route was recorded against, no search is
    /// needed: the recorded index matches exactly. Re-insert the
    /// skipped construct first and the index names the construct, the
    /// fingerprint matches twice, the lengths no longer agree, and the
    /// key is dropped with a warning instead of written.
    ///
    /// Duplicate entries are not hypothetical hygiene: nothing in the
    /// loader or in `format/validation.md` makes a `custom_mutations`
    /// id unique, and a map assembled by concatenating two files has
    /// them by construction.
    #[test]
    fn test_the_splice_order_decides_when_two_elements_are_alike() {
        // Two entries this build reads and cannot tell apart, and one
        // it cannot read at all sitting in front of them.
        let twin = r#"{"id": "dup", "name": "dup", "target_scope": "SelfOnly"}"#;
        let unreadable = r#"{"id": "future", "name": "future", "target_scope": "SelfOnly",
             "mutator": {"zzTieGlow": {"channel": 0}}}"#;
        let carrying = r#"{"id": "dup", "name": "dup", "target_scope": "SelfOnly",
             "zzambiguous_note": 1}"#;
        let json = shape_map(
            "",
            &node_json("0", "null"),
            "",
            &format!(r#", "custom_mutations": [{unreadable}, {carrying}, {twin}]"#),
        );

        let map = load_from_str(&json).expect("the map loads around the unreadable entry");
        assert_eq!(map.custom_mutations.len(), 2, "the two twins survive the load");
        assert_eq!(map.skipped_constructs.len(), 1);
        assert_eq!(map.unknown_keys.len(), 1);

        // **The premise, checked rather than assumed.** The whole test
        // rests on this build being unable to tell the two entries
        // apart. A field added to the written shape that happens to
        // differ between them would make the fingerprint search
        // succeed, the ordering stop mattering, and this test go on
        // passing while proving nothing.
        let written = serde_json::to_value(&map).expect("the model serializes");
        let written = written["custom_mutations"].as_array().expect("an array");
        assert_eq!(
            written[0], written[1],
            "the two entries have to be indistinguishable on the wire for this to be \
             testing the splice order at all"
        );

        let saved = to_json_value(&map).expect("serializes");
        let entries = saved["custom_mutations"].as_array().expect("an array");
        assert_eq!(entries.len(), 3, "everything authored comes back: {saved}");

        // The construct returns to the index it was authored at …
        assert!(
            entries[0]["mutator"].get("zzTieGlow").is_some(),
            "the skipped construct belongs at index 0: {}",
            entries[0]
        );
        // … and the key returns to the twin that carried it, not to
        // its indistinguishable neighbor and not to nowhere.
        assert_eq!(
            entries[1]["zzambiguous_note"],
            serde_json::json!(1),
            "the preserved key belongs on the first twin: {saved}"
        );
        assert_eq!(
            entries[2].get("zzambiguous_note"),
            None,
            "and on no other element: {saved}"
        );
    }

    /// **The order a reader is told things in is behavior too.** A
    /// map whose nodes each carry a construct from the future
    /// produces one `warn!` per construct, and which order they come
    /// out in is decided by the order
    /// `excise_unreadable_constructs` walks `nodes`. Two nodes
    /// authored high-id-first must still be reported low-id-first, or
    /// the same file read twice tells the user two different stories.
    ///
    /// The mechanism is that `serde_json::Map` is a `BTreeMap` here,
    /// so `keys()` is already sorted; an explicit `sort()` used to
    /// restate that and was indistinguishable from its own absence.
    /// This pins the property rather than the restatement, so it goes
    /// red exactly when the mechanism stops holding — which is when
    /// something in the dependency graph unions serde_json's
    /// `preserve_order` feature in and object iteration becomes
    /// document order. The fix then is to sort explicitly again.
    #[test]
    fn test_skip_warnings_come_out_in_node_id_order() {
        crate::util::test_logger::install();
        let binding = |variant: &str| {
            format!(r#", "trigger_bindings": [{{"trigger": "{variant}", "mutation_id": "m"}}]"#)
        };
        // Authored high id first, so document order and id order
        // disagree and only one of them can be what comes out.
        let json = map_json_with_nodes(&format!(
            "{}, {}",
            node_json_with("2", "null", &binding("zzOrderProbeHigh")),
            node_json_with("1", "null", &binding("zzOrderProbeLow")),
        ));

        let map = load_from_str(&json).expect("the map loads around both constructs");
        assert_eq!(map.skipped_constructs.len(), 2);

        let reported = crate::util::test_logger::lines_containing("zzOrderProbe");
        assert_eq!(reported.len(), 2, "expected both warnings, got {reported:?}");
        assert!(
            reported[0].contains("node \"1\""),
            "node \"1\" must be reported before node \"2\" even though the file authors \
             \"2\" first — if this went red, serde_json's `preserve_order` feature is on \
             somewhere in the graph and `excise_unreadable_constructs` has to sort the \
             node ids itself again:\n  {reported:?}"
        );
        assert!(reported[1].contains("node \"2\""), "{reported:?}");
    }

    /// **The second shape the splice order decides, and the one the
    /// twins test misses.** Its sibling above needs two
    /// indistinguishable elements; this one needs no duplicates at
    /// all, only an ordinary **edit** to the element the key hangs
    /// off while a skipped construct shares its array.
    ///
    /// After the edit the element's load-time fingerprint matches
    /// nowhere, so `resolve_index`'s first two rungs — exact position,
    /// then unique match — both come up empty. The one left is "every
    /// *other* element is unchanged and the array is still the length
    /// the route was recorded against", and the route was recorded
    /// against the array with the construct lifted out. Splice the
    /// construct back first and the array is one longer than that, the
    /// last rung fails on the length alone, and the key is dropped
    /// with a warning — from nothing more unusual than renaming a
    /// custom mutation in a map that also carries one from the future.
    #[test]
    fn test_the_splice_order_survives_an_edit_beside_a_skipped_construct() {
        let unreadable = r#"{"id": "future", "name": "future", "target_scope": "SelfOnly",
             "mutator": {"zzEditGlow": {"channel": 0}}}"#;
        let carrying = r#"{"id": "keep", "name": "as authored", "target_scope": "SelfOnly",
             "zzedited_note": 1}"#;
        let json = shape_map(
            "",
            &node_json("0", "null"),
            "",
            &format!(r#", "custom_mutations": [{unreadable}, {carrying}]"#),
        );

        let mut map = load_from_str(&json).expect("the map loads around the unreadable entry");
        assert_eq!(map.custom_mutations.len(), 1, "one readable entry survives");
        assert_eq!(map.skipped_constructs.len(), 1);
        assert_eq!(map.unknown_keys.len(), 1);

        // The edit. **The premise, checked rather than assumed:** the
        // element the key hangs off has to actually change on the
        // wire, or its load-time fingerprint still matches and the
        // last rung is never the one deciding.
        let before = serde_json::to_value(&map).expect("the model serializes");
        map.custom_mutations[0].name = "renamed by the user".to_string();
        let after = serde_json::to_value(&map).expect("the model serializes");
        assert_ne!(
            before["custom_mutations"][0], after["custom_mutations"][0],
            "the edit has to be visible on the wire for this to test the splice order"
        );

        let saved = to_json_value(&map).expect("serializes");
        let entries = saved["custom_mutations"].as_array().expect("an array");
        assert_eq!(entries.len(), 2, "everything authored comes back: {saved}");
        assert!(
            entries[0]["mutator"].get("zzEditGlow").is_some(),
            "the skipped construct belongs at index 0: {}",
            entries[0]
        );
        assert_eq!(
            entries[1]["zzedited_note"],
            serde_json::json!(1),
            "an edit to the element must not cost the key it carried: {saved}"
        );
        assert_eq!(
            entries[1]["name"],
            serde_json::json!("renamed by the user"),
            "and the edit itself has to be what was saved: {saved}"
        );
    }

    /// A trigger binding is the other unit — on a node and on a
    /// section, both of which a newer build can attach a trigger kind
    /// to.
    #[test]
    fn test_an_unknown_trigger_variant_costs_only_its_binding() {
        let node = node_with_sections(
            r#"{"text": "n", "trigger_bindings": [{"trigger": "zzOnGaze", "mutation_id": "m"}]}"#,
        )
        .replace(
            r#""folded": false"#,
            r#""trigger_bindings": [{"trigger": "OnClick", "mutation_id": "keep"}], "folded": false"#,
        );
        let json = shape_map("", &node, "", "");
        let map = load_from_str(&json).expect("the node must still load");

        assert_eq!(map.nodes["0"].sections.len(), 1, "the section is still there");
        assert!(map.nodes["0"].sections[0].trigger_bindings.is_empty());
        assert_eq!(
            map.nodes["0"].trigger_bindings.len(),
            1,
            "a binding this build *can* read is untouched"
        );
        assert_eq!(map.skipped_constructs.len(), 1);

        let saved = to_json_value(&map).expect("serializes");
        assert_eq!(
            saved["nodes"]["0"]["sections"][0]["trigger_bindings"][0]["trigger"],
            serde_json::json!("zzOnGaze")
        );
    }

    /// **Skipping is for constructs, not for the map.** A node this
    /// build cannot read is not a thing the reader can be shown
    /// "around" — the map they get would be missing part of itself
    /// with no sign of which part. So the load still refuses, with the
    /// message it always had.
    #[test]
    fn test_an_unreadable_node_still_fails_the_whole_load() {
        let json = shape_map(
            "",
            &node_json("0", "null").replace(r#""folded": false"#, r#""folded": "yes-please""#),
            "",
            "",
        );
        let error = load_from_str(&json).expect_err("an unreadable node must not be skipped");
        assert!(error.contains("node \"0\""), "got: {error}");
    }

    /// **Every enum a load can meet has to sit inside something the
    /// loader can skip**, or a variant from the future in it is still
    /// a dead document. The two skippable units are hand-named in
    /// `excise_unreadable_constructs`, which is exactly the twin
    /// surface §B4 warns about — so the source walk checks the claim
    /// rather than the comment asserting it.
    ///
    /// **"Only reachable through a skippable construct", not "also
    /// reachable from one."** The distinction is the whole strength
    /// of the check. An enum used *both* inside a custom mutation and
    /// directly on a node passes the weaker test — it is reachable
    /// from `CustomMutation`, after all — and a variant a newer build
    /// adds to it still kills the document when it turns up at the
    /// node. The skippable use is not what makes the unskippable one
    /// safe. So the walk cuts the skippable roots out of the graph
    /// and asks what enums are still reachable, which is a question
    /// the double use cannot satisfy.
    ///
    /// It costs nothing today — the model deliberately spells the
    /// unskippable open vocabularies (`shape`, `line_style`,
    /// `edge_type`) as `String` with a documented fallback, so the
    /// unskippable closure holds no variant-bearing Deserialize enum
    /// at all. That is the property being pinned, and it is only
    /// worth pinning in the form that would notice it breaking.
    #[test]
    fn test_every_variant_bearing_enum_sits_inside_a_skippable_construct() {
        use crate::util::serde_coverage::{crate_src_root, TypeGraph, TypeKind};

        // The units `excise_unreadable_constructs` lifts out, plus
        // the proxy that carries `CustomMutation`'s on-disk shape.
        const SKIPPABLE: &[&str] = &["CustomMutation", "CustomMutationIn", "TriggerBinding"];

        let graph = TypeGraph::build(&crate_src_root());
        let unskippable = graph.reachable_from_excluding("MindMap", SKIPPABLE);
        let stranded: Vec<&str> = unskippable
            .iter()
            .filter(|info| {
                info.kind == TypeKind::Enum && info.derives_deserialize && !info.variants.is_empty()
            })
            .map(|info| info.name.as_str())
            .collect();
        assert!(
            stranded.is_empty(),
            "these enums are reachable from a `.mindmap.json` load along a path that \
             passes through no construct the loader can skip, so a variant a newer build \
             adds to one of them still makes the whole document unloadable. Being \
             reachable from a custom mutation as well does not help — the unskippable \
             use is the one that decides. Either widen the skippable set in \
             `excise_unreadable_constructs` — and say why the new unit's absence is \
             visible rather than a silent partial behavior — or spell the field as a \
             `String` with a documented fallback the way the open vocabularies are: \
             {stranded:?}"
        );

        // An exclusion that excluded everything would pass this just
        // as loudly, so the walk has to prove it still reached the
        // unskippable part of the model *and* that cutting the
        // skippable roots out removed something.
        assert!(
            unskippable.iter().any(|info| info.name == "MindNode"),
            "cutting the skippable roots must not cut the model out from under the check"
        );
        assert!(
            !unskippable.iter().any(|info| info.name == "MutationBehavior"),
            "a type only reachable through a skippable construct must be excluded, or \
             the check has stopped distinguishing the two paths"
        );
    }

    /// **A load followed by a save, with nothing touched in between,
    /// must not lose a single authored key — in any of the places one
    /// can sit.**
    ///
    /// That is issue #115's acceptance criterion, and it is the one
    /// the first implementation of the capture failed: a key nested
    /// inside a container the saver omits (a section `offset` holding
    /// its own default) was warned about as "kept as written and saved
    /// back unchanged" and then dropped by the very next save, with no
    /// user action of any kind.
    ///
    /// Table-driven rather than one case per position on purpose. The
    /// positions are a *set* — the property is "everywhere", and a
    /// hand-picked case is exactly how four of them went unnoticed.
    #[test]
    fn test_a_zero_edit_round_trip_keeps_every_authored_key() {
        let mut failures: Vec<String> = Vec::new();
        for (what, source) in unknown_key_shapes() {
            let map = match load_from_str(&source) {
                Ok(map) => map,
                Err(e) => {
                    failures.push(format!("{what}: the fixture does not even load: {e}"));
                    continue;
                }
            };
            assert!(
                !map.unknown_keys.is_empty(),
                "{what}: the fixture is supposed to carry an unrecognized key, \
                 and the loader found none — the shape stopped testing anything"
            );
            let saved = to_json_value(&map).expect("serializing a loaded map");
            let before = key_path_set(&source);
            let after = key_path_set(&serde_json::to_string(&saved).expect("render"));
            let lost: Vec<&String> = before.difference(&after).collect();
            if !lost.is_empty() {
                failures.push(format!("{what}: lost {lost:?}"));
            }
            // A preserved key is worth nothing if the file it lands in
            // no longer loads.
            if let Err(e) = load_from_str(&serde_json::to_string(&saved).expect("render")) {
                failures.push(format!("{what}: the saved document no longer loads: {e}"));
            }
        }
        assert!(
            failures.is_empty(),
            "a load → save with no edit lost authored keys:\n  {}",
            failures.join("\n  ")
        );
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

    /// [`OMITTABLE_WHEN`] has to account for **every spelling the
    /// loader accepts** for an omitted key, not only the ones some
    /// fixture happens to use — the whole point of hand-modeling
    /// the predicates is that the indexed pass can then be strict.
    ///
    /// `ColorOverrides::is_empty`'s entry tested `as_object()`
    /// alone, which is `false` for `Value::Null`, one commit after
    /// `"overrides": null` became legal input that the writer drops.
    /// The first round-trip fixture to carry that spelling would
    /// have been reported as authored-key loss on a file that is
    /// correct in both directions.
    ///
    /// Each spelling is put through the real load and the real save
    /// first, so the test proves the model matches the code rather
    /// than restating the predicate against itself.
    #[test]
    fn test_the_omission_model_accounts_for_every_empty_overrides_spelling() {
        let (_, omits) = OMITTABLE_WHEN
            .iter()
            .find(|(name, _)| *name == "ColorOverrides::is_empty")
            .expect("the overrides predicate is modeled");

        for spelling in [
            "null",
            "{}",
            r#"{"background": null, "frame": null, "text": null, "title": null}"#,
        ] {
            let json = map_json_with_nodes(&node_json_with(
                "0",
                "null",
                &format!(
                    r#", "color_schema": {{"palette": "coral", "level": 0,
                         "starts_at_root": true, "connections_colored": true,
                         "overrides": {spelling}}}"#
                ),
            ));
            let map = load_from_str(&json).unwrap_or_else(|e| panic!("`{spelling}` must load: {e}"));
            let saved: Value =
                serde_json::from_str(&saved_json(&map, "overrides-omission")).expect("valid JSON");
            assert!(
                saved["nodes"]["0"]["color_schema"].get("overrides").is_none(),
                "`{spelling}` carries no override, so the writer omits the key"
            );
            assert!(
                omits(&serde_json::from_str::<Value>(spelling).expect("valid JSON")),
                "…and the omission model has to forgive `{spelling}`, or the next \
                 round-trip fixture spelling it that way reports data loss"
            );
        }

        assert!(
            !omits(&serde_json::json!({"frame": "#020202"})),
            "a channel the author named is not an omission the model forgives"
        );
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

    /// A `nodes` key that disagrees with the node's own `id` is
    /// rejected.
    ///
    /// Not a tidiness rule — it is what makes the cycle check below
    /// sound. The two spellings address different graphs, so a
    /// mismatch lets a file be acyclic to `detect_parent_cycle`
    /// (which walks keys) and a self-loop to `ChildIndex` (which
    /// walks `node.id`), which is what every scene build and fold
    /// walk actually traverses.
    #[test]
    fn test_node_key_must_match_node_id() {
        let json =
            map_json_with_nodes(&node_json("0", "null").replace(r#""id": "0""#, r#""id": "elsewhere""#));
        let err = load_from_str(&json).expect_err("a key / id mismatch must be rejected");
        assert!(err.contains("elsewhere"), "must name the node's id: {err}");
        assert!(err.contains("must match"), "must state the rule: {err}");
    }

    /// **The self-loop the key / id mismatch used to smuggle past
    /// the cycle check.** `detect_parent_cycle` sees key `"k"` whose
    /// parent `"a"` is absent — a dangling root, no cycle. But the
    /// node's `id` *is* `"a"`, so `ChildIndex` files it as its own
    /// child, and the scene builder descending that edge never
    /// terminates.
    #[test]
    fn test_id_graph_self_loop_is_rejected() {
        let node = node_json("k", "\"a\"").replace(r#""id": "k""#, r#""id": "a""#);
        let err = load_from_str(&map_json_with_nodes(&node))
            .expect_err("a node that is its own child in the id graph must be rejected");
        assert!(
            err.contains("must match"),
            "the key / id rule is what catches this: {err}"
        );
    }

    /// **The stack-overflow regression, on a deliberately small
    /// stack.**
    ///
    /// A linear `parent_id` chain is a legal acyclic tree, so the
    /// loader accepts it and every walker downstream inherits its
    /// depth. While those walkers recursed, a chain like this
    /// exhausted the thread stack and killed the process with
    /// `SIGABRT` — not a panic, so nothing could catch, log, or
    /// degrade it, and the user's unsaved work went with it.
    ///
    /// The walks run on a 256 KiB stack rather than a test
    /// thread's default couple of megabytes. That is what keeps the
    /// test both fast and honest: a few thousand nodes is cheap to
    /// build, and any reintroduced recursion blows a stack that
    /// small long before the chain ends, while the iterative form
    /// is indifferent to it because its frontier lives on the heap.
    ///
    /// A failure here aborts the test binary instead of failing the
    /// assertion — that is what a stack overflow does, and it is
    /// precisely the outcome under test.
    #[test]
    fn test_deep_parent_chain_does_not_exhaust_the_stack() {
        const DEPTH: usize = 6_000;
        const SMALL_STACK: usize = 256 * 1024;

        let nodes = (0..DEPTH)
            .map(|i| {
                let parent = if i == 0 {
                    "null".to_string()
                } else {
                    format!("\"n{}\"", i - 1)
                };
                node_json(&format!("n{i}"), &parent)
            })
            .collect::<Vec<_>>()
            .join(",");
        let map = load_from_str(&map_json_with_nodes(&nodes)).expect("a deep chain is acyclic and loads");

        let walked = std::thread::Builder::new()
            .stack_size(SMALL_STACK)
            .spawn(move || {
                let hidden = map.fold_hidden_set().len();
                let descendants = map.all_descendants("n0").len();
                let mut tree = crate::mindmap::tree_builder::build_mindmap_tree(&map);
                let arena_nodes = tree.tree.arena.len();
                // The scene build alone does not reach the two
                // walkers that made this crash land on a *mouse
                // move*: `compute_subtree_aabbs` is gated behind the
                // dirty flag and `bvh_find` only runs on a hit test.
                // `descendant_at` is what drives both, so the probe
                // has to ask for a hit or the highest-impact half of
                // the conversion goes untested.
                let _hit = tree.tree.descendant_at(glam::Vec2::new(10.0, 10.0));
                (hidden, descendants, arena_nodes)
            })
            .expect("spawn the small-stack walker")
            .join()
            .expect("the walkers must not exhaust a 256 KiB stack");

        let (hidden, descendants, arena_nodes) = walked;
        assert_eq!(hidden, 0, "nothing is folded, so nothing is hidden");
        assert_eq!(
            descendants,
            DEPTH - 1,
            "every node below the root is a descendant"
        );
        assert!(
            arena_nodes >= DEPTH,
            "the scene tree must carry every node: {arena_nodes} < {DEPTH}"
        );
    }

    /// **The zero that aborted the process.** A border font size of
    /// zero reaches cosmic-text's `Buffer::new`, whose
    /// `assert_ne!(line_height, 0.0)` fires on the scene-build path
    /// — outside any `catch_unwind`, so the editor dies on the frame
    /// after the map opens.
    #[test]
    fn test_zero_border_font_size_is_rejected() {
        let node = node_json("0", "null").replace(
            r#""show_shadow":false"#,
            r#""show_shadow":false,"border":{"font_size_pt":0.0}"#,
        );
        let err = load_from_str(&map_json_with_nodes(&node)).expect_err("a zero font size must be rejected");
        assert!(err.contains("node \"0\""), "must name the node: {err}");
        assert!(err.contains("font_size_pt"), "must name the field: {err}");
    }

    /// JSON has no `Infinity` literal, but `1e39` does not fit an
    /// `f32` and arrives as one — so the finiteness screens are
    /// reachable from an ordinary-looking number rather than an
    /// exotic token.
    #[test]
    fn test_f32_overflow_to_infinity_is_rejected() {
        let node = node_json("0", "null").replace(
            r#""show_shadow":false"#,
            r#""show_shadow":false,"border":{"font_size_pt":1e39}"#,
        );
        let err = load_from_str(&map_json_with_nodes(&node))
            .expect_err("a font size that overflows f32 must be rejected");
        assert!(
            err.contains("not finite") || err.contains("ceiling"),
            "must reject the overflowed size: {err}"
        );
    }

    /// **The inverted clamp.** `f32::clamp` panics when its bounds
    /// cross, and every size cascade resolves a `min` / `max` pair
    /// straight out of the document into one.
    #[test]
    fn test_inverted_font_size_clamp_is_rejected() {
        let nodes = format!("{},{}", node_json("a", "null"), node_json("b", "\"a\""));
        let edge =
            edge_json_with(r#", "glyph_connection": {"min_font_size_pt": 40.0, "max_font_size_pt": 8.0}"#);
        let err = load_from_str(&map_json(&nodes, &edge)).expect_err("an inverted clamp must be rejected");
        assert!(err.contains("edge[0]"), "must name the edge: {err}");
        assert!(err.contains("above max"), "must explain the inversion: {err}");
    }

    /// A canvas default is the fallback for every element that does
    /// not override it, so one hostile number there poisons the
    /// whole document rather than a single node.
    #[test]
    fn test_canvas_default_border_is_screened_too() {
        let json = map_json_with_nodes(&node_json("0", "null")).replace(
            r##""canvas": {"background_color": "#000"}"##,
            r##""canvas": {"background_color": "#000", "default_border": {"font_size_pt": 0.0}}"##,
        );
        let err = load_from_str(&json).expect_err("a hostile canvas default must be rejected");
        assert!(err.contains("canvas:"), "must name the canvas: {err}");
    }

    /// Node geometry that would explode a downstream allocation is
    /// refused at the boundary. `validate::node_size` had always
    /// known this shape; nothing called it on the load path.
    #[test]
    fn test_absurd_node_size_is_rejected_at_load() {
        let node = node_json("0", "null").replace(
            r#""size": {"width": 100, "height": 50}"#,
            r#""size": {"width": 1e12, "height": 50}"#,
        );
        let err =
            load_from_str(&map_json_with_nodes(&node)).expect_err("an absurd node size must be rejected");
        assert!(err.contains("node \"0\""), "must name the node: {err}");
        assert!(err.contains("ceiling"), "must cite the ceiling: {err}");
    }

    /// Text runs must be sorted, non-overlapping, non-empty, and
    /// inside the section's text — the invariants `text_run_ops`
    /// and the styled-span bridge already assumed, and the same
    /// four `maptool verify` reports. Each is checked with the
    /// wording the tool uses so the two agree about what a valid
    /// map is.
    #[test]
    fn test_malformed_text_runs_are_rejected() {
        let run = |start: usize, end: usize| {
            format!(
                r##"{{"start":{start},"end":{end},"bold":false,"italic":false,"underline":false,
                     "font":"LiberationSans","size_pt":14,"color":"#fff","hyperlink":null}}"##
            )
        };
        let with_runs = |runs: String| {
            let styled = node_json("0", "null").replace(
                r#""sections": [{"text": "n"}]"#,
                &format!(r#""sections": [{{"text": "abcdef", "text_runs": [{runs}]}}]"#),
            );
            map_json_with_nodes(&styled)
        };

        for (runs, expected) in [
            (run(3, 3), "not less than end"),
            (run(4, 2), "not less than end"),
            (format!("{},{}", run(0, 4), run(2, 6)), "overlaps previous run"),
            (run(0, 99), "exceeds text length"),
        ] {
            let err = load_from_str(&with_runs(runs)).expect_err("a malformed run table must be rejected");
            assert!(
                err.contains(expected),
                "expected {expected:?} in the rejection, got: {err}"
            );
        }

        // The well-formed table the four above are deviations from.
        let map = load_from_str(&with_runs(format!("{},{}", run(0, 2), run(2, 6))))
            .expect("sorted, non-overlapping, in-bounds runs load");
        assert_eq!(map.nodes["0"].sections[0].text_runs.len(), 2);
    }

    /// **The envelope that pins the event loop.** While an
    /// animation is live the loop holds `ControlFlow::Poll`, so a
    /// `u32` millisecond field is also how long a map can keep the
    /// process off its idle path — about 49 days per field. A maxed
    /// *delay* draws nothing at all, so the app looks idle while
    /// spinning a core.
    #[test]
    fn test_absurd_animation_envelope_is_rejected() {
        let mutation = |timing: &str| {
            format!(
                r#"{{"id":"spin","name":"spin","description":"","contexts":["map.node"],
                     "target_scope":"SelfOnly","timing":{timing}}}"#
            )
        };
        let map_with = |body: String| {
            map_json_with_nodes(&node_json("0", "null")).replace(
                r#""edges": []"#,
                &format!(r#""edges": [], "custom_mutations": [{body}]"#),
            )
        };

        let err = load_from_str(&map_with(mutation(r#"{"duration_ms": 4000000000}"#)))
            .expect_err("an absurd animation duration must be rejected");
        assert!(
            err.contains("custom_mutations[0]"),
            "must name the mutation: {err}"
        );
        assert!(err.contains("duration_ms"), "must name the field: {err}");

        let err = load_from_str(&map_with(mutation(
            r#"{"duration_ms": 200, "delay_ms": 4000000000}"#,
        )))
        .expect_err("an absurd animation delay must be rejected");
        assert!(err.contains("delay_ms"), "the delay is the invisible half: {err}");

        // An ordinary transition is untouched.
        load_from_str(&map_with(mutation(r#"{"duration_ms": 250}"#)))
            .expect("a quarter-second transition is ordinary and must load");
    }

    /// **The trust boundary reaches the mutation payloads too.** A
    /// map's own `custom_mutations` and a node's `inline_mutations`
    /// enter the model alongside its geometry but take effect later,
    /// on a click — so a payload the sweep skipped would be a number
    /// arriving at the same shaper one interaction after the load
    /// the checks were supposed to gate.
    #[test]
    fn test_inline_mutation_payloads_are_screened_too() {
        let node = node_json_with(
            "0",
            "null",
            r#", "inline_mutations": [{"id":"n.spin","name":"","description":"",
                 "contexts":["map.node"],"target_scope":"SelfOnly",
                 "timing":{"duration_ms": 4000000000}}]"#,
        );
        let err = load_from_str(&map_json_with_nodes(&node))
            .expect_err("a hostile inline mutation payload must be rejected");
        assert!(err.contains("node \"0\""), "must name the node: {err}");
        assert!(
            err.contains("inline_mutations[0]"),
            "must name the mutation: {err}"
        );
    }

    /// **The multiplier behind a bounded sample count.** The body
    /// glyph is emitted once per sampled point along a path, so its
    /// length multiplies that count: capping the samples alone still
    /// leaves `samples × |body|` unbounded, and each repeat is a
    /// clone, a grapheme walk, a `GlyphArea`, and a shaped buffer.
    #[test]
    fn test_overlong_connection_glyph_is_rejected() {
        let nodes = format!("{},{}", node_json("a", "null"), node_json("b", "\"a\""));
        let long = "\u{25c8}".repeat(64);
        let edge = edge_json_with(&format!(r#", "glyph_connection": {{"body": "{long}"}}"#));
        let err =
            load_from_str(&map_json(&nodes, &edge)).expect_err("an overlong body glyph must be rejected");
        assert!(err.contains("edge[0]"), "must name the edge: {err}");
        assert!(err.contains("body"), "must name the field: {err}");
        assert!(
            err.contains("once per sampled point"),
            "must say why length matters here: {err}"
        );

        // A real multi-grapheme motif still loads — the cap is on
        // absurdity, not on decorative connections.
        let motif = edge_json_with(r#", "glyph_connection": {"body": "◈··"}"#);
        let map = load_from_str(&map_json(&nodes, &motif)).expect("a short motif is ordinary");
        assert_eq!(map.edges.len(), 1);
    }

    /// **The cluster ceiling does not bound the allocation; the byte
    /// ceiling does.** A UAX #29 extended grapheme cluster has no
    /// length bound — a base character plus any number of combining
    /// marks is exactly *one* cluster — so a body glyph can be
    /// arbitrarily large and still satisfy
    /// `MAX_CONNECTION_GLYPH_GRAPHEMES`. Since the body is cloned
    /// once per sampled point, that turned a bounded sample count
    /// back into an unbounded allocation: at `MAX_PATH_SAMPLES` a
    /// one-megabyte cluster asks for gigabytes during the first
    /// scene build after open.
    ///
    /// The cluster count is deliberately kept alongside it rather
    /// than replaced. They bound different things — clusters make
    /// the field a motif rather than a paragraph, bytes make the
    /// product finite — so this asserts the byte rejection on a
    /// glyph the cluster check *passes*.
    #[test]
    fn test_single_cluster_overlong_connection_glyph_is_rejected() {
        let nodes = format!("{},{}", node_json("a", "null"), node_json("b", "\"a\""));
        // One base character plus 4,000 combining acute accents:
        // 1 grapheme cluster, 8,001 bytes.
        let fat_cluster = format!("a{}", "\u{0301}".repeat(4_000));
        assert_eq!(
            crate::util::grapheme_chad::count_grapheme_clusters(&fat_cluster),
            1,
            "the fixture must pass the cluster ceiling, or it tests the wrong guard"
        );
        assert!(
            fat_cluster.len() > validate::MAX_CONNECTION_GLYPH_BYTES,
            "the fixture must exceed the byte ceiling"
        );

        let edge = edge_json_with(&format!(r#", "glyph_connection": {{"body": "{fat_cluster}"}}"#));
        let err = load_from_str(&map_json(&nodes, &edge))
            .expect_err("a single-cluster megabyte body must be rejected");
        assert!(err.contains("edge[0]"), "must name the edge: {err}");
        assert!(err.contains("body"), "must name the field: {err}");
        assert!(
            err.contains("bytes"),
            "must reject on the byte ceiling, not the cluster one: {err}"
        );
        assert!(
            err.contains("combining"),
            "must say why a cluster count did not catch this: {err}"
        );

        // The same shape on a cap glyph, so the loop covers every
        // field rather than only the one the exploit used.
        let cap = edge_json_with(&format!(
            r#", "glyph_connection": {{"cap_end": "{fat_cluster}"}}"#
        ));
        let err = load_from_str(&map_json(&nodes, &cap)).expect_err("cap glyphs carry the same ceiling");
        assert!(err.contains("cap_end"), "must name the field: {err}");

        // A single emoji ZWJ sequence is one cluster and many bytes,
        // and must still load — the ceiling is on absurdity, not on
        // legitimate multi-byte glyphs.
        let family = edge_json_with(r#", "glyph_connection": {"body": "👨‍👩‍👧‍👦"}"#);
        let map = load_from_str(&map_json(&nodes, &family)).expect("an emoji ZWJ motif is ordinary");
        assert_eq!(map.edges.len(), 1);
    }

    /// **The corners were bounded in no unit at all.** The two side
    /// patterns each got a ceiling on what they *emit*, but the four
    /// corner glyphs are copied verbatim out of the file into their
    /// run specs and handed to `glyph_ink_with` four times per node
    /// per frame, over a string the map chose. The shaping itself is
    /// memoized, so the recurring cost is the cache *key* — an owned
    /// copy of the glyph, built before the cache is read — which
    /// makes a multi-megabyte corner a multi-megabyte clone on every
    /// frame, and a permanent cache entry besides. Same "opening a
    /// map cannot kill the editor" property the rest of this module
    /// exists to hold.
    ///
    /// All eight authored glyph fields carry both ceilings, because
    /// the sides are an authored string too and the emitted-side cap
    /// bounds the repetition rather than its source.
    #[test]
    fn test_overlong_border_glyphs_are_rejected() {
        let many = "\u{25c8}".repeat(validate::MAX_BORDER_GLYPH_CLUSTERS + 1);
        // One cluster, past the byte ceiling: the case a cluster
        // count cannot see.
        let fat = format!("a{}", "\u{0301}".repeat(validate::MAX_BORDER_GLYPH_BYTES));

        for glyph_field in [
            "top",
            "bottom",
            "left",
            "right",
            "top_left",
            "top_right",
            "bottom_left",
            "bottom_right",
        ] {
            let node = node_json("0", "null").replace(
                r#""show_shadow":false"#,
                &format!(r#""show_shadow":false,"border":{{"glyphs":{{"{glyph_field}":"{many}"}}}}"#),
            );
            let err = load_from_str(&map_json_with_nodes(&node))
                .expect_err("an overlong border glyph must be rejected");
            assert!(err.contains(glyph_field), "must name the field: {err}");
            assert!(
                err.contains("grapheme clusters"),
                "must reject on the cluster ceiling: {err}"
            );

            let node = node_json("0", "null").replace(
                r#""show_shadow":false"#,
                &format!(r#""show_shadow":false,"border":{{"glyphs":{{"{glyph_field}":"{fat}"}}}}"#),
            );
            let err = load_from_str(&map_json_with_nodes(&node))
                .expect_err("a single-cluster overlong border glyph must be rejected");
            assert!(err.contains(glyph_field), "must name the field: {err}");
            assert!(
                err.contains("bytes"),
                "must reject on the byte ceiling, not the cluster one: {err}"
            );
        }

        // The longest glyph authored anywhere in this repository is
        // six clusters, so an ordinary decorative border is nowhere
        // near either ceiling and must still load.
        let ordinary = node_json("0", "null").replace(
            r#""show_shadow":false"#,
            r#""show_shadow":false,"border":{"glyphs":{"top":"◆·","top_left":"◆"}}"#,
        );
        load_from_str(&map_json_with_nodes(&ordinary)).expect("an ordinary authored border still loads");
    }

    /// **The byte ceiling stopped being a memory bound when unknown
    /// keys began to be kept, and this is what restores it.**
    ///
    /// Every captured key costs a heap-allocated route whose steps own
    /// their strings, and the file that buys one is about twelve bytes
    /// (`"k1":0,`). Measured on this build with `maptool grep` driving
    /// this door: 3 MiB / 200 000 keys reached 113 MiB peak RSS,
    /// 10 MiB / 800 000 keys reached 438 MiB — linear, at roughly 575
    /// bytes of peak per key. Extrapolated to a document at
    /// `MAX_MAP_BYTES`, that is on the order of 11 GB, from a file the
    /// size limit was written to permit. With the cap the same three
    /// fixtures peak at 29, 35 and 45 MiB: the cost follows the file
    /// again, not the key count.
    ///
    /// Both doors, because the tolerant path collects its own routes
    /// and would otherwise be the way around the ceiling.
    #[test]
    fn test_a_document_over_the_unknown_key_ceiling_is_refused_at_both_doors() {
        use crate::mindmap::unknown_keys::{with_key_ceiling, MAX_UNKNOWN_KEYS};
        const CAP: usize = 64;
        assert!(
            MAX_UNKNOWN_KEYS.is_none(),
            "native must not cap preserved keys — the cost of keeping them is linear in the \
             document that carries them, and a large map is a supported map"
        );

        // Comfortably past the ceiling rather than one over it. An
        // exactly-one-over fixture cannot tell a working cap from a
        // missing one: both leave `MAX_UNKNOWN_KEYS + 1` routes, so
        // the length assertion below passes either way. The boundary
        // itself is covered by the under-ceiling control at the end.
        let mut extra = String::new();
        for i in 0..CAP + 8 {
            extra.push_str(&format!(r#", "k{i}": 0"#));
        }
        let json = map_json_with_nodes(&node_json_with("0", "null", &extra));

        // **The allocation is what is bounded, so that is what is
        // asserted.** Refusing after the walk would refuse just as
        // loudly while the route vector had already grown to whatever
        // the document asked for — which is the cost, not the
        // symptom. `deserialize_capturing` stops pushing at the
        // ceiling and keeps exactly one route past it as the signal,
        // so the vector can never exceed `MAX_UNKNOWN_KEYS + 1` no
        // matter how many keys the file carries.
        let (_, routes): (MindMap, Vec<Vec<crate::mindmap::unknown_keys::Step>>) =
            with_key_ceiling(Some(CAP), || unknown_keys::deserialize_capturing(&json))
                .expect("the document parses");
        assert_eq!(
            routes.len(),
            CAP + 1,
            "the capture must stop one past the ceiling rather than collecting every key"
        );

        let err = with_key_ceiling(Some(CAP), || load_from_str(&json))
            .expect_err("over the ceiling must be refused");
        assert!(
            err.contains("unrecognized keys") && err.contains(&CAP.to_string()),
            "must name the ceiling it refused on: {err}"
        );
        let err = with_key_ceiling(Some(CAP), || parse_for_inspection(&json))
            .expect_err("the inspection door carries it too");
        assert!(
            err.contains("unrecognized keys"),
            "the inspection door must refuse on the same ceiling: {err}"
        );

        // The negative control, and the half that matters most: a
        // document *inside* the ceiling still loads and still keeps
        // every key. A cap that refused the ordinary forward-compat
        // case would break the promise the capture exists to keep.
        let mut few = String::new();
        for i in 0..8 {
            few.push_str(&format!(r#", "k{i}": {i}"#));
        }
        let map = load_from_str(&map_json_with_nodes(&node_json_with("0", "null", &few)))
            .expect("a handful of unknown keys must still load");
        assert_eq!(
            map.unknown_keys.len(),
            8,
            "every key inside the ceiling must still be captured"
        );
    }

    /// **The size ceiling, where one exists.**
    ///
    /// `MAX_MAP_BYTES` is `None` on native: Mandala opens maps as large
    /// as the machine can hold, and the previous 256 MiB ceiling —
    /// justified by "far past any authored map… the canonical fixture
    /// is 545 KB" — refused the repository's own stress generator. It
    /// is `Some` only on wasm32, where 32-bit `usize` and a 4 GiB
    /// address space make the limit physics rather than policy.
    ///
    /// Which leaves the refusal itself reachable on no target the test
    /// suite runs on. A test written against the constant would assert
    /// nothing and pass, so this drives `check_text_cap_against` with a
    /// ceiling of its own — the same code the browser reaches, with a
    /// number a test can afford.
    ///
    /// The `read_capped` stat-before-read door is exercised separately
    /// below; it is the one that keeps an oversized file from being
    /// read into memory at all.
    #[test]
    fn test_the_size_ceiling_refuses_where_a_platform_imposes_one() {
        // No policy ceiling on the target running this test.
        assert!(
            MAX_MAP_BYTES.is_none(),
            "native must not carry a byte ceiling — a large map is a supported map, and a \
             limit defended by the size of today's fixtures is a limit that expires"
        );

        // The wasm32 arm, driven with an affordable ceiling.
        let payload = " ".repeat(64);
        let err =
            check_text_cap_against(&payload, Some(32)).expect_err("text over the ceiling must be refused");
        assert!(err.contains("over the"), "must name the limit: {err}");
        assert!(
            !err.contains("expected value"),
            "must refuse before parsing, not report a parse error: {err}"
        );
        assert!(
            check_text_cap_against(&payload, Some(64)).is_ok(),
            "text exactly at the ceiling is inside it"
        );
        assert!(
            check_text_cap_against(&payload, None).is_ok(),
            "no ceiling means no refusal — the native posture"
        );
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
