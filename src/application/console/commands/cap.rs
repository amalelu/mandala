// SPDX-License-Identifier: MPL-2.0

//! `cap from=arrow to=none` — set the start/end cap glyph on the
//! selected edge. Edge-specific.

use super::Command;
use crate::application::console::helpers::{require_edge_or_portal, ApplyTally};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{bare_words, kvs, usage, Bare, Form, Grammar, Key, Vocabulary, Word};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

pub const NAMES: &[&str] = &["arrow", "circle", "diamond", "none"];
const CAP_WORDS: &[Word] = &bare_words::<4>(NAMES);

const KEYS: &[Key] = &[
    Key::new(
        "from",
        "cap glyph at the edge's start",
        Vocabulary::Words(CAP_WORDS),
    ),
    Key::new("to", "cap glyph at the edge's end", Vocabulary::Words(CAP_WORDS)),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "cap",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new("composed", &[Form::opt(&["from", "to"])])),
};

pub const COMMAND: Command = Command {
    name: "cap",
    aliases: &[],
    summary: "Set the start/end cap glyph of the selected edge",
    applicable: edge_selected,
    grammar: &GRAMMAR,
    synonyms: &["edge", "end", "start"],
    execute: execute_cap,
};

pub(crate) fn resolve_cap(endpoint_from: bool, name: &str) -> Option<Option<&'static str>> {
    match (endpoint_from, name) {
        (_, "none") => Some(None),
        (_, "circle") => Some(Some("\u{25CF}")),
        (_, "diamond") => Some(Some("\u{25C6}")),
        (true, "arrow") => Some(Some("\u{25C0}")),
        (false, "arrow") => Some(Some("\u{25B6}")),
        _ => None,
    }
}

/// Mutation core: apply cap-glyph changes to the currently-selected
/// edge. Both the `cap` console verb and the parametric
/// `Action::SetEdgeCap` route through this helper. Invalid preset
/// names silently no-op the corresponding slot — the verb path
/// surfaces typed errors via `ApplyTally`; the Action path warns
/// upstream and returns `Handled` (no scrollback surface).
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_cap_to_selection(
    doc: &mut MindMapDocument,
    from: Option<&str>,
    to: Option<&str>,
) -> bool {
    let mut changed = false;
    if let Some(v) = from {
        changed |= apply_cap_slot_to_selection(doc, true, v);
    }
    if let Some(v) = to {
        changed |= apply_cap_slot_to_selection(doc, false, v);
    }
    changed
}

/// Single-slot mutation core: write one cap glyph
/// (`is_from`-decided) to the currently-selected edge. `name` is
/// the user-facing preset (`arrow|circle|diamond|none`); invalid
/// names silently no-op (the verb pre-validates with typed errors;
/// the parametric Action arm warn-logs upstream).
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_cap_slot_to_selection(doc: &mut MindMapDocument, is_from: bool, name: &str) -> bool {
    let Some(er) = doc.selection.selected_edge_or_portal_edge() else {
        return false;
    };
    let Some(glyph) = resolve_cap(is_from, name) else {
        return false;
    };
    if is_from {
        doc.set_edge_cap_start(&er, glyph)
    } else {
        doc.set_edge_cap_end(&er, glyph)
    }
}

fn execute_cap(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
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
        // Pre-validate the preset so the verb surfaces a typed
        // error; only then route through the slot core.
        if resolve_cap(is_from, v).is_none() {
            tally.note_error(format!("'{}': expected arrow|circle|diamond|none", v));
            continue;
        }
        let changed = apply_cap_slot_to_selection(eff.document_mut(), is_from, v);
        tally.note(changed, || format!("cap {} already {}", k, v));
    }
    tally.finalize("cap")
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
    fn resolve_cap_picks_directional_arrow_glyph() {
        // `arrow` is direction-sensitive: from-side arrow is
        // ◀ (U+25C0), to-side arrow is ▶ (U+25B6).
        assert_eq!(resolve_cap(true, "arrow"), Some(Some("\u{25C0}")));
        assert_eq!(resolve_cap(false, "arrow"), Some(Some("\u{25B6}")));
        // `none` clears the cap (Some(None) — Some-wraps the
        // unset answer).
        assert_eq!(resolve_cap(true, "none"), Some(None));
        // Unknown name returns None (the outer Option = "no answer").
        assert_eq!(resolve_cap(true, "totally-invalid"), None);
    }

    #[test]
    fn apply_cap_to_selection_writes_both_ends() {
        let mut doc = doc_with_first_edge_selected();
        let _ = apply_cap_to_selection(&mut doc, Some("circle"), Some("diamond"));
        let cfg = doc.mindmap.edges[0]
            .glyph_connection
            .as_ref()
            .expect("body-glyph fork should leave a cfg");
        assert_eq!(cfg.cap_start.as_deref(), Some("\u{25CF}"));
        assert_eq!(cfg.cap_end.as_deref(), Some("\u{25C6}"));
    }

    #[test]
    fn apply_cap_to_selection_returns_false_with_no_selection() {
        let mut doc = load_test_doc();
        assert!(!apply_cap_to_selection(&mut doc, Some("arrow"), Some("none")));
    }

    #[test]
    fn apply_cap_to_selection_silently_skips_invalid_name() {
        let mut doc = doc_with_first_edge_selected();
        let original = doc.mindmap.edges[0].glyph_connection.clone();
        // Invalid `from` value — core silently no-ops the bad slot.
        assert!(!apply_cap_to_selection(&mut doc, Some("nonsense"), None));
        assert_eq!(doc.mindmap.edges[0].glyph_connection, original);
    }

    #[test]
    fn apply_cap_to_selection_returns_false_for_node_selection() {
        // L1 — cap is edge-only; a node selection no-ops.
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().unwrap().clone();
        doc.selection = SelectionState::Single(id);
        assert!(!apply_cap_to_selection(&mut doc, Some("arrow"), Some("none")));
    }
}
