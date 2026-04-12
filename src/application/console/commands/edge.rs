//! `edge type <cross_link|parent_child>` — convert the selected
//! edge between cross-link and hierarchy types.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const EDGE_TYPES: &[&str] = &["cross_link", "parent_child"];

pub const COMMAND: Command = Command {
    name: "edge",
    aliases: &[],
    summary: "Edge-type and related operations on the selected edge",
    usage: "edge type <cross_link|parent_child>",
    tags: &["edge", "type", "cross_link", "parent_child", "hierarchy"],
    applicable: edge_selected,
    complete: complete_edge,
    execute: execute_edge,
};

fn complete_edge(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match state.cursor_token {
        1 => enum_completion(&["type"], state.partial),
        2 => enum_completion(EDGE_TYPES, state.partial),
        _ => Vec::new(),
    }
}

fn execute_edge(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match &eff.document.selection {
        SelectionState::Edge(e) => e.clone(),
        _ => return ExecResult::err("no edge selected"),
    };
    let sub = match args.positional(0) {
        Some(s) => s.to_ascii_lowercase(),
        None => return ExecResult::err("usage: edge type <cross_link|parent_child>"),
    };
    if sub != "type" {
        return ExecResult::err(format!("unknown edge subcommand '{}'", sub));
    }
    let t = match args.positional(1) {
        Some(s) => s.to_ascii_lowercase(),
        None => return ExecResult::err("usage: edge type <cross_link|parent_child>"),
    };
    if !EDGE_TYPES.iter().any(|v| *v == t) {
        return ExecResult::err(format!(
            "edge type '{}' must be cross_link or parent_child",
            t
        ));
    }
    let changed = eff.document.set_edge_type(&er, &t);
    if changed {
        ExecResult::ok_msg(format!("edge type set to {}", t))
    } else {
        ExecResult::ok_msg(format!("edge already of type {}", t))
    }
}
