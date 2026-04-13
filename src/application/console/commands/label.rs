//! `label text="hi"` / `label clear` / `label position=middle` —
//! edge label operations.
//!
//! `text=` routes through the `HasLabel` trait (edge-only today).
//! `position=` is edge-specific and therefore handled outside the
//! trait layer — if the selection isn't an edge the pair reports
//! not-applicable. `clear` is the positional form of `text=<empty>`.
//! `edit` is a positional verb that hands off to the inline label
//! editor modal.

use super::color::finalize_report;
use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::traits::{apply_kvs, HasLabel};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const VERBS: &[&str] = &["edit", "clear"];
pub const KEYS: &[&str] = &["text", "position"];
pub const POSITIONS: &[&str] = &["start", "middle", "end"];

pub const COMMAND: Command = Command {
    name: "label",
    aliases: &[],
    summary: "Edit, clear, set, or reposition the selected edge's label",
    usage: "label text=\"<text>\" [position=<start|middle|end>]   |   label edit   |   label clear",
    tags: &["edge", "label", "text", "position", "clear", "edit"],
    applicable: edge_selected,
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
            let er = match &eff.document.selection {
                SelectionState::Edge(e) => e.clone(),
                _ => return ExecResult::err("no edge selected"),
            };
            eff.open_label_edit = Some(er);
            eff.close_console = true;
            return ExecResult::ok_empty();
        }
        Some("clear") => {
            let er = match &eff.document.selection {
                SelectionState::Edge(e) => e.clone(),
                _ => return ExecResult::err("no edge selected"),
            };
            let changed = eff.document.set_edge_label(&er, None);
            return if changed {
                ExecResult::ok_msg("label cleared")
            } else {
                ExecResult::ok_msg("label already empty")
            };
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

    // Position is edge-specific; we handle it before the trait
    // dispatch so the dispatcher doesn't need a dedicated trait for
    // a one-field concept.
    let position_kv = kvs.iter().find(|(k, _)| k == "position").cloned();
    let trait_kvs: Vec<(String, String)> =
        kvs.iter().filter(|(k, _)| k != "position").cloned().collect();

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

    if let Some((_, value)) = position_kv {
        match &eff.document.selection {
            SelectionState::Edge(er) => {
                let er = er.clone();
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
                let changed = eff.document.set_edge_label_position(&er, t);
                any_applied |= changed;
                if !changed {
                    messages.push(format!("position already {}", value));
                }
            }
            _ => messages.push("position: not applicable to selection".into()),
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

