//! `mutate <list|run|bind|unbind>` — inspect and apply custom
//! mutations.
//!
//! Commit 1 stub: only the verb + completion skeleton ship here. The
//! subcommand logic lands in commits 3 (list/run) and 4 (bind/unbind).
//! The `execute` fn currently returns an informative error so the
//! surface is discoverable without yet affecting behavior.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const SUBCOMMANDS: &[&str] = &["list", "run", "bind", "unbind"];

pub const COMMAND: Command = Command {
    name: "mutate",
    aliases: &[],
    summary: "Inspect and apply custom mutations",
    usage: "mutate <list | run <id> [node_id] | bind <key> <id> | unbind <key>>",
    tags: &["mutate", "mutation", "custom", "run", "bind"],
    applicable: always,
    complete: complete_mutate,
    execute: execute_mutate,
};

fn complete_mutate(state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
    match state.cursor_token {
        1 => enum_completion(SUBCOMMANDS, state.partial),
        2 => {
            let sub = state.tokens.get(1).map(|s| s.as_str()).unwrap_or("");
            if sub.eq_ignore_ascii_case("run") {
                let ids: Vec<&str> = ctx.mutation_registry.keys().map(|s| s.as_str()).collect();
                enum_completion(&ids, state.partial)
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn execute_mutate(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    ExecResult::err(
        "mutate: not yet wired up (landing in commit 3 for list/run, commit 4 for bind/unbind)"
            .to_string(),
    )
}
