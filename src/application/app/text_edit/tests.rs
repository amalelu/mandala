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

    let mut area = GlyphArea::new_with_str(
        "initial",
        14.0,
        16.8,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 30.0),
    );
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

    let mut area = GlyphArea::new_with_str(
        "",
        14.0,
        16.8,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 30.0),
    );
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
    use baumhard::core::primitives::{
        Applicable, ApplyOperation, ColorFontRegion, ColorFontRegions, Range,
    };
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
        buffer: "hi".to_string(),
        cursor_grapheme_pos: 2,
        buffer_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        original_text: String::new(),
        original_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
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
