//! `alias <name> <expansion...> [--save]` — user-defined command
//! aliases.
//!
//! The `name` becomes a first-token shortcut: typing `alias a anchor
//! set from auto` then `a` on its own line runs `anchor set from
//! auto`. Without `--save` the alias only lives in the current
//! session; with `--save` the dispatcher writes it into the
//! user-mutations file alongside any existing `aliases` entries.
//!
//! Parsing splits off the `--save` flag via `Args::has_flag`; every
//! remaining token after the name joins back with a single space to
//! form the expansion.

use super::Command;
use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult, SetAliasRequest};

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

fn execute_alias(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let name = match args.positional(0) {
        Some(n) => n.to_string(),
        None => return ExecResult::err("usage: alias <name> <expansion...> [--save]"),
    };
    if name.is_empty() {
        return ExecResult::err("alias name must be non-empty");
    }
    // Collect every positional after `name` into the expansion.
    // `Args::positional` already skips flags, so `--save` is
    // harmlessly left out of the expansion.
    let mut parts: Vec<String> = Vec::new();
    let mut idx = 1;
    while let Some(tok) = args.positional(idx) {
        parts.push(tok.to_string());
        idx += 1;
    }
    if parts.is_empty() {
        return ExecResult::err("alias expansion must not be empty");
    }
    let expansion = parts.join(" ");
    let save = args.has_flag("save");
    eff.set_alias = Some(SetAliasRequest {
        name: name.clone(),
        expansion: expansion.clone(),
        save,
    });
    if save {
        ExecResult::ok_msg(format!("alias {} = {} (saved)", name, expansion))
    } else {
        ExecResult::ok_msg(format!("alias {} = {}", name, expansion))
    }
}
