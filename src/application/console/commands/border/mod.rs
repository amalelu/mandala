// SPDX-License-Identifier: MPL-2.0

//! `border` — configure a node's glyph border.
//!
//! Selection-aware (per `font` / `color`): operates on the current
//! [`crate::application::document::SelectionState::Single`] /
//! [`crate::application::document::SelectionState::Multi`].
//! Edge-adjacent selections surface a "not applicable to `<kind>`"
//! message — borders are node-only.
//!
//! ## Verbs
//!
//! - `border on` / `border off` — flip `style.show_frame`.
//! - `border show` — multi-line readout of the resolved config.
//! - `border reset` — drop the per-node override.
//! - kv form: `preset=`, `font=`, `size=`, `color=`, `palette=`,
//!   `field=`, `padding=`, `top=`, `bottom=`, `left=`, `right=`,
//!   `tl=`, `tr=`, `bl=`, `br=`. Multiple kvs compose in a single
//!   atomic edit, so `border on preset=heavy size=12 palette=coral`
//!   is one call.
//!
//! See `format/border-patterns.md` for the side-pattern grammar.

use baumhard::mindmap::border::BORDER_PRESETS;

use super::Command;
use crate::application::console::predicates::node_or_section_selected;

mod execute;
mod finish;
pub(crate) mod grammar;
mod positional;
mod preview;
mod show;

#[cfg(test)]
mod tests;

pub use execute::execute_border;
pub(crate) use execute::{
    apply_border_field_to_selection, cycle_border_preset_on_selection, prepend_line,
    toggle_border_visible_on_selection,
};
// Re-exported for the `section frame preview …` and
// `canvas border preview …` / `canvas section-frame [focused]
// preview …` verbs. Each verb's `preview` arm wraps
// `dispatch_border_preview` with a target-resolver closure;
// commit / cancel terminator paths route through that helper
// too. The other three preview symbols
// (`cancel_border_preview_verb`, `commit_border_preview_verb`,
// `stage_kv_for_preview`) are private to `border::preview` —
// no downstream consumer reaches in.
pub(crate) use preview::dispatch_border_preview;
// Re-exports consumed by sibling levels that share the kv
// vocabulary (`section frame …` and `canvas …`). The duplication
// these replaced — three copies each of the hint table,
// `edits_has_glyph_field` and `custom_preset_hint` — violated
// `CODE_CONVENTIONS.md` §5; the hint table itself is now a column
// of `grammar::KEYS` rather than a function, and
// `custom_preset_hint` is reached only through [`BorderEdit`],
// which is the only thing that still emits it.
pub(crate) use execute::{edits_has_glyph_field, nodes_in_selection, stage_kv};
// The closing move all four of them make — see `finish.rs` for
// what the four copies of it had in common and where they differed.
pub(crate) use finish::BorderEdit;
// The positional subverb grammar (`preset` / `color` / `padding` /
// `palette` / `font` / `side` / `corner`) is surface-agnostic:
// `canvas border …`, `canvas section-frame [focused] …` and
// `section frame …` all name the same table rather than
// transcribing it.
pub(crate) use positional::{positional_subverb_to_edits, BorderSurface};

/// Border preset names — the vocabulary `stage_preset` and the
/// positional `preset` subverb validate against.
pub const PRESETS: &[&str] = BORDER_PRESETS;

pub(crate) use grammar::{FIELDS, KEY_NAMES as KEYS};

pub const COMMAND: Command = Command {
    name: "border",
    aliases: &[],
    summary: "Configure the node border (preset, font, color, custom glyphs, palette)",
    //borders are node-only, so the verb hides on
    // edge / edge-label / portal selections in completion +
    // help. Pre-fix the predicate was `always` which surfaced
    // the verb and then errored at execute-time on the wrong
    // selection — wasted user time. The predicate matches the
    // `section` verb's surface (every section sits inside a
    // node, so a section selection implies a node selection).
    applicable: node_or_section_selected,
    grammar: &grammar::BORDER,
    // Every structural word — the thirteen subverbs, the sixteen
    // keys — is derived from the grammar. These are the search
    // words that are neither: the preset names a user might grep
    // for, and the three nouns the verb is about.
    synonyms: &[
        "frame", "glyph", "pattern", "rounded", "heavy", "double", "light", "custom",
    ],
    execute: execute_border,
};
