//! `alias <name> <expansion...>` — user-defined command aliases.
//!
//! Commit 1 stub: the verb is registered for discoverability but the
//! storage + dispatch live in commit 4. Until then `execute` returns
//! an informative error.

use super::Command;
use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const COMMAND: Command = Command {
    name: "alias",
    aliases: &[],
    summary: "Define a command alias",
    usage: "alias <name> <expansion...> [--save]",
    tags: &["alias", "shortcut"],
    applicable: always,
    complete: complete_alias,
    execute: execute_alias,
};

fn complete_alias(_state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    Vec::new()
}

fn execute_alias(_args: &Args, _eff: &mut ConsoleEffects) -> ExecResult {
    ExecResult::err("alias: not yet wired up (landing in commit 4)".to_string())
}
