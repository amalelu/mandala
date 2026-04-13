use lazy_static::lazy_static;
use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::font::fonts::AppFont;

lazy_static!(
    pub static ref OVERLAPS_TEST: Vec<(Range, Range, bool)> = vec![
            (Range::new(0, 10), Range::new(10, 20), false),
            (Range::new(0, 10), Range::new(9, 20), true),
            (Range::new(0, 10), Range::new(0, 20), true),
            (Range::new(5, 10), Range::new(0, 20), true),
            (Range::new(5, 10), Range::new(0, 5), false),
            (Range::new(5, 10), Range::new(0, 6), true),
            (Range::new(5, 10), Range::new(8, 9), true),
        ];
    );

#[test]
fn test_overlaps() {
   do_overlaps();
}

pub fn do_overlaps() {
   for (a, b, expected) in OVERLAPS_TEST.clone() {
      let result = a.overlaps(&b);
      assert_eq!(result, expected);
      assert_eq!(result, b.overlaps(&a))
   }
}

#[test]
fn test_split_and_separate_1() {
   do_split_and_separate_1();
}

pub fn do_split_and_separate_1() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 16)));
   regions.split_and_separate(Range::new(4, 8));
   assert_eq!(regions.num_regions(), 2);
   let _region_1 = regions.get(Range::new(0, 4)).unwrap();
   let _region_2 = regions.get(Range::new(8, 20)).unwrap();
}
#[test]
fn test_split_and_separate_2() {
   do_split_and_separate_2();
}

pub fn do_split_and_separate_2() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 16)));
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(16, 32)));
   regions.split_and_separate(Range::new(4, 8));
   assert_eq!(regions.num_regions(), 3);
   let _region_1 = regions.get(Range::new(0, 4)).unwrap();
   let _region_2 = regions.get(Range::new(8, 20)).unwrap();
   let _region_3 = regions.get(Range::new(20, 36)).unwrap();
}

#[test]
fn test_submit_region_drops_inverted_range() {
   do_submit_region_drops_inverted_range();
}

/// Regression for the `panic!` removed from `submit_region` in chunk
/// 2: an inverted (`start > end`) range used to abort the editor.
/// It now logs and is silently dropped, so a malformed mutation
/// degrades the frame instead.
pub fn do_submit_region_drops_inverted_range() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 16)));
   // Intentionally inverted — start > end. Pre-fix this would panic.
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(20, 5)));
   assert_eq!(regions.num_regions(), 1);
   let _kept = regions.get(Range::new(0, 16)).unwrap();
}

#[test]
fn test_single_span_empty_is_empty() {
   do_single_span_empty_is_empty();
}

/// `single_span(0, ...)` returns a region set with zero regions —
/// matches the `cluster_count > 0` guard every former open-coded
/// call site wrote by hand.
pub fn do_single_span_empty_is_empty() {
   let regions = ColorFontRegions::single_span(0, Some([1.0, 0.0, 0.0, 1.0]), None);
   assert_eq!(regions.num_regions(), 0);
}

#[test]
fn test_single_span_non_empty_covers_range() {
   do_single_span_non_empty_covers_range();
}

/// `single_span(N, color, font)` produces one region covering
/// `[0, N)` with the given color + font pin.
pub fn do_single_span_non_empty_covers_range() {
   let red = [1.0, 0.0, 0.0, 1.0];
   let regions = ColorFontRegions::single_span(
      7,
      Some(red),
      Some(AppFont::NotoSerifTibetanRegular),
   );
   assert_eq!(regions.num_regions(), 1);
   let r = regions.get(Range::new(0, 7)).unwrap();
   assert_eq!(r.range.start, 0);
   assert_eq!(r.range.end, 7);
   assert_eq!(r.color, Some(red));
   assert_eq!(r.font, Some(AppFont::NotoSerifTibetanRegular));
}

#[test]
fn test_single_span_none_color_none_font() {
   do_single_span_none_color_none_font();
}

/// Both `color` and `font` may be `None` — matches the renderer's
/// border-text areas where the renderer default color wins.
pub fn do_single_span_none_color_none_font() {
   let regions = ColorFontRegions::single_span(3, None, None);
   assert_eq!(regions.num_regions(), 1);
   let r = regions.get(Range::new(0, 3)).unwrap();
   assert_eq!(r.color, None);
   assert_eq!(r.font, None);
}

#[test]
fn test_shrink_regions_after_fully_right_shifts_left() {
   do_shrink_regions_after_fully_right_shifts_left();
}

/// Regions that sit fully right of the deletion window shift left
/// by `magnitude`.
pub fn do_shrink_regions_after_fully_right_shifts_left() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 3)));
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(10, 15)));
   regions.shrink_regions_after(5, 2);
   assert_eq!(regions.num_regions(), 2);
   assert!(regions.get(Range::new(0, 3)).is_some());
   assert!(regions.get(Range::new(8, 13)).is_some());
}

#[test]
fn test_shrink_regions_after_spanning_region_absorbs() {
   do_shrink_regions_after_spanning_region_absorbs();
}

/// A region that straddles the deletion window with strict room on
/// both sides absorbs the deletion: its `end` shrinks by the cut's
/// magnitude.
pub fn do_shrink_regions_after_spanning_region_absorbs() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 10)));
   regions.shrink_regions_after(3, 4);
   assert_eq!(regions.num_regions(), 1);
   assert!(regions.get(Range::new(0, 6)).is_some());
}

#[test]
fn test_shrink_regions_after_fully_inside_collapses() {
   do_shrink_regions_after_fully_inside_collapses();
}

/// A region that lies fully inside the deletion window is dropped
/// from the set — the text it covered is gone.
pub fn do_shrink_regions_after_fully_inside_collapses() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 3)));
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(4, 7)));
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(10, 15)));
   regions.shrink_regions_after(4, 3);
   assert_eq!(regions.num_regions(), 2);
   assert!(regions.get(Range::new(0, 3)).is_some());
   assert!(regions.get(Range::new(7, 12)).is_some());
}

#[test]
fn test_shrink_regions_after_left_partial_clamps() {
   do_shrink_regions_after_left_partial_clamps();
}

/// A region whose right edge falls inside the deletion window
/// clamps its `end` to the cut's start.
pub fn do_shrink_regions_after_left_partial_clamps() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 5)));
   regions.shrink_regions_after(3, 4);
   assert_eq!(regions.num_regions(), 1);
   assert!(regions.get(Range::new(0, 3)).is_some());
}

#[test]
fn test_shrink_regions_after_right_partial_clamps() {
   do_shrink_regions_after_right_partial_clamps();
}

/// A region whose left edge falls inside the deletion window
/// clamps its `start` to the cut's start and shifts its `end` left
/// by the cut's magnitude, so the region sits flush against the
/// remaining-text boundary.
pub fn do_shrink_regions_after_right_partial_clamps() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(5, 15)));
   regions.shrink_regions_after(3, 4);
   assert_eq!(regions.num_regions(), 1);
   assert!(regions.get(Range::new(3, 11)).is_some());
}

#[test]
fn test_shrink_regions_after_zero_magnitude_is_noop() {
   do_shrink_regions_after_zero_magnitude_is_noop();
}

/// `magnitude == 0` means "nothing was deleted"; the region set
/// must be unchanged.
pub fn do_shrink_regions_after_zero_magnitude_is_noop() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 3)));
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(5, 10)));
   regions.shrink_regions_after(4, 0);
   assert_eq!(regions.num_regions(), 2);
   assert!(regions.get(Range::new(0, 3)).is_some());
   assert!(regions.get(Range::new(5, 10)).is_some());
}

#[test]
fn test_insert_regions_at_straddling_region_absorbs() {
   do_insert_regions_at_straddling_region_absorbs();
}

/// A region that straddles the insertion point absorbs the new
/// chars — its `end` grows by `magnitude`.
pub fn do_insert_regions_at_straddling_region_absorbs() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 6)));
   let absorbed = regions.insert_regions_at(3, 2);
   assert!(absorbed);
   assert_eq!(regions.num_regions(), 1);
   assert!(regions.get(Range::new(0, 8)).is_some());
}

#[test]
fn test_insert_regions_at_left_adjacent_region_absorbs() {
   do_insert_regions_at_left_adjacent_region_absorbs();
}

/// A region whose `end == idx` (left-adjacent to the insertion)
/// absorbs the new chars rather than leaving them uncovered.
pub fn do_insert_regions_at_left_adjacent_region_absorbs() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 3)));
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(3, 6)));
   let absorbed = regions.insert_regions_at(3, 2);
   assert!(absorbed);
   // The left region extends; the right region shifts.
   assert_eq!(regions.num_regions(), 2);
   assert!(regions.get(Range::new(0, 5)).is_some());
   assert!(regions.get(Range::new(5, 8)).is_some());
}

#[test]
fn test_insert_regions_at_shifts_right_regions() {
   do_insert_regions_at_shifts_right_regions();
}

/// Regions entirely right of the insertion shift right by `magnitude`.
pub fn do_insert_regions_at_shifts_right_regions() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(10, 15)));
   let absorbed = regions.insert_regions_at(5, 3);
   assert!(!absorbed);
   assert_eq!(regions.num_regions(), 1);
   assert!(regions.get(Range::new(13, 18)).is_some());
}

#[test]
fn test_insert_regions_at_zero_position_shifts_all() {
   do_insert_regions_at_zero_position_shifts_all();
}

/// `idx == 0` shifts every region right by `magnitude` (no absorber
/// can exist because no region can have `start < 0`).
pub fn do_insert_regions_at_zero_position_shifts_all() {
   let mut regions = ColorFontRegions::new_empty();
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 5)));
   regions.submit_region(ColorFontRegion::new_key_only(Range::new(5, 10)));
   let absorbed = regions.insert_regions_at(0, 2);
   assert!(!absorbed);
   assert_eq!(regions.num_regions(), 2);
   assert!(regions.get(Range::new(2, 7)).is_some());
   assert!(regions.get(Range::new(7, 12)).is_some());
}

#[test]
fn test_insert_regions_at_empty_returns_false() {
   do_insert_regions_at_empty_returns_false();
}

/// Inserting into an empty region set returns `false` — the caller
/// (the text-editor caret path) uses this to insert a fresh region
/// for the caret glyph so it renders in an empty-buffer node.
pub fn do_insert_regions_at_empty_returns_false() {
   let mut regions = ColorFontRegions::new_empty();
   let absorbed = regions.insert_regions_at(0, 1);
   assert!(!absorbed);
   assert_eq!(regions.num_regions(), 0);
}
