//! `label text="hi"` / `label clear` / `label position=middle` —
//! edge label operations.
//!
//! `text=` routes through the `HasLabel` trait (edge-only today).
//! `position=` is edge-specific and therefore handled outside the
//! trait layer — if the selection isn't an edge the pair reports
//! not-applicable. `clear` is the positional form of `text=<empty>`.
//! `edit` is a positional verb that hands off to the inline label
//! editor modal.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_or_portal_label_selected;
use crate::application::console::traits::{apply_kvs, HasLabel};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const VERBS: &[&str] = &["edit", "clear"];
pub const KEYS: &[&str] = &["text", "position", "position_t", "perpendicular"];
pub const POSITIONS: &[&str] = &["start", "middle", "end"];

pub const COMMAND: Command = Command {
    name: "label",
    aliases: &[],
    summary: "Edit, clear, reposition, or offset the selected edge's label",
    usage: "label text=\"<text>\" [position=<start|middle|end>] [position_t=<f32>] [perpendicular=<f32>]   |   label edit   |   label clear",
    tags: &[
        "edge", "label", "text", "position", "position_t",
        "perpendicular", "offset", "drag", "clear", "edit",
    ],
    applicable: edge_or_portal_label_selected,
    complete: complete_label,
    execute: execute_label,
};

fn complete_label(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index: 0 } => {
            // Position 0: either a verb (`edit`, `clear`) or a kv key.
            let mut out = prefix_filter(VERBS, state.partial);
            for k in KEYS {
                if k.starts_with(state.partial) {
                    out.push(Completion {
                        text: format!("{}=", k),
                        display: format!("{}=", k),
                        hint: None,
                    });
                }
            }
            out
        }
        CompletionContext::Token { .. } => KEYS
            .iter()
            .filter(|k| k.starts_with(state.partial))
            .map(|k| Completion {
                text: format!("{}=", k),
                display: format!("{}=", k),
                hint: None,
            })
            .collect(),
        CompletionContext::KvValue { key } if key == "position" => {
            prefix_filter(POSITIONS, state.partial)
        }
        _ => Vec::new(),
    }
}

fn execute_label(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Positional verbs: `edit`, `clear`. These sit *alongside* the
    // kv surface — `label edit` with no kvs hands off to the modal;
    // `label clear` empties the label.
    match args.positional(0) {
        Some("edit") => {
            // `label edit` opens the inline editor. Dispatches
            // to the edge label editor for `Edge` selections and
            // to the portal-text editor for `PortalLabel`
            // selections — the console effect fields are
            // mutually exclusive (only one can be Some per
            // command execution).
            match &eff.document.selection {
                SelectionState::Edge(e) => {
                    eff.open_label_edit = Some(e.clone());
                    eff.close_console = true;
                    return ExecResult::ok_empty();
                }
                SelectionState::PortalLabel(s) => {
                    eff.open_portal_text_edit =
                        Some((s.edge_ref(), s.endpoint_node_id.clone()));
                    eff.close_console = true;
                    return ExecResult::ok_empty();
                }
                _ => return ExecResult::err("no edge selected"),
            }
        }
        Some("clear") => {
            match &eff.document.selection {
                SelectionState::Edge(e) => {
                    let changed = eff.document.set_edge_label(&e.clone(), None);
                    return if changed {
                        ExecResult::ok_msg("label cleared")
                    } else {
                        ExecResult::ok_msg("label already empty")
                    };
                }
                SelectionState::PortalLabel(s) => {
                    let er = s.edge_ref();
                    let ep = s.endpoint_node_id.clone();
                    let changed = eff.document.set_portal_label_text(&er, &ep, None);
                    return if changed {
                        ExecResult::ok_msg("portal label text cleared")
                    } else {
                        ExecResult::ok_msg("portal label text already empty")
                    };
                }
                _ => return ExecResult::err("no edge selected"),
            }
        }
        Some(other) => {
            return ExecResult::err(format!(
                "unknown label verb '{}'; use kv form (text=... position=...) or 'edit' / 'clear'",
                other
            ))
        }
        None => {}
    }

    let kvs: Vec<(String, String)> = args
        .kvs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if kvs.is_empty() {
        return ExecResult::err("usage: label text=\"<text>\" [position=<start|middle|end>]");
    }

    // Position / position_t / perpendicular are edge-label-specific
    // (they address the `EdgeLabelConfig` geometry channels on a
    // line-mode edge) — handle them directly so the trait
    // dispatcher doesn't need a dedicated trait for each single-
    // field concept.
    let position_kv = kvs.iter().find(|(k, _)| k == "position").cloned();
    let position_t_kv = kvs.iter().find(|(k, _)| k == "position_t").cloned();
    let perpendicular_kv = kvs
        .iter()
        .find(|(k, _)| k == "perpendicular")
        .cloned();
    let trait_kvs: Vec<(String, String)> = kvs
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "position" | "position_t" | "perpendicular"))
        .cloned()
        .collect();

    let mut messages = Vec::new();
    let mut any_applied = false;

    if !trait_kvs.is_empty() {
        let report = apply_kvs(eff.document, &trait_kvs, |view, key, value| match key {
            "text" => Some(view.set_label(Some(value.to_string()))),
            _ => None,
        });
        any_applied |= report.any_applied;
        messages.extend(report.messages);
    }

    // Resolve the target edge ref once for every geometry kv —
    // all three (position, position_t, perpendicular) address the
    // same `label_config` on the owning edge. EdgeLabel / Edge /
    // PortalLabel / PortalText all collapse to the owning edge
    // via `selected_edge_or_portal_edge`; applying geometry to
    // anything else is a user error reported as a per-key
    // message.
    let target_edge: Option<crate::application::document::EdgeRef> =
        match &eff.document.selection {
            SelectionState::Edge(er) => Some(er.clone()),
            SelectionState::EdgeLabel(s) => Some(s.edge_ref.clone()),
            _ => None,
        };

    if let Some((_, value)) = position_kv {
        match target_edge.as_ref() {
            Some(er) => {
                let t = match value.as_str() {
                    "start" => 0.0,
                    "middle" => 0.5,
                    "end" => 1.0,
                    other => {
                        return ExecResult::err(format!(
                            "position '{}' must be start|middle|end",
                            other
                        ))
                    }
                };
                let changed = eff.document.set_edge_label_position(er, t);
                any_applied |= changed;
                if !changed {
                    messages.push(format!("position already {}", value));
                }
            }
            None => messages.push("position: not applicable to selection".into()),
        }
    }

    if let Some((_, value)) = position_t_kv {
        match target_edge.as_ref() {
            Some(er) => match value.parse::<f32>() {
                Ok(t) if t.is_finite() => {
                    // `set_edge_label_position` clamps into [0, 1]
                    // silently — the user-supplied value is
                    // normalised before landing on the model.
                    let changed = eff.document.set_edge_label_position(er, t);
                    any_applied |= changed;
                    if !changed {
                        messages
                            .push(format!("position_t already ≈ {:.4}", t.clamp(0.0, 1.0)));
                    }
                }
                Ok(_) => {
                    return ExecResult::err(format!(
                        "position_t '{}' must be finite",
                        value
                    ))
                }
                Err(_) => {
                    return ExecResult::err(format!(
                        "position_t '{}' is not a number",
                        value
                    ))
                }
            },
            None => messages.push("position_t: not applicable to selection".into()),
        }
    }

    if let Some((_, value)) = perpendicular_kv {
        match target_edge.as_ref() {
            Some(er) => {
                // Empty string clears back to on-path. Any other
                // value must parse as a finite f32.
                let offset: Option<f32> = if value.is_empty() {
                    None
                } else {
                    match value.parse::<f32>() {
                        Ok(v) if v.is_finite() => Some(v),
                        Ok(_) => {
                            return ExecResult::err(format!(
                                "perpendicular '{}' must be finite",
                                value
                            ))
                        }
                        Err(_) => {
                            return ExecResult::err(format!(
                                "perpendicular '{}' is not a number",
                                value
                            ))
                        }
                    }
                };
                let changed = eff
                    .document
                    .set_edge_label_perpendicular_offset(er, offset);
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
        return ExecResult::Lines(messages);
    }
    if any_applied {
        ExecResult::ok_msg("label applied")
    } else {
        ExecResult::ok_empty()
    }
}

