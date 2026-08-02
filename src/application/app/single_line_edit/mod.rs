// SPDX-License-Identifier: MPL-2.0

//! The inline single-line editor.
//!
//! One buffer, one grapheme cursor, one pre-edit snapshot, bound to
//! one editable string in the model. Two strings qualify today — a
//! connection's label and a portal endpoint's caption — and the
//! difference between them lives entirely in
//! [`target::SingleLineEditTarget`]; [`editor`] holds the lifecycle
//! and never learns which kind it is driving.
//!
//! The node text editor ([`super::text_edit`]) stays its own thing:
//! multi-line, region-styled, and previewed by mutating the live
//! tree rather than by staging a string. Only the grapheme-cursor
//! primitives are shared.
//!
//! The `keybinds.json` vocabulary keeps its `label_edit_*` spelling
//! (`Action::LabelEdit*`, `InputContext::LabelEdit`) — those are
//! user-facing binding names, and both targets have always shared
//! them.

use baumhard::util::grapheme_chad;

use crate::application::platform::input::Key;

mod editor;
mod target;

#[cfg(test)]
mod tests;

pub(in crate::application::app) use editor::{
    apply_single_line_edit_action, close_single_line_edit, handle_single_line_edit_key,
    open_single_line_edit, SingleLineEditor,
};
pub(in crate::application::app) use target::{resolve_single_line_target, SingleLineEditTarget};

/// Pure router for the *character-input* path. Inserts printable
/// chars from a `Key::Character` payload into the buffer at the
/// grapheme cursor.
///
/// Structural keys (Backspace, Delete, arrows, Home, End) are
/// deliberately not handled here: they route through
/// `Action::LabelEdit*` and
/// `dispatch::apply_label_edit_action_to_buffer` instead, so
/// unbinding `label_edit_*` in `keybinds.json` actually disables the
/// key. A fallback here would shadow user config.
///
/// Returns `true` iff a printable character was inserted.
pub(super) fn route_single_line_key(logical_key: &Key, buffer: &mut String, cursor: &mut usize) -> bool {
    if let Key::Character(c) = logical_key {
        // Control chars (and any non-printing payload winit attaches
        // to a structural key) are filtered, which mirrors the
        // original guard intent. Insert the remaining payload as one
        // counted splice so IME / dead-key multi-codepoint clusters
        // advance the grapheme cursor by their visible delta.
        let filtered: String = c.as_str().chars().filter(|ch| !ch.is_control()).collect();
        if !filtered.is_empty() {
            *cursor += grapheme_chad::insert_str_at_grapheme_counted(buffer, *cursor, &filtered);
            return true;
        }
    }
    false
}

/// Seed a single-line editor's `(buffer, cursor)` pair on open.
///
/// `clean` is `Action::EditSelectionClean`'s empty-buffer contract
/// — the single-line counterpart of `open_text_edit`'s
/// `from_creation`. The cursor always lands at the end of whatever
/// was seeded (0 on the clean path, end-of-text otherwise), and it
/// is counted in grapheme clusters so an existing label ending in
/// an emoji or ZWJ sequence puts the caret after the whole cluster.
fn seed_edit_buffer(clean: bool, original: Option<&str>) -> (String, usize) {
    let buffer = if clean {
        String::new()
    } else {
        original.unwrap_or_default().to_string()
    };
    let cursor = grapheme_chad::count_grapheme_clusters(&buffer);
    (buffer, cursor)
}
