//! `select <node|edge|portal|none> [id]` — programmatic selection
//! manipulation. Lets scripts/macros set the selection without a
//! click. `select none` clears.
//!
//! `select node <id>` requires the id to exist in the current map.
//! `select edge` / `select portal` are handled via the fully-qualified
//! triples a command like `mutate` would print.
//!
//! Only `node` selection is fleshed out here — edge and portal
//! addressing is deferred to a future session; unknown kinds report
//! an error.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const KINDS: &[&str] = &["node", "none", "edge", "portal"];

pub const COMMAND: Command = Command {
    name: "select",
    aliases: &["sel"],
    summary: "Set the current selection programmatically",
    usage: "select <none | node <id>>",
    tags: &["selection", "select", "node", "none"],
    applicable: always,
    complete: complete_select,
    execute: execute_select,
};

fn complete_select(state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
    match state.cursor_token {
        1 => enum_completion(KINDS, state.partial),
        2 => {
            let kind = state.tokens.get(1).map(|s| s.as_str()).unwrap_or("");
            if kind.eq_ignore_ascii_case("node") {
                let ids: Vec<&str> =
                    ctx.document.mindmap.nodes.keys().map(|s| s.as_str()).collect();
                enum_completion(&ids, state.partial)
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn execute_select(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let kind = match args.positional(0) {
        Some(s) => s.to_ascii_lowercase(),
        None => return ExecResult::err("usage: select <none|node <id>>"),
    };
    match kind.as_str() {
        "none" => {
            eff.document.selection = SelectionState::None;
            ExecResult::ok_msg("selection cleared")
        }
        "node" => {
            let id = match args.positional(1) {
                Some(s) => s.to_string(),
                None => return ExecResult::err("usage: select node <id>"),
            };
            if !eff.document.mindmap.nodes.contains_key(&id) {
                return ExecResult::err(format!("no node with id '{}'", id));
            }
            eff.document.selection = SelectionState::Single(id.clone());
            ExecResult::ok_msg(format!("selected node {}", id))
        }
        "edge" | "portal" => ExecResult::err(format!(
            "select {} is not yet wired up in the console; click it on the canvas",
            kind
        )),
        other => ExecResult::err(format!("unknown selection kind '{}'", other)),
    }
}
