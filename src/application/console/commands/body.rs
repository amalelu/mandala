// SPDX-License-Identifier: MPL-2.0

//! `body glyph=dash` — set the body glyph of the selected edge.
//! Edge-specific; the concept doesn't generalize beyond edges.

use super::Command;
use crate::application::console::helpers::require_edge_or_portal;
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{kvs, usage, Bare, Form, Grammar, Key, Vocabulary, Word};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

/// Body-glyph presets. Kept as `(name, glyph)` pairs so the command
/// table stays one source of truth for both completion and exec.
pub const PRESETS: &[(&str, &str)] = &[
    ("dot", "\u{00B7}"),    // ·
    ("dash", "\u{2500}"),   // ─
    ("double", "\u{2550}"), // ═
    ("wave", "\u{223C}"),   // ∼
    ("chain", "\u{22EF}"),  // ⋯
];

/// The preset names, lifted out of [`PRESETS`] at const-fn time so
/// the popup's vocabulary and the glyph table cannot come apart.
const GLYPH_WORDS: &[Word] = &{
    const N: usize = PRESETS.len();
    let mut out = [Word::bare(""); N];
    let mut i = 0;
    while i < N {
        out[i] = Word::bare(PRESETS[i].0);
        i += 1;
    }
    out
};

const KEYS: &[Key] = &[Key::new(
    "glyph",
    "the repeated glyph the edge body is drawn from",
    Vocabulary::Words(GLYPH_WORDS),
)];

pub static GRAMMAR: Grammar = Grammar {
    label: "body",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new("composed", &[Form::keys(&["glyph"], &[])])),
};

pub const COMMAND: Command = Command {
    name: "body",
    aliases: &[],
    summary: "Set the body glyph of the selected edge",
    applicable: edge_selected,
    grammar: &GRAMMAR,
    synonyms: &["edge", "style"],
    execute: execute_body,
};

/// Resolve a preset glyph name (`dot|dash|double|wave|chain`) to
/// its Unicode codepoint. Shared between completion, the verb
/// body, and the parametric Action arm.
pub(crate) fn glyph_for_preset(name: &str) -> Option<&'static str> {
    PRESETS.iter().find(|(n, _)| *n == name).map(|(_, g)| *g)
}

/// Mutation core: apply a body-glyph change to the currently-
/// selected edge. Both the `body` console verb and the parametric
/// `Action::SetEdgeBodyGlyph` route through this helper. Returns
/// `true` when the glyph actually changed; `false` when no edge is
/// selected, the preset name is invalid, or the glyph already
/// matches.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_body_glyph_to_selection(doc: &mut MindMapDocument, preset: &str) -> bool {
    let Some(er) = doc.selection.selected_edge_or_portal_edge() else {
        return false;
    };
    let Some(glyph) = glyph_for_preset(&preset.to_ascii_lowercase()) else {
        return false;
    };
    doc.set_edge_body_glyph(&er, glyph)
}

fn execute_body(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Accept a portal-label selection too: the same `body` glyph
    // drives the portal marker symbol, so `body glyph=…` on a
    // portal label retargets the owning edge.
    if let Err(r) = require_edge_or_portal(eff) {
        return r;
    }
    let descent = descend(&GRAMMAR, args.tokens());
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    let name = match kvs::value(&pairs, "glyph") {
        Some(n) => n.to_ascii_lowercase(),
        None => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
    };
    if glyph_for_preset(&name).is_none() {
        return ExecResult::err(format!(
            "glyph '{}' must be one of dot|dash|double|wave|chain",
            name
        ));
    }
    // Route through the mutation core — same setter, single
    // source of truth with the parametric `Action::SetEdgeBodyGlyph`
    // arm. The verb keeps the typed pre-validation (so a bad
    // preset surfaces as `Err`, not a silent no-op the way the
    // Action arm intentionally does).
    if apply_body_glyph_to_selection(eff.document_mut(), &name) {
        ExecResult::ok_msg(format!("body glyph set to {}", name))
    } else {
        ExecResult::ok_msg(format!("body glyph already {}", name))
    }
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
    fn glyph_for_preset_resolves_known_names() {
        assert_eq!(glyph_for_preset("dot"), Some("\u{00B7}"));
        assert_eq!(glyph_for_preset("dash"), Some("\u{2500}"));
        assert_eq!(glyph_for_preset("chain"), Some("\u{22EF}"));
        assert_eq!(glyph_for_preset("bogus"), None);
    }

    fn body_of_first_edge(doc: &MindMapDocument) -> Option<String> {
        doc.mindmap.edges[0]
            .glyph_connection
            .as_ref()
            .map(|c| c.body.clone())
    }

    #[test]
    fn apply_body_glyph_to_selection_writes_glyph() {
        let mut doc = doc_with_first_edge_selected();
        // Two calls with different presets — at least one must
        // produce a real change regardless of the testament edge's
        // starting state.
        let changed_first = apply_body_glyph_to_selection(&mut doc, "dash");
        let changed_second = apply_body_glyph_to_selection(&mut doc, "double");
        assert!(changed_first || changed_second);
        assert_eq!(body_of_first_edge(&doc), Some("\u{2550}".to_string()));
    }

    #[test]
    fn apply_body_glyph_to_selection_returns_false_with_no_selection() {
        let mut doc = load_test_doc();
        assert!(!apply_body_glyph_to_selection(&mut doc, "dash"));
    }

    #[test]
    fn apply_body_glyph_to_selection_silently_skips_invalid_preset() {
        let mut doc = doc_with_first_edge_selected();
        let original = body_of_first_edge(&doc);
        assert!(!apply_body_glyph_to_selection(&mut doc, "totally-invalid"));
        assert_eq!(body_of_first_edge(&doc), original);
    }

    #[test]
    fn apply_body_glyph_to_selection_is_case_insensitive() {
        let mut doc = doc_with_first_edge_selected();
        let _ = apply_body_glyph_to_selection(&mut doc, "DASH");
        assert_eq!(body_of_first_edge(&doc), Some("\u{2500}".to_string()));
    }

    #[test]
    fn apply_body_glyph_to_selection_returns_false_for_node_selection() {
        // L1 — body glyph is edge-only; a node selection no-ops.
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().unwrap().clone();
        doc.selection = SelectionState::Single(id);
        assert!(!apply_body_glyph_to_selection(&mut doc, "dash"));
    }
}
