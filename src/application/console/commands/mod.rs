// SPDX-License-Identifier: MPL-2.0

//! Console command registry.
//!
//! Each command lives in its own submodule so the surface stays
//! scannable. The public `COMMANDS` slice gathers them in one place,
//! matching the `const PALETTE_ACTIONS` pattern — zero-cost startup,
//! no HashMap construction, and `action_by_id`-style lookup is a
//! linear scan over a dozen entries.
//!
//! # Casing
//!
//! One rule, and the engine is now the one place that applies it:
//!
//! - **Command names, aliases and positional subverbs are matched
//!   case-insensitively.** `mode DEFAULT`, `font SET Norse` and
//!   `border PRESET heavy` all run. The grammar descent lowercases
//!   once and the popup filters its partial the same way, so the
//!   two cannot disagree; before the declaration, five verbs
//!   honored the rule and the rest did not, which meant a word the
//!   popup listed could still be refused when typed out in full.
//! - **Kv *keys* are exact.** `border TOP=x` is `unknown key
//!   'TOP'`, and the engine's key rows filter case-sensitively to
//!   say so. A key is a field name, not a word the user picks.
//! - **Kv *values* belong to the key's own parser.** A closed
//!   vocabulary is matched case-insensitively (`preset=HEAVY`,
//!   `color=ACCENT`, `side=TOP`); a document-derived one is
//!   matched the way its own rows are found, which is how a
//!   palette name is stored verbatim and still found by
//!   `palette=CO`. One flag survives outside the rule —
//!   `mutation list --all` is a slot *value*, not a subverb (#135).
//!
//! What makes the first bullet checkable rather than aspirational
//! is `console::tests::oracle_corpus`: every verb whose subverb
//! dispatch normalizes carries an upper-case corpus row beside its
//! lower-case one. (Named rather than linked — the module is
//! `#[cfg(test)]`, so rustdoc cannot see it even under
//! `--document-private-items`, and an intra-doc link to it fails
//! the `-D warnings` doc gate.)
//!
//! # Usage, tags and completion come from the grammar
//!
//! A verb declares one [`crate::application::console::spec::Grammar`]
//! and nothing else: [`Command::usage_forms`], [`Command::key_lines`],
//! [`Command::tag_list`] and [`Command::completions`] are all derived
//! from it, and the kv parse loop reads it too. Adding a key is one
//! table row, and `help <verb>` documents it in the same edit that
//! makes it parseable.
//!
//! That replaces a genuine defect rather than a stylistic one.
//! `Command` used to carry `usage` and `tags` as `&'static str`
//! literals that `help` printed verbatim while nothing derived them
//! from the verb's key list, so the three declarations could
//! disagree: a key added to a verb's key list was offered by the
//! popup on the next keystroke and stayed absent from `help <verb>`
//! until somebody wrote it in by hand. `font` and `color` each carried
//! that drift for `range=` — parseable, named in the verb's own
//! rejection, and documented nowhere. The two per-verb assertions
//! that closed those *instances* were themselves per-verb copies of
//! a check; `console::spec`'s own tests hold the invariant over every
//! declared level instead, in both directions — a form naming a key
//! its level does not declare, and a key no form prints.
//!
//! There is no second way to declare a verb. [`Command`] has no
//! `usage`, no `tags` and no `complete` field for a grammar to
//! disagree with — a framework with two adopters would leave two
//! grammars in the tree, which is worse than the one hand-rolled
//! grammar it replaced, so every verb reads the engine or none
//! would.

use super::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::spec::{self, Grammar};

pub mod anchor;
pub mod body;
pub mod border;
pub mod canvas;
pub mod cap;
pub mod color;
pub mod edge;
pub mod font;
pub mod fps;
pub mod help;
pub mod label;
pub mod mode;
pub mod mutation;
pub mod new;
pub mod node;
pub mod open;
pub mod range_kv;
pub mod save;
pub mod section;
pub mod spacing;
pub mod zoom;

/// One entry in the console command registry. Kept small and
/// `'static` so the whole registry can live in a `const` slice.
#[derive(Clone, Copy)]
pub struct Command {
    /// Primary name — the token users type at position 0.
    pub name: &'static str,
    /// Alternative names. Case-insensitive in [`command_by_name`].
    pub aliases: &'static [&'static str],
    /// One-line summary shown in `help` with no args.
    pub summary: &'static str,
    /// The verb's declarative grammar (`console::spec`) — the
    /// single source of its usage forms, its `keys:` block, its
    /// tags, its completion popup, its kv parse loop and its hint
    /// surface.
    pub grammar: &'static Grammar,
    /// Search words the grammar does not contain — `wheel` under
    /// `color`, `lod` under `zoom`. Every structural word (subverb
    /// names, kv keys) is derived; this is only what is neither.
    pub synonyms: &'static [&'static str],
    /// Returns `true` when the command should appear in the filtered
    /// `help` list and in completion. Commands whose args are
    /// context-specific but whose verb is always meaningful should
    /// return `true` here and validate in `execute`.
    pub applicable: fn(&ConsoleContext) -> bool,
    /// Run the command. The dispatcher clears the scene cache and
    /// rebuilds after every non-`Err` result.
    pub execute: fn(&Args, &mut ConsoleEffects) -> ExecResult,
}

impl Command {
    /// The usage lines `help <cmd>` prints, one per form.
    pub fn usage_forms(&self) -> Vec<String> {
        spec::usage::forms(self.grammar)
    }

    /// One line per kv key the verb declares, printed by
    /// `help <cmd>` under a `keys:` block. Empty for a verb that
    /// declares no keys.
    pub fn key_lines(&self) -> Vec<String> {
        spec::usage::key_lines(self.grammar)
    }

    /// The search words `help <cmd>` publishes.
    pub fn tag_list(&self) -> Vec<&'static str> {
        spec::usage::tags(self.grammar, self.synonyms)
    }

    /// The completion popup for one cursor position.
    pub fn completions(&self, state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
        spec::complete::completions(self.grammar, state, ctx)
    }
}

/// The global command registry. Order matters only for `help` — the
/// listing iterates this slice in declaration order.
pub const COMMANDS: &[Command] = &[
    help::COMMAND,
    anchor::COMMAND,
    body::COMMAND,
    border::COMMAND,
    canvas::COMMAND,
    cap::COMMAND,
    color::COMMAND,
    edge::COMMAND,
    font::COMMAND,
    fps::COMMAND,
    spacing::COMMAND,
    label::COMMAND,
    mode::COMMAND,
    mutation::COMMAND,
    save::COMMAND,
    open::COMMAND,
    new::COMMAND,
    node::COMMAND,
    section::COMMAND,
    zoom::COMMAND,
];

/// Look up a command by its name or any alias. Case-insensitive.
pub fn command_by_name(name: &str) -> Option<&'static Command> {
    let lower = name.to_ascii_lowercase();
    COMMANDS.iter().find(|c| {
        c.name.eq_ignore_ascii_case(&lower) || c.aliases.iter().any(|a| a.eq_ignore_ascii_case(&lower))
    })
}

/// Single-source success-or-no-op message for verbs that aggregate
/// across a set of selection targets. `verb` is the noun the user
/// typed (`"font"`, `"zoom"`, `"color"`, …); `kind` is the
/// selection scope (`"node"`, `"section"`, `"edge"`, …); `changed`
/// is whether at least one target actually mutated.
///
/// Two formats: `"<verb> applied to <kind>"` on change, `"<verb>:
/// no change on <kind>"` on no-op. Mirrors the previous open-coded
/// `finalize` helpers in `commands/font.rs` and `commands/zoom.rs`.
pub(super) fn applied_or_no_change(verb: &str, kind: &str, changed: bool) -> ExecResult {
    if changed {
        ExecResult::ok_msg(format!("{verb} applied to {kind}"))
    } else {
        ExecResult::ok_msg(format!("{verb}: no change on {kind}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lookup is case-insensitive across both names and aliases,
    /// returns `None` on unknown, and resolves aliases to their
    /// canonical entry. Also pins the registered verb names — the
    /// compiler enforces that the `COMMANDS` slice exists, but a
    /// typo in `name: "border"` → `"boder"` would compile and
    /// silently break user-facing console input without this list.
    #[test]
    fn test_command_by_name_lookup() {
        assert!(command_by_name("HELP").is_some());
        assert!(command_by_name("AnChOr").is_some());
        assert_eq!(command_by_name("?").map(|c| c.name), Some("help"));
        assert_eq!(command_by_name("visibility").map(|c| c.name), Some("zoom"));
        assert!(command_by_name("nope").is_none());

        for name in [
            "help", "anchor", "body", "border", "cap", "color", "edge", "font", "fps", "spacing", "label",
            "mutation", "save", "open", "new", "zoom",
        ] {
            assert!(
                command_by_name(name).is_some(),
                "console verb '{name}' missing from registry"
            );
        }
    }
}
