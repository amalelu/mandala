// SPDX-License-Identifier: MPL-2.0

//! Inline node text editor: state, grapheme-aware cursor helpers
//! shared with `super::single_line_edit` (native-gated, so a plain
//! code-span — an intra-doc link to it fails the wasm32 doc build).
//! Lifecycle in [`editor`].

use baumhard::util::grapheme_chad;

mod editor;

#[cfg(test)]
mod tests;

pub(in crate::application::app) use editor::{
    close_text_edit, handle_text_edit_key, open_text_edit, open_text_edit_with_close_target,
};

/// Inline multi-line text editor for a node. Entered via
/// double-click on a node (or on empty canvas, which creates a new
/// orphan and opens the editor on it). Key input is routed to
/// [`handle_text_edit_key`] before the normal keybind dispatch, so
/// Tab/Enter/etc. become literal character inserts while typing.
///
/// Commit is via click-outside-the-edited-node; Esc cancels. The
/// `buffer` is the transient in-progress text; `cursor_grapheme_pos`
/// is a grapheme-cluster offset within `buffer`. The transient edits
/// flow through Baumhard's `Mutation::AreaDelta` vocabulary applied
/// to the live tree — the model is untouched until commit.
#[derive(Debug, Clone)]
pub(in crate::application::app) enum TextEditState {
    Closed,
    Open {
        node_id: String,
        /// Index inside `MindNode.sections` identifying which
        /// section the buffer is editing. Defaults to `0` when
        /// the editor is opened from a `SelectionState::Single`
        /// (whole-node) selection, matching the migration default.
        /// A `SelectionState::Section` open carries through its
        /// `section_idx` so per-section edits target the right
        /// section across the entire edit lifecycle (open / revert
        /// / commit / per-keystroke tree apply).
        section_idx: usize,
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
        /// Shift-select anchor: where the user first held shift
        /// when extending a selection. `None` = no active sub-
        /// range selection (cursor moves are unanchored, plain
        /// keystroke). `Some(idx)` = the (anchor, cursor) pair
        /// defines a `[min, max)` grapheme range; on close, if
        /// `anchor != cursor`, the range lifts to
        /// `SelectionState::SectionRange` so per-section verbs
        /// (color, font) target only those graphemes.
        selection_anchor: Option<usize>,
        /// True when this editor was opened by the single-section
        /// short-circuit inside `apply_enter_node_edit` (the user
        /// pressed Enter on a node with one section, which flips
        /// `InteractionMode::NodeEdit` and immediately opens the
        /// editor). On close, the closer flips `interaction_mode`
        /// back to `Default` rather than leaving the user in
        /// NodeEdit on a single-section node — there's nothing
        /// else to edit there, and a stranded NodeEdit + dimming +
        /// status bar reads as a UX dead-end. Multi-section opens
        /// (where the user explicitly entered NodeEdit and then
        /// asked to edit a specific section) keep the `false` value
        /// so `close_text_edit` returns to NodeEdit for further
        /// section-picking.
        exit_to_default_on_close: bool,
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

/// Is `canvas_pos` inside `node_id`, counting overflowing sections?
///
/// Refreshes the subtree-AABB cache **before** the containment test,
/// which is the load-bearing half.
/// [`crate::application::document::point_in_node_aabb`] reads
/// `subtree_aabb()`, which returns `None` while the cache is dirty
/// (post-mutation / post-tree-rebuild) and then falls back to the
/// container-only AABB — reporting a point over an *overflowing*
/// second section as "outside". `ensure_subtree_aabbs` is O(1) on a
/// clean cache and O(arena) on the first call after a mutation;
/// either way it is cheap relative to a click handler.
///
/// Every click-outside gate in the app runs this pair, and the
/// refresh is exactly the step that gets forgotten when the pair is
/// written by hand. Callers: the text editor's click-outside-commit
/// gate on both targets (via [`release_stays_inside_edited_node`])
/// and the `NodeEdit` mode-exit gate in `event_mouse_click`.
///
/// Returns `false` when no tree is built — there is nothing to be
/// inside of.
pub(in crate::application::app) fn point_inside_node_fresh_aabb(
    node_id: &str,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    canvas_pos: glam::Vec2,
) -> bool {
    if let Some(tree) = mindmap_tree.as_mut() {
        tree.tree.ensure_subtree_aabbs();
    }
    mindmap_tree
        .as_ref()
        .map(|tree| crate::application::document::point_in_node_aabb(canvas_pos, node_id, tree))
        .unwrap_or(false)
}

/// Did a pointer release land on the node the editor has open?
///
/// `true` keeps the edit alive and consumes the release; `false`
/// means the release was outside and the caller commits through the
/// funnel. Both targets' release paths run this — it is the whole of
/// what they used to have written twice.
///
/// Returns `false` when the editor is closed — there is nothing to
/// stay inside of.
pub(in crate::application::app) fn release_stays_inside_edited_node(
    text_edit_state: &TextEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    release_canvas: glam::Vec2,
) -> bool {
    text_edit_state
        .node_id()
        .map(str::to_string)
        .map(|id| point_inside_node_fresh_aabb(&id, mindmap_tree, release_canvas))
        .unwrap_or(false)
}

/// Glyph rendered at the cursor position while a node, edge-label,
/// or portal-text editor is open. ASCII `|` (U+007C) is deliberate:
/// the earlier pick `▌` (U+258C, Left Half Block) fell through
/// cosmic-text's font fallback to a different face for connection
/// labels — the Block Elements range isn't in the edge-label body
/// font, so the block renders at the fallback face's em height,
/// which on many fonts is visibly larger than the surrounding
/// text. Users reported the stray caret as "a huge pause icon
/// replacing the last character" after backspacing a label. ASCII
/// `|` is in every text font we ship against, so the caret always
/// matches the body font's metrics.
const TEXT_EDIT_CARET: char = '|';

// Text-edit cursor helpers.
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
    cursor + grapheme_chad::insert_str_at_grapheme_counted(buffer, cursor, &ch.to_string())
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
/// `cursor` — i.e. the position just after the most recent line
/// terminator strictly before `cursor`, or 0 if no terminator
/// precedes it.
///
/// Editor-vocabulary name for the first half of
/// [`grapheme_chad::line_bounds_at`], which owns the walk
/// (`lib/baumhard/CONVENTIONS.md` §B3). Prefer calling
/// `line_bounds_at` directly when the caller wants both bounds — it
/// produces them from one walk.
pub(in crate::application::app) fn cursor_to_line_start(buffer: &str, cursor: usize) -> usize {
    grapheme_chad::line_bounds_at(buffer, cursor).0
}

/// Return the grapheme index of the end of the line containing
/// `cursor` — the position of the next line terminator at or after
/// `cursor`, or the total grapheme count if none follows.
///
/// Editor-vocabulary name for the second half of
/// [`grapheme_chad::line_bounds_at`].
pub(in crate::application::app) fn cursor_to_line_end(buffer: &str, cursor: usize) -> usize {
    grapheme_chad::line_bounds_at(buffer, cursor).1
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
    // One walk for both bounds of the current line.
    let (line_start, line_end) = grapheme_chad::line_bounds_at(buffer, cursor);
    if line_end == total {
        return cursor;
    }
    let next_line_start = line_end + 1;
    let next_line_end = cursor_to_line_end(buffer, next_line_start);
    let col = cursor - line_start;
    let next_line_len = next_line_end - next_line_start;
    next_line_start + col.min(next_line_len)
}

/// Apply a TextEdit cursor / delete primitive to the editor state.
/// Pure: only mutates the in-memory buffer + cursor + regions, no
/// renderer touches. Returns `true` when state changed (caller
/// refreshes the preview iff this returns true).
///
/// Lives here (cross-platform) rather than in
/// `app/dispatch.rs` (native-only) so the WASM build of the modal
/// handler in `editor.rs` can call it on every keystroke. Native
/// dispatch arms re-export from here via `dispatch::apply_text_edit_action`.
pub(in crate::application::app) fn apply_text_edit_action(
    action: crate::application::keybinds::Action,
    state: &mut TextEditState,
) -> bool {
    use crate::application::keybinds::Action;
    let TextEditState::Open {
        buffer,
        cursor_grapheme_pos,
        buffer_regions,
        selection_anchor,
        ..
    } = state
    else {
        return false;
    };
    let cursor = cursor_grapheme_pos;
    let before = *cursor;
    let len_before = buffer.len();
    // Classify the action against the shift-select state:
    //  - `extends_anchor` keeps the anchor live and seeds it
    //    from `before` if it wasn't set yet (the user pressed
    //    a shift-cursor key for the first time).
    //  - `clears_anchor` collapses any active range — non-shift
    //    cursor moves and any text edit (typing / delete) drop
    //    the anchor. Range-aware text editing (typing replaces
    //    the selection) is deferred; clearing the anchor on
    //    edits matches the simpler "anchor only lives across
    //    cursor-key sequences" invariant.
    let extends_anchor = matches!(
        action,
        Action::TextEditCursorLeftSelect
            | Action::TextEditCursorRightSelect
            | Action::TextEditCursorUpSelect
            | Action::TextEditCursorDownSelect
            | Action::TextEditCursorHomeSelect
            | Action::TextEditCursorEndSelect
    );
    if extends_anchor && selection_anchor.is_none() {
        *selection_anchor = Some(before);
    } else if !extends_anchor {
        *selection_anchor = None;
    }
    match action {
        Action::TextEditCursorLeft | Action::TextEditCursorLeftSelect => {
            if *cursor > 0 {
                *cursor -= 1;
            }
        }
        Action::TextEditCursorRight | Action::TextEditCursorRightSelect => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor += 1;
            }
        }
        Action::TextEditCursorUp | Action::TextEditCursorUpSelect => {
            *cursor = move_cursor_up_line(buffer, *cursor);
        }
        Action::TextEditCursorDown | Action::TextEditCursorDownSelect => {
            *cursor = move_cursor_down_line(buffer, *cursor);
        }
        Action::TextEditCursorHome | Action::TextEditCursorHomeSelect => {
            *cursor = cursor_to_line_start(buffer, *cursor);
        }
        Action::TextEditCursorEnd | Action::TextEditCursorEndSelect => {
            *cursor = cursor_to_line_end(buffer, *cursor);
        }
        Action::TextEditDeleteBack => {
            if *cursor > 0 {
                buffer_regions.shrink_regions_after(*cursor - 1, 1);
                *cursor = delete_before_cursor(buffer, *cursor);
            }
        }
        Action::TextEditDeleteForward => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                buffer_regions.shrink_regions_after(*cursor, 1);
                *cursor = delete_at_cursor(buffer, *cursor);
            }
        }
        Action::TextEditWordLeft => {
            *cursor = grapheme_chad::word_left(buffer, *cursor);
        }
        Action::TextEditWordRight => {
            *cursor = grapheme_chad::word_right(buffer, *cursor);
        }
        Action::TextEditDeleteWordBack => {
            let target = grapheme_chad::word_left(buffer, *cursor);
            while *cursor > target {
                buffer_regions.shrink_regions_after(*cursor - 1, 1);
                *cursor = delete_before_cursor(buffer, *cursor);
            }
        }
        Action::TextEditDeleteWordForward => {
            let target = grapheme_chad::word_right(buffer, *cursor);
            let total = grapheme_chad::count_grapheme_clusters(buffer);
            let to_delete = target.saturating_sub(*cursor).min(total - *cursor);
            for _ in 0..to_delete {
                buffer_regions.shrink_regions_after(*cursor, 1);
                *cursor = delete_at_cursor(buffer, *cursor);
            }
        }
        _ => {}
    }
    *cursor != before || buffer.len() != len_before
}

// `word_left` / `word_right` moved to `baumhard::util::grapheme_chad`
// (CONVENTIONS §B3 — text primitives belong in the foundation crate).
// Reach for them via `grapheme_chad::word_left` / `word_right`.

/// Build the display text for the edited node by inserting the caret
/// glyph at the cursor's grapheme position. Used on every keystroke
/// to produce the `Mutation::AreaDelta` payload.
pub(in crate::application::app) fn insert_caret(buffer: &str, cursor: usize) -> String {
    let byte = grapheme_chad::find_byte_index_of_grapheme(buffer, cursor).unwrap_or(buffer.len());
    let mut out = String::with_capacity(buffer.len() + TEXT_EDIT_CARET.len_utf8());
    out.push_str(&buffer[..byte]);
    out.push(TEXT_EDIT_CARET);
    out.push_str(&buffer[byte..]);
    out
}
