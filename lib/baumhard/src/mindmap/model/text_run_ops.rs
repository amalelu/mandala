// SPDX-License-Identifier: MPL-2.0

//! Pure manipulation primitives for `Vec<TextRun>`. Preserves the
//! `format/text-runs.md` invariants (sorted ascending, half-open
//! `[start, end)`, no overlaps, gaps allowed). Linear-time, no
//! `unsafe`, debug-asserts invariants on every public entry.
//! Callers clamp indices to grapheme-cluster counts; these
//! primitives don't see the section text.

use super::node::TextRun;

/// Debug-only invariant check: runs sorted ascending, no
/// overlaps, every `start < end`. Every public helper calls this
/// first so caller drift surfaces in tests; release builds
/// trust the precondition.
#[inline]
fn debug_assert_invariants(runs: &[TextRun]) {
    debug_assert!(
        runs.iter().all(|r| r.start < r.end),
        "text_run_ops: zero-length or inverted run"
    );
    debug_assert!(
        runs.windows(2).all(|w| w[0].end <= w[1].start),
        "text_run_ops: runs out of order or overlapping"
    );
}

/// Index of the run containing `grapheme_idx`, or `None` when
/// it falls in a gap or past the end. Half-open: `idx == run.end`
/// is not contained.
pub fn find_run_containing(runs: &[TextRun], grapheme_idx: usize) -> Option<usize> {
    debug_assert_invariants(runs);
    for (i, run) in runs.iter().enumerate() {
        if run.start > grapheme_idx {
            return None;
        }
        if grapheme_idx < run.end {
            return Some(i);
        }
    }
    None
}

/// Index of the run whose `start == grapheme_idx`. Locates the
/// right half after a successful [`split_at`].
pub fn find_run_starting_at(runs: &[TextRun], grapheme_idx: usize) -> Option<usize> {
    debug_assert_invariants(runs);
    runs.iter().position(|r| r.start == grapheme_idx)
}

/// Carve a boundary at `grapheme_idx` by splitting the
/// straddling run into two adjacent runs sharing all style
/// attributes. Returns `true` on a real split; `false` when
/// the boundary already exists, the index falls in a gap, or
/// past every run.
pub fn split_at(runs: &mut Vec<TextRun>, grapheme_idx: usize) -> bool {
    debug_assert_invariants(runs);
    let Some(target_idx) = find_run_containing(runs, grapheme_idx) else {
        return false;
    };
    // Boundary already at start of the run — no split needed.
    if runs[target_idx].start == grapheme_idx {
        return false;
    }
    let mut right = runs[target_idx].clone();
    runs[target_idx].end = grapheme_idx;
    right.start = grapheme_idx;
    runs.insert(target_idx + 1, right);
    true
}

/// Insert `run` at its sorted position; returns the insertion
/// index. Caller guarantees `run.start < run.end` and non-overlap
/// (debug-asserted). Used by range-targeted setters to fill an
/// uncovered gap inside a target range.
pub fn insert_run(runs: &mut Vec<TextRun>, run: TextRun) -> usize {
    debug_assert_invariants(runs);
    debug_assert!(run.start < run.end, "insert_run: empty run");
    debug_assert!(
        runs.iter().all(|r| r.end <= run.start || r.start >= run.end),
        "insert_run: overlap with existing run"
    );
    let pos = runs.iter().position(|r| r.start >= run.end).unwrap_or(runs.len());
    runs.insert(pos, run);
    pos
}

/// Clone every run intersecting `[start, end)`, clamped to
/// the slice bounds. Original-coordinate output for attribute
/// scans over the range. Output is not re-merged.
pub fn slice(runs: &[TextRun], slice_start: usize, slice_end: usize) -> Vec<TextRun> {
    debug_assert_invariants(runs);
    if slice_start >= slice_end {
        return Vec::new();
    }
    let mut out = Vec::new();
    for run in runs {
        if run.end <= slice_start {
            continue;
        }
        if run.start >= slice_end {
            break;
        }
        let mut clipped = run.clone();
        if clipped.start < slice_start {
            clipped.start = slice_start;
        }
        if clipped.end > slice_end {
            clipped.end = slice_end;
        }
        out.push(clipped);
    }
    out
}

/// Coalesce style-equal neighbors that meet at a common
/// boundary. Gap-separated equals stay separate — the gap
/// carries semantic information (uncovered ranges fall
/// through to section / node defaults).
pub fn merge_adjacent_equal(runs: &mut Vec<TextRun>) {
    debug_assert_invariants(runs);
    if runs.len() < 2 {
        return;
    }
    let mut write = 0usize;
    for read in 1..runs.len() {
        let prev_end = runs[write].end;
        if runs[read].start == prev_end && style_eq(&runs[write], &runs[read]) {
            runs[write].end = runs[read].end;
        } else {
            write += 1;
            if write != read {
                runs.swap(write, read);
            }
        }
    }
    runs.truncate(write + 1);
}

/// Apply an attribute change to every grapheme in
/// `[range_start, range_end)`. The canonical entry point for
/// range-targeted setters: split → mutate → gap-fill → merge.
///
/// `template_for_gap` carries the cascade defaults plus the
/// caller's attribute; its `start`/`end` are overwritten per
/// gap. No-op when `range_start >= range_end`.
pub fn mutate_in_range<F>(
    runs: &mut Vec<TextRun>,
    range_start: usize,
    range_end: usize,
    template_for_gap: &TextRun,
    mut mutate: F,
) where
    F: FnMut(&mut TextRun),
{
    if range_start >= range_end {
        return;
    }
    split_at(runs, range_start);
    split_at(runs, range_end);

    for run in runs.iter_mut() {
        if run.start >= range_start && run.end <= range_end {
            mutate(run);
        }
    }

    // Detect gaps inside [range_start, range_end). Walk in-range
    // runs in order; any uncovered span becomes a gap to fill.
    let mut gaps: Vec<(usize, usize)> = Vec::new();
    let mut prev_end = range_start;
    for run in runs.iter() {
        if run.end <= range_start {
            continue;
        }
        if run.start >= range_end {
            break;
        }
        if run.start > prev_end {
            gaps.push((prev_end, run.start));
        }
        prev_end = run.end;
    }
    if prev_end < range_end {
        gaps.push((prev_end, range_end));
    }

    for (gap_start, gap_end) in gaps {
        let mut filler = template_for_gap.clone();
        filler.start = gap_start;
        filler.end = gap_end;
        insert_run(runs, filler);
    }

    merge_adjacent_equal(runs);
}

/// Splice over `[start, end)`: drop runs (or partials) inside
/// the range, shift later runs by `replacement_len -
/// (end - start)`, and (when `replacement_len > 0`) insert one
/// fresh run carrying `template`'s style at `[start,
/// start + replacement_len)`. The canonical entry point for
/// range-aware Cut (`replacement_len = 0`) and range-aware
/// Paste (`replacement_len = grapheme_count(content)`).
pub fn splice_range(
    runs: &mut Vec<TextRun>,
    start: usize,
    end: usize,
    replacement_len: usize,
    template: &TextRun,
) {
    debug_assert_invariants(runs);
    if start > end {
        return;
    }
    split_at(runs, start);
    split_at(runs, end);
    runs.retain(|r| r.end <= start || r.start >= end);
    let removed = end - start;
    if replacement_len != removed {
        let shift_pos = replacement_len as i64 - removed as i64;
        for run in runs.iter_mut() {
            if run.start >= end {
                run.start = (run.start as i64 + shift_pos) as usize;
                run.end = (run.end as i64 + shift_pos) as usize;
            }
        }
    }
    if replacement_len > 0 {
        let mut filler = template.clone();
        filler.start = start;
        filler.end = start + replacement_len;
        insert_run(runs, filler);
    }
    merge_adjacent_equal(runs);
}

/// Coalesce predicate for [`merge_adjacent_equal`].
fn style_eq(a: &TextRun, b: &TextRun) -> bool {
    a.bold == b.bold
        && a.italic == b.italic
        && a.underline == b.underline
        && a.font == b.font
        && a.size_pt == b.size_pt
        && a.color == b.color
        && a.hyperlink == b.hyperlink
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `font` is the empty string — `format/fonts.md`'s "clears the
    /// pin". This fixture used to say `"Sans"`, which no family
    /// resolves, so it pinned a font that did not exist.
    fn run(start: usize, end: usize, color: &str) -> TextRun {
        TextRun {
            start,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: String::new(),
            size_pt: 12.0,
            color: color.into(),
            hyperlink: None,
        }
    }

    // ── find_run_containing ──────────────────────────────────────

    #[test]
    fn test_find_run_containing_hits_inside() {
        let runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        assert_eq!(find_run_containing(&runs, 0), Some(0));
        assert_eq!(find_run_containing(&runs, 4), Some(0));
        assert_eq!(find_run_containing(&runs, 5), Some(1));
        assert_eq!(find_run_containing(&runs, 9), Some(1));
    }

    #[test]
    fn test_find_run_containing_excludes_end() {
        // `[start, end)` half-open — `idx == last.end` is past
        // the last run and falls in no run.
        let runs = vec![run(0, 10, "red")];
        assert_eq!(find_run_containing(&runs, 10), None);
    }

    #[test]
    fn test_find_run_containing_gap_is_none() {
        let runs = vec![run(0, 5, "red"), run(7, 10, "blue")];
        assert_eq!(find_run_containing(&runs, 5), None);
        assert_eq!(find_run_containing(&runs, 6), None);
    }

    #[test]
    fn test_find_run_containing_empty_runs_is_none() {
        let runs: Vec<TextRun> = Vec::new();
        assert_eq!(find_run_containing(&runs, 0), None);
        assert_eq!(find_run_containing(&runs, 100), None);
    }

    // ── find_run_starting_at ─────────────────────────────────────

    #[test]
    fn test_find_run_starting_at_finds_exact_boundary() {
        let runs = vec![run(0, 5, "red"), run(5, 10, "blue"), run(10, 15, "green")];
        assert_eq!(find_run_starting_at(&runs, 0), Some(0));
        assert_eq!(find_run_starting_at(&runs, 5), Some(1));
        assert_eq!(find_run_starting_at(&runs, 10), Some(2));
    }

    #[test]
    fn test_find_run_starting_at_returns_none_for_non_boundary() {
        let runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        assert_eq!(find_run_starting_at(&runs, 3), None);
        assert_eq!(find_run_starting_at(&runs, 7), None);
        assert_eq!(find_run_starting_at(&runs, 10), None);
    }

    // ── split_at ─────────────────────────────────────────────────

    #[test]
    fn test_split_at_inside_run_inserts_right_half() {
        let mut runs = vec![run(0, 10, "red")];
        let split = split_at(&mut runs, 5);
        assert!(split, "split inside a run must report true");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 5);
        assert_eq!(runs[1].start, 5);
        assert_eq!(runs[1].end, 10);
        // Style attributes inherit on both halves.
        assert_eq!(runs[0].color, "red");
        assert_eq!(runs[1].color, "red");
    }

    #[test]
    fn test_split_at_on_left_boundary_is_noop() {
        let mut runs = vec![run(0, 10, "red")];
        let split = split_at(&mut runs, 0);
        assert!(!split);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn test_split_at_on_right_boundary_is_noop() {
        // `idx == run.end` falls past the run (half-open). No
        // split because the boundary at `run.end` already
        // exists implicitly — there's no run-half on the right
        // to carve out.
        let mut runs = vec![run(0, 10, "red")];
        let split = split_at(&mut runs, 10);
        assert!(!split);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn test_split_at_in_gap_is_noop() {
        let mut runs = vec![run(0, 5, "red"), run(7, 10, "blue")];
        let split = split_at(&mut runs, 6);
        assert!(!split);
        assert_eq!(runs.len(), 2);
    }

    #[test]
    fn test_split_at_past_all_runs_is_noop() {
        let mut runs = vec![run(0, 10, "red")];
        let split = split_at(&mut runs, 100);
        assert!(!split);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn test_split_at_followed_by_find_run_starting_at_locates_right_half() {
        // The intended call pattern for range-targeted setters:
        // split, then locate the right half by `start == idx`.
        let mut runs = vec![run(0, 20, "red")];
        split_at(&mut runs, 7);
        assert_eq!(find_run_starting_at(&runs, 7), Some(1));
    }

    /// Calling `split_at` at the same idx a second time is a
    /// no-op — the boundary already exists from the first call.
    /// Pins the idempotency property a range-targeted setter
    /// relies on when both `range.start` and `range.end` happen
    /// to land on existing boundaries.
    #[test]
    fn test_split_at_is_idempotent_at_same_idx() {
        let mut runs = vec![run(0, 10, "red")];
        assert!(split_at(&mut runs, 5));
        assert!(!split_at(&mut runs, 5));
        assert_eq!(runs.len(), 2);
    }

    // ── insert_run ───────────────────────────────────────────────

    #[test]
    fn test_insert_run_into_empty() {
        let mut runs: Vec<TextRun> = Vec::new();
        let pos = insert_run(&mut runs, run(0, 5, "red"));
        assert_eq!(pos, 0);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn test_insert_run_into_gap_preserves_sort_order() {
        let mut runs = vec![run(0, 5, "red"), run(15, 20, "blue")];
        let pos = insert_run(&mut runs, run(7, 12, "green"));
        assert_eq!(pos, 1);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].end, 5);
        assert_eq!(runs[1].start, 7);
        assert_eq!(runs[1].end, 12);
        assert_eq!(runs[1].color, "green");
        assert_eq!(runs[2].start, 15);
    }

    #[test]
    fn test_insert_run_at_end() {
        let mut runs = vec![run(0, 5, "red")];
        let pos = insert_run(&mut runs, run(10, 15, "blue"));
        assert_eq!(pos, 1);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[1].start, 10);
    }

    #[test]
    fn test_insert_run_followed_by_merge_coalesces_with_neighbor() {
        // Range-targeted setter use case: insert a fresh run
        // into a gap, then merge with an adjacent same-style
        // neighbor. The fresh run's `start` matches the
        // neighbor's `end` so the merge fires.
        let mut runs = vec![run(0, 5, "red")];
        insert_run(&mut runs, run(5, 10, "red"));
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 10);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "overlap")]
    fn test_insert_run_panics_on_overlap_in_debug() {
        let mut runs = vec![run(0, 10, "red")];
        // [5, 15) overlaps [0, 10) — debug_assert fires.
        insert_run(&mut runs, run(5, 15, "blue"));
    }

    // ── slice ────────────────────────────────────────────────────

    #[test]
    fn test_slice_clips_runs_to_bounds() {
        let runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        let out = slice(&runs, 2, 8);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start, 2);
        assert_eq!(out[0].end, 5);
        assert_eq!(out[0].color, "red");
        assert_eq!(out[1].start, 5);
        assert_eq!(out[1].end, 8);
        assert_eq!(out[1].color, "blue");
    }

    #[test]
    fn test_slice_preserves_internal_gaps() {
        let runs = vec![run(0, 5, "red"), run(7, 10, "blue")];
        let out = slice(&runs, 3, 9);
        // Gap `[5, 7)` survives — clipped output drops nothing
        // beyond runs that don't intersect.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start, 3);
        assert_eq!(out[0].end, 5);
        assert_eq!(out[1].start, 7);
        assert_eq!(out[1].end, 9);
    }

    #[test]
    fn test_slice_empty_when_start_ge_end() {
        let runs = vec![run(0, 10, "red")];
        assert!(slice(&runs, 5, 5).is_empty());
        assert!(slice(&runs, 7, 3).is_empty());
    }

    #[test]
    fn test_slice_empty_when_range_in_gap() {
        let runs = vec![run(0, 5, "red"), run(10, 15, "blue")];
        assert!(slice(&runs, 6, 9).is_empty());
    }

    #[test]
    fn test_slice_full_range_returns_clones() {
        let runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        let out = slice(&runs, 0, 10);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], runs[0]);
        assert_eq!(out[1], runs[1]);
    }

    // ── merge_adjacent_equal ─────────────────────────────────────

    #[test]
    fn test_merge_adjacent_equal_coalesces_matching_neighbors() {
        let mut runs = vec![run(0, 5, "red"), run(5, 10, "red")];
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 10);
    }

    #[test]
    fn test_merge_adjacent_equal_preserves_mismatched_style() {
        let mut runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 2);
    }

    #[test]
    fn test_merge_adjacent_equal_preserves_gap_boundary() {
        // Gap means `runs[i].end != runs[i+1].start` — the gap
        // carries semantic information (uncovered range falls
        // through to defaults), so neighbors separated by a
        // gap MUST stay separate even when their attributes
        // match.
        let mut runs = vec![run(0, 5, "red"), run(7, 10, "red")];
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].end, 5);
        assert_eq!(runs[1].start, 7);
    }

    #[test]
    fn test_merge_adjacent_equal_chains_three_runs() {
        let mut runs = vec![run(0, 5, "red"), run(5, 10, "red"), run(10, 15, "red")];
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 15);
    }

    #[test]
    fn test_merge_adjacent_equal_partial_chain() {
        // First two runs match, third differs — only the first
        // pair coalesces.
        let mut runs = vec![run(0, 5, "red"), run(5, 10, "red"), run(10, 15, "blue")];
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 10);
        assert_eq!(runs[1].start, 10);
        assert_eq!(runs[1].end, 15);
    }

    #[test]
    fn test_merge_adjacent_equal_empty_and_single_are_noops() {
        let mut empty: Vec<TextRun> = Vec::new();
        merge_adjacent_equal(&mut empty);
        assert!(empty.is_empty());

        let mut single = vec![run(0, 5, "red")];
        merge_adjacent_equal(&mut single);
        assert_eq!(single.len(), 1);
    }

    // ── Round-trip integration: split + mutate + merge ───────────

    /// The intended call shape for a range-targeted setter:
    /// split at both ends → mutate runs in `[range.start,
    /// range.end)` → merge adjacent equals. Pins the
    /// composition contract — mutating the carved-out slice
    /// followed by a merge produces exactly the runs the user
    /// would expect from "set [3, 7) to blue".
    #[test]
    fn test_split_mutate_merge_round_trip() {
        let mut runs = vec![run(0, 10, "red")];
        // Carve out [3, 7).
        split_at(&mut runs, 3);
        split_at(&mut runs, 7);
        assert_eq!(runs.len(), 3);

        // Mutate the carved-out middle run's color.
        for r in runs.iter_mut() {
            if r.start >= 3 && r.end <= 7 {
                r.color = "blue".into();
            }
        }

        // Merge — neighbors don't match the new blue run, so
        // the three-run shape survives.
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].color, "red");
        assert_eq!(runs[1].color, "blue");
        assert_eq!(runs[2].color, "red");
    }

    /// When the user "sets [3, 7) to red" on an already-red
    /// section, the carved-out middle run matches its
    /// neighbors and merge collapses back to a single run —
    /// the no-op-write should not leave the section with split
    /// runs.
    #[test]
    fn test_split_mutate_merge_no_op_recoalesces() {
        let mut runs = vec![run(0, 10, "red")];
        split_at(&mut runs, 3);
        split_at(&mut runs, 7);
        assert_eq!(runs.len(), 3);

        // No actual mutation — every run stays red.
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 10);
        assert_eq!(runs[0].color, "red");
    }

    // ── mutate_in_range ──────────────────────────────────────────

    /// Range entirely inside one run: split at both ends, mutate
    /// the middle, merge. Pins the simplest composition path.
    #[test]
    fn test_mutate_in_range_inside_one_run() {
        let mut runs = vec![run(0, 10, "red")];
        let template = run(0, 0, "blue");
        mutate_in_range(&mut runs, 3, 7, &template, |r| r.color = "blue".into());
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0], run(0, 3, "red"));
        assert_eq!(runs[1], run(3, 7, "blue"));
        assert_eq!(runs[2], run(7, 10, "red"));
    }

    /// Range crosses run boundaries: split + mutate every fully-
    /// in-range run + merge. The two original runs share a
    /// boundary, so after mutation they're adjacent same-style
    /// and merge collapses them.
    #[test]
    fn test_mutate_in_range_crosses_boundary_merges() {
        let mut runs = vec![run(0, 5, "red"), run(5, 10, "green")];
        let template = run(0, 0, "blue");
        mutate_in_range(&mut runs, 2, 8, &template, |r| r.color = "blue".into());
        // Expect: [0..2 red, 2..8 blue, 8..10 green]
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0], run(0, 2, "red"));
        assert_eq!(runs[1], run(2, 8, "blue"));
        assert_eq!(runs[2], run(8, 10, "green"));
    }

    /// Range falls entirely in a gap: only the gap-fill fires,
    /// no existing run is mutated.
    #[test]
    fn test_mutate_in_range_fills_pure_gap() {
        let mut runs = vec![run(0, 3, "red"), run(15, 20, "green")];
        let template = run(0, 0, "blue");
        mutate_in_range(&mut runs, 5, 10, &template, |r| r.color = "blue".into());
        // Expect: [0..3 red, 5..10 blue, 15..20 green] (gap-fill).
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0], run(0, 3, "red"));
        assert_eq!(runs[1], run(5, 10, "blue"));
        assert_eq!(runs[2], run(15, 20, "green"));
    }

    /// Range partially overlaps a gap: existing run mutated +
    /// gap-fill for the uncovered portion. Pins the
    /// no-grapheme-left-behind property.
    #[test]
    fn test_mutate_in_range_fills_partial_gap() {
        let mut runs = vec![run(0, 5, "red")];
        let template = run(0, 0, "blue");
        // [3, 8) overlaps run [0, 5) on [3, 5) and gap on [5, 8).
        mutate_in_range(&mut runs, 3, 8, &template, |r| r.color = "blue".into());
        // Expect: [0..3 red, 3..8 blue] (3..5 mutated, 5..8 gap-fill,
        // adjacent same-style merge).
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0], run(0, 3, "red"));
        assert_eq!(runs[1], run(3, 8, "blue"));
    }

    /// Range that already exactly matches a run — closure runs
    /// but no boundary changes. Idempotent in the no-mutate case.
    #[test]
    fn test_mutate_in_range_exact_match_idempotent_when_no_op() {
        let mut runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        let template = run(0, 0, "green");
        mutate_in_range(&mut runs, 5, 10, &template, |r| r.color = r.color.clone());
        // No actual mutation — runs unchanged after merge.
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0], run(0, 5, "red"));
        assert_eq!(runs[1], run(5, 10, "blue"));
    }

    /// Empty range (`start >= end`) is a no-op.
    #[test]
    fn test_mutate_in_range_empty_range_is_noop() {
        let mut runs = vec![run(0, 10, "red")];
        let template = run(0, 0, "blue");
        let before = runs.clone();
        mutate_in_range(&mut runs, 5, 5, &template, |r| r.color = "blue".into());
        mutate_in_range(&mut runs, 7, 3, &template, |r| r.color = "blue".into());
        assert_eq!(runs, before);
    }

    /// Range past the end of every run with no existing runs:
    /// the gap-fill fires, producing a fresh single run.
    #[test]
    fn test_mutate_in_range_fills_when_runs_empty() {
        let mut runs: Vec<TextRun> = Vec::new();
        let template = run(0, 0, "blue");
        mutate_in_range(&mut runs, 0, 5, &template, |r| r.color = "blue".into());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0], run(0, 5, "blue"));
    }

    /// User applies the same color the runs already carry —
    /// after the mutate-then-merge dance, the runs collapse
    /// back to their original shape.
    #[test]
    fn test_mutate_in_range_no_change_recoalesces() {
        let mut runs = vec![run(0, 10, "red")];
        let template = run(0, 0, "red");
        mutate_in_range(&mut runs, 3, 7, &template, |r| r.color = "red".into());
        // Mutation was a no-op; merge collapses splits.
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0], run(0, 10, "red"));
    }

    // ── Invariant guards ─────────────────────────────────────────

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "out of order")]
    fn test_invariants_panic_on_unsorted_input() {
        let runs = vec![run(5, 10, "red"), run(0, 3, "blue")];
        let _ = find_run_containing(&runs, 0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "overlapping")]
    fn test_invariants_panic_on_overlapping_input() {
        let runs = vec![run(0, 5, "red"), run(3, 8, "blue")];
        let _ = find_run_containing(&runs, 0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "zero-length")]
    fn test_invariants_panic_on_zero_length_run() {
        let runs = vec![
            run(0, 5, "red"),
            TextRun {
                start: 5,
                end: 5,
                bold: false,
                italic: false,
                underline: false,
                font: String::new(),
                size_pt: 12.0,
                color: "blue".into(),
                hyperlink: None,
            },
        ];
        let _ = find_run_containing(&runs, 0);
    }

    #[test]
    fn test_splice_range_cut_drops_in_range_and_shifts_later_runs_left() {
        let mut runs = vec![run(0, 3, "red"), run(3, 8, "blue"), run(8, 12, "green")];
        let template = run(0, 0, "red");
        splice_range(&mut runs, 3, 8, 0, &template);
        assert_eq!(runs.len(), 2);
        assert_eq!(
            (runs[0].start, runs[0].end, runs[0].color.as_str()),
            (0, 3, "red")
        );
        assert_eq!(
            (runs[1].start, runs[1].end, runs[1].color.as_str()),
            (3, 7, "green")
        );
    }

    #[test]
    fn test_splice_range_paste_inserts_template_run_and_shifts_later_runs_right() {
        let mut runs = vec![run(0, 3, "red"), run(3, 8, "blue"), run(8, 12, "green")];
        let template = run(0, 0, "yellow");
        splice_range(&mut runs, 3, 8, 7, &template);
        assert_eq!(runs.len(), 3);
        assert_eq!(
            (runs[0].start, runs[0].end, runs[0].color.as_str()),
            (0, 3, "red")
        );
        assert_eq!(
            (runs[1].start, runs[1].end, runs[1].color.as_str()),
            (3, 10, "yellow")
        );
        assert_eq!(
            (runs[2].start, runs[2].end, runs[2].color.as_str()),
            (10, 14, "green")
        );
    }

    #[test]
    fn test_splice_range_cut_partial_overlap_collapses_via_merge() {
        // Both halves of the cut neighbor are style-equal so
        // `merge_adjacent_equal` collapses them into one.
        let mut runs = vec![run(0, 10, "red")];
        let template = run(0, 0, "red");
        splice_range(&mut runs, 3, 7, 0, &template);
        assert_eq!(runs, vec![run(0, 6, "red")]);
    }

    #[test]
    fn test_splice_range_cut_partial_overlap_keeps_distinct_styles_separate() {
        // Distinct styles on the two halves don't merge.
        let mut runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        let template = run(0, 0, "red");
        splice_range(&mut runs, 3, 7, 0, &template);
        assert_eq!(runs.len(), 2);
        assert_eq!(
            (runs[0].start, runs[0].end, runs[0].color.as_str()),
            (0, 3, "red")
        );
        assert_eq!(
            (runs[1].start, runs[1].end, runs[1].color.as_str()),
            (3, 6, "blue")
        );
    }

    #[test]
    fn test_splice_range_paste_into_gap_inserts_template_run() {
        let mut runs = vec![run(0, 3, "red"), run(8, 12, "green")];
        let template = run(0, 0, "yellow");
        splice_range(&mut runs, 5, 5, 4, &template);
        assert_eq!(runs.len(), 3);
        assert_eq!((runs[0].start, runs[0].end), (0, 3));
        assert_eq!(
            (runs[1].start, runs[1].end, runs[1].color.as_str()),
            (5, 9, "yellow")
        );
        assert_eq!((runs[2].start, runs[2].end), (12, 16));
    }
}
