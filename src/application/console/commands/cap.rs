//! `cap from=arrow to=none` — set the start/end cap glyph on the
//! selected edge. Edge-specific.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const KEYS: &[&str] = &["from", "to"];
pub const NAMES: &[&str] = &["arrow", "circle", "diamond", "none"];

pub const COMMAND: Command = Command {
    name: "cap",
    aliases: &[],
    summary: "Set the start/end cap glyph of the selected edge",
    usage: "cap from=<arrow|circle|diamond|none> to=<arrow|circle|diamond|none>",
    tags: &["edge", "cap", "arrow", "end", "start"],
    applicable: edge_selected,
    complete: complete_cap,
    execute: execute_cap,
};

fn complete_cap(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
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
            prefix_filter(NAMES, state.partial)
        }
        _ => Vec::new(),
    }
}

fn resolve_cap(endpoint_from: bool, name: &str) -> Option<Option<&'static str>> {
    match (endpoint_from, name) {
        (_, "none") => Some(None),
        (_, "circle") => Some(Some("\u{25CF}")),
        (_, "diamond") => Some(Some("\u{25C6}")),
        (true, "arrow") => Some(Some("\u{25C0}")),
        (false, "arrow") => Some(Some("\u{25B6}")),
        _ => None,
    }
}

fn execute_cap(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match eff.document.selection.selected_edge_or_portal_edge() {
        Some(e) => e,
        None => return ExecResult::err("no edge selected"),
    };
    let kvs: Vec<(String, String)> = args
        .kvs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if kvs.is_empty() {
        return ExecResult::err("usage: cap from=<name> to=<name>");
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
        let Some(glyph) = resolve_cap(is_from, &v) else {
            messages.push(format!("'{}': expected arrow|circle|diamond|none", v));
            continue;
        };
        let changed = if is_from {
            eff.document.set_edge_cap_start(&er, glyph)
        } else {
            eff.document.set_edge_cap_end(&er, glyph)
        };
        if changed {
            any_applied = true;
        } else {
            messages.push(format!("cap {} already {}", k, v));
        }
    }
    if !messages.is_empty() {
        if !any_applied {
            return ExecResult::err(messages.join("; "));
        }
        return ExecResult::Lines(messages);
    }
    ExecResult::ok_msg("cap applied")
}
