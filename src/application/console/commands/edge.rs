//! `edge type=cross_link` / `edge reset=straight` / `edge
//! reset=style` — edge-type and path/style resets on the selected
//! edge. Absorbs the former `connection` command so the user has one
//! top-level edge verb instead of two.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::constants::{EDGE_TYPE_CROSS_LINK, EDGE_TYPE_PARENT_CHILD};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const KEYS: &[&str] = &["type", "reset"];
pub const EDGE_TYPES: &[&str] = &[EDGE_TYPE_CROSS_LINK, EDGE_TYPE_PARENT_CHILD];
pub const RESETS: &[&str] = &["straight", "style"];

pub const COMMAND: Command = Command {
    name: "edge",
    aliases: &[],
    summary: "Convert edge type or reset path/style on the selected edge",
    usage: "edge type=<cross_link|parent_child>   |   edge reset=<straight|style>",
    tags: &["edge", "type", "reset", "straight", "style", "cross_link", "parent_child"],
    applicable: edge_selected,
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
        _ => Vec::new(),
    }
}

fn execute_edge(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match &eff.document.selection {
        SelectionState::Edge(e) => e.clone(),
        _ => return ExecResult::err("no edge selected"),
    };
    let kvs: Vec<(String, String)> = args
        .kvs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if kvs.is_empty() {
        return ExecResult::err("usage: edge type=<...>   |   edge reset=<straight|style>");
    }

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
