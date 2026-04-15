//! Inline node text editor — module root.
//!
//! Holds the text-editor `TextEditState` enum + the shared cursor /
//! grapheme-aware buffer helpers that both the editor and the label
//! editor (`app::label_edit`) consume. The editor lifecycle itself
//! (open / close / apply / handle_key / revert) lives in
//! [`editor`]. The 800+ lines of cursor-math unit tests live in
//! [`tests`] (`#[cfg(test)]`).
//!
//! Pulled out of `app::mod` as part of the text_edit submodule
//! consolidation — `TextEditState`, the cursor helpers, and the
//! cursor-math tests no longer bloat the event-loop file.

use baumhard::util::grapheme_chad;

mod editor;

#[cfg(test)]
mod tests;

pub(in crate::application::app) use editor::{
    apply_text_and_regions_delta, apply_text_edit_to_tree, close_text_edit, handle_text_edit_key,
    open_text_edit, read_node_regions, read_node_text, revert_node_text_on_tree,
};

/// Session 7A: inline multi-line text editor for a node. Entered via
/// double-click on a node (or on empty canvas, which creates a new
/// orphan and opens the editor on it). Key input is routed to
/// `handle_text_edit_key` before the normal keybind dispatch, so
/// Tab/Enter/etc. become literal character inserts while typing.
///
/// Commit is via click-outside-the-edited-node; Esc cancels. The
/// `buffer` is the transient in-progress text; `cursor_char_pos` is a
/// char-index offset within `buffer`. The transient edits flow
/// through Baumhard's `Mutation::AreaDelta` vocabulary applied to the
/// live tree — the model is untouched until commit.
#[derive(Debug, Clone)]
pub(in crate::application::app) enum TextEditState {
    Closed,
    Open {
        node_id: String,
        /// The in-progress multi-line buffer.
        buffer: String,
        /// Cursor position as a grapheme-cluster index into `buffer`.
        /// Valid range `[0, count_grapheme_clusters(buffer)]`. Stored
        /// in graphemes (not chars or bytes) so backspace over an
        /// emoji or ZWJ cluster removes the whole user-visible
        /// character — see `CODE_CONVENTIONS.md §2`/`§B2`.
        cursor_grapheme_pos: usize,
        /// Char-range `ColorFontRegions` over `buffer` (no caret
        /// coverage). Seeded from the node's `GlyphArea::regions` at
        /// open time — which itself came from the model's
        /// `text_runs` via the tree builder — and mutated alongside
        /// `buffer` on every keystroke via Baumhard's
        /// `shift_regions_after` / `shrink_regions_after` primitives
        /// so per-run color and `AppFont` pins survive character
        /// insertion and deletion. `apply_text_edit_to_tree` composes
        /// display-text regions from this by inserting caret coverage
        /// at `cursor_grapheme_pos`.
        buffer_regions: baumhard::core::primitives::ColorFontRegions,
        /// Snapshot of the tree's `GlyphArea::text` at open time,
        /// before any caret or typing mutations landed. On cancel
        /// we apply these back via a `DeltaGlyphArea` so the tree
        /// returns to its pre-edit state without going through the
        /// full `doc.build_tree()` + scene rebuild that `rebuild_all`
        /// would trigger. The model is untouched during editing, so
        /// the snapshot stays valid for the whole session.
        original_text: String,
        /// Snapshot of the tree's `GlyphArea::regions` at open time.
        /// Pairs with `original_text` — together they let cancel
        /// restore the exact pre-edit tree state (including any
        /// selection-highlight regions that the last `rebuild_all`
        /// stamped into the node).
        original_regions: baumhard::core::primitives::ColorFontRegions,
    },
}

impl TextEditState {
    pub(in crate::application::app) fn is_open(&self) -> bool {
        matches!(self, TextEditState::Open { .. })
    }
    pub(in crate::application::app) fn node_id(&self) -> Option<&str> {
        match self {
            TextEditState::Open { node_id, .. } => Some(node_id.as_str()),
            TextEditState::Closed => None,
        }
    }
}

/// Session 7A: glyph rendered at the cursor position while a node
/// text editor is open. Reuses the same caret as `LabelEditState`.
const TEXT_EDIT_CARET: char = '\u{258C}';


// Session 7A text-edit cursor helpers.
//
// These all operate on **grapheme-cluster indices** (not chars or
// bytes), routing through `baumhard::util::grapheme_chad`. This is
// what `CODE_CONVENTIONS.md §2` and `§B2` mandate for any code that
// touches user-typed text — char indexing splits emoji and combining
// marks mid-cluster, leaving a corrupted buffer the next time the
// renderer shapes it.
//
// For ASCII-only buffers grapheme indices coincide with char indices,
// which is why the existing test suite still passes unchanged.

/// Insert one character at grapheme index `cursor` in `buffer`,
/// returning the new cursor position (one grapheme past the insert).
pub(in crate::application::app) fn insert_at_cursor(buffer: &mut String, cursor: usize, ch: char) -> usize {
    grapheme_chad::insert_str_at_grapheme(buffer, cursor, &ch.to_string());
    cursor + 1
}

/// Delete the grapheme cluster immediately before `cursor` (Backspace
/// semantics). Returns the new cursor position. No-op at `cursor == 0`.
pub(in crate::application::app) fn delete_before_cursor(buffer: &mut String, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    grapheme_chad::delete_grapheme_at(buffer, cursor - 1);
    cursor - 1
}

/// Delete the grapheme cluster at `cursor` (Delete semantics). Returns
/// the unchanged cursor position. No-op at end of buffer.
pub(in crate::application::app) fn delete_at_cursor(buffer: &mut String, cursor: usize) -> usize {
    let total = grapheme_chad::count_grapheme_clusters(buffer);
    if cursor >= total {
        return cursor;
    }
    grapheme_chad::delete_grapheme_at(buffer, cursor);
    cursor
}

/// Return the grapheme index of the start of the line containing
/// `cursor` — i.e. the position just after the most recent `\n`
/// strictly before `cursor`, or 0 if no prior `\n`. `\n` is always its
/// own grapheme cluster, so walking by graphemes is correct here.
pub(in crate::application::app) fn cursor_to_line_start(buffer: &str, cursor: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    let mut line_start = 0usize;
    for (i, g) in buffer.graphemes(true).enumerate() {
        if i >= cursor {
            break;
        }
        if g == "\n" {
            line_start = i + 1;
        }
    }
    line_start
}

/// Return the grapheme index of the end of the line containing
/// `cursor` — the position of the next `\n` at or after `cursor`, or
/// the total grapheme count if no `\n` follows.
pub(in crate::application::app) fn cursor_to_line_end(buffer: &str, cursor: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    let mut total = 0usize;
    for (i, g) in buffer.graphemes(true).enumerate() {
        total = i + 1;
        if i >= cursor && g == "\n" {
            return i;
        }
    }
    total
}

/// Move the cursor up one line, preserving the visual column. Column
/// is computed as `cursor - line_start` in graphemes; the new
/// position lands at `prev_line_start + min(col, prev_line_len)`.
/// No-op if already on the first line.
pub(in crate::application::app) fn move_cursor_up_line(buffer: &str, cursor: usize) -> usize {
    let line_start = cursor_to_line_start(buffer, cursor);
    if line_start == 0 {
        return cursor;
    }
    // Move to the grapheme just before the '\n' that terminates the previous line.
    let prev_line_end = line_start - 1;
    let prev_line_start = cursor_to_line_start(buffer, prev_line_end);
    let col = cursor - line_start;
    let prev_line_len = prev_line_end - prev_line_start;
    prev_line_start + col.min(prev_line_len)
}

/// Move the cursor down one line, preserving the visual column.
/// No-op if already on the last line.
pub(in crate::application::app) fn move_cursor_down_line(buffer: &str, cursor: usize) -> usize {
    let total = grapheme_chad::count_grapheme_clusters(buffer);
    let line_start = cursor_to_line_start(buffer, cursor);
    let line_end = cursor_to_line_end(buffer, cursor);
    if line_end == total {
        return cursor;
    }
    let next_line_start = line_end + 1;
    let next_line_end = cursor_to_line_end(buffer, next_line_start);
    let col = cursor - line_start;
    let next_line_len = next_line_end - next_line_start;
    next_line_start + col.min(next_line_len)
}

/// Build the display text for the edited node by inserting the caret
/// glyph at the cursor's grapheme position. Used on every keystroke
/// to produce the `Mutation::AreaDelta` payload.
pub(in crate::application::app) fn insert_caret(buffer: &str, cursor: usize) -> String {
    let byte = grapheme_chad::find_byte_index_of_grapheme(buffer, cursor)
        .unwrap_or(buffer.len());
    let mut out = String::with_capacity(buffer.len() + TEXT_EDIT_CARET.len_utf8());
    out.push_str(&buffer[..byte]);
    out.push(TEXT_EDIT_CARET);
    out.push_str(&buffer[byte..]);
    out
}
