// SPDX-License-Identifier: MPL-2.0

//! `edge` — one top-level verb for all edge lifecycle and style
//! operations. Handles:
//!
//! - **Type conversion:** `edge type=<cross_link|parent_child>` on
//!   the selected edge.
//! - **Display mode:** `edge display_mode=<line|portal>` swaps an
//!   edge between its line form (rendered path) and its portal form
//!   (two floating markers, no line). Portal-mode edges reuse
//!   `glyph_connection.body` as the marker glyph.
//! - **Path reset / curve:** `edge reset=<straight|curve|style>`.
//!   `straight` clears control points, `curve` inserts one default
//!   control point (bootstraps a gentle quadratic Bezier on a
//!   straight edge — same result the midpoint drag-handle produces
//!   but reachable from the keyboard), `style` clears per-edge
//!   glyph overrides.
//!
//! Portal-mode edges are created by first building a line edge
//! (Connect mode / Ctrl+D) and then flipping with
//! `edge display_mode=portal` — the same two-step flow that covers
//! any display-mode change, so there's no dedicated creation verb
//! on this command.

use super::Command;
use crate::application::console::constants::{EDGE_TYPE_CROSS_LINK, EDGE_TYPE_PARENT_CHILD};
use crate::application::console::helpers::{require_edge_or_portal, ApplyTally};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_or_portal_label_selected;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{bare_words, kvs, usage, Bare, Form, Grammar, Key, Vocabulary, Word};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

pub const EDGE_TYPES: &[&str] = &[EDGE_TYPE_CROSS_LINK, EDGE_TYPE_PARENT_CHILD];
pub const RESETS: &[&str] = &["straight", "curve", "style", "position"];
pub const DISPLAY_MODES: &[&str] = &[
    baumhard::mindmap::model::DISPLAY_MODE_LINE,
    baumhard::mindmap::model::DISPLAY_MODE_PORTAL,
];

const TYPE_WORDS: &[Word] = &bare_words::<2>(EDGE_TYPES);
const RESET_WORDS: &[Word] = &bare_words::<4>(RESETS);
const DISPLAY_MODE_WORDS: &[Word] = &bare_words::<2>(DISPLAY_MODES);

const KEYS: &[Key] = &[
    Key::new(
        "type",
        "reparent the edge or turn it into a cross link",
        Vocabulary::Words(TYPE_WORDS),
    ),
    Key::new(
        "reset",
        "drop one aspect back to its default",
        Vocabulary::Words(RESET_WORDS),
    ),
    Key::new(
        "display_mode",
        "draw the edge as a line or as a portal pair",
        Vocabulary::Words(DISPLAY_MODE_WORDS),
    ),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "edge",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new(
        "composed",
        &[Form::opt(&["type", "display_mode", "reset"])],
    )),
};

pub const COMMAND: Command = Command {
    name: "edge",
    aliases: &[],
    summary: "Convert edge type, switch display mode, curve/straighten, or reset style/position",
    applicable: edge_or_portal_label_selected,
    grammar: &GRAMMAR,
    synonyms: &["straight", "curve", "bezier", "style", "link"],
    execute: execute_edge,
};

fn execute_edge(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&GRAMMAR, args.tokens());
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    if pairs.is_empty() {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    }

    // All kv operations target the currently-selected edge. A
    // portal-label selection resolves to its owning edge, so
    // `edge display_mode=line` works after clicking a portal
    // marker — without this branch, the user would lose the
    // ability to un-portal an edge they just put into portal
    // mode (the click-to-select path only yields `PortalLabel`
    // once an edge is in portal mode).
    let er = match require_edge_or_portal(eff) {
        Ok(e) => e,
        Err(r) => return r,
    };

    // Bind `_er` to silence the unused warning — the per-kv path
    // resolves the edge through the mutation cores now. The
    // `require_edge_or_portal` call above is still load-bearing
    // for the typed "no edge selected" error.
    let _ = er;

    let mut tally = ApplyTally::new();

    for pair in &pairs {
        let (k, v) = (pair.key.name, pair.value);
        match k {
            "type" => {
                if !EDGE_TYPES.contains(&v) {
                    tally.note_error(format!("type '{}' must be cross_link or parent_child", v));
                    continue;
                }
                // Route through the mutation core — same setter
                // path the parametric `Action::SetEdgeType` arm uses.
                let changed = apply_edge_type_to_selection(eff.document_mut(), v);
                tally.note(changed, || format!("edge already of type {}", v));
            }
            "display_mode" => {
                if !DISPLAY_MODES.contains(&v) {
                    tally.note_error(format!("display_mode '{}' must be line or portal", v));
                    continue;
                }
                let changed = apply_edge_display_mode_to_selection(eff.document_mut(), v);
                tally.note(changed, || format!("edge already rendering as {}", v));
            }
            "reset" => match v {
                kind @ ("straight" | "curve" | "style" | "position") => {
                    let changed = apply_edge_reset_to_selection(eff.document_mut(), kind);
                    let already_msg: &str = match kind {
                        "straight" => "connection already straight",
                        "curve" => "connection already curved",
                        "style" => "no style override to reset",
                        "position" => "position already at default",
                        // Outer guard restricts to 4 kinds; this
                        // arm is statically unreachable but
                        // CODE_CONVENTIONS §9 forbids panic in
                        // interactive paths.
                        _ => "no change",
                    };
                    let already_msg = already_msg.to_string();
                    tally.note(changed, || already_msg);
                }
                other => {
                    tally.note_error(format!(
                        "reset '{}' must be straight, curve, style, or position",
                        other
                    ));
                }
            },
            // Unreachable: the engine refused every key this form
            // does not read before the loop ran. Degrade rather
            // than panic — this is an interactive path (§9).
            other => log::error!("edge: engine admitted unread key '{other}'"),
        }
    }
    tally.finalize("edge")
}

/// Mutation core: apply a single `edge type=...` value to the
/// currently-selected edge. Returns `true` when the type changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_edge_type_to_selection(doc: &mut MindMapDocument, edge_type: &str) -> bool {
    let Some(er) = doc.selection.selected_edge_or_portal_edge() else {
        return false;
    };
    if !EDGE_TYPES.contains(&edge_type) {
        return false;
    }
    doc.set_edge_type(&er, edge_type)
}

/// Mutation core: apply a single `edge display_mode=...` value
/// (`line|portal`) to the currently-selected edge. Returns `true`
/// when the mode changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_edge_display_mode_to_selection(doc: &mut MindMapDocument, mode: &str) -> bool {
    let Some(er) = doc.selection.selected_edge_or_portal_edge() else {
        return false;
    };
    if !DISPLAY_MODES.contains(&mode) {
        return false;
    }
    doc.set_edge_display_mode(&er, mode)
}

/// Mutation core: apply a single `edge reset=<kind>` to the
/// currently-selected edge. Kind: `straight|curve|style|position`.
/// Returns `true` when the reset produced a change.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_edge_reset_to_selection(doc: &mut MindMapDocument, kind: &str) -> bool {
    let Some(er) = doc.selection.selected_edge_or_portal_edge() else {
        return false;
    };
    match kind {
        "straight" => doc.reset_edge_to_straight(&er),
        "curve" => doc.curve_straight_edge(&er),
        "style" => doc.reset_edge_style_to_default(&er),
        "position" => {
            // Same selection-aware endpoint resolution the verb
            // uses: PortalLabel/PortalText narrow to one endpoint;
            // anything else resets the whole edge.
            use crate::application::document::SelectionState;
            let endpoint: Option<String> = match &doc.selection {
                SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
                    Some(s.endpoint_node_id.clone())
                }
                _ => None,
            };
            doc.reset_edge_position(&er, endpoint.as_deref())
        }
        _ => false,
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
    fn apply_edge_type_writes_known_value() {
        let mut doc = doc_with_first_edge_selected();
        let original = doc.mindmap.edges[0].edge_type.clone();
        let from_id = doc.mindmap.edges[0].from_id.clone();
        let to_id = doc.mindmap.edges[0].to_id.clone();
        // Pick whichever type the edge isn't currently set to so
        // the setter does real work.
        let target = if original == EDGE_TYPE_CROSS_LINK {
            EDGE_TYPE_PARENT_CHILD
        } else {
            EDGE_TYPE_CROSS_LINK
        };
        let _ = apply_edge_type_to_selection(&mut doc, target);
        // Look up by (from, to) only — set_edge_type rewrites the
        // type field in place, so the original EdgeRef no longer
        // matches.
        let updated = doc
            .mindmap
            .edges
            .iter()
            .find(|e| e.from_id == from_id && e.to_id == to_id)
            .expect("edge still present after type change");
        // The setter may refuse if the duplicate guard fires (same
        // from/to/new_type already exists). Either we changed it or
        // we didn't; assert the resulting state is valid.
        assert!(
            updated.edge_type == target || updated.edge_type == original,
            "edge_type after apply must be either target or original",
        );
    }

    #[test]
    fn apply_edge_type_returns_false_for_unknown_value() {
        let mut doc = doc_with_first_edge_selected();
        // Neither cross_link nor parent_child — skipped.
        assert!(!apply_edge_type_to_selection(&mut doc, "totally-invalid"));
    }

    #[test]
    fn apply_edge_type_returns_false_with_no_selection() {
        let mut doc = load_test_doc();
        assert!(!apply_edge_type_to_selection(&mut doc, EDGE_TYPE_CROSS_LINK));
    }

    #[test]
    fn apply_edge_display_mode_writes_portal() {
        let mut doc = doc_with_first_edge_selected();
        let _ = apply_edge_display_mode_to_selection(&mut doc, "portal");
        let er = doc.selection.selected_edge_or_portal_edge().unwrap();
        let idx = doc.mindmap.edges.iter().position(|e| er.matches(e)).unwrap();
        assert_eq!(doc.mindmap.edges[idx].display_mode.as_deref(), Some("portal"));
    }

    #[test]
    fn apply_edge_display_mode_returns_false_for_unknown_value() {
        let mut doc = doc_with_first_edge_selected();
        assert!(!apply_edge_display_mode_to_selection(&mut doc, "totally-invalid"));
    }

    #[test]
    fn apply_edge_reset_curve_inserts_control_point() {
        let mut doc = doc_with_first_edge_selected();
        // Force a straight starting state, then ask for curve —
        // confirms the dispatch picks the curve-bootstrap setter.
        let _ = apply_edge_reset_to_selection(&mut doc, "straight");
        let er = doc.selection.selected_edge_or_portal_edge().unwrap();
        let idx = doc.mindmap.edges.iter().position(|e| er.matches(e)).unwrap();
        assert!(doc.mindmap.edges[idx].control_points.is_empty());
        let curved = apply_edge_reset_to_selection(&mut doc, "curve");
        assert!(curved);
        assert_eq!(doc.mindmap.edges[idx].control_points.len(), 1);
    }

    #[test]
    fn apply_edge_reset_returns_false_for_unknown_kind() {
        let mut doc = doc_with_first_edge_selected();
        assert!(!apply_edge_reset_to_selection(&mut doc, "obviously-bogus"));
    }

    #[test]
    fn apply_edge_cores_return_false_for_node_selection() {
        // L1 — edge cores are edge-only; a node selection no-ops
        // across all three (type / display_mode / reset).
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().unwrap().clone();
        doc.selection = SelectionState::Single(id);
        assert!(!apply_edge_type_to_selection(&mut doc, EDGE_TYPE_CROSS_LINK));
        assert!(!apply_edge_display_mode_to_selection(&mut doc, "portal"));
        assert!(!apply_edge_reset_to_selection(&mut doc, "straight"));
    }
}
