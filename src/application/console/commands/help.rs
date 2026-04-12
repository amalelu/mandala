//! `help [command]` — list commands or print full usage.
//!
//! With no args: show every *applicable* command for the current
//! selection with its summary. `help --all` shows everything.
//!
//! With one arg: print usage + summary for that command. Unknown
//! names are reported as an `Err` result so the line shows up in the
//! error color.

use super::{command_by_name, Command, COMMANDS};
use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const COMMAND: Command = Command {
    name: "help",
    aliases: &["?", "h"],
    summary: "List commands or print usage for one",
    usage: "help [command] [--all]",
    tags: &["list", "usage", "commands"],
    applicable: always,
    complete: complete_help,
    execute: execute_help,
};

fn complete_help(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    // Only complete at position 1 (the command-name arg).
    if state.cursor_token != 1 {
        return Vec::new();
    }
    let partial = state.partial.to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|c| c.name.to_ascii_lowercase().starts_with(&partial))
        .map(|c| Completion {
            text: c.name.to_string(),
            display: c.name.to_string(),
            hint: Some(c.summary.to_string()),
        })
        .collect()
}

fn execute_help(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let ctx = ConsoleContext::from_document(eff.document);
    match args.positional(0) {
        Some(name) => help_for(name, &ctx),
        None => help_listing(&ctx, args.has_flag("all")),
    }
}

fn help_for(name: &str, _ctx: &ConsoleContext) -> ExecResult {
    match command_by_name(name) {
        Some(cmd) => {
            let mut lines = vec![
                format!("{} — {}", cmd.name, cmd.summary),
                format!("usage: {}", cmd.usage),
            ];
            if !cmd.aliases.is_empty() {
                lines.push(format!("aliases: {}", cmd.aliases.join(", ")));
            }
            ExecResult::Lines(lines)
        }
        None => ExecResult::err(format!("unknown command: {}", name)),
    }
}

fn help_listing(ctx: &ConsoleContext, show_all: bool) -> ExecResult {
    let mut lines: Vec<String> = Vec::with_capacity(COMMANDS.len() + 1);
    lines.push(if show_all {
        "all commands:".to_string()
    } else {
        "commands (use `help --all` to see non-applicable ones):".to_string()
    });
    for cmd in COMMANDS {
        if !show_all && !(cmd.applicable)(ctx) {
            continue;
        }
        lines.push(format!("  {:<12} {}", cmd.name, cmd.summary));
    }
    ExecResult::Lines(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::tokenize;

    fn args_from(line: &str) -> Vec<String> {
        tokenize(line)
    }

    #[test]
    fn test_complete_help_takes_one_arg() {
        let toks: Vec<String> = args_from("help a");
        let state = CompletionState {
            tokens: &toks,
            cursor_token: 1,
            partial: "a",
        };
        assert_eq!(state.cursor_token, 1);
    }

    #[test]
    fn test_help_summary_line_is_not_empty() {
        assert!(!COMMAND.summary.is_empty());
        assert!(!COMMAND.usage.is_empty());
    }
}
