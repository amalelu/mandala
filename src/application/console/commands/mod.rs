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

pub mod alias;
pub mod anchor;
pub mod body;
pub mod cap;
pub mod color;
pub mod connection;
pub mod edge;
pub mod font;
pub mod help;
pub mod label;
pub mod mutate;
pub mod portal;
pub mod select;
pub mod spacing;

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
    /// Extra search tokens, folded into the help fuzzy-search
    /// haystack. Lets users find `color pick` by typing "wheel".
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
    select::COMMAND,
    anchor::COMMAND,
    body::COMMAND,
    cap::COMMAND,
    color::COMMAND,
    connection::COMMAND,
    edge::COMMAND,
    font::COMMAND,
    spacing::COMMAND,
    label::COMMAND,
    portal::COMMAND,
    mutate::COMMAND,
    alias::COMMAND,
];

/// Look up a command by its name or any alias. Case-insensitive.
pub fn command_by_name(name: &str) -> Option<&'static Command> {
    let lower = name.to_ascii_lowercase();
    COMMANDS.iter().find(|c| {
        c.name.eq_ignore_ascii_case(&lower)
            || c.aliases.iter().any(|a| a.eq_ignore_ascii_case(&lower))
    })
}

/// Build the fuzzy-search haystack string for a command — joins
/// name, aliases, summary, and tags for `help` / completion
/// filtering.
pub fn command_haystack(cmd: &Command) -> String {
    let mut s = String::with_capacity(64);
    s.push_str(cmd.name);
    for a in cmd.aliases {
        s.push(' ');
        s.push_str(a);
    }
    s.push(' ');
    s.push_str(cmd.summary);
    for t in cmd.tags {
        s.push(' ');
        s.push_str(t);
    }
    s
}

/// Backward-compat mapping from every former `PaletteAction.id` to
/// the equivalent console invocation. Enables a table-driven test
/// that ensures every palette action still has a reachable
/// console path. Kept in-module so commands::* can stay private to
/// the console surface.
pub const BACKCOMPAT_INVOCATIONS: &[(&str, &str)] = &[
    // Edge connection reset
    ("reset_edge_to_straight", "connection reset-straight"),
    ("edge_reset_style", "connection reset-style"),
    // Anchors
    ("edge_set_anchor_from_auto", "anchor set from auto"),
    ("edge_set_anchor_from_top", "anchor set from top"),
    ("edge_set_anchor_from_right", "anchor set from right"),
    ("edge_set_anchor_from_bottom", "anchor set from bottom"),
    ("edge_set_anchor_from_left", "anchor set from left"),
    ("edge_set_anchor_to_auto", "anchor set to auto"),
    ("edge_set_anchor_to_top", "anchor set to top"),
    ("edge_set_anchor_to_right", "anchor set to right"),
    ("edge_set_anchor_to_bottom", "anchor set to bottom"),
    ("edge_set_anchor_to_left", "anchor set to left"),
    // Body glyphs
    ("edge_set_body_dot", "body dot"),
    ("edge_set_body_dash", "body dash"),
    ("edge_set_body_double", "body double"),
    ("edge_set_body_wave", "body wave"),
    ("edge_set_body_chain", "body chain"),
    // Caps
    ("edge_set_cap_start_arrow", "cap from arrow"),
    ("edge_set_cap_start_circle", "cap from circle"),
    ("edge_set_cap_start_diamond", "cap from diamond"),
    ("edge_set_cap_start_none", "cap from none"),
    ("edge_set_cap_end_arrow", "cap to arrow"),
    ("edge_set_cap_end_circle", "cap to circle"),
    ("edge_set_cap_end_diamond", "cap to diamond"),
    ("edge_set_cap_end_none", "cap to none"),
    // Color
    ("edge_pick_color", "color pick"),
    ("portal_pick_color", "color pick"),
    ("edge_color_accent", "color accent"),
    ("edge_color_edge", "color edge"),
    ("edge_color_fg", "color fg"),
    ("edge_color_reset", "color reset"),
    // Font
    ("edge_font_size_smaller", "font smaller"),
    ("edge_font_size_larger", "font larger"),
    ("edge_font_size_reset", "font reset"),
    // Spacing
    ("edge_spacing_tight", "spacing tight"),
    ("edge_spacing_normal", "spacing normal"),
    ("edge_spacing_wide", "spacing wide"),
    // Edge type
    ("edge_convert_to_cross_link", "edge type cross_link"),
    ("edge_convert_to_parent_child", "edge type parent_child"),
    // Label
    ("edge_edit_label", "label edit"),
    ("edge_clear_label", "label clear"),
    ("edge_label_position_start", "label position start"),
    ("edge_label_position_middle", "label position middle"),
    ("edge_label_position_end", "label position end"),
    // Portal
    ("portal_create", "portal create"),
    ("portal_delete", "portal delete"),
    ("portal_glyph_hexagon", "portal glyph hexagon"),
    ("portal_glyph_diamond", "portal glyph diamond"),
    ("portal_glyph_star", "portal glyph star"),
    ("portal_glyph_circle", "portal glyph circle"),
    ("portal_color_accent", "portal color accent"),
    ("portal_color_edge", "portal color edge"),
    ("portal_color_fg", "portal color fg"),
    ("portal_color_reset", "portal color reset"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_by_name_finds_help() {
        assert!(command_by_name("help").is_some());
    }

    #[test]
    fn test_command_by_name_is_case_insensitive() {
        assert!(command_by_name("HELP").is_some());
        assert!(command_by_name("AnChOr").is_some());
    }

    #[test]
    fn test_command_by_name_finds_alias() {
        // `?` is the conventional alias for help.
        assert_eq!(command_by_name("?").map(|c| c.name), Some("help"));
    }

    #[test]
    fn test_command_by_name_unknown_is_none() {
        assert!(command_by_name("nope").is_none());
    }

    #[test]
    fn test_command_registry_has_every_migrated_verb() {
        let expected = [
            "help", "select", "anchor", "body", "cap", "color",
            "connection", "edge", "font", "spacing", "label", "portal",
            "mutate", "alias",
        ];
        for name in expected {
            assert!(
                command_by_name(name).is_some(),
                "command '{name}' missing from registry"
            );
        }
    }

    #[test]
    fn test_backcompat_table_has_every_palette_action_id() {
        // Mirror palette.rs counts: 11 (6C) + 31 (6D) + 10 (6E) + 2
        // (glyph-wheel picker) + reset-style(already in 6D count) = 54.
        // One palette id maps to "color pick" twice (edge_pick_color and
        // portal_pick_color), so the table has 54 distinct entries.
        assert_eq!(
            BACKCOMPAT_INVOCATIONS.len(),
            54,
            "expected 54 palette ids in BACKCOMPAT_INVOCATIONS"
        );
    }

    #[test]
    fn test_backcompat_invocations_all_resolve_to_a_registered_command() {
        for (palette_id, invocation) in BACKCOMPAT_INVOCATIONS {
            let first_token = invocation
                .split_whitespace()
                .next()
                .expect("non-empty invocation");
            assert!(
                command_by_name(first_token).is_some(),
                "palette id '{palette_id}' maps to '{invocation}' but '{first_token}' is not a known command"
            );
        }
    }
}
