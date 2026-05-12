// SPDX-License-Identifier: MPL-2.0

//! Console command registry.
//!
//! Each command lives in its own submodule so the surface stays
//! scannable. The public `COMMANDS` slice gathers them in one place,
//! matching the `const PALETTE_ACTIONS` pattern — zero-cost startup,
//! no HashMap construction, and `action_by_id`-style lookup is a
//! linear scan over a dozen entries.

use super::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;

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
    /// Full usage line shown in `help <cmd>`. Conventionally starts
    /// with the command name: `"anchor set <from|to> <side>"`.
    pub usage: &'static str,
    /// Extra search tokens surfaced in `help --all` output so a
    /// user grepping the command list can find "pick" under
    /// `color` even though the name doesn't include it.
    pub tags: &'static [&'static str],
    /// Returns `true` when the command should appear in the filtered
    /// `help` list and in completion. Commands whose args are
    /// context-specific but whose verb is always meaningful should
    /// return `true` here and validate in `execute`.
    pub applicable: fn(&ConsoleContext) -> bool,
    /// Build completion candidates for the token currently under the
    /// cursor. Return an empty `Vec` when the command can't offer
    /// any useful completion for that position.
    pub complete: fn(&CompletionState, &ConsoleContext) -> Vec<Completion>,
    /// Run the command. The dispatcher clears the scene cache and
    /// rebuilds after every non-`Err` result.
    pub execute: fn(&Args, &mut ConsoleEffects) -> ExecResult,
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
