// SPDX-License-Identifier: MPL-2.0

//! `GlyphMatrix` — a column-of-[`GlyphLine`]s wrapper. The `place_in`
//! method paints the matrix onto a target `String` + `ColorFontRegions`
//! at an offset: the composition primitive a scene builder reaches for
//! when several models share one text buffer. Nothing in the crate
//! calls it yet — today's builders hand each `GlyphModel` its own
//! buffer — so it is a library surface held to library standards
//! rather than a per-frame hot path.

use super::component::GlyphComponent;
use super::line::GlyphLine;
use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::util::grapheme_chad::{
    count_grapheme_clusters, count_number_lines, find_nth_line_grapheme_range, insert_new_lines,
    insert_spaces, push_spaces, replace_graphemes_until_newline,
};
use log::debug;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::ops::{AddAssign, Index, IndexMut, MulAssign, SubAssign};

/// Stacked collection of [`GlyphLine`]s rendered top-to-bottom.
/// Wraps a single `Vec<GlyphLine>`; indexing past the end of the
/// matrix auto-expands with empty lines, so callers can write into
/// arbitrary coordinates without pre-sizing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphMatrix {
    /// Ordered lines. Index 0 is the top-most visual line.
    pub matrix: Vec<GlyphLine>,
}

impl Index<usize> for GlyphMatrix {
    type Output = GlyphLine;

    fn index(&self, index: usize) -> &Self::Output {
        self.matrix.get(index).unwrap()
    }
}

impl IndexMut<usize> for GlyphMatrix {
    /// Mutable line access — panics on out-of-bounds, matching
    /// `Vec::index_mut`. Use [`Self::ensure_line`] before
    /// indexing if the line might not exist yet.
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.matrix[index]
    }
}

impl SubAssign for GlyphMatrix {
    /// Rows absent from `self` fall away — there is no such thing as
    /// a negative glyph. `rhs` is consumed line by line, so no line
    /// is cloned.
    fn sub_assign(&mut self, rhs: Self) {
        for (i, line) in rhs.matrix.into_iter().enumerate() {
            debug!("Looking at rhs line {}", i);
            match self.get_mut(i) {
                Some(target) => target.sub_assign(line),
                None => debug!("line {} does not exist in self, so falls away", i),
            }
        }
    }
}

impl MulAssign for GlyphMatrix {
    /// Row-wise product. Rows absent from `self` fall away — pairing
    /// them with nothing is multiplication by zero. `rhs` is consumed
    /// line by line, so no line is cloned.
    fn mul_assign(&mut self, rhs: Self) {
        for (i, line) in rhs.matrix.into_iter().enumerate() {
            debug!("Looking at rhs line {}", i);
            match self.get_mut(i) {
                Some(target) => target.mul_assign(line),
                None => debug!("line {} does not exist in self, so falls away", i),
            }
        }
    }
}

impl AddAssign for GlyphMatrix {
    /// Overriding row-wise add: `rhs` may be taller than `self`, in
    /// which case the surplus rows are appended. `rhs` is consumed
    /// line by line, so no line is cloned — and the append is a
    /// `push` (the iteration is contiguous from 0, so a missing row
    /// is always exactly one past the end; `insert` would be a
    /// panic waiting for a non-contiguous caller).
    fn add_assign(&mut self, rhs: Self) {
        for (i, line) in rhs.matrix.into_iter().enumerate() {
            debug!("Looking at rhs line {}", i);
            match self.get_mut(i) {
                Some(target) => target.add_assign(line),
                None => {
                    self.matrix.push(line);
                    debug!("Moved line {} from rhs into self", i);
                }
            }
        }
    }
}

impl GlyphMatrix {
    /// Empty matrix. O(1), no allocation.
    pub fn new() -> Self {
        GlyphMatrix { matrix: vec![] }
    }
    /// Append a line. O(1) amortized.
    pub fn push(&mut self, line: GlyphLine) {
        self.matrix.push(line);
    }

    /// Borrow the line at `line_num`. O(1).
    pub fn get(&self, line_num: usize) -> Option<&GlyphLine> {
        self.matrix.get(line_num)
    }

    /// Mutable borrow of the line at `line_num`. O(1).
    pub fn get_mut(&mut self, line_num: usize) -> Option<&mut GlyphLine> {
        self.matrix.get_mut(line_num)
    }

    /// Grow `matrix` with empty lines until `line_num` is in
    /// bounds, then return a mutable reference to that line. The
    /// explicit-grow path that the old `IndexMut` quietly did on
    /// every out-of-bounds index — callers that need auto-grow
    /// must say so. O(growth) amortized.
    pub fn ensure_line(&mut self, line_num: usize) -> &mut GlyphLine {
        if line_num >= self.matrix.len() {
            self.matrix.resize_with(line_num + 1, GlyphLine::new);
        }
        &mut self.matrix[line_num]
    }

    /// Expanding-insert at `(line_num, idx)`: grows rows / columns
    /// with blank space as needed, then delegates to
    /// [`GlyphLine::expanding_insert`]. O(line growth + grapheme
    /// walk).
    pub fn expanding_insert(&mut self, line_num: usize, idx: usize, component: &GlyphComponent) {
        self.expand_to_line(line_num, idx);
        self.matrix[line_num].expanding_insert(idx, component);
    }

    /// Overriding-insert at `(line_num, idx)`: grows rows / columns
    /// with blank space as needed, then delegates to
    /// [`GlyphLine::overriding_insert`]. O(line growth + grapheme
    /// walk).
    pub fn overriding_insert(&mut self, line_num: usize, idx: usize, component: &GlyphComponent) {
        self.expand_to_line(line_num, idx);
        self.matrix[line_num].overriding_insert(idx, component);
    }

    /// Paint this matrix into the caller-owned `string` +
    /// `regions` pair, offset by `(cols, rows)` graphemes.
    /// The target `string` is padded with newlines / spaces as
    /// needed so every source component lands on the intended
    /// grapheme cell. `regions` gains one `ColorFontRegion` per
    /// painted component so the renderer can color/fontify the
    /// right spans.
    ///
    /// A component's text is arbitrary and may itself contain
    /// newlines, so painting row `n` can push every row after it
    /// further down the target. `place_in` tracks that drift from the
    /// `added_lines` field of
    /// [`LineReplacement`](crate::util::grapheme_chad::LineReplacement)
    /// and offsets its subsequent row lookups by it; without that, row
    /// `n + 1` used to be painted into the middle of the text row `n`
    /// had just inserted.
    ///
    /// Every span this emits — and the head it advances between
    /// components — comes from the measurement
    /// [`LineReplacement`](crate::util::grapheme_chad::LineReplacement)
    /// took against the buffer, never from the component's own cluster
    /// count. UAX #29 segmentation is not compositional across a splice
    /// seam, so a count taken in isolation is not a position in the
    /// target. Two consequences are worth stating plainly:
    ///
    /// - Every emitted region is in bounds, non-degenerate, and
    ///   disjoint from every other region this call emits — so the
    ///   output satisfies the "non-degenerate and non-overlapping"
    ///   precondition the region primitives document.
    /// - A component's region slices back to exactly the component's
    ///   text unless a seam fused, and there are exactly two ways a
    ///   seam fuses. **Left seam:** a component whose text begins with
    ///   a cluster-extending scalar (a lone combining mark, an LF
    ///   landing after a CR) merges into the cell before it, so it owns
    ///   only the cells whose base scalar it contributed — none at all
    ///   for a lone mark, in which case **no region is emitted**.
    ///   **Right seam:** a component whose text ends in a CR keeps that
    ///   cell when the target's following LF completes a `"\r\n"`, so
    ///   its region slices to `"\r\n"` rather than to `"\r"`. Both are
    ///   [`LineReplacement::written_end`](crate::util::grapheme_chad::LineReplacement::written_end)
    ///   applying its own rule — a cluster belongs to whichever text
    ///   contributed its first scalar — and neither is bookkeeping that
    ///   could be tightened. Preventing the fusion, as opposed to
    ///   reporting it exactly, is a question about the matrix cell
    ///   model and is not answered here.
    ///
    /// Padding inserted for a non-zero x-offset is reported to
    /// `regions` too, so caller spans on the rows below survive it. It
    /// goes through `ColorFontRegions::split_and_separate`, which
    /// shifts a span anchored at the padding point without letting a
    /// span that merely *ends* there absorb the blanks: those blanks
    /// are the next row's indent and belong to no component, so
    /// absorbing them would pin one row's font onto another row's
    /// leading whitespace. A span that *straddles* the padding point
    /// splits around the blanks for the same reason.
    ///
    /// A caller span straddling a **component's** write splits too,
    /// which is unambiguously right when that write is a pure
    /// insertion into an empty line tail and merely accepted when it
    /// overwrites a non-empty one — the comment at the call site
    /// states what is claimed and what is not.
    ///
    /// # Costs
    ///
    /// O(components × target size) — the walk over source components
    /// is linear, but each `replace_graphemes_until_newline` call
    /// walks the whole target twice to measure the cluster delta its
    /// region shift rides on, plus once more inside
    /// `replace_substring`. The target's *line* count, by contrast, is
    /// scanned once up front and then tracked incrementally, so the
    /// drift bookkeeping specifically adds no extra pass. This is a
    /// composition primitive, not a per-frame path (see the module
    /// header); nothing here has been measured against a control, so
    /// no performance claim is made either way.
    pub fn place_in(&self, string: &mut String, regions: &mut ColorFontRegions, offset: (usize, usize)) {
        // Ensure that there's enough lines present in the string
        let num_lines = count_number_lines(string);
        let needed_lines = self.matrix.len() + offset.1;

        // Running line count of `string`, kept exact without ever
        // re-scanning the buffer: the only things below that change it
        // are `insert_new_lines` (exactly `n` lines) and a
        // `LineReplacement` (exactly `added_lines`). It exists to make
        // the "the row I am about to address already exists"
        // invariant checkable rather than merely argued — see the
        // `debug_assert` in the loop.
        let mut have_lines = num_lines;
        if needed_lines > num_lines {
            insert_new_lines(string, needed_lines - num_lines);
            have_lines = needed_lines;
        }

        // Extra target lines contributed by multi-line component
        // text painted on the rows above the current one.
        let mut line_drift = 0usize;

        for (line_num, line) in self.matrix.iter().enumerate() {
            let target_row = line_num + offset.1 + line_drift;
            // The padding above guarantees `have_lines >=
            // self.matrix.len() + offset.1`, and every line the drift
            // counts is one a source physically inserted — so
            // `target_row <= (len - 1) + offset.1 + line_drift <=
            // have_lines - 1`. The row we are about to address always
            // exists; no top-up is reachable here.
            debug_assert!(target_row < have_lines);

            let graph_line_start_index: usize;
            {
                // If there's an x-offset, then we also need to ensure that each line is at least the length of that;
                let target_line_grapheme_range = find_nth_line_grapheme_range(string, target_row);
                if let Some(line_graph_range) = target_line_grapheme_range {
                    let target_line_len = line_graph_range.1 - line_graph_range.0;
                    graph_line_start_index = line_graph_range.0;
                    if target_line_len < offset.0 {
                        let pad = offset.0 - target_line_len;
                        // The padding goes into the *middle* of the
                        // buffer — every row but the last has text
                        // after it — so the region table has to be
                        // told, or every caller span below this row
                        // stops pointing at its text.
                        // `split_and_separate` is the exact semantics:
                        // a span starting *at* the insertion point
                        // must move (`start >= idx`, which
                        // `shift_regions_after`'s replace-and-shift
                        // rule deliberately does not do), a span
                        // *ending* there must not absorb the blanks
                        // (which `insert_regions_at` would do), and a
                        // span *straddling* the point must split
                        // around them rather than swallow them. These
                        // blanks are the indent of the row about to be
                        // painted; they belong to no run and to no
                        // component. Absorbing them stretched the
                        // previous row's own emitted span across the
                        // line terminator and onto the next row's
                        // indent, pinning a component's font to cells
                        // it never wrote. This site is a pure
                        // insertion of cells nothing owns, which is
                        // the primitive's contract verbatim.
                        insert_spaces(string, line_graph_range.1, pad);
                        regions.split_and_separate(Range::new(line_graph_range.1, line_graph_range.1 + pad));
                    }
                } else {
                    // Important that this is done before pushing spaces.
                    // `push_spaces` appends past the end of the buffer,
                    // so there is no text — and therefore no region —
                    // after it to shift.
                    graph_line_start_index = count_grapheme_clusters(string);
                    push_spaces(string, offset.0);
                }
            }

            // Copy each component into the target line. Source
            // always wins the cell — a future refinement where
            // whitespace source preserves non-whitespace target is
            // not yet implemented.
            let mut comp_head = graph_line_start_index + offset.0;
            for component in line.line.iter() {
                let replacement = replace_graphemes_until_newline(string, comp_head, &component.text);
                // `growth` is the buffer's measured cluster delta and
                // can go either way: a component whose first scalar
                // extends the cluster before it fuses with it and the
                // buffer *shrinks*.
                //
                // The grow side is `split_and_separate`, not
                // `shift_regions_after`. At every non-zero x-offset —
                // and for every component after the first on a line
                // whose tail the previous one already consumed — the
                // write is a pure *insertion* into an empty line tail,
                // not an overwrite: nothing at `at` is displaced, so a
                // caller span anchored exactly there has moved right
                // by `growth` like everything else after it.
                // `shift_regions_after`'s strict `start > at` left
                // such a span behind, and the `submit_region` below
                // then evicted it outright, because the region set is
                // keyed on the range alone and the component's own
                // span is the same range. Placement never re-covers a
                // span it displaced — it submits only the cells it
                // wrote — so the "the caller is about to re-cover
                // these cells" justification for the strict `>` does
                // not hold here.
                //
                // With `>=` on the grow side the pairing with
                // `shrink_regions_after` is a true mirror at the seam:
                // both treat a span anchored at `at` as affected.
                //
                // A caller span *straddling* `at` is **split**: the
                // head keeps `[start, at)`, the tail is pushed to
                // `[at + growth, end + growth)`, and the cells the
                // component wrote are left uncovered for its own
                // `submit_region` below. Where the write is the pure
                // insertion described above, that is exactly right —
                // the inserted cells belong to no run. Where it is not
                // — a component overwriting a non-empty line tail,
                // with the buffer merely growing on net — no rule is
                // right: the cells from `at` on no longer carry the
                // caller's text, so neither splitting the span nor
                // leaving it whole describes the buffer. What the
                // split buys there is an exact head: `[start, at)`
                // really is the part of the caller's run the write did
                // not touch, whereas leaving the span whole parks it
                // over cells the component just replaced. That is the
                // behavior as accepted, not as claimed correct — the
                // correct answer needs the cell model to settle who
                // owns a cell whose text was overwritten, the same
                // question the fusion note in this method's doc leaves
                // open, and it is not settled here.
                match replacement.growth.cmp(&0) {
                    Ordering::Greater => regions.split_and_separate(Range::new(
                        replacement.at,
                        replacement.at + replacement.growth as usize,
                    )),
                    Ordering::Less => {
                        regions.shrink_regions_after(replacement.at, replacement.growth.unsigned_abs())
                    }
                    Ordering::Equal => {}
                }
                line_drift += replacement.added_lines;
                have_lines += replacement.added_lines;
                // The span comes from the same measurement `growth`
                // does. `component.length()` counts the component's
                // clusters *in isolation*, which stops being the truth
                // the moment the write fuses with the text on either
                // side of the seam: adding it up walks the head off
                // the end of the buffer and lands spans on the line
                // terminators this code goes to such lengths to
                // protect. `at .. written_end` is measured against the
                // post-edit buffer, so it is in bounds by construction
                // and equals the component's own clusters whenever
                // nothing fused.
                //
                // An *empty* span means the component fused whole into
                // the cell before it and owns no cell at all, and no
                // region is emitted for it. `Range::new(k, k)` would
                // violate the "the region set is non-degenerate"
                // precondition that `insert_regions_at` and
                // `split_and_separate` both document, by construction —
                // and it is worse than useless: `ColorFontRegions` is
                // keyed on the range alone, so two components that both
                // fuse into the same cell collapse into one set slot
                // and the second silently evicts the first's font and
                // color, while neither has a cell to put them on. The
                // fused cell keeps the identity of the component that
                // contributed its base scalar, which is the only
                // answer the cell model can give.
                if replacement.written_end > replacement.at {
                    regions.submit_region(ColorFontRegion::new(
                        Range::new(replacement.at, replacement.written_end),
                        Some(component.font),
                        Some(component.color.to_float()),
                    ));
                }
                comp_head = replacement.written_end;
            }
        }
    }

    fn expand_to_line(&mut self, line_num: usize, idx: usize) {
        let matrix_len = self.matrix.len();

        if matrix_len <= line_num {
            let line_delta = line_num - matrix_len;
            for _ in 0..line_delta {
                self.matrix.push(GlyphLine::new());
            }
            let line = if idx > 0 {
                GlyphLine::new_with(GlyphComponent::space(idx))
            } else {
                GlyphLine::new()
            };
            self.matrix.push(line);
        }
    }
}

impl Default for GlyphMatrix {
    fn default() -> Self {
        GlyphMatrix::new()
    }
}
