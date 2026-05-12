// SPDX-License-Identifier: MPL-2.0

//! `help [command | all]` — list commands or print full usage.
//!
//! With no args: show every *applicable* command for the current
//! selection with its summary. `help all` shows everything.
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
    usage: "help [command | all]",
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
            font_family: None,
        })
        .collect()
}

fn execute_help(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let ctx = ConsoleContext::from_document(eff.document);
    match args.positional(0) {
        Some("all") => help_listing(&ctx, true),
        Some(name) => help_for(name, &ctx),
        None => help_listing(&ctx, false),
    }
}

/// Split a usage string on the top-level verb-form separator
/// ` | ` (space-pipe-space) — angle-bracket-aware so embedded
/// alternation inside `<...>` survives. Existing usage strings
/// like `spacing value=<tight|normal|wide | <float>>` and
/// `mutation <list [--all] [filter] | apply <id> ...>` carry
/// ` | ` inside `<...>` to mean "alternation among parameter
/// values"; only the top-level (depth==0) separator marks form
/// boundaries.
fn split_usage_forms(usage: &str) -> Vec<&str> {
    let bytes = usage.as_bytes();
    let mut forms = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => depth = (depth - 1).max(0),
            b' ' if depth == 0 && i + 2 < bytes.len() && bytes[i + 1] == b'|' && bytes[i + 2] == b' ' => {
                forms.push(&usage[start..i]);
                start = i + 3;
                i += 3;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    forms.push(&usage[start..]);
    forms
}

fn help_for(name: &str, _ctx: &ConsoleContext) -> ExecResult {
    match command_by_name(name) {
        Some(cmd) => {
            let mut lines = vec![format!("{} — {}", cmd.name, cmd.summary)];
            let forms = split_usage_forms(cmd.usage);
            if forms.len() == 1 {
                lines.push(format!("usage: {}", forms[0]));
            } else {
                lines.push(format!("usage: {}", forms[0].trim()));
                for form in forms.iter().skip(1) {
                    lines.push(format!("       {}", form.trim()));
                }
            }
            if !cmd.aliases.is_empty() {
                lines.push(format!("aliases: {}", cmd.aliases.join(", ")));
            }
            ExecResult::lines(lines)
        }
        None => ExecResult::err(format!("unknown command: {}", name)),
    }
}

fn help_listing(ctx: &ConsoleContext, show_all: bool) -> ExecResult {
    let mut lines: Vec<String> = Vec::with_capacity(COMMANDS.len() + 1);
    lines.push(if show_all {
        "all commands:".to_string()
    } else {
        "commands (use `help all` to see non-applicable ones):".to_string()
    });
    for cmd in COMMANDS {
        if !show_all && !(cmd.applicable)(ctx) {
            continue;
        }
        lines.push(format!("  {:<12} {}", cmd.name, cmd.summary));
    }
    ExecResult::lines(lines)
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
        use crate::application::console::completion::CompletionContext;
        let toks: Vec<String> = args_from("help a");
        let state = CompletionState {
            tokens: &toks,
            cursor_token: 1,
            partial: "a",
            context: CompletionContext::Token { index: 0 },
        };
        assert_eq!(state.cursor_token, 1);
    }

    #[test]
    fn test_help_summary_line_is_not_empty() {
        assert!(!COMMAND.summary.is_empty());
        assert!(!COMMAND.usage.is_empty());
    }

    /// `help section` splits the multi-form usage string on the
    /// top-level " | " separator so each form lands on its own
    /// line. Pin specific expected forms — a regression that
    /// dropped 9 of 11 forms would still pass a `>= 2` assertion.
    #[test]
    fn test_help_for_section_splits_multi_form_usage_to_separate_lines() {
        let doc = crate::application::document::tests_common::load_test_doc();
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let result = help_for("section", &ctx);
        let lines = match result {
            crate::application::console::ExecResult::Lines(ls) => ls,
            other => panic!("expected Lines, got {:?}", other),
        };
        let usage_lines: Vec<&str> = lines
            .iter()
            .filter(|l| l.text.starts_with("usage:") || l.text.starts_with("       "))
            .map(|l| l.text.as_str())
            .collect();
        for marker in &[
            "section show",
            "section move dx=",
            "section move x=",
            "section resize w=",
            "section resize fill",
            "section text",
            "section edit",
            "section add",
            "section delete",
            "section split",
            "section frame",
        ] {
            assert!(
                usage_lines.iter().any(|l| l.contains(marker)),
                "section help must surface form '{}' on its own line; got {:?}",
                marker,
                usage_lines
            );
        }
        for line in &usage_lines {
            assert!(
                line.len() < 250,
                "no usage line should be wall-of-text: {} chars: {}",
                line.len(),
                line
            );
        }
    }

    /// Top-level ` | ` is the form separator; ` | ` *inside*
    /// `<...>` parameter brackets is alternation and must
    /// survive. Pre-fix the splitter was greedy and broke
    /// `help spacing` (`value=<tight|normal|wide | <float>>`)
    /// and `help mutation` (`<list ... | apply ... | help ...>`)
    /// into ungrammatical fragments.
    #[test]
    fn test_help_for_spacing_does_not_split_inside_angle_brackets() {
        let doc = crate::application::document::tests_common::load_test_doc();
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let result = help_for("spacing", &ctx);
        let lines = match result {
            crate::application::console::ExecResult::Lines(ls) => ls,
            other => panic!("expected Lines, got {:?}", other),
        };
        let usage_lines: Vec<&str> = lines
            .iter()
            .filter(|l| l.text.starts_with("usage:") || l.text.starts_with("       "))
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(
            usage_lines.len(),
            1,
            "spacing's single-form usage must stay on one line; got {:?}",
            usage_lines
        );
        let line = usage_lines[0];
        assert!(
            line.contains("<tight|normal|wide | <float>>"),
            "spacing's parameter alternation must survive intact; got '{}'",
            line
        );
    }

    #[test]
    fn test_help_for_mutation_does_not_split_inside_angle_brackets() {
        let doc = crate::application::document::tests_common::load_test_doc();
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let result = help_for("mutation", &ctx);
        let lines = match result {
            crate::application::console::ExecResult::Lines(ls) => ls,
            other => panic!("expected Lines, got {:?}", other),
        };
        let usage_lines: Vec<&str> = lines
            .iter()
            .filter(|l| l.text.starts_with("usage:") || l.text.starts_with("       "))
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(
            usage_lines.len(),
            1,
            "mutation's single-form usage must stay on one line; got {:?}",
            usage_lines
        );
    }

    /// Direct unit test for the splitter: depth-aware split on
    /// top-level ` | ` only. Pin every interesting case so a
    /// future contributor refactoring `split_usage_forms` can
    /// see what the contract is at a glance.
    #[test]
    fn test_split_usage_forms_depth_aware() {
        // Single form, no separator.
        assert_eq!(split_usage_forms("foo"), vec!["foo"]);
        // Top-level separator splits.
        assert_eq!(split_usage_forms("a | b | c"), vec!["a", "b", "c"]);
        // No-spaces pipes inside enums survive (existing behaviour).
        assert_eq!(
            split_usage_forms("cap from=<arrow|circle|none>"),
            vec!["cap from=<arrow|circle|none>"]
        );
        // Spaces-around pipes INSIDE angle brackets survive (the
        // bug fix).
        assert_eq!(
            split_usage_forms("spacing value=<tight|wide | <float>>"),
            vec!["spacing value=<tight|wide | <float>>"]
        );
        // Mixed: one form with embedded ` | ` plus a top-level
        // separator at depth 0.
        assert_eq!(
            split_usage_forms("a value=<x | y> | b"),
            vec!["a value=<x | y>", "b"]
        );
    }

    /// Single-form verbs (e.g. `cap`) keep their one-line
    /// usage. The split shouldn't fire when there's no `|`
    /// separator at the form level — value enums like
    /// `<arrow|circle|diamond|none>` (no spaces around the
    /// pipe) survive intact.
    #[test]
    fn test_help_for_single_form_keeps_one_usage_line() {
        let doc = crate::application::document::tests_common::load_test_doc();
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let result = help_for("cap", &ctx);
        let lines = match result {
            crate::application::console::ExecResult::Lines(ls) => ls,
            other => panic!("expected Lines, got {:?}", other),
        };
        let usage_count = lines.iter().filter(|l| l.text.starts_with("usage:")).count();
        let cont_count = lines.iter().filter(|l| l.text.starts_with("       ")).count();
        assert_eq!(usage_count, 1);
        assert_eq!(
            cont_count, 0,
            "single-form usage shouldn't produce continuation lines"
        );
    }
}
