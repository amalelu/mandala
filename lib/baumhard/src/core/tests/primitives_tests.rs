// SPDX-License-Identifier: MPL-2.0

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::font::fonts::AppFont;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref OVERLAPS_TEST: Vec<(Range, Range, bool)> = vec![
        (Range::new(0, 10), Range::new(10, 20), false),
        (Range::new(0, 10), Range::new(9, 20), true),
        (Range::new(0, 10), Range::new(0, 20), true),
        (Range::new(5, 10), Range::new(0, 20), true),
        (Range::new(5, 10), Range::new(0, 5), false),
        (Range::new(5, 10), Range::new(0, 6), true),
        (Range::new(5, 10), Range::new(8, 9), true),
    ];

    /// Truth table for [`ColorFontRegions::split_and_separate`]:
    /// `(case name, starting regions, inserted range, expected regions)`.
    /// Expected lists are in `BTreeSet` order (`start` then `end`)
    /// because that is the order the set iterates in.
    ///
    /// The table exists because the primitive's three arms are decided
    /// by where a region sits relative to the *insertion point*
    /// (`range.start`) — not relative to the inserted span as a whole,
    /// which is the confusion that produced the inverted ranges this
    /// table now pins down.
    pub static ref SPLIT_AND_SEPARATE_TABLE: Vec<(&'static str, Vec<Range>, Range, Vec<Range>)> = vec![
        (
            "region ends exactly at the insertion point — left-adjacent, untouched",
            vec![Range::new(0, 4)],
            Range::new(4, 8),
            vec![Range::new(0, 4)],
        ),
        (
            "region fully left with a gap — untouched",
            vec![Range::new(0, 3)],
            Range::new(4, 8),
            vec![Range::new(0, 3)],
        ),
        (
            "region straddles the insertion point — head keeps the prefix, tail is pushed past the span",
            vec![Range::new(0, 16)],
            Range::new(4, 8),
            vec![Range::new(0, 4), Range::new(8, 20)],
        ),
        (
            "straddler with a one-cluster tail — the tail survives as a one-cluster region",
            vec![Range::new(0, 5)],
            Range::new(4, 8),
            vec![Range::new(0, 4), Range::new(8, 9)],
        ),
        (
            "region begins exactly at the insertion point — pure shift, never split",
            vec![Range::new(4, 10)],
            Range::new(4, 8),
            vec![Range::new(8, 14)],
        ),
        (
            "region exactly equal to the inserted range — pure shift, no empty husk",
            vec![Range::new(4, 8)],
            Range::new(4, 8),
            vec![Range::new(8, 12)],
        ),
        (
            "region right of the insertion point but overlapping the inserted span — pure shift, no inverted half",
            vec![Range::new(5, 10)],
            Range::new(3, 7),
            vec![Range::new(9, 14)],
        ),
        (
            "region fully right and disjoint — pure shift",
            vec![Range::new(10, 15)],
            Range::new(4, 8),
            vec![Range::new(14, 19)],
        ),
        (
            "insertion at index 0 pushes the whole set right",
            vec![Range::new(0, 10)],
            Range::new(0, 3),
            vec![Range::new(3, 13)],
        ),
        (
            "straddler plus a follower — one splits, the other shifts",
            vec![Range::new(0, 16), Range::new(16, 32)],
            Range::new(4, 8),
            vec![Range::new(0, 4), Range::new(8, 20), Range::new(20, 36)],
        ),
        (
            "zero-magnitude insertion is a no-op, not a split-with-no-gap",
            vec![Range::new(0, 10)],
            Range::new(4, 4),
            vec![Range::new(0, 10)],
        ),
        (
            "inverted insertion range is dropped, not underflowed",
            vec![Range::new(0, 10)],
            Range::new(8, 4),
            vec![Range::new(0, 10)],
        ),
    ];
}

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
fn test_split_and_separate_truth_table() {
    do_split_and_separate_truth_table();
}

/// Walks [`SPLIT_AND_SEPARATE_TABLE`], asserting the exact resulting
/// region set for each case plus two invariants: no range is inverted
/// or empty, and the total number of covered grapheme clusters is
/// conserved (splitting moves coverage, it never creates or destroys
/// any).
///
/// Both invariants hold only above the primitive's documented
/// precondition — a non-degenerate, non-overlapping input set — so
/// every row here satisfies it. The two violation shapes and what
/// they actually do live in
/// [`do_split_and_separate_precondition_violations_propagate`].
pub fn do_split_and_separate_truth_table() {
    for (name, initial, inserted, expected) in SPLIT_AND_SEPARATE_TABLE.iter() {
        let mut regions = ColorFontRegions::new_empty();
        for range in initial {
            regions.submit_region(ColorFontRegion::new_key_only(*range));
        }
        let covered_before: usize = initial.iter().map(|r| r.magnitude()).sum();

        regions.split_and_separate(*inserted);

        let got: Vec<Range> = regions.all_regions().iter().map(|r| r.range).collect();
        assert_eq!(&got, expected, "case `{}`", name);
        let mut covered_after = 0usize;
        for range in &got {
            assert!(
                range.start < range.end,
                "case `{}` produced the degenerate range {}..{}",
                name,
                range.start,
                range.end
            );
            covered_after += range.magnitude();
        }
        assert_eq!(
            covered_before, covered_after,
            "case `{}` did not conserve covered cluster count",
            name
        );
    }
}

#[test]
fn test_split_and_separate_overflow_drops_the_whole_call() {
    do_split_and_separate_overflow_drops_the_whole_call();
}

/// A shift that would carry a region's `end` past `usize::MAX` is
/// refused before anything is written, so the set is left exactly as
/// it was rather than partly shifted. Pairs with the inverted-range
/// and zero-magnitude guards: the primitive now has no input that
/// panics it.
pub fn do_split_and_separate_overflow_drops_the_whole_call() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 8)));
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(10, usize::MAX)));

    regions.split_and_separate(Range::new(2, 5));

    assert_eq!(regions.num_regions(), 2);
    assert!(regions.get(Range::new(0, 8)).is_some());
    assert!(regions.get(Range::new(10, usize::MAX)).is_some());

    // The control: without the `usize::MAX` region present, the very
    // same call shifts. Otherwise "the set was left untouched" is
    // indistinguishable from "the primitive does nothing".
    let mut ordinary = ColorFontRegions::new_empty();
    ordinary.submit_region(ColorFontRegion::new_key_only(Range::new(10, 15)));
    ordinary.split_and_separate(Range::new(2, 5));
    assert!(ordinary.get(Range::new(13, 18)).is_some());
}

#[test]
fn test_range_checked_push_right() {
    do_range_checked_push_right();
}

/// [`Range::checked_push_right`] shifts both endpoints and reports
/// success, or reports failure and leaves the range **exactly as it
/// was** — the all-or-nothing contract every region primitive relies
/// on to bail without half-shifting its set.
pub fn do_range_checked_push_right() {
    let mut ordinary = Range::new(3, 9);
    assert!(ordinary.checked_push_right(4));
    assert_eq!(ordinary, Range::new(7, 13));

    // A zero shift succeeds and changes nothing.
    let mut zero = Range::new(3, 9);
    assert!(zero.checked_push_right(0));
    assert_eq!(zero, Range::new(3, 9));

    // Landing exactly on `usize::MAX` is fine — it is the last legal
    // `end`, not an overflow.
    let mut exact = Range::new(1, usize::MAX - 5);
    assert!(exact.checked_push_right(5));
    assert_eq!(exact, Range::new(6, usize::MAX));

    // One past is not, and the range must be untouched afterwards.
    let mut overflowing = Range::new(1, usize::MAX - 5);
    assert!(!overflowing.checked_push_right(6));
    assert_eq!(overflowing, Range::new(1, usize::MAX - 5));

    // `start` is never advanced on the failing path either — the bug
    // this shape guards against is a half-applied shift that inverts
    // the range.
    let mut wide = Range::new(usize::MAX - 2, usize::MAX);
    assert!(!wide.checked_push_right(1));
    assert_eq!(wide, Range::new(usize::MAX - 2, usize::MAX));
    assert!(wide.start <= wide.end);
}

#[test]
fn test_shift_regions_after_overflow_drops_the_whole_call() {
    do_shift_regions_after_overflow_drops_the_whole_call();
}

/// [`ColorFontRegions::shift_regions_after`] carries the same
/// all-or-nothing overflow posture as its siblings. It is on the live
/// insertion path, so an unchecked `+= magnitude` would wrap silently
/// in release (the workspace sets no `overflow-checks` override) and
/// turn a far-right region into a far-left one.
pub fn do_shift_regions_after_overflow_drops_the_whole_call() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 8)));
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(10, usize::MAX)));

    regions.shift_regions_after(5, 3);

    assert_eq!(regions.num_regions(), 2);
    assert!(regions.get(Range::new(0, 8)).is_some());
    assert!(regions.get(Range::new(10, usize::MAX)).is_some());

    // The ordinary path still shifts.
    let mut ordinary = ColorFontRegions::new_empty();
    ordinary.submit_region(ColorFontRegion::new_key_only(Range::new(10, 15)));
    ordinary.shift_regions_after(5, 3);
    assert!(ordinary.get(Range::new(13, 18)).is_some());
}

#[test]
fn test_insertion_primitives_differ_only_at_the_three_seams() {
    do_insertion_primitives_differ_only_at_the_three_seams();
}

/// The three insertion primitives are the same function everywhere
/// except at the three positions that touch `idx`, and every caller
/// needs exactly one of the three combinations. This pins all nine
/// cells of that table side by side, so that choosing between them is
/// a matter of reading one test rather than of already knowing all
/// three exist:
///
/// | | `start == idx` | `end == idx` | straddling `idx` |
/// |---|---|---|---|
/// | `shift_regions_after` | stays | stays | stays |
/// | `insert_regions_at` | shifts | absorbs | absorbs |
/// | `split_and_separate` | shifts | stays | splits |
///
/// The straddling column is the one that matters most and was the
/// last to be written down: `split_and_separate` and a `(idx,
/// magnitude)`-shaped duplicate of it that briefly lived beside it
/// agreed on every non-straddling region over an exhaustive 1 950-case
/// sweep and disagreed on all 630 straddling ones, which is exactly
/// why the duplicate was invisible to the suite until the column
/// existed.
pub fn do_insertion_primitives_differ_only_at_the_three_seams() {
    // `end == idx` — the left-adjacent region.
    let mut after = ColorFontRegions::new_empty();
    after.submit_region(ColorFontRegion::new_key_only(Range::new(0, 4)));
    after.shift_regions_after(4, 2);
    assert!(
        after.get(Range::new(0, 4)).is_some(),
        "shift_regions_after leaves it"
    );

    let mut insert = ColorFontRegions::new_empty();
    insert.submit_region(ColorFontRegion::new_key_only(Range::new(0, 4)));
    assert!(insert.insert_regions_at(4, 2));
    assert!(
        insert.get(Range::new(0, 6)).is_some(),
        "insert_regions_at absorbs the new cells into it"
    );

    let mut split = ColorFontRegions::new_empty();
    split.submit_region(ColorFontRegion::new_key_only(Range::new(0, 4)));
    split.split_and_separate(Range::new(4, 6));
    assert!(
        split.get(Range::new(0, 4)).is_some(),
        "split_and_separate leaves it — the new cells belong to no run"
    );

    // `start == idx` — the region anchored exactly at the insertion.
    let mut after = ColorFontRegions::new_empty();
    after.submit_region(ColorFontRegion::new_key_only(Range::new(4, 6)));
    after.shift_regions_after(4, 2);
    assert!(
        after.get(Range::new(4, 6)).is_some(),
        "shift_regions_after's strict `>` leaves it behind"
    );

    let mut insert = ColorFontRegions::new_empty();
    insert.submit_region(ColorFontRegion::new_key_only(Range::new(4, 6)));
    assert!(!insert.insert_regions_at(4, 2));
    assert!(
        insert.get(Range::new(6, 8)).is_some(),
        "insert_regions_at shifts it"
    );

    let mut split = ColorFontRegions::new_empty();
    split.submit_region(ColorFontRegion::new_key_only(Range::new(4, 6)));
    split.split_and_separate(Range::new(4, 6));
    assert!(
        split.get(Range::new(6, 8)).is_some(),
        "split_and_separate shifts it — nothing at `idx` was displaced, so its text moved"
    );

    // Straddling (`start < idx < end`) — the column that tells
    // `split_and_separate` apart from a mere `start >= idx` shift.
    let mut after = ColorFontRegions::new_empty();
    after.submit_region(ColorFontRegion::new_key_only(Range::new(2, 6)));
    after.shift_regions_after(4, 4);
    assert_eq!(after.num_regions(), 1);
    assert!(
        after.get(Range::new(2, 6)).is_some(),
        "shift_regions_after leaves the straddler exactly as it was"
    );

    let mut insert = ColorFontRegions::new_empty();
    insert.submit_region(ColorFontRegion::new_key_only(Range::new(2, 6)));
    assert!(insert.insert_regions_at(4, 4));
    assert_eq!(insert.num_regions(), 1);
    assert!(
        insert.get(Range::new(2, 10)).is_some(),
        "insert_regions_at grows the straddler over the new cells"
    );

    let mut split = ColorFontRegions::new_empty();
    split.submit_region(ColorFontRegion::new_key_only(Range::new(2, 6)));
    split.split_and_separate(Range::new(4, 8));
    assert_eq!(
        split.num_regions(),
        2,
        "split_and_separate splits the straddler in two"
    );
    assert!(
        split.get(Range::new(2, 4)).is_some(),
        "the head keeps the two cells left of the insertion"
    );
    assert!(
        split.get(Range::new(8, 10)).is_some(),
        "and the two cells that moved are pushed past the four inserted ones, rather than the \
         run swallowing all six"
    );

    // All three agree everywhere else: strictly left stays, strictly
    // right shifts.
    let mut both = ColorFontRegions::new_empty();
    both.submit_region(ColorFontRegion::new_key_only(Range::new(0, 3)));
    both.submit_region(ColorFontRegion::new_key_only(Range::new(5, 9)));
    both.split_and_separate(Range::new(4, 6));
    assert_eq!(both.num_regions(), 2);
    assert!(both.get(Range::new(0, 3)).is_some());
    assert!(both.get(Range::new(7, 11)).is_some());

    // `idx == 0` shifts everything — the case `shift_regions_after`
    // cannot express at all.
    let mut all = ColorFontRegions::new_empty();
    all.submit_region(ColorFontRegion::new_key_only(Range::new(0, 3)));
    all.split_and_separate(Range::new(0, 2));
    assert!(all.get(Range::new(2, 5)).is_some());

    // Zero magnitude is a no-op.
    let mut zero = ColorFontRegions::new_empty();
    zero.submit_region(ColorFontRegion::new_key_only(Range::new(4, 6)));
    zero.split_and_separate(Range::new(4, 4));
    assert!(zero.get(Range::new(4, 6)).is_some());
}

#[test]
fn test_insert_regions_at_overflow_drops_the_whole_call() {
    do_insert_regions_at_overflow_drops_the_whole_call();
}

/// [`ColorFontRegions::insert_regions_at`] is the live text-edit
/// insertion primitive, so its overflow posture matters most: it
/// leaves the set untouched and returns `false`, which the caret path
/// already handles as "the new chars are uncovered" rather than
/// acting on a half-shifted table.
pub fn do_insert_regions_at_overflow_drops_the_whole_call() {
    // Shift arm.
    let mut shifting = ColorFontRegions::new_empty();
    shifting.submit_region(ColorFontRegion::new_key_only(Range::new(10, usize::MAX)));
    assert!(!shifting.insert_regions_at(5, 3));
    assert!(shifting.get(Range::new(10, usize::MAX)).is_some());

    // Absorb arm — the straddler's `end` is what overflows.
    let mut absorbing = ColorFontRegions::new_empty();
    absorbing.submit_region(ColorFontRegion::new_key_only(Range::new(0, usize::MAX)));
    assert!(!absorbing.insert_regions_at(5, 3));
    assert!(absorbing.get(Range::new(0, usize::MAX)).is_some());

    // The ordinary path still absorbs and reports it.
    let mut ordinary = ColorFontRegions::new_empty();
    ordinary.submit_region(ColorFontRegion::new_key_only(Range::new(0, 10)));
    assert!(ordinary.insert_regions_at(5, 3));
    assert!(ordinary.get(Range::new(0, 13)).is_some());
}

#[test]
fn test_split_and_separate_precondition_violations_propagate() {
    do_split_and_separate_precondition_violations_propagate();
}

/// Pins what the primitive does when its documented precondition — a
/// non-degenerate, non-overlapping region set — is violated.
///
/// This records the consequence; it does not endorse it. The
/// primitive is a per-region rewrite over a `BTreeSet` keyed on the
/// range alone, so it has no way to detect either violation, and the
/// doc comment states the precondition rather than pretending
/// otherwise. The invariants asserted by
/// [`do_split_and_separate_truth_table`] — no degenerate output, and
/// conserved coverage — hold only above that precondition, which is
/// why these two shapes live here instead of as rows in that table.
pub fn do_split_and_separate_precondition_violations_propagate() {
    // Degenerate input: an empty region stays empty, shifted.
    let mut degenerate = ColorFontRegions::new_empty();
    degenerate.submit_region(ColorFontRegion::new_key_only(Range::new(5, 5)));
    degenerate.split_and_separate(Range::new(0, 2));
    assert_eq!(degenerate.num_regions(), 1);
    assert!(degenerate.get(Range::new(7, 7)).is_some());

    // Overlapping input: the straddler's tail and the shifted region
    // land on the same range, collide in the one `BTreeSet` slot that
    // range keys, and coverage is lost with them.
    let gold = [1.0, 0.84, 0.0, 1.0];
    let mut overlapping = ColorFontRegions::new_empty();
    overlapping.submit_region(ColorFontRegion::new_key_only(Range::new(0, 5)));
    overlapping.submit_region(ColorFontRegion::new(Range::new(3, 5), None, Some(gold)));
    let covered_before: usize = overlapping
        .all_regions()
        .iter()
        .map(|r| r.range.magnitude())
        .sum();
    assert_eq!(covered_before, 7);

    overlapping.split_and_separate(Range::new(3, 4));

    let after: Vec<Range> = overlapping.all_regions().iter().map(|r| r.range).collect();
    assert_eq!(after, vec![Range::new(0, 3), Range::new(4, 6)]);
    let covered_after: usize = after.iter().map(Range::magnitude).sum();
    assert_eq!(covered_after, 5, "coverage is lost, as the precondition warns");
}

#[test]
fn test_split_and_separate_preserves_payload_on_both_halves() {
    do_split_and_separate_preserves_payload_on_both_halves();
}

/// Splitting a straddler must carry the color and font pin onto
/// *both* halves — the two halves are the same styled run, just with
/// the newly inserted (and deliberately unstyled) span between them.
pub fn do_split_and_separate_preserves_payload_on_both_halves() {
    let gold = [1.0, 0.84, 0.0, 1.0];
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(0, 16),
        Some(AppFont::NotoSerifTibetanRegular),
        Some(gold),
    ));

    regions.split_and_separate(Range::new(4, 8));

    let head = regions.get(Range::new(0, 4)).unwrap();
    let tail = regions.get(Range::new(8, 20)).unwrap();
    assert_eq!(head.color, Some(gold));
    assert_eq!(tail.color, Some(gold));
    assert_eq!(head.font, Some(AppFont::NotoSerifTibetanRegular));
    assert_eq!(tail.font, Some(AppFont::NotoSerifTibetanRegular));
    // The inserted span itself is left uncovered on purpose — the
    // caller styles it with a follow-up `submit_region`.
    assert!(regions.get(Range::new(4, 8)).is_none());
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
    let regions = ColorFontRegions::single_span(7, Some(red), Some(AppFont::NotoSerifTibetanRegular));
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
fn test_region_shift_and_shrink_disagree_at_the_seam() {
    do_region_shift_and_shrink_disagree_at_the_seam();
}

/// `shift_regions_after` and `shrink_regions_after` are *not* mirrors
/// of one another at `start == idx`, and a caller that pairs them must
/// not assume they are.
///
/// A span anchored exactly at the edit position covers the text an
/// *overwriting* edit is replacing. Growing, `shift_regions_after`
/// leaves it alone: its "replace and shift" contract has the caller
/// submit a fresh span for the new text, and moving the old one right
/// would slide it off the cells it still describes onto the untouched
/// tail. Shrinking, `shrink_regions_after` is describing a *deletion*
/// — the cells that span covered are gone, so a span that lies wholly
/// inside the cut collapses. Both answers are correct for their
/// direction; what would be wrong is assuming one from the other, so
/// this pins the seam in both directions.
///
/// `GlyphMatrix::place_in` used to be the caller that paired these
/// two, and the asymmetry bit it: its grow side is a pure *insertion*
/// at every non-zero x-offset, where nothing at `at` is displaced and
/// leaving a span behind parks it on cells it never described. It now
/// pairs `split_and_separate` with `shrink_regions_after`, which *are*
/// mirrors at the seam — both treat a span anchored at `idx` as
/// affected. `do_insertion_primitives_differ_only_at_the_three_seams`
/// pins that side.
pub fn do_region_shift_and_shrink_disagree_at_the_seam() {
    let mut grown = ColorFontRegions::new_empty();
    grown.submit_region(ColorFontRegion::new_key_only(Range::new(2, 3)));
    grown.shift_regions_after(2, 1);
    assert_eq!(grown.num_regions(), 1);
    assert!(
        grown.get(Range::new(2, 3)).is_some(),
        "a span anchored at the edit position keeps the cells it describes across a grow"
    );

    let mut shrunk = ColorFontRegions::new_empty();
    shrunk.submit_region(ColorFontRegion::new_key_only(Range::new(2, 3)));
    shrunk.shrink_regions_after(2, 1);
    assert_eq!(
        shrunk.num_regions(),
        0,
        "the same span lies wholly inside the cut, and deleted text keeps no span"
    );

    // One cell further out the two do agree, which is the case
    // `place_in`'s caller spans actually ride.
    let mut grown_after = ColorFontRegions::new_empty();
    grown_after.submit_region(ColorFontRegion::new_key_only(Range::new(3, 4)));
    grown_after.shift_regions_after(2, 1);
    assert!(grown_after.get(Range::new(4, 5)).is_some());

    let mut shrunk_after = ColorFontRegions::new_empty();
    shrunk_after.submit_region(ColorFontRegion::new_key_only(Range::new(3, 4)));
    shrunk_after.shrink_regions_after(2, 1);
    assert!(shrunk_after.get(Range::new(2, 3)).is_some());
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

#[test]
fn test_same_content_separates_a_recolor_from_equality() {
    do_same_content_separates_a_recolor_from_equality();
}

/// `same_content` sees the font and color pins that `Eq` — which is
/// range identity, so the owning `BTreeSet` can find a region from a
/// bare range — deliberately does not.
///
/// Each pair below is asserted **both** ways round: `==` says the
/// same thing about it, and `same_content` disagrees. Without the
/// `==` half these would pass against a `same_content` that was a
/// plain alias for `==`, which is exactly the implementation the
/// renderer's overlay re-shape cache cannot use — under it a
/// color-picker hover recolors a cell inside an unchanged range and
/// the cell keeps its old shaped buffers.
pub fn do_same_content_separates_a_recolor_from_equality() {
    let range = Range::new(0, 4);
    let red = ColorFontRegion::new(range, None, Some([1.0, 0.0, 0.0, 1.0]));
    let blue = ColorFontRegion::new(range, None, Some([0.0, 0.0, 1.0, 1.0]));
    let red_pinned = ColorFontRegion::new(range, Some(AppFont::AppleTea), Some([1.0, 0.0, 0.0, 1.0]));

    assert!(red.same_content(&red));
    assert_eq!(red, blue, "range identity is what `Eq` means here");
    assert!(!red.same_content(&blue), "a recolor is a content change");
    assert_eq!(red, red_pinned);
    assert!(!red.same_content(&red_pinned), "a font pin is a content change");

    // Same, at the table level the renderer actually compares.
    let reds = ColorFontRegions::new_from(vec![red]);
    let blues = ColorFontRegions::new_from(vec![blue]);
    assert_eq!(reds, blues);
    assert!(!reds.same_content(&blues));
    assert!(reds.same_content(&ColorFontRegions::new_from(vec![red])));
}

#[test]
fn test_same_content_compares_whole_tables() {
    do_same_content_compares_whole_tables();
}

/// Table-level `same_content` is length-sensitive and positional:
/// a differing region anywhere in the table is a difference, and a
/// table that is a prefix of another is not equal to it.
pub fn do_same_content_compares_whole_tables() {
    let build = |second_color: [f32; 4]| {
        ColorFontRegions::new_from(vec![
            ColorFontRegion::new(Range::new(0, 4), None, Some([1.0, 1.0, 1.0, 1.0])),
            ColorFontRegion::new(Range::new(4, 8), None, Some(second_color)),
        ])
    };
    let base = build([1.0, 0.0, 0.0, 1.0]);
    assert!(base.same_content(&build([1.0, 0.0, 0.0, 1.0])));
    assert!(
        !base.same_content(&build([0.0, 1.0, 0.0, 1.0])),
        "a change in the last region must not hide behind an equal first one"
    );

    let shorter = ColorFontRegions::new_from(vec![ColorFontRegion::new(
        Range::new(0, 4),
        None,
        Some([1.0, 1.0, 1.0, 1.0]),
    )]);
    assert!(!shorter.same_content(&base));
    assert!(!base.same_content(&shorter));
    assert!(ColorFontRegions::new_empty().same_content(&ColorFontRegions::new_empty()));
}
