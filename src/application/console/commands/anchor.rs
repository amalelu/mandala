// SPDX-License-Identifier: MPL-2.0

//! `anchor from=top to=auto` — edge anchor side setter.
//!
//! Component-specific (edge only); the anchor concept doesn't
//! generalize to nodes or portals, so this bypasses the trait layer
//! and calls `set_edge_anchor` directly.

use super::Command;
use crate::application::console::helpers::{require_edge_or_portal, ApplyTally};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{bare_words, kvs, usage, Bare, Form, Grammar, Key, Vocabulary, Word};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

pub const SIDES: &[&str] = &["auto", "top", "right", "bottom", "left"];
const SIDE_WORDS: &[Word] = &bare_words::<5>(SIDES);

const KEYS: &[Key] = &[
    Key::new(
        "from",
        "which side of the source node the edge leaves",
        Vocabulary::Words(SIDE_WORDS),
    ),
    Key::new(
        "to",
        "which side of the target node the edge arrives at",
        Vocabulary::Words(SIDE_WORDS),
    ),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "anchor",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new("composed", &[Form::opt(&["from", "to"])])),
};

pub const COMMAND: Command = Command {
    name: "anchor",
    aliases: &[],
    summary: "Set the from/to anchor side of the selected edge",
    applicable: edge_selected,
    grammar: &GRAMMAR,
    synonyms: &["edge", "side"],
    execute: execute_anchor,
};

pub(crate) fn side_value(name: &str) -> Option<&str> {
    match name {
        "auto" | "top" | "right" | "bottom" | "left" => Some(name),
        _ => None,
    }
}

/// Mutation core: apply an anchor change to the currently-selected
/// edge (or portal-marker edge). Both the `anchor` console verb and
/// the parametric `Action::SetEdgeAnchor` route through this helper
/// — single source of truth for the resolve-edge + side-validation
/// + setter sequence.
///
/// Returns `true` when at least one slot actually changed; `false`
/// when nothing changed (no edge selected, invalid sides, or values
/// already at target). The Action arm uses the bool to decide
/// whether to trigger a scene rebuild; the verb uses it for tally /
/// scrollback messaging.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_anchor_to_selection(
    doc: &mut MindMapDocument,
    from: Option<&str>,
    to: Option<&str>,
) -> bool {
    let mut changed = false;
    if let Some(v) = from {
        if let Some(side) = side_value(v) {
            changed |= apply_anchor_slot_to_selection(doc, true, side);
        }
    }
    if let Some(v) = to {
        if let Some(side) = side_value(v) {
            changed |= apply_anchor_slot_to_selection(doc, false, side);
        }
    }
    changed
}

/// Single-slot mutation core: write one anchor side
/// (`is_from`-decided) to the currently-selected edge. Pre-validated
/// `side` is required (the verb path validates per-kv with typed
/// errors; the bundle path goes through
/// [`apply_anchor_to_selection`] which validates internally and
/// silently no-ops on bad input). Returns `true` on a real change.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_anchor_slot_to_selection(doc: &mut MindMapDocument, is_from: bool, side: &str) -> bool {
    let Some(er) = doc.selection.selected_edge_or_portal_edge() else {
        return false;
    };
    doc.set_edge_anchor(&er, is_from, side)
}

fn execute_anchor(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Pre-check the selection so a non-edge selection surfaces a
    // typed error instead of "no change". The mutation core
    // re-resolves the edge per call (cheap match on selection).
    if let Err(r) = require_edge_or_portal(eff) {
        return r;
    }
    let descent = descend(&GRAMMAR, args.tokens());
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    if pairs.is_empty() {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    }

    let mut tally = ApplyTally::new();
    for pair in &pairs {
        let (k, v) = (pair.key.name, pair.value);
        let is_from = k == "from";
        let Some(val) = side_value(v) else {
            tally.note_error(format!("'{}': expected auto|top|right|bottom|left", v));
            continue;
        };
        // Route through the mutation core — same setter path the
        // parametric `Action::SetEdgeAnchor` uses. The verb keeps
        // the per-kv tally so each side reports its own outcome.
        let changed = apply_anchor_slot_to_selection(eff.document_mut(), is_from, val);
        tally.note(changed, || format!("{} already {}", k, v));
    }
    tally.finalize("anchor")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::{EdgeLabelSel, EdgeRef, SelectionState};

    fn doc_with_first_edge_selected() -> MindMapDocument {
        let mut doc = load_test_doc();
        let e = doc.mindmap.edges.first().expect("testament edges");
        let er = EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type);
        doc.selection = SelectionState::EdgeLabel(EdgeLabelSel::new(er));
        doc
    }

    #[test]
    fn apply_anchor_to_selection_writes_both_sides() {
        let mut doc = doc_with_first_edge_selected();
        // Pick values different from whatever the testament edge
        // already has so the setters do real work.
        let changed = apply_anchor_to_selection(&mut doc, Some("top"), Some("bottom"));
        assert!(changed);
        let e = &doc.mindmap.edges[0];
        assert_eq!(e.anchor_from, "top");
        assert_eq!(e.anchor_to, "bottom");
    }

    #[test]
    fn apply_anchor_to_selection_returns_false_with_no_selection() {
        let mut doc = load_test_doc();
        // Default selection is None — no edge to target.
        assert!(!apply_anchor_to_selection(&mut doc, Some("top"), Some("auto")));
    }

    #[test]
    fn apply_anchor_to_selection_silently_skips_invalid_sides() {
        let mut doc = doc_with_first_edge_selected();
        let original_from = doc.mindmap.edges[0].anchor_from.clone();
        // Bogus side — core silently no-ops the bad slot. The
        // typed-Action arm has no scrollback, so a warn happens
        // upstream and the dispatch returns Handled.
        let changed = apply_anchor_to_selection(&mut doc, Some("diagonal"), None);
        assert!(!changed);
        assert_eq!(doc.mindmap.edges[0].anchor_from, original_from);
    }

    #[test]
    fn apply_anchor_to_selection_returns_false_for_node_selection() {
        // L1 — selection-mismatch coverage. Anchor is edge-only;
        // a node selection should silently no-op (the helper's
        // `selected_edge_or_portal_edge` returns None for nodes).
        let mut doc = load_test_doc();
        let id = doc
            .mindmap
            .nodes
            .keys()
            .next()
            .expect("testament has nodes")
            .clone();
        doc.selection = SelectionState::Single(id);
        assert!(!apply_anchor_to_selection(&mut doc, Some("top"), None));
    }
}
