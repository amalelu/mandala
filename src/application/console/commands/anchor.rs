//! `anchor from=top to=auto` — edge anchor side setter.
//!
//! Component-specific (edge only); the anchor concept doesn't
//! generalize to nodes or portals, so this bypasses the trait layer
//! and calls `set_edge_anchor` directly.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const SIDES: &[&str] = &["auto", "top", "right", "bottom", "left"];
pub const KEYS: &[&str] = &["from", "to"];

pub const COMMAND: Command = Command {
    name: "anchor",
    aliases: &[],
    summary: "Set the from/to anchor side of the selected edge",
    usage: "anchor from=<side> to=<side>   (side: auto|top|right|bottom|left)",
    tags: &["edge", "anchor", "side"],
    applicable: edge_selected,
    complete: complete_anchor,
    execute: execute_anchor,
};

fn complete_anchor(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
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
        CompletionContext::KvValue { key } if KEYS.iter().any(|k| k == key) => {
            prefix_filter(SIDES, state.partial)
        }
        _ => Vec::new(),
    }
}

fn side_value(name: &str) -> Option<&str> {
    match name {
        "auto" | "top" | "right" | "bottom" | "left" => Some(name),
        _ => None,
    }
}

fn execute_anchor(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match eff.document.selection.selected_edge_or_portal_edge() {
        Some(e) => e,
        None => return ExecResult::err("no edge selected"),
    };
    let kvs: Vec<(String, String)> = args
        .kvs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if kvs.is_empty() {
        return ExecResult::err("usage: anchor from=<side> to=<side>");
    }

    let mut messages: Vec<String> = Vec::new();
    let mut any_applied = false;
    for (k, v) in kvs {
        let is_from = match k.as_str() {
            "from" => true,
            "to" => false,
            other => {
                messages.push(format!("unknown key '{}'", other));
                continue;
            }
        };
        let Some(val) = side_value(&v) else {
            messages.push(format!("'{}': expected auto|top|right|bottom|left", v));
            continue;
        };
        let changed = eff.document.set_edge_anchor(&er, is_from, val);
        if changed {
            any_applied = true;
        } else {
            messages.push(format!("{} already {}", k, v));
        }
    }
    if !messages.is_empty() {
        if !any_applied {
            return ExecResult::err(messages.join("; "));
        }
        return ExecResult::Lines(messages);
    }
    ExecResult::ok_msg("anchor applied")
}
