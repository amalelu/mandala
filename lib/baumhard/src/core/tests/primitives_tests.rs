use lazy_static::lazy_static;
use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};

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