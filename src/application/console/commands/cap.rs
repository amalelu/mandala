//! `cap <from|to> <arrow|circle|diamond|none>` — set the cap glyph
//! on the source or target end of the selected edge.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

/// Cap presets. `None` is a sentinel meaning "clear the cap".
pub const PRESETS: &[(&str, Option<&str>)] = &[
    ("arrow_start", Some("\u{25C0}")), // ◀
    ("arrow_end", Some("\u{25B6}")),   // ▶
    ("circle", Some("\u{25CF}")),       // ●
    ("diamond", Some("\u{25C6}")),      // ◆
    ("none", None),
];

pub const NAMES: &[&str] = &["arrow", "circle", "diamond", "none"];
pub const ENDPOINTS: &[&str] = &["from", "to"];

pub const COMMAND: Command = Command {
    name: "cap",
    aliases: &[],
    summary: "Set the start/end cap glyph of the selected edge",
    usage: "cap <from|to> <arrow|circle|diamond|none>",
    tags: &["edge", "cap", "arrow", "end", "start", "connection"],
    applicable: edge_selected,
    complete: complete_cap,
    execute: execute_cap,
};

fn complete_cap(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match state.cursor_token {
        1 => enum_completion(ENDPOINTS, state.partial),
        2 => enum_completion(NAMES, state.partial),
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
    let er = match &eff.document.selection {
        SelectionState::Edge(e) => e.clone(),
        _ => return ExecResult::err("no edge selected"),
    };
    let endpoint = match args.positional(0) {
        Some(e) => e.to_ascii_lowercase(),
        None => return ExecResult::err("usage: cap <from|to> <name>"),
    };
    let is_from = match endpoint.as_str() {
        "from" => true,
        "to" => false,
        other => return ExecResult::err(format!("endpoint '{}' must be 'from' or 'to'", other)),
    };
    let name = match args.positional(1) {
        Some(n) => n.to_ascii_lowercase(),
        None => return ExecResult::err("usage: cap <from|to> <name>"),
    };
    let glyph = match resolve_cap(is_from, &name) {
        Some(g) => g,
        None => {
            return ExecResult::err(format!(
                "cap '{}' must be one of arrow|circle|diamond|none",
                name
            ))
        }
    };
    let changed = if is_from {
        eff.document.set_edge_cap_start(&er, glyph)
    } else {
        eff.document.set_edge_cap_end(&er, glyph)
    };
    if changed {
        ExecResult::ok_msg(format!("cap {} set to {}", endpoint, name))
    } else {
        ExecResult::ok_msg(format!("cap {} already {}", endpoint, name))
    }
}
