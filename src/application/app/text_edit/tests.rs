// SPDX-License-Identifier: MPL-2.0

//! Unit tests for the text-edit cursor / grapheme / caret helpers
//! defined in the parent module, plus `Mutation::AreaDelta`
//! round-trips that pin the editor-to-baumhard contract.

use super::*;
use glam::Vec2;

// -----------------------------------------------------------------
// Cursor math
// -----------------------------------------------------------------

#[test]
fn test_insert_at_cursor_start() {
    let mut s = String::from("bcd");
    let cursor = insert_at_cursor(&mut s, 0, 'a');
    assert_eq!(s, "abcd");
    assert_eq!(cursor, 1);
}

#[test]
fn test_insert_at_cursor_middle() {
    let mut s = String::from("abd");
    let cursor = insert_at_cursor(&mut s, 2, 'c');
    assert_eq!(s, "abcd");
    assert_eq!(cursor, 3);
}

#[test]
fn test_insert_at_cursor_end() {
    let mut s = String::from("abc");
    let cursor = insert_at_cursor(&mut s, 3, 'd');
    assert_eq!(s, "abcd");
    assert_eq!(cursor, 4);
}

#[test]
fn test_insert_at_cursor_newline() {
    let mut s = String::from("abcd");
    let cursor = insert_at_cursor(&mut s, 2, '\n');
    assert_eq!(s, "ab\ncd");
    assert_eq!(cursor, 3);
}

#[test]
fn test_delete_before_cursor_at_start_noop() {
    let mut s = String::from("abc");
    let cursor = delete_before_cursor(&mut s, 0);
    assert_eq!(s, "abc");
    assert_eq!(cursor, 0);
}

#[test]
fn test_delete_before_cursor_middle() {
    let mut s = String::from("abcd");
    let cursor = delete_before_cursor(&mut s, 2);
    assert_eq!(s, "acd");
    assert_eq!(cursor, 1);
}

#[test]
fn test_delete_at_cursor_end_noop() {
    let mut s = String::from("abc");
    let cursor = delete_at_cursor(&mut s, 3);
    assert_eq!(s, "abc");
    assert_eq!(cursor, 3);
}

#[test]
fn test_delete_at_cursor_middle() {
    let mut s = String::from("abcd");
    let cursor = delete_at_cursor(&mut s, 1);
    assert_eq!(s, "acd");
    assert_eq!(cursor, 1);
}

#[test]
fn test_cursor_to_line_start_single_line() {
    assert_eq!(cursor_to_line_start("abc", 2), 0);
}

#[test]
fn test_cursor_to_line_start_multiline() {
    let s = "ab\ncd\nef";
    // cursor on 'd' (index 4): line starts at 3
    assert_eq!(cursor_to_line_start(s, 4), 3);
    // cursor on 'f' (index 7): line starts at 6
    assert_eq!(cursor_to_line_start(s, 7), 6);
}

#[test]
fn test_cursor_to_line_end_multiline() {
    let s = "ab\ncd\nef";
    // cursor on 'a' (index 0): end at '\n' position (2)
    assert_eq!(cursor_to_line_end(s, 0), 2);
    // cursor on 'e' (index 6): end at buffer end (8)
    assert_eq!(cursor_to_line_end(s, 6), 8);
}

#[test]
fn test_move_cursor_up_line_preserves_column() {
    let s = "abcd\nwxyz";
    // cursor on 'y' (index 7, col 2 on line 1): up → 'c' (index 2)
    assert_eq!(move_cursor_up_line(s, 7), 2);
}

#[test]
fn test_move_cursor_up_line_short_prev_line() {
    let s = "ab\nwxyz";
    // cursor on 'z' (index 6, col 3 on line 1): up → end of "ab" (index 2)
    assert_eq!(move_cursor_up_line(s, 6), 2);
}

#[test]
fn test_move_cursor_up_line_first_line_is_noop() {
    assert_eq!(move_cursor_up_line("abc", 1), 1);
}

#[test]
fn test_move_cursor_down_line_preserves_column() {
    let s = "abcd\nwxyz";
    // cursor on 'c' (index 2): down → 'y' (index 7)
    assert_eq!(move_cursor_down_line(s, 2), 7);
}

#[test]
fn test_move_cursor_down_line_last_line_is_noop() {
    let s = "ab\ncd";
    assert_eq!(move_cursor_down_line(s, 4), 4);
}

// -----------------------------------------------------------------
// Caret insertion
// -----------------------------------------------------------------

#[test]
fn test_insert_caret_middle() {
    let out = insert_caret("abcd", 2);
    assert_eq!(out, "ab|cd");
}

#[test]
fn test_insert_caret_end() {
    let out = insert_caret("abc", 3);
    assert_eq!(out, "abc|");
}

#[test]
fn test_insert_caret_empty() {
    let out = insert_caret("", 0);
    assert_eq!(out, "|");
}

// -----------------------------------------------------------------
// Baumhard Mutation round-trip: constructing and applying a
// `Mutation::AreaDelta` with `GlyphAreaField::Text + Assign`
// mutates the target GlyphArea's text in place. This verifies we
// really are flowing text edits through Baumhard's existing
// vocabulary instead of patching around it.
// -----------------------------------------------------------------

#[test]
fn test_text_edit_mutation_assigns_via_baumhard() {
    use baumhard::core::primitives::{Applicable, ApplyOperation};
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaField};

    let mut area =
        GlyphArea::new_with_str("initial", 14.0, 16.8, Vec2::new(0.0, 0.0), Vec2::new(100.0, 30.0));
    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Text("updated".to_string()),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    delta.apply_to(&mut area);
    assert_eq!(area.text, "updated");
}

#[test]
fn test_text_edit_mutation_with_caret_glyph_via_baumhard() {
    use baumhard::core::primitives::{Applicable, ApplyOperation};
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaField};

    let mut area = GlyphArea::new_with_str("", 14.0, 16.8, Vec2::new(0.0, 0.0), Vec2::new(100.0, 30.0));
    let buffer = "hello world";
    let cursor = 5;
    let display_text = insert_caret(buffer, cursor);
    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Text(display_text.clone()),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    delta.apply_to(&mut area);
    // Caret after "hello", before " world".
    assert_eq!(area.text, "hello| world");
    assert_eq!(area.text, display_text);
}

/// A keystroke insertion in the middle of a multi-run node must
/// preserve run identity: per-run colors and `AppFont` pins
/// survive, and the caret lands inside one of the expanded runs
/// rather than collapsing the set to a single span. Regression
/// test for the glyph-alignment session's Issue 2 — the old
/// path discarded regions and inherited only the first region's
/// color, wiping pins on emoji / Tibetan / Egyptian hieroglyph
/// runs on the first keystroke.
#[test]
fn test_text_edit_preserves_multi_run_regions_on_insertion() {
    use baumhard::core::primitives::{Applicable, ApplyOperation, ColorFontRegion, ColorFontRegions, Range};
    use baumhard::font::fonts::AppFont;
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaField};

    // Two-run buffer: "Helmo" → [0..3) red (plain font), [3..5)
    // blue pinned to `NotoSerifTibetanRegular` (stand-in for the
    // per-script `AppFont` pin that a sacred-script run carries).
    let red = [1.0f32, 0.0, 0.0, 1.0];
    let blue = [0.0f32, 0.0, 1.0, 1.0];
    let mut buffer_regions = ColorFontRegions::new_empty();
    buffer_regions.submit_region(ColorFontRegion::new(Range::new(0, 3), None, Some(red)));
    buffer_regions.submit_region(ColorFontRegion::new(
        Range::new(3, 5),
        Some(AppFont::NotoSerifTibetanRegular),
        Some(blue),
    ));

    // User inserts 'X' at cursor=4 (inside the blue run, between
    // the two existing chars). `insert_regions_at` on the buffer
    // regions extends the straddling run's end by 1.
    buffer_regions.insert_regions_at(4, 1);

    // Compose display regions by inserting caret coverage at the
    // new cursor=5 — exactly what `apply_text_edit_to_tree` does.
    let mut display_regions = buffer_regions.clone();
    let absorbed = display_regions.insert_regions_at(5, 1);
    assert!(absorbed, "caret must be absorbed into the trailing run");

    // Apply the delta to a mock area the same way the production
    // path does.
    let mut area = GlyphArea::new_with_str(
        "Helmo|", // placeholder, will be overwritten
        14.0,
        16.8,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 30.0),
    );
    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Text("HelXmo|".to_string()),
        GlyphAreaField::ColorFontRegions(display_regions),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    delta.apply_to(&mut area);

    // Two regions survive; colors are intact; the `AppFont` pin
    // survives; the caret is covered.
    assert_eq!(area.regions.num_regions(), 2);
    let red_run = area.regions.get(Range::new(0, 3)).unwrap();
    assert_eq!(red_run.color, Some(red));
    assert_eq!(red_run.font, None);
    let blue_run = area.regions.get(Range::new(3, 7)).unwrap();
    assert_eq!(blue_run.color, Some(blue));
    assert_eq!(blue_run.font, Some(AppFont::NotoSerifTibetanRegular));
}

/// Backspace inside a multi-run node shrinks the containing run
/// without bleeding the neighbour run's color in. Exercises the
/// new `shrink_regions_after` primitive through the text-edit
/// path's delete handler contract.
#[test]
fn test_text_edit_preserves_multi_run_regions_on_deletion() {
    use baumhard::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
    use baumhard::font::fonts::AppFont;

    let red = [1.0f32, 0.0, 0.0, 1.0];
    let blue = [0.0f32, 0.0, 1.0, 1.0];
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(Range::new(0, 3), None, Some(red)));
    regions.submit_region(ColorFontRegion::new(
        Range::new(3, 6),
        Some(AppFont::NotoSerifTibetanRegular),
        Some(blue),
    ));

    // Backspace at cursor=5 deletes the char at position 4 (inside
    // the blue run). `shrink_regions_after(4, 1)` clips the blue
    // run's end to 5 — the red run is untouched and the
    // `AppFont` pin survives.
    regions.shrink_regions_after(4, 1);

    assert_eq!(regions.num_regions(), 2);
    let red_run = regions.get(Range::new(0, 3)).unwrap();
    assert_eq!(red_run.color, Some(red));
    let blue_run = regions.get(Range::new(3, 5)).unwrap();
    assert_eq!(blue_run.color, Some(blue));
    assert_eq!(blue_run.font, Some(AppFont::NotoSerifTibetanRegular));
}

// -----------------------------------------------------------------
// TextEditState shape + guard semantics
// -----------------------------------------------------------------

#[test]
fn test_text_edit_state_node_id_round_trip() {
    let closed = TextEditState::Closed;
    assert!(closed.node_id().is_none());
    assert!(!closed.is_open());

    let open = TextEditState::Open {
        node_id: "n-42".to_string(),
        section_idx: 0,
        buffer: "hi".to_string(),
        cursor_grapheme_pos: 2,
        buffer_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        original_text: String::new(),
        original_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        selection_anchor: None,
        exit_to_default_on_close: false,
    };
    assert_eq!(open.node_id(), Some("n-42"));
    assert!(open.is_open());
}

#[test]
fn test_text_edit_state_is_open_closed_variant() {
    assert!(!TextEditState::Closed.is_open());
}

// -----------------------------------------------------------------
// Cursor helpers: boundary cases
// -----------------------------------------------------------------

#[test]
fn test_cursor_to_line_start_trailing_newline() {
    // Cursor positioned just after a trailing '\n' (on an empty
    // final line). Line start should be the char index right
    // after the '\n', i.e. the cursor itself.
    let s = "abc\n";
    assert_eq!(cursor_to_line_start(s, 4), 4);
}

#[test]
fn test_cursor_to_line_start_at_zero() {
    assert_eq!(cursor_to_line_start("anything", 0), 0);
}

#[test]
fn test_cursor_to_line_start_empty_buffer() {
    assert_eq!(cursor_to_line_start("", 0), 0);
}

#[test]
fn test_cursor_to_line_end_empty_buffer() {
    assert_eq!(cursor_to_line_end("", 0), 0);
}

#[test]
fn test_cursor_to_line_end_cursor_exactly_at_newline() {
    // Cursor is at the '\n' position; line end IS that position.
    let s = "ab\ncd";
    assert_eq!(cursor_to_line_end(s, 2), 2);
}

#[test]
fn test_cursor_to_line_end_walks_past_cursor() {
    // Cursor in the middle of a line, next '\n' several chars ahead.
    let s = "alpha beta\ngamma";
    // Cursor on 'p' (index 2): line_end should be at '\n' (index 10).
    assert_eq!(cursor_to_line_end(s, 2), 10);
}

// -----------------------------------------------------------------
// insert_caret / insert_at_cursor with multi-byte chars
// -----------------------------------------------------------------

#[test]
fn test_insert_caret_with_multibyte_prefix() {
    // 'é' is a 2-byte UTF-8 char. insert_caret must not split it.
    let out = insert_caret("café", 3);
    // "caf" + caret + "é"
    assert_eq!(out, "caf|é");
}

#[test]
fn test_insert_at_cursor_with_multibyte_buffer() {
    let mut s = String::from("café");
    // Insert 'x' between 'f' and 'é' (char pos 3).
    let new_cursor = insert_at_cursor(&mut s, 3, 'x');
    assert_eq!(s, "cafxé");
    assert_eq!(new_cursor, 4);
}

#[test]
fn test_delete_before_cursor_with_multibyte() {
    let mut s = String::from("café");
    // Delete the 'é' (grapheme pos 3, cursor at 4).
    let new_cursor = delete_before_cursor(&mut s, 4);
    assert_eq!(s, "caf");
    assert_eq!(new_cursor, 3);
}

// -----------------------------------------------------------------
// Grapheme-cluster regression tests
// -----------------------------------------------------------------
//
// These guard the rule that a single Backspace/Delete removes a
// whole grapheme cluster, not a Unicode scalar. An earlier char-
// indexed implementation would corrupt emoji and ZWJ sequences
// mid-cluster on the first Backspace.

#[test]
fn test_cursor_edit_with_emoji_backspace() {
    // 🍕 is a single grapheme but two `char`s (it's a single
    // codepoint above U+FFFF, encoded as a surrogate pair in
    // UTF-16; in UTF-8 it's 4 bytes / 1 char).
    let mut s = String::from("ab🍕cd");
    // Cursor sits just after the pizza (grapheme index 3).
    let new_cursor = delete_before_cursor(&mut s, 3);
    // The whole pizza is gone, not just half of it.
    assert_eq!(s, "abcd");
    assert_eq!(new_cursor, 2);
}

#[test]
fn test_cursor_edit_with_zwj_backspace() {
    // 🧑‍🚀 is a ZWJ sequence: 🧑 + ZWJ + 🚀, three codepoints
    // and five chars, but a single user-visible grapheme cluster.
    // Backspace must remove the whole thing in one keystroke.
    let mut s = String::from("hi🧑\u{200D}🚀!");
    let new_cursor = delete_before_cursor(&mut s, 3);
    assert_eq!(s, "hi!");
    assert_eq!(new_cursor, 2);
}

#[test]
fn test_cursor_edit_with_emoji_delete_forward() {
    // Delete (forward delete) at the position before the pizza
    // removes the whole cluster.
    let mut s = String::from("ab🍕cd");
    let new_cursor = delete_at_cursor(&mut s, 2);
    assert_eq!(s, "abcd");
    // Forward delete leaves the cursor in place.
    assert_eq!(new_cursor, 2);
}

#[test]
fn test_insert_caret_after_emoji() {
    // Caret rendered after a pizza emoji should not split it.
    let out = insert_caret("ab🍕cd", 3);
    assert_eq!(out, "ab🍕|cd");
}

// `word_left` / `word_right` unit tests moved to baumhard alongside
// the primitives — see `lib/baumhard/src/util/tests/grapheme_chad_tests.rs`
// `do_word_left` / `do_word_right`. The `apply_text_edit_action` integration
// tests below still exercise the dispatcher path through `Action::TextEditWordLeft`
// / `TextEditWordRight`.

// -----------------------------------------------------------------
// `apply_text_edit_action` — pure dispatcher, called from every
// TextEdit keystroke. Cross-platform per the §2 module-boundary
// gating; tests run on native (cfg(test)).
// -----------------------------------------------------------------

fn make_open_state(buffer: &str, cursor: usize) -> TextEditState {
    TextEditState::Open {
        node_id: "n-test".to_string(),
        section_idx: 0,
        buffer: buffer.to_string(),
        cursor_grapheme_pos: cursor,
        buffer_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        original_text: String::new(),
        original_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        selection_anchor: None,
        exit_to_default_on_close: false,
    }
}

fn cursor_of(state: &TextEditState) -> usize {
    match state {
        TextEditState::Open {
            cursor_grapheme_pos, ..
        } => *cursor_grapheme_pos,
        TextEditState::Closed => panic!("expected Open"),
    }
}

fn buffer_of(state: &TextEditState) -> &str {
    match state {
        TextEditState::Open { buffer, .. } => buffer,
        TextEditState::Closed => panic!("expected Open"),
    }
}

#[test]
fn test_apply_text_edit_cursor_left() {
    use crate::application::keybinds::Action;
    let mut s = make_open_state("hello", 3);
    let changed = apply_text_edit_action(Action::TextEditCursorLeft, &mut s);
    assert!(changed);
    assert_eq!(cursor_of(&s), 2);
}

#[test]
fn test_apply_text_edit_cursor_left_at_start_is_noop() {
    use crate::application::keybinds::Action;
    let mut s = make_open_state("hello", 0);
    let changed = apply_text_edit_action(Action::TextEditCursorLeft, &mut s);
    assert!(!changed);
    assert_eq!(cursor_of(&s), 0);
}

#[test]
fn test_apply_text_edit_cursor_right_at_end_is_noop() {
    use crate::application::keybinds::Action;
    let mut s = make_open_state("hi", 2);
    let changed = apply_text_edit_action(Action::TextEditCursorRight, &mut s);
    assert!(!changed);
    assert_eq!(cursor_of(&s), 2);
}

#[test]
fn test_apply_text_edit_word_left() {
    use crate::application::keybinds::Action;
    let mut s = make_open_state("foo bar", 7);
    let changed = apply_text_edit_action(Action::TextEditWordLeft, &mut s);
    assert!(changed);
    assert_eq!(cursor_of(&s), 4);
}

#[test]
fn test_apply_text_edit_word_right() {
    use crate::application::keybinds::Action;
    let mut s = make_open_state("foo bar", 0);
    let changed = apply_text_edit_action(Action::TextEditWordRight, &mut s);
    assert!(changed);
    assert_eq!(cursor_of(&s), 3);
}

#[test]
fn test_apply_text_edit_delete_back() {
    use crate::application::keybinds::Action;
    let mut s = make_open_state("abcd", 3);
    let changed = apply_text_edit_action(Action::TextEditDeleteBack, &mut s);
    assert!(changed);
    assert_eq!(buffer_of(&s), "abd");
    assert_eq!(cursor_of(&s), 2);
}

#[test]
fn test_apply_text_edit_delete_word_back() {
    use crate::application::keybinds::Action;
    let mut s = make_open_state("foo bar", 7);
    let changed = apply_text_edit_action(Action::TextEditDeleteWordBack, &mut s);
    assert!(changed);
    assert_eq!(buffer_of(&s), "foo ");
    assert_eq!(cursor_of(&s), 4);
}

#[test]
fn test_apply_text_edit_delete_word_forward() {
    use crate::application::keybinds::Action;
    let mut s = make_open_state("foo bar", 4);
    let changed = apply_text_edit_action(Action::TextEditDeleteWordForward, &mut s);
    assert!(changed);
    assert_eq!(buffer_of(&s), "foo ");
    assert_eq!(cursor_of(&s), 4);
}

#[test]
fn test_apply_text_edit_on_closed_state_returns_false() {
    use crate::application::keybinds::Action;
    let mut s = TextEditState::Closed;
    let changed = apply_text_edit_action(Action::TextEditCursorLeft, &mut s);
    assert!(!changed);
}

#[test]
fn test_apply_text_edit_unrelated_action_is_noop() {
    use crate::application::keybinds::Action;
    let mut s = make_open_state("hello", 3);
    // `Action::Undo` doesn't belong to TextEdit context — the
    // function silently returns without mutating state.
    let changed = apply_text_edit_action(Action::Undo, &mut s);
    assert!(!changed);
    assert_eq!(buffer_of(&s), "hello");
    assert_eq!(cursor_of(&s), 3);
}

#[test]
fn test_apply_text_edit_cursor_home_end() {
    use crate::application::keybinds::Action;
    let mut s = make_open_state("ab\ncd\nef", 4);
    let _ = apply_text_edit_action(Action::TextEditCursorHome, &mut s);
    assert_eq!(cursor_of(&s), 3); // start of "cd"
    let _ = apply_text_edit_action(Action::TextEditCursorEnd, &mut s);
    assert_eq!(cursor_of(&s), 5); // end of "cd"
}

// -----------------------------------------------------------------
// §T1 Unicode-edge tests on the keystroke path
// -----------------------------------------------------------------
//
// `insert_caret` and `insert_at_cursor` operate on grapheme-cluster
// indices. Pre-Tier-D, the codepoint vs grapheme cluster
// distinction was tested at the model layer only; the keystroke
// hot path that produces the `Mutation::AreaDelta` payload was
// untested. A regression here would silently drop the caret or
// truncate emoji on every keystroke into a multi-section node.

/// A ZWJ-joined emoji family is one grapheme cluster, five
/// codepoints. Inserting the caret one cluster *into* that
/// family lands the caret AFTER the whole cluster, not in the
/// middle of the codepoint sequence.
#[test]
fn test_insert_caret_after_zwj_emoji_family() {
    let zwj = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let combined = format!("{zwj}A");
    // After cluster 1 (the family) — the caret lands between
    // the family and 'A'.
    let with_caret = insert_caret(&combined, 1);
    let zwj_byte_len = zwj.len();
    assert!(
        with_caret.starts_with(zwj),
        "caret insertion must not split the ZWJ family: {with_caret:?}"
    );
    assert_eq!(
        &with_caret[zwj_byte_len..zwj_byte_len + TEXT_EDIT_CARET.len_utf8()],
        TEXT_EDIT_CARET.to_string().as_str(),
        "caret must sit right after the ZWJ family"
    );
}

/// Combining-mark sequences ("e" + U+0301 = "é" as one cluster)
/// keep the same one-cluster contract: insert the caret after
/// cluster 0 lands AFTER the combining mark, not between base
/// and combiner.
#[test]
fn test_insert_caret_after_combining_mark() {
    let combining = "e\u{0301}";
    let with_caret = insert_caret(combining, 1);
    assert!(
        with_caret.starts_with(combining),
        "caret must not split the base + combiner: {with_caret:?}"
    );
}

/// Inserting a fresh codepoint at a cursor positioned mid-emoji
/// is impossible — `insert_at_cursor` operates on grapheme-
/// cluster indices, so cursor=1 inside a one-cluster ZWJ family
/// is past the family, not inside it. The new codepoint lands
/// after the family.
#[test]
fn test_insert_at_cursor_after_zwj_emoji_family() {
    let zwj = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let mut s = String::from(zwj);
    let new_cursor = insert_at_cursor(&mut s, 1, 'X');
    assert!(s.starts_with(zwj));
    assert!(s.ends_with('X'));
    // Cursor advances by one grapheme cluster.
    assert_eq!(new_cursor, 2);
}

/// Deleting before the cursor at cluster index 1 of a ZWJ
/// family removes the *whole* family, not just one codepoint.
/// Pins the grapheme-aware deletion contract on the keystroke
/// path; pre-fix a codepoint-only delete would orphan ZWJ
/// joiners in the buffer.
#[test]
fn test_delete_before_cursor_removes_whole_zwj_family() {
    let zwj = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let combined = format!("{zwj}A");
    let mut s = String::from(combined);
    let new_cursor = delete_before_cursor(&mut s, 1);
    assert_eq!(s, "A", "ZWJ family must be deleted as one cluster");
    assert_eq!(new_cursor, 0);
}

// ── Shift-select anchor lifecycle (N4-C.b.2) ─────────────────────

/// Helper: build an open editor state with a specific buffer
/// and cursor position, no anchor set yet.
fn open_editor_state(buffer: &str, cursor: usize) -> TextEditState {
    TextEditState::Open {
        node_id: "n-1".into(),
        section_idx: 0,
        buffer: buffer.into(),
        cursor_grapheme_pos: cursor,
        buffer_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        original_text: buffer.into(),
        original_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        selection_anchor: None,
        exit_to_default_on_close: false,
    }
}

/// Read `(cursor, anchor)` out of an open editor state for
/// assertion convenience. Panics on `Closed`.
fn cursor_and_anchor(state: &TextEditState) -> (usize, Option<usize>) {
    match state {
        TextEditState::Open {
            cursor_grapheme_pos,
            selection_anchor,
            ..
        } => (*cursor_grapheme_pos, *selection_anchor),
        _ => panic!("editor closed"),
    }
}

/// First shift-cursor action seeds the anchor at the
/// pre-action cursor position and moves the cursor.
#[test]
fn test_shift_cursor_seeds_anchor_at_pre_action_cursor() {
    use crate::application::keybinds::Action;
    let mut s = open_editor_state("abcdef", 3);
    apply_text_edit_action(Action::TextEditCursorRightSelect, &mut s);
    let (cursor, anchor) = cursor_and_anchor(&s);
    assert_eq!(cursor, 4);
    assert_eq!(anchor, Some(3));
}

/// Subsequent shift-cursor actions extend the cursor without
/// disturbing the anchor.
#[test]
fn test_subsequent_shift_cursor_preserves_anchor() {
    use crate::application::keybinds::Action;
    let mut s = open_editor_state("abcdef", 3);
    apply_text_edit_action(Action::TextEditCursorRightSelect, &mut s);
    apply_text_edit_action(Action::TextEditCursorRightSelect, &mut s);
    let (cursor, anchor) = cursor_and_anchor(&s);
    assert_eq!(cursor, 5);
    assert_eq!(anchor, Some(3));
}

/// Non-shift cursor action collapses the selection: the anchor
/// drops to `None` and the cursor moves normally.
#[test]
fn test_non_shift_cursor_clears_anchor() {
    use crate::application::keybinds::Action;
    let mut s = open_editor_state("abcdef", 3);
    apply_text_edit_action(Action::TextEditCursorRightSelect, &mut s);
    apply_text_edit_action(Action::TextEditCursorLeft, &mut s);
    let (cursor, anchor) = cursor_and_anchor(&s);
    assert_eq!(cursor, 3);
    assert_eq!(anchor, None);
}

/// Text edits (delete / type) clear the anchor — range-aware
/// editing is deferred. Pin the simpler "anchor only lives
/// across cursor-key sequences" invariant so it doesn't drift.
#[test]
fn test_text_edit_action_clears_anchor() {
    use crate::application::keybinds::Action;
    let mut s = open_editor_state("abcdef", 3);
    apply_text_edit_action(Action::TextEditCursorRightSelect, &mut s);
    apply_text_edit_action(Action::TextEditDeleteBack, &mut s);
    let (_, anchor) = cursor_and_anchor(&s);
    assert_eq!(anchor, None);
}

/// Shift-cursor on the anchor's own position is a degenerate
/// case: anchor is set, cursor moves backward, post-action
/// `cursor < anchor` is the canonical "selection from anchor
/// down to cursor" shape. Pin both directions to confirm the
/// (anchor, cursor) pair commutes correctly.
#[test]
fn test_shift_cursor_left_seeds_anchor_above_cursor() {
    use crate::application::keybinds::Action;
    let mut s = open_editor_state("abcdef", 3);
    apply_text_edit_action(Action::TextEditCursorLeftSelect, &mut s);
    let (cursor, anchor) = cursor_and_anchor(&s);
    assert_eq!(cursor, 2);
    assert_eq!(anchor, Some(3));
}

/// Shift+Home seeds anchor and jumps cursor to start of line.
#[test]
fn test_shift_home_seeds_anchor_at_line_start() {
    use crate::application::keybinds::Action;
    let mut s = open_editor_state("abcdef", 4);
    apply_text_edit_action(Action::TextEditCursorHomeSelect, &mut s);
    let (cursor, anchor) = cursor_and_anchor(&s);
    assert_eq!(cursor, 0);
    assert_eq!(anchor, Some(4));
}

// ── Editor close → SectionRange lift (N4-C.b.2) ──────────────────

/// `lift_anchor_to_section_range` lifts the editor's
/// shift-select pair into a `Section(SectionSel)` selection
/// when the anchor differs from the cursor — the function name
/// is historical; today it discards the grapheme range because
/// every `SectionRange` consumer interprets the field as
/// section indices, so writing grapheme positions silently
/// broke downstream fan-out (`border preview`, structural
/// cleanup, `commit_border_preview`'s `Sections` target). The
/// post-commit selection lands at the section the user was
/// editing — the right anchor for follow-up per-section verbs.
#[test]
fn test_lift_anchor_lifts_to_section_when_anchor_below_cursor() {
    use super::editor::lift_anchor_to_section_range;
    use crate::application::document::SelectionState;
    let lifted = lift_anchor_to_section_range(Some(3), 7, "node-1", 2).expect("anchor != cursor → lift");
    match lifted {
        SelectionState::Section(sel) => {
            assert_eq!(sel.node_id, "node-1");
            assert_eq!(sel.section_idx, 2);
        }
        other => panic!("expected Section, got {:?}", other),
    }
}

#[test]
fn test_lift_anchor_lifts_to_section_when_anchor_above_cursor() {
    use super::editor::lift_anchor_to_section_range;
    use crate::application::document::SelectionState;
    let lifted = lift_anchor_to_section_range(Some(7), 3, "node-1", 2).expect("anchor != cursor → lift");
    assert!(matches!(lifted, SelectionState::Section(_)));
}

/// `lift_anchor_to_section_range` returns `None` when the
/// anchor is unset (no shift-select happened during the editor
/// session) — the existing `Section` selection from open time
/// stays in place.
#[test]
fn test_lift_anchor_returns_none_when_anchor_unset() {
    use super::editor::lift_anchor_to_section_range;
    let lifted = lift_anchor_to_section_range(None, 7, "node-1", 0);
    assert!(lifted.is_none());
}

/// `lift_anchor_to_section_range` returns `None` when the
/// anchor coincides with the cursor — the user pressed shift-
/// arrow then arrowed back, collapsing the selection.
#[test]
fn test_lift_anchor_returns_none_when_anchor_equals_cursor() {
    use super::editor::lift_anchor_to_section_range;
    let lifted = lift_anchor_to_section_range(Some(5), 5, "node-1", 0);
    assert!(lifted.is_none());
}

// ── Literal-char insertion clears anchor (N4-C.b.2.1) ────────────

/// Typing a printable character while a shift-select range is
/// active drops the anchor. Without this the editor close path
/// would lift a stale `(anchor, cursor)` pair pointing at
/// shifted-around graphemes — N4-C.b.2.1 Major #1.
#[test]
fn test_literal_char_typing_clears_anchor() {
    use super::editor::apply_literal_char_insert;
    use crate::application::platform::input::Key;
    let mut s = open_editor_state("abcdef", 3);
    // Seed an anchor by pretending the user shift-selected
    // forward by one grapheme.
    if let TextEditState::Open { selection_anchor, .. } = &mut s {
        *selection_anchor = Some(2);
    }
    // Now type a literal `x`.
    let changed = apply_literal_char_insert(None, &Key::Character("x".into()), &mut s);
    assert!(changed);
    let (_, anchor) = cursor_and_anchor(&s);
    assert_eq!(anchor, None, "literal-char typing must clear anchor");
}

/// Pressing Enter while a shift-select range is active drops
/// the anchor (mirror of the printable-char branch).
#[test]
fn test_literal_enter_clears_anchor() {
    use super::editor::apply_literal_char_insert;
    use crate::application::platform::input::Key;
    let mut s = open_editor_state("abcdef", 3);
    if let TextEditState::Open { selection_anchor, .. } = &mut s {
        *selection_anchor = Some(2);
    }
    let changed = apply_literal_char_insert(
        Some("enter"),
        &Key::Named(crate::application::platform::input::NamedKey::Enter),
        &mut s,
    );
    assert!(changed);
    let (_, anchor) = cursor_and_anchor(&s);
    assert_eq!(anchor, None, "literal-Enter must clear anchor");
}

/// Pressing Tab while a shift-select range is active drops the
/// anchor.
#[test]
fn test_literal_tab_clears_anchor() {
    use super::editor::apply_literal_char_insert;
    use crate::application::platform::input::Key;
    let mut s = open_editor_state("abcdef", 3);
    if let TextEditState::Open { selection_anchor, .. } = &mut s {
        *selection_anchor = Some(2);
    }
    let changed = apply_literal_char_insert(
        Some("tab"),
        &Key::Named(crate::application::platform::input::NamedKey::Tab),
        &mut s,
    );
    assert!(changed);
    let (_, anchor) = cursor_and_anchor(&s);
    assert_eq!(anchor, None, "literal-Tab must clear anchor");
}
