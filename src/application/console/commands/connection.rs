//! `connection <reset-straight|reset-style>` — connection-path and
//! style resets on the selected edge.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const SUBCOMMANDS: &[&str] = &["reset-straight", "reset-style"];

pub const COMMAND: Command = Command {
    name: "connection",
    aliases: &["conn"],
    summary: "Reset the path or style of the selected edge",
    usage: "connection <reset-straight|reset-style>",
    tags: &["edge", "connection", "reset", "straight", "style"],
    applicable: edge_selected,
    complete: complete_connection,
    execute: execute_connection,
};

fn complete_connection(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    if state.cursor_token != 1 {
        return Vec::new();
    }
    enum_completion(SUBCOMMANDS, state.partial)
}

fn execute_connection(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match &eff.document.selection {
        SelectionState::Edge(e) => e.clone(),
        _ => return ExecResult::err("no edge selected"),
    };
    let sub = match args.positional(0) {
        Some(s) => s.to_ascii_lowercase(),
        None => return ExecResult::err("usage: connection <reset-straight|reset-style>"),
    };
    match sub.as_str() {
        "reset-straight" => {
            let changed = eff.document.reset_edge_to_straight(&er);
            if changed {
                ExecResult::ok_msg("connection reset to straight")
            } else {
                ExecResult::ok_msg("connection already straight")
            }
        }
        "reset-style" => {
            let changed = eff.document.reset_edge_style_to_default(&er);
            if changed {
                ExecResult::ok_msg("connection style reset")
            } else {
                ExecResult::ok_msg("no style override to reset")
            }
        }
        other => ExecResult::err(format!(
            "unknown connection subcommand '{}'",
            other
        )),
    }
}
