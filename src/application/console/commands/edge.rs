//! `edge` — one top-level verb for all edge lifecycle and style
//! operations. Handles:
//!
//! - **Type conversion:** `edge type=<cross_link|parent_child>` on
//!   the selected edge.
//! - **Display mode:** `edge display_mode=<line|portal>` swaps an
//!   edge between its line form (rendered path) and its portal form
//!   (two floating markers, no line). Portal-mode edges reuse
//!   `glyph_connection.body` as the marker glyph.
//! - **Path reset:** `edge reset=<straight|style>` clears control
//!   points (straight) or per-edge glyph overrides (style).
//!
//! Portal-mode edges are created by first building a line edge
//! (Connect mode / Ctrl+D) and then flipping with
//! `edge display_mode=portal` — the same two-step flow that covers
//! any display-mode change, so there's no dedicated creation verb
//! on this command.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::constants::{EDGE_TYPE_CROSS_LINK, EDGE_TYPE_PARENT_CHILD};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_or_portal_label_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const KEYS: &[&str] = &["type", "reset", "display_mode"];
pub const EDGE_TYPES: &[&str] = &[EDGE_TYPE_CROSS_LINK, EDGE_TYPE_PARENT_CHILD];
pub const RESETS: &[&str] = &["straight", "style"];
pub const DISPLAY_MODES: &[&str] = &[
    baumhard::mindmap::model::DISPLAY_MODE_LINE,
    baumhard::mindmap::model::DISPLAY_MODE_PORTAL,
];

pub const COMMAND: Command = Command {
    name: "edge",
    aliases: &[],
    summary: "Convert edge type, switch display mode, or reset path/style",
    usage: "edge type=<cross_link|parent_child>   |   edge display_mode=<line|portal>   |   edge reset=<straight|style>",
    tags: &[
        "edge",
        "type",
        "reset",
        "straight",
        "style",
        "cross_link",
        "parent_child",
        "display_mode",
        "line",
        "link",
    ],
    applicable: edge_or_portal_label_selected,
    complete: complete_edge,
    execute: execute_edge,
};

fn complete_edge(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { .. } => KEYS
            .iter()
            .filter(|k| k.starts_with(state.partial))
            .map(|k| Completion {
                text: format!("{}=", k),
                display: format!("{}=", k),
                hint: None,
            })
            .collect(),
        CompletionContext::KvValue { key } if key == "type" => {
            prefix_filter(EDGE_TYPES, state.partial)
        }
        CompletionContext::KvValue { key } if key == "reset" => {
            prefix_filter(RESETS, state.partial)
        }
        CompletionContext::KvValue { key } if key == "display_mode" => {
            prefix_filter(DISPLAY_MODES, state.partial)
        }
        _ => Vec::new(),
    }
}

fn execute_edge(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let kvs: Vec<(String, String)> = args
        .kvs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if kvs.is_empty() {
        return ExecResult::err(
            "usage: edge type=<...>   |   edge display_mode=<...>   |   edge reset=<straight|style>",
        );
    }

    // All kv operations target the currently-selected edge. A
    // portal-label selection resolves to its owning edge, so
    // `edge display_mode=line` works after clicking a portal
    // marker — without this branch, the user would lose the
    // ability to un-portal an edge they just put into portal
    // mode (the click-to-select path only yields `PortalLabel`
    // once an edge is in portal mode).
    let er = match eff.document.selection.selected_edge_or_portal_edge() {
        Some(e) => e,
        None => return ExecResult::err("no edge selected"),
    };

    let mut messages: Vec<String> = Vec::new();
    let mut any_applied = false;

    for (k, v) in kvs {
        match k.as_str() {
            "type" => {
                if !EDGE_TYPES.iter().any(|t| *t == v) {
                    messages.push(format!(
                        "type '{}' must be cross_link or parent_child",
                        v
                    ));
                    continue;
                }
                let changed = eff.document.set_edge_type(&er, &v);
                if changed {
                    any_applied = true;
                } else {
                    messages.push(format!("edge already of type {}", v));
                }
            }
            "display_mode" => {
                if !DISPLAY_MODES.iter().any(|m| *m == v) {
                    messages.push(format!(
                        "display_mode '{}' must be line or portal",
                        v
                    ));
                    continue;
                }
                let changed = eff.document.set_edge_display_mode(&er, &v);
                if changed {
                    any_applied = true;
                } else {
                    messages.push(format!("edge already rendering as {}", v));
                }
            }
            "reset" => match v.as_str() {
                "straight" => {
                    let changed = eff.document.reset_edge_to_straight(&er);
                    if changed {
                        any_applied = true;
                    } else {
                        messages.push("connection already straight".into());
                    }
                }
                "style" => {
                    let changed = eff.document.reset_edge_style_to_default(&er);
                    if changed {
                        any_applied = true;
                    } else {
                        messages.push("no style override to reset".into());
                    }
                }
                other => {
                    messages.push(format!(
                        "reset '{}' must be straight or style",
                        other
                    ));
                }
            },
            other => messages.push(format!("unknown key '{}'", other)),
        }
    }
    if !messages.is_empty() {
        if !any_applied {
            return ExecResult::err(messages.join("; "));
        }
        return ExecResult::Lines(messages);
    }
    ExecResult::ok_msg("edge applied")
}
