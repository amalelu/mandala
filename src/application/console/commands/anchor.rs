//! `anchor set <from|to> <auto|top|right|bottom|left>` — edge anchor
//! side setter. Collapses the ten per-side palette actions into a
//! single command with two enum args.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const SIDES: &[&str] = &["auto", "top", "right", "bottom", "left"];
pub const ENDPOINTS: &[&str] = &["from", "to"];

pub const COMMAND: Command = Command {
    name: "anchor",
    aliases: &[],
    summary: "Set the from/to anchor side of the selected edge",
    usage: "anchor set <from|to> <auto|top|right|bottom|left>",
    tags: &["edge", "anchor", "side", "connection"],
    applicable: edge_selected,
    complete: complete_anchor,
    execute: execute_anchor,
};

fn complete_anchor(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match state.cursor_token {
        1 => enum_completion(&["set"], state.partial),
        2 => enum_completion(ENDPOINTS, state.partial),
        3 => enum_completion(SIDES, state.partial),
        _ => Vec::new(),
    }
}

fn execute_anchor(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match &eff.document.selection {
        SelectionState::Edge(e) => e.clone(),
        _ => return ExecResult::err("no edge selected"),
    };

    let sub = match args.positional(0) {
        Some(s) => s,
        None => return ExecResult::err("usage: anchor set <from|to> <side>"),
    };
    if !sub.eq_ignore_ascii_case("set") {
        return ExecResult::err(format!("unknown subcommand '{}'; expected 'set'", sub));
    }

    let endpoint = match args.positional(1) {
        Some(e) => e.to_ascii_lowercase(),
        None => return ExecResult::err("usage: anchor set <from|to> <side>"),
    };
    let is_from = match endpoint.as_str() {
        "from" => true,
        "to" => false,
        other => return ExecResult::err(format!("endpoint '{}' must be 'from' or 'to'", other)),
    };

    let side = match args.positional(2) {
        Some(s) => s.to_ascii_lowercase(),
        None => return ExecResult::err("usage: anchor set <from|to> <side>"),
    };
    let value: i32 = match side.as_str() {
        "auto" => 0,
        "top" => 1,
        "right" => 2,
        "bottom" => 3,
        "left" => 4,
        other => {
            return ExecResult::err(format!(
                "side '{}' must be one of auto|top|right|bottom|left",
                other
            ))
        }
    };

    let changed = eff.document.set_edge_anchor(&er, is_from, value);
    if changed {
        ExecResult::ok_msg(format!("anchor {} set to {}", endpoint, side))
    } else {
        ExecResult::ok_msg(format!("anchor {} already {}", endpoint, side))
    }
}
