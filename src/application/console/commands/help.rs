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
use crate::application::console::completion::Completion;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::{Bare, Form, Grammar, Slot, Vocabulary, Word};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

/// `all` is not a command, which is why the registry walk below
/// never reaches it — but `execute_help` dispatches on it, so it is
/// declared as this slot's one sentinel and appears in the popup and
/// in the usage line alike.
const ALL: &[Word] = &[Word::new(
    "all",
    "include commands the current selection can't use",
)];

/// One row per registered command, each hinted with its summary.
/// Not filtered by applicability: `help border` prints the border
/// usage whatever is selected, so the popup offers every name.
fn command_rows(_ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    let partial = partial.to_ascii_lowercase();
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

pub static GRAMMAR: Grammar = Grammar {
    label: "help",
    subverb_sets: &[],
    key_sets: &[],
    bare: Some(Bare::new(
        "listing",
        &[Form::slots(&[Slot::opt(Vocabulary::Rows {
            placeholder: "command",
            rows: command_rows,
            sentinels: ALL,
        })])],
    )),
};

pub const COMMAND: Command = Command {
    name: "help",
    aliases: &["?", "h"],
    summary: "List commands or print usage for one",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["list", "usage", "commands"],
    execute: execute_help,
};

fn execute_help(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let ctx = ConsoleContext::from_document(eff.document());
    match args.positional(0) {
        // `all` is the one argument that is not a command name, and
        // it was the one matched exactly while `command_by_name`
        // beside it is case-insensitive — so `help BORDER` printed
        // the border usage and `help ALL` answered "unknown
        // command: ALL". Same rule for both now (see
        // `commands/mod.rs` § Casing).
        Some(name) if name.eq_ignore_ascii_case("all") => help_listing(&ctx, true),
        Some(name) => help_for(name, &ctx),
        None => help_listing(&ctx, false),
    }
}

fn help_for(name: &str, _ctx: &ConsoleContext) -> ExecResult {
    match command_by_name(name) {
        Some(cmd) => {
            let mut lines = vec![format!("{} — {}", cmd.name, cmd.summary)];
            let forms = cmd.usage_forms();
            match forms.split_first() {
                Some((head, rest)) => {
                    lines.push(format!("usage: {}", head));
                    for form in rest {
                        lines.push(format!("       {}", form));
                    }
                }
                None => lines.push(format!("usage: {}", cmd.name)),
            }
            // The kv vocabulary, one line per key. A long composed
            // form collapses to `<key>=<value> …` in the usage
            // block above precisely because this block carries the
            // detail; both derive from the one grammar, so a key
            // added to a verb is documented here by the same edit
            // that makes it parseable.
            let keys = cmd.key_lines();
            if let Some((head, rest)) = keys.split_first() {
                lines.push(format!("keys:  {}", head));
                for key in rest {
                    lines.push(format!("       {}", key));
                }
            }
            if !cmd.aliases.is_empty() {
                lines.push(format!("aliases: {}", cmd.aliases.join(", ")));
            }
            // `tags` carries the search words a verb's name doesn't
            // contain ("pick" under `color`). Twenty commands author
            // them and nothing read the field until #41 found it
            // dead; printing them here is what makes them
            // discoverable rather than decorative. A migrated verb
            // derives the whole list from its grammar, so a key
            // added to the table is searchable by the same edit.
            let tags = cmd.tag_list();
            if !tags.is_empty() {
                lines.push(format!("tags: {}", tags.join(", ")));
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

    /// `help` completes its one argument slot and no other — the
    /// verb takes a single command name (or `all`), so a second
    /// positional has no vocabulary to offer.
    #[test]
    fn test_complete_help_takes_one_arg() {
        let doc = crate::application::document::tests_common::load_test_doc();
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let popup = |line: &str| crate::application::console::completion::complete(line, line.len(), &ctx);
        assert!(!popup("help a").is_empty());
        assert!(popup("help anchor a").is_empty());
    }

    #[test]
    fn test_help_summary_line_is_not_empty() {
        assert!(!COMMAND.summary.is_empty());
        assert!(!COMMAND.usage_forms().is_empty());
    }

    /// `help <verb>` publishes the four blocks its output is made
    /// of — the derived `usage:` forms, the derived `keys:` block,
    /// `aliases:` and the derived `tags:` line — and omits the two
    /// optional ones when the verb has nothing for them.
    ///
    /// The failing inputs are: a `usage_forms` that stops naming a
    /// declared subverb, a `key_lines` that stops naming a declared
    /// key, an alias list printed for a verb that has none, and a
    /// tags line that loses either a structural word or a declared
    /// synonym. All four used to be `&'static str` literals `help`
    /// echoed, which is why `font range=` could be parseable and
    /// undocumented at the same time.
    #[test]
    fn test_help_for_publishes_its_four_derived_blocks() {
        let doc = crate::application::document::tests_common::load_test_doc();
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let text_of = |name: &str| -> Vec<String> {
            match help_for(name, &ctx) {
                crate::application::console::ExecResult::Lines(ls) => {
                    ls.into_iter().map(|l| l.text).collect()
                }
                other => panic!("expected Lines, got {:?}", other),
            }
        };

        // `help` has aliases and no keys: the alias line appears,
        // the `keys:` block does not.
        let help_lines = text_of("help");
        assert!(
            help_lines.iter().any(|l| l == "aliases: ?, h"),
            "help's aliases must be published verbatim; got {:?}",
            help_lines
        );
        assert!(
            !help_lines.iter().any(|l| l.starts_with("keys:")),
            "a verb with no keys must not emit an empty keys block; got {:?}",
            help_lines
        );

        // `color` has keys and no aliases. Every key it declares
        // gets a `keys:` line, and every one of its subverbs and
        // keys — plus the one synonym the grammar does not contain
        // — reaches the tags line.
        let color_lines = text_of("color");
        assert!(
            !color_lines.iter().any(|l| l.starts_with("aliases:")),
            "a verb with no aliases must not emit an empty aliases line; got {:?}",
            color_lines
        );
        let blob = color_lines.join("\n");
        for key in super::super::color::GRAMMAR.keys() {
            assert!(
                blob.contains(&format!("{}=", key.name)),
                "`help color` must publish the key '{}': {blob}",
                key.name
            );
        }
        let tags = color_lines
            .iter()
            .find(|l| l.starts_with("tags: "))
            .expect("color publishes a tags line");
        for word in [
            "color", "bg", "text", "border", "pick", "picker", "section", "range", "wheel",
        ] {
            assert!(
                tags.contains(word),
                "`help color`'s tags must carry '{word}': {tags}"
            );
        }
        // `usage:` leads with the forms, one per line — the first
        // under the `usage:` label, the rest indented under it.
        assert!(
            color_lines.iter().any(|l| l == "usage: color bg"),
            "the first derived form leads the usage block; got {color_lines:?}"
        );
        assert!(
            color_lines.iter().any(|l| l == "       color pick"),
            "every later form gets its own line; got {color_lines:?}"
        );
    }

    /// `help all` is dispatched by `execute_help` and named in the
    /// verb's own usage line, but the completer only ever walked
    /// `COMMANDS` — and `all` is not a command, so the popup was
    /// silent about the one argument `help` documents.
    #[test]
    fn test_help_completion_offers_the_all_argument() {
        let doc = crate::application::document::tests_common::load_test_doc();
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let popup = |line: &str| -> Vec<String> {
            crate::application::console::completion::complete(line, line.len(), &ctx)
                .into_iter()
                .map(|c| c.text)
                .collect()
        };
        assert!(popup("help ").iter().any(|r| r == "all"));
        assert_eq!(popup("help al"), vec!["all"]);
        // The command names it sits among are untouched.
        assert!(popup("help ").iter().any(|r| r == "color"));
        assert!(
            COMMAND.usage_forms().iter().any(|f| f.contains("all")),
            "the derived usage line must name the sentinel: {:?}",
            COMMAND.usage_forms()
        );
    }
}
