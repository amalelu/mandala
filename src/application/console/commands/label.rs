// SPDX-License-Identifier: MPL-2.0

//! `label text="hi"` / `label clear` / `label position=middle` —
//! edge label operations.
//!
//! `text=` routes through the `HasLabel` trait (edge-only today).
//! `position=` is edge-specific and therefore handled outside the
//! trait layer — if the selection isn't an edge the pair reports
//! not-applicable. `clear` is the positional form of `text=<empty>`.
//! `edit` is a positional verb that hands off to the inline label
//! editor modal.

use baumhard::util::geometry::pretty_inequal;

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_or_portal_label_selected;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::{
    bare_words, free, kvs, usage, Bare, Form, Grammar, Key, Subverb, Vocabulary, Word,
};
use crate::application::console::traits::{apply_kvs, HasLabel};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{MindMapDocument, SelectionState};

pub const POSITIONS: &[&str] = &["start", "middle", "end"];
const POSITION_WORDS: &[Word] = &bare_words::<3>(POSITIONS);

const KEYS: &[Key] = &[
    Key::new("text", "label text (quote multi-word values)", free("text")),
    Key::new(
        "position",
        "anchor the label at the start, middle or end of the edge",
        Vocabulary::Words(POSITION_WORDS),
    ),
    Key::new(
        "position_t",
        "fractional position along the edge, 0.0 … 1.0",
        free("f32"),
    ),
    Key::new(
        "perpendicular",
        "offset away from the edge, in canvas units",
        free("f32"),
    ),
];

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("edit", "editor", "open the inline single-line editor"),
    Subverb::bare("clear", "reset", "drop the label text"),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "label",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS],
    bare: Some(Bare::new(
        "composed",
        &[Form::opt(&["text", "position", "position_t", "perpendicular"])],
    )),
};

pub const COMMAND: Command = Command {
    name: "label",
    aliases: &[],
    summary: "Edit, clear, reposition, or offset the selected edge's label",
    applicable: edge_or_portal_label_selected,
    grammar: &GRAMMAR,
    synonyms: &["edge", "offset", "drag"],
    execute: execute_label,
};

fn execute_label(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Positional subverbs: `edit`, `clear`. These sit *alongside*
    // the kv surface — `label edit` with no kvs hands off to the
    // modal; `label clear` empties the label. The descent matches
    // them case-insensitively, console-wide
    // (`commands/mod.rs` § Casing).
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    let name = match descent.stop {
        Stop::Matched(subverb) => Some(subverb.name),
        Stop::Bare => None,
        _ => {
            return ExecResult::err(usage::unknown_subverb_message(
                descent.level,
                descent.typed.unwrap_or_default(),
            ))
        }
    };
    match name {
        Some("edit") => {
            // `label edit` opens the inline editor. Dispatches
            // to the edge label editor for `Edge` selections and
            // to the portal-text editor for `PortalLabel`
            // selections — the console effect fields are
            // mutually exclusive (only one can be Some per
            // command execution).
            match &eff.document().selection {
                SelectionState::Edge(e) => {
                    eff.side_effect = Some(super::super::ConsoleSideEffect::OpenLabelEdit(e.clone()));
                    eff.close_console = true;
                    return ExecResult::ok_empty();
                }
                SelectionState::PortalLabel(s) => {
                    eff.side_effect = Some(super::super::ConsoleSideEffect::OpenPortalTextEdit(
                        s.edge_ref(),
                        s.endpoint_node_id.clone(),
                    ));
                    eff.close_console = true;
                    return ExecResult::ok_empty();
                }
                _ => return ExecResult::err("no edge selected"),
            }
        }
        Some("clear") => match eff.document().selection.clone() {
            SelectionState::Edge(e) => {
                let changed = eff.document_mut().set_edge_label(&e, None);
                return if changed {
                    ExecResult::ok_msg("label cleared")
                } else {
                    ExecResult::ok_msg("label already empty")
                };
            }
            SelectionState::PortalLabel(s) => {
                let er = s.edge_ref();
                let ep = s.endpoint_node_id.clone();
                let changed = eff.document_mut().set_portal_label_text(&er, &ep, None);
                return if changed {
                    ExecResult::ok_msg("portal label text cleared")
                } else {
                    ExecResult::ok_msg("portal label text already empty")
                };
            }
            _ => return ExecResult::err("no edge selected"),
        },
        _ => {}
    }

    let kvs: Vec<(String, String)> = args.kvs().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    if kvs.is_empty() {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    }

    // Position / position_t / perpendicular are edge-label-specific
    // (they address the `EdgeLabelConfig` geometry channels on a
    // line-mode edge) — handle them directly so the trait
    // dispatcher doesn't need a dedicated trait for each single-
    // field concept.
    let position_kv = kvs.iter().find(|(k, _)| k == "position").cloned();
    let position_t_kv = kvs.iter().find(|(k, _)| k == "position_t").cloned();
    let perpendicular_kv = kvs.iter().find(|(k, _)| k == "perpendicular").cloned();
    let trait_kvs: Vec<(String, String)> = kvs
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "position" | "position_t" | "perpendicular"))
        .cloned()
        .collect();

    let mut messages = Vec::new();
    let mut any_applied = false;

    if !trait_kvs.is_empty() {
        let report = apply_kvs(eff.document_mut(), &trait_kvs, |view, key, value| match key {
            "text" => Some(view.set_label(Some(value.to_string()))),
            _ => None,
        });
        any_applied |= report.any_applied;
        messages.extend(report.messages);
    }

    // Geometry kv routing splits on selection:
    //
    // - `Edge` / `EdgeLabel` → line-mode label fields on the edge's
    //   `label_config` (position_t in [0, 1], perpendicular in canvas
    //   units along the path normal).
    // - `PortalLabel` / `PortalText` → per-endpoint fields on the
    //   owning edge's `portal_from` / `portal_to` state:
    //   `position_t` → `border_t` in [0, 4), `perpendicular` →
    //   `perpendicular_offset` along the border's outward normal.
    //
    // The `position=<start|middle|end>` shortcut is a line-mode
    // concept only — it names anchor points on the connection path,
    // not on a node's border — so it reports "not applicable" for
    // portal selections.
    enum TargetKind {
        LineEdge(crate::application::document::EdgeRef),
        PortalEndpoint {
            edge_ref: crate::application::document::EdgeRef,
            endpoint_node_id: String,
        },
    }
    let target: Option<TargetKind> = match &eff.document().selection {
        SelectionState::Edge(er) => Some(TargetKind::LineEdge(er.clone())),
        SelectionState::EdgeLabel(s) => Some(TargetKind::LineEdge(s.edge_ref.clone())),
        SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => Some(TargetKind::PortalEndpoint {
            edge_ref: s.edge_ref(),
            endpoint_node_id: s.endpoint_node_id.clone(),
        }),
        _ => None,
    };

    if let Some((_, value)) = position_kv {
        match target.as_ref() {
            Some(TargetKind::LineEdge(_)) => {
                // Pre-validate so a bad anchor name surfaces as a
                // typed error (the parametric Action arm silently
                // no-ops on bad input via the same parse helper).
                if parse_label_position_anchor(&value).is_none() {
                    return ExecResult::err(format!("position '{}' must be start|middle|end", value));
                }
                // Route through the mutation core — same setter
                // path the parametric `Action::SetEdgeLabelPosition`
                // arm uses.
                let changed = apply_label_position_to_selection(eff.document_mut(), &value);
                any_applied |= changed;
                if !changed {
                    messages.push(format!("position already {}", value));
                }
            }
            Some(TargetKind::PortalEndpoint { .. }) => {
                messages.push(
                    "position: portal labels slide along the node border; use \
                     position_t=<f32 in [0,4)> instead"
                        .into(),
                );
            }
            None => messages.push("position: not applicable to selection".into()),
        }
    }

    if let Some((_, value)) = position_t_kv {
        match target.as_ref() {
            Some(TargetKind::LineEdge(er)) => match value.parse::<f32>() {
                Ok(t) if t.is_finite() => {
                    // `set_edge_label_position` clamps into [0, 1].
                    // Echo the clamped value when the user's input
                    // was out of range so they notice the
                    // normalisation — silent-clamp would look like
                    // "worked" even though the stored value
                    // differs from what they typed.
                    let clamped = t.clamp(0.0, 1.0);
                    if pretty_inequal(t, clamped) {
                        messages.push(format!("position_t {} clamped to {}", value, clamped));
                    }
                    let changed = eff.document_mut().set_edge_label_position(er, t);
                    any_applied |= changed;
                    if !changed {
                        messages.push(format!("position_t already ≈ {:.4}", clamped));
                    }
                }
                Ok(_) => return ExecResult::err(format!("position_t '{}' must be finite", value)),
                Err(_) => return ExecResult::err(format!("position_t '{}' is not a number", value)),
            },
            Some(TargetKind::PortalEndpoint {
                edge_ref,
                endpoint_node_id,
            }) => match value.parse::<f32>() {
                Ok(t) if t.is_finite() => {
                    // Portal `border_t` is wrapped into `[0, 4)` by
                    // the setter; echo that wrap when the user's
                    // value falls outside the canonical range so
                    // the stored value isn't silently shifted.
                    let wrapped = baumhard::mindmap::portal_geometry::wrap_border_t(t);
                    if pretty_inequal(t, wrapped) {
                        messages.push(format!("position_t {} wrapped to {:.4}", value, wrapped));
                    }
                    let changed = eff
                        .document
                        .set_portal_label_border_t(edge_ref, endpoint_node_id, Some(t));
                    any_applied |= changed;
                    if !changed {
                        messages.push(format!("position_t already ≈ {:.4}", wrapped));
                    }
                }
                Ok(_) => return ExecResult::err(format!("position_t '{}' must be finite", value)),
                Err(_) => return ExecResult::err(format!("position_t '{}' is not a number", value)),
            },
            None => messages.push("position_t: not applicable to selection".into()),
        }
    }

    if let Some((_, value)) = perpendicular_kv {
        // Empty string clears back to default. Any other value must
        // parse as a finite f32. Shared parse is cheap enough to
        // inline per branch for clarity.
        let offset: Option<f32> = if value.is_empty() {
            None
        } else {
            match value.parse::<f32>() {
                Ok(v) if v.is_finite() => Some(v),
                Ok(_) => return ExecResult::err(format!("perpendicular '{}' must be finite", value)),
                Err(_) => return ExecResult::err(format!("perpendicular '{}' is not a number", value)),
            }
        };
        match target.as_ref() {
            Some(TargetKind::LineEdge(er)) => {
                let changed = eff.document_mut().set_edge_label_perpendicular_offset(er, offset);
                any_applied |= changed;
                if !changed {
                    messages.push("perpendicular already applied".into());
                }
            }
            Some(TargetKind::PortalEndpoint {
                edge_ref,
                endpoint_node_id,
            }) => {
                let changed = eff.document_mut().set_portal_label_perpendicular_offset(
                    edge_ref,
                    endpoint_node_id,
                    offset,
                );
                any_applied |= changed;
                if !changed {
                    messages.push("perpendicular already applied".into());
                }
            }
            None => messages.push("perpendicular: not applicable to selection".into()),
        }
    }

    if !messages.is_empty() {
        if !any_applied {
            return ExecResult::err(messages.join("; "));
        }
        return ExecResult::lines(messages);
    }
    if any_applied {
        ExecResult::ok_msg("label applied")
    } else {
        ExecResult::ok_empty()
    }
}

/// Mutation core: write `text` to the current edge / portal label.
/// Returns `true` when the label changed. Empty `text` clears the
/// label (mirrors `label clear`).
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_label_text_to_selection(doc: &mut MindMapDocument, text: &str) -> bool {
    let payload = if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    };
    match doc.selection.clone() {
        SelectionState::Edge(er) => doc.set_edge_label(&er, payload),
        SelectionState::EdgeLabel(s) => doc.set_edge_label(&s.edge_ref, payload),
        SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
            let er = s.edge_ref();
            doc.set_portal_label_text(&er, &s.endpoint_node_id, payload)
        }
        _ => false,
    }
}

/// Resolve a named position anchor (`start|middle|end`) to its
/// `position_t` value in `[0.0, 1.0]`. Shared between the verb's
/// typed-error path (which surfaces `Err` on a bad name) and the
/// parametric Action arm's silent-no-op path.
pub(crate) fn parse_label_position_anchor(name: &str) -> Option<f32> {
    match name {
        "start" => Some(0.0),
        "middle" => Some(0.5),
        "end" => Some(1.0),
        _ => None,
    }
}

/// Mutation core: apply `position=<start|middle|end>` to the
/// currently-selected line-mode edge. Portal selections (which use
/// the `position_t=<f32 in [0,4)>` shape) are not applicable and
/// silently no-op. Returns `true` on a real change.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_label_position_to_selection(doc: &mut MindMapDocument, position: &str) -> bool {
    let Some(t) = parse_label_position_anchor(position) else {
        return false;
    };
    let er = match doc.selection.clone() {
        SelectionState::Edge(er) => er,
        SelectionState::EdgeLabel(s) => s.edge_ref.clone(),
        // Portal selections route through `position_t=` instead;
        // the named-anchor concept doesn't translate.
        _ => return false,
    };
    doc.set_edge_label_position(&er, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::{EdgeLabelSel, EdgeRef};

    fn doc_with_first_edge_selected() -> MindMapDocument {
        let mut doc = load_test_doc();
        let e = doc.mindmap.edges.first().expect("testament edges");
        let er = EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type);
        doc.selection = SelectionState::EdgeLabel(EdgeLabelSel::new(er));
        doc
    }

    #[test]
    fn apply_label_text_writes_label() {
        let mut doc = doc_with_first_edge_selected();
        let _ = apply_label_text_to_selection(&mut doc, "hello");
        // The label text lives on `edge.label` (Option<String>),
        // not in EdgeLabelConfig (that struct holds geometry only).
        let er = doc.selection.selected_edge_or_portal_edge().unwrap();
        let idx = doc.edge_index(&er).unwrap();
        assert_eq!(doc.mindmap.edges[idx].label.as_deref(), Some("hello"));
    }

    #[test]
    fn apply_label_text_with_empty_clears() {
        let mut doc = doc_with_first_edge_selected();
        let _ = apply_label_text_to_selection(&mut doc, "hello");
        let _ = apply_label_text_to_selection(&mut doc, "");
        let er = doc.selection.selected_edge_or_portal_edge().unwrap();
        let idx = doc.edge_index(&er).unwrap();
        // Empty text normalises to `None` per `set_edge_label` semantics.
        assert!(doc.mindmap.edges[idx].label.is_none());
    }

    #[test]
    fn apply_label_position_writes_t_for_named_anchor() {
        let mut doc = doc_with_first_edge_selected();
        // First place the label at "start", then move it to "end" —
        // at least one of the two must produce a real change.
        let a = apply_label_position_to_selection(&mut doc, "start");
        let b = apply_label_position_to_selection(&mut doc, "end");
        assert!(a || b);
        let er = doc.selection.selected_edge_or_portal_edge().unwrap();
        let idx = doc.edge_index(&er).unwrap();
        let t = doc.mindmap.edges[idx]
            .label_config
            .as_ref()
            .and_then(|c| c.position_t)
            .expect("position write should set position_t");
        assert!((t - 1.0).abs() < 1e-3);
    }

    #[test]
    fn apply_label_position_returns_false_for_unknown_anchor() {
        let mut doc = doc_with_first_edge_selected();
        assert!(!apply_label_position_to_selection(&mut doc, "totally-bogus"));
    }

    #[test]
    fn apply_label_position_returns_false_with_no_selection() {
        let mut doc = load_test_doc();
        assert!(!apply_label_position_to_selection(&mut doc, "middle"));
    }

    #[test]
    fn apply_label_position_returns_false_for_node_selection() {
        // L1 — label position-anchor is line-edge-only; node and
        // portal selections no-op (the named-anchor concept doesn't
        // translate to portal endpoints, which use position_t).
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().unwrap().clone();
        doc.selection = SelectionState::Single(id);
        assert!(!apply_label_position_to_selection(&mut doc, "middle"));
    }

    #[test]
    fn apply_label_text_returns_false_for_node_selection() {
        // L1 — label text is edge / portal-only; a node selection
        // no-ops (the core's match has no Single arm).
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().unwrap().clone();
        doc.selection = SelectionState::Single(id);
        assert!(!apply_label_text_to_selection(&mut doc, "hi"));
    }
}
