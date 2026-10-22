// SPDX-License-Identifier: MPL-2.0

//! Core primitives shared across the Baumhard crate.
//!
//! This module defines the foundational data types that every higher-level
//! abstraction in Baumhard rests on: character-range color/font regions
//! (`ColorFontRegions`, `ColorFontRegion`), the arithmetic operation
//! enum (`ApplyOperation`), spatial anchoring (`Anchor`, `AnchorPoint`,
//! `AnchorTarget`), element flags (`Flag`), and the `Applicable` trait
//! that the mutation pipeline dispatches through.
//!
//! Nothing in this module touches the GPU, the font system, or the arena
//! — it is pure data + O(n)-or-better algorithms over sorted sets.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::ops::{AddAssign, MulAssign, SubAssign};
use strum::IntoDiscriminant;

use crate::font::fonts::AppFont;
use crate::util::color::FloatRgba;

/// One contiguous span of text with an optional color and font
/// pin. Identified by its [`Range`]; `Eq` / `Hash` only inspect the
/// range so two regions with the same bounds collide in the owning
/// [`ColorFontRegions`] set regardless of color / font.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ColorFontRegion {
    /// Half-open grapheme-cluster span this region covers in the
    /// backing text. Sole key for `Eq` / `Ord` / `Hash` on the
    /// owning [`ColorFontRegions`] set.
    pub range: Range,
    /// Optional font pin overriding the area's default for this
    /// span. `None` lets the area's font show through.
    pub font: Option<AppFont>,
    /// Optional color pin overriding the area's default. `None`
    /// lets the area's text color show through.
    pub color: Option<FloatRgba>,
}

impl Hash for ColorFontRegion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Only the range is hashed — the set semantics key on it alone
        // so a lookup by bare range can find a stored region with any
        // color / font payload.
        self.range.hash(state);
    }
}

/// Field selector for a [`ColorFontRegion`]. Used by mutation paths
/// that update a single facet of an existing region.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ColorFontRegionField {
    /// Replace the region's grapheme-cluster span. Repositions the
    /// region within the owning set's `BTreeSet` ordering.
    Range(Range),
    /// Set the region's font pin to a specific [`AppFont`],
    /// overriding the area's default for the region's span.
    Font(AppFont),
    /// Set the region's color pin to an explicit [`FloatRgba`],
    /// overriding the area's default text color for the span.
    Color(FloatRgba),
    /// Mutates the region as a whole (insert, replace, or delete
    /// the entry), without targeting an individual sub-field. The
    /// outer mutation's [`ApplyOperation`] decides which.
    This,
}

impl PartialOrd for ColorFontRegion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // Delegate to `Ord` — the two must not be able to disagree.
        Some(self.cmp(other))
    }
}

impl Eq for ColorFontRegion {}

impl PartialEq for ColorFontRegion {
    fn eq(&self, other: &Self) -> bool {
        self.range == other.range
    }
}

impl Ord for ColorFontRegion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.range.cmp(&other.range)
    }
}

impl ColorFontRegion {
    /// Construct a region with an explicit range, font pin, and color
    /// pin. All fields may be `None` except the range.
    pub fn new(range: Range, font: Option<AppFont>, color: Option<FloatRgba>) -> Self {
        ColorFontRegion { range, font, color }
    }
    /// Construct a keying-only region — no font, no color — suitable
    /// for `BTreeSet::get` / `remove` lookups by range alone.
    pub fn new_key_only(range: Range) -> Self {
        ColorFontRegion {
            range,
            font: None,
            color: None,
        }
    }

    /// Field-by-field equality, including the font and color pins
    /// that `Eq` deliberately ignores.
    ///
    /// `PartialEq` on this type is *set identity*: two regions with
    /// the same [`Range`] are the same region, whatever they are
    /// painted, which is what lets the owning `BTreeSet` find a
    /// stored region from a bare range. That makes `==` the wrong
    /// question for anyone asking "would this render differently?"
    /// — a recolored span compares equal to the one it replaced.
    /// This is that question. The renderer's overlay re-shape cache
    /// asks it on every frame of a color-picker hover, where the
    /// only thing that changes is a cell's color inside an unchanged
    /// range.
    ///
    /// Cost: three field compares, no allocation. The body
    /// destructures `self`, so a field added to this struct will not
    /// compile until it is accounted for here.
    pub fn same_content(&self, other: &Self) -> bool {
        let ColorFontRegion { range, font, color } = self;
        *range == other.range && *font == other.font && *color == other.color
    }

    /// Hash every field, including the font and color pins that
    /// [`Hash`] deliberately ignores.
    ///
    /// The hashing counterpart of [`Self::same_content`], and it
    /// exists for the same reason the comparison does: the derived
    /// `Hash` on this type hashes the [`Range`] alone, because it
    /// has to agree with a `PartialEq` that is set identity. A
    /// consumer summarising "would this render differently?" into
    /// one integer therefore cannot use `Hash` — a recolored span
    /// hashes to the value it replaced.
    ///
    /// Colors go in through `f32::to_bits`, which makes this a
    /// strictly *finer* equivalence than [`Self::same_content`]'s
    /// `==` rather than the same one: `==` calls `0.0` and `-0.0`
    /// equal and calls a `NaN` unequal to itself, and bits do
    /// neither. Finer is the safe direction for a reuse decision —
    /// it can cost a redundant update, never a skipped one — and
    /// neither value reaches a region from any resolver here, since
    /// colors arrive from hex parsing and palette lookup.
    ///
    /// Cost: three field hashes, no allocation. The body
    /// destructures `self`, so a field added to this struct will not
    /// compile until it is accounted for here.
    pub fn hash_content<H: Hasher>(&self, state: &mut H) {
        let ColorFontRegion { range, font, color } = self;
        range.hash(state);
        font.hash(state);
        color.map(|c| c.map(f32::to_bits)).hash(state);
    }
}

/// Ordered set of [`ColorFontRegion`]s keyed on `Range`. Acts as the
/// styled-span table for a single string; nearly every method on it
/// is a mutation primitive that keeps the set consistent under
/// insertion / deletion of characters in the backing text.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub struct ColorFontRegions {
    /// Sorted set of styled spans, keyed on
    /// [`ColorFontRegion::range`]. Stored in a `BTreeSet` so range
    /// lookup, insertion, and ordered iteration over overlapping
    /// spans are all O(log n). Direct mutation bypasses the
    /// [`RegionIndexer`](crate::gfx_structs::util::regions::RegionIndexer)
    /// — see CONVENTIONS §B6: writes go through the mutator
    /// pipeline, never through this field directly.
    pub regions: BTreeSet<ColorFontRegion>,
}

impl ColorFontRegions {
    /// Build a set from an existing `Vec<ColorFontRegion>`. Last-wins
    /// on duplicate ranges — the `BTreeSet` collect collapses them.
    pub fn new_from(source: Vec<ColorFontRegion>) -> Self {
        ColorFontRegions {
            regions: source.into_iter().collect(),
        }
    }
    /// Empty region set.
    pub fn new_empty() -> Self {
        ColorFontRegions {
            regions: BTreeSet::new(),
        }
    }

    /// Borrow every region as a `Vec<&ColorFontRegion>` (allocates the
    /// outer vec; the regions themselves are not copied).
    pub fn all_regions(&self) -> Vec<&ColorFontRegion> {
        self.regions.iter().collect()
    }

    /// Number of regions currently stored.
    pub fn num_regions(&self) -> usize {
        self.regions.len()
    }

    /// Whether two tables would style their text identically —
    /// same ranges, same font pins, same colors.
    ///
    /// The set-level counterpart to
    /// [`ColorFontRegion::same_content`], and needed for the same
    /// reason: `PartialEq` on this type reduces to `BTreeSet`
    /// equality, whose element equality is range identity, so
    /// `regions_a == regions_b` is true for two tables that paint
    /// the same spans in different colors. Any consumer deciding
    /// whether shaped output can be reused has to ask this one
    /// instead.
    ///
    /// Cost: O(n_regions) — a length check then a paired walk over
    /// two `BTreeSet`s, which iterate in `Range` order, so equal
    /// tables line up positionally. No allocation.
    pub fn same_content(&self, other: &Self) -> bool {
        self.regions.len() == other.regions.len()
            && self
                .regions
                .iter()
                .zip(other.regions.iter())
                .all(|(a, b)| a.same_content(b))
    }

    /// Hash the whole styling table — every range, font pin and
    /// color — into `state`.
    ///
    /// The set-level counterpart to
    /// [`ColorFontRegion::hash_content`], and the hashing
    /// counterpart of [`Self::same_content`]. The derived `Hash` on
    /// this type reduces to hashing the `BTreeSet`, whose element
    /// hash is range identity, so two tables that paint the same
    /// spans in different colors hash the same. A consumer folding
    /// "would this render differently?" into a signature has to
    /// call this one instead.
    ///
    /// The length is hashed first so `[a]` and `[a, b]` cannot
    /// collide through prefix ambiguity.
    ///
    /// Cost: O(n_regions) — one pass over a `BTreeSet`, which
    /// iterates in `Range` order, so tables that `same_content`
    /// each other are walked in the same order. No allocation. The
    /// body destructures `self`, so a field added to this struct
    /// will not compile until it is accounted for here — the same
    /// guarantee every other link in this hashing chain gives, and
    /// the property the callers' completeness argument rests on.
    pub fn hash_content<H: Hasher>(&self, state: &mut H) {
        let ColorFontRegions { regions } = self;
        regions.len().hash(state);
        for region in regions {
            region.hash_content(state);
        }
    }

    /// Build a `ColorFontRegions` containing a single region that
    /// covers `[0, cluster_count)` with the given `color` and `font`
    /// pin. Returns `new_empty()` when `cluster_count == 0` — the
    /// guard every caller used to write by hand. Used by app-crate
    /// `make_area` / `mk_area` factories that build a `GlyphArea`
    /// from one string and one color, so they don't have to
    /// open-code `new_empty + submit_region` three places.
    ///
    /// `cluster_count` is in grapheme clusters (the unit `Range`
    /// carries — see this struct's doc); callers typically derive
    /// it via `baumhard::util::grapheme_chad::count_grapheme_clusters`.
    /// Passing a char or byte count silently mis-styles ZWJ-emoji
    /// or combining-mark text.
    ///
    /// Costs: one `BTreeSet::insert`; no walks, no clones.
    pub fn single_span(cluster_count: usize, color: Option<FloatRgba>, font: Option<AppFont>) -> Self {
        if cluster_count == 0 {
            return Self::new_empty();
        }
        let mut out = Self::new_empty();
        out.regions
            .insert(ColorFontRegion::new(Range::new(0, cluster_count), font, color));
        out
    }

    /// Insert a region, replacing any existing region with the same
    /// key. An inverted range (`start > end`) is dropped with a
    /// warning rather than panicking — `submit_region` is reachable
    /// from interactive text-edit paths (the `Type` action in the
    /// app-level `text_edit` module), and CODE_CONVENTIONS.md §4 says
    /// those must not abort the editor over a single bad mutation.
    pub fn submit_region(&mut self, region: ColorFontRegion) {
        if region.range.start > region.range.end {
            warn!(
                "submit_region dropped inverted range {}..{}",
                region.range.start, region.range.end
            );
            return;
        }
        if self.regions.contains(&region) {
            self.regions.remove(&region);
        }
        self.regions.insert(region);
    }

    /// Insertion primitive for the "split around the insertion, do not
    /// absorb it" policy: `range.magnitude()` grapheme clusters were
    /// inserted at `range.start`, and the inserted span is to be left
    /// *uncovered* so the caller can style it with a follow-up
    /// [`Self::submit_region`]. Semantics, per region relative to the
    /// insertion point `range.start`:
    ///
    /// - **Fully left** (`end <= range.start`): unchanged.
    /// - **At or right of the insertion point** (`start >= range.start`):
    ///   both bounds shift right by `range.magnitude()`. A region that
    ///   *begins* at the insertion point is never split — it has
    ///   nothing on the left of the cut, so splitting it would emit an
    ///   empty `[start, start)` husk and an inverted right half.
    /// - **Straddling** (`start < range.start < end`): splits in two.
    ///   The head keeps `[start, range.start)`; the tail becomes
    ///   `[range.end, end + range.magnitude())` — the same graphemes it
    ///   covered before, pushed past the inserted span.
    ///
    /// An inverted `range` (`end < start`) is dropped with a warning
    /// rather than underflowing, matching [`Self::submit_region`] —
    /// this primitive is reachable from the same interactive
    /// mutation pipeline (CODE_CONVENTIONS.md §9). A zero-magnitude
    /// `range` is a no-op: nothing was inserted, so there is nothing
    /// to split around. A shift that would carry any region's `end`
    /// past `usize::MAX` is likewise dropped whole, leaving the set
    /// untouched rather than half-shifted.
    ///
    /// **Precondition: the region set is non-degenerate and
    /// non-overlapping.** Each region must satisfy `start < end`, and
    /// no two may cover the same cluster. The primitive is a
    /// per-region rewrite over a `BTreeSet` keyed on the range alone,
    /// so it cannot repair violations and will propagate them: an
    /// empty `[n, n)` region shifts to another empty region, and two
    /// regions that the rewrite maps onto the *same* range collapse
    /// into one set slot, silently dropping the loser's color and
    /// font. Its sibling [`Self::shift_regions_after`] carries the
    /// same precondition for the same reason. Callers building a
    /// region set from text runs satisfy it by construction; callers
    /// merging sets from elsewhere must enforce it themselves.
    ///
    /// It is one of the crate's three insertion primitives, and the
    /// three differ **only** in what they do to the regions that touch
    /// the insertion point. Every caller wants exactly one of the three
    /// combinations:
    ///
    /// | | region with `start == idx` | region with `end == idx` | region **straddling** `idx` |
    /// |---|---|---|---|
    /// | [`Self::shift_regions_after`] | stays | stays | stays |
    /// | [`Self::insert_regions_at`] | shifts | **absorbs** the new cells | **absorbs** the new cells |
    /// | `split_and_separate` | shifts | stays | **splits around them** |
    ///
    /// (`idx` is `range.start` here; the two siblings take the
    /// insertion point and the magnitude as separate arguments rather
    /// than as one `Range`. Away from those three positions all three
    /// primitives agree exactly — strictly left stays, strictly right
    /// shifts.)
    ///
    /// Reach for this one when the inserted cells belong to **no
    /// existing region**: structural filler that the caller will
    /// describe itself or leave undescribed. `insert_regions_at`'s
    /// absorption is for the typing case, where the new chars should
    /// inherit the run they were typed into; `shift_regions_after`'s
    /// "everything at or left of `idx` stays put" is for the overwrite
    /// case, where the cells at `idx` are being *replaced* rather than
    /// displaced, and the caller re-covers them itself.
    ///
    /// [`GlyphMatrix::place_in`](crate::gfx_structs::model::matrix::GlyphMatrix::place_in)
    /// is the caller at both of its insertion points: the blanks it
    /// pads a short row out to an x-offset with belong to the *next*
    /// row's indent and to no component, and the cells a component
    /// writes into an empty line tail displace a caller span anchored
    /// there rather than overwriting it.
    ///
    /// Costs: O(n) over existing regions; one `Vec` of the resulting
    /// regions plus one `BTreeSet` rebuild. Regions are `Copy`, so
    /// nothing deep-clones.
    pub fn split_and_separate(&mut self, range: Range) {
        if range.end < range.start {
            warn!(
                "split_and_separate dropped inverted range {}..{}",
                range.start, range.end
            );
            return;
        }
        let magnitude = range.magnitude();
        if magnitude == 0 {
            return;
        }
        // `+ 1` covers the single split that the common case performs;
        // pathological inputs with several straddlers simply grow.
        let mut updated: Vec<ColorFontRegion> = Vec::with_capacity(self.regions.len() + 1);
        for region in self.regions.iter() {
            // Both the shift and the split move `end` right by
            // `magnitude`, and `start <= end`, so this one check covers
            // every arm. Bailing here is safe precisely because
            // `self.regions` is not touched until the loop completes —
            // the set is left exactly as it was rather than half
            // shifted (§5: no half-state).
            let Some(shifted_end) = region.range.end.checked_add(magnitude) else {
                warn!(
                    "split_and_separate dropped: shifting region {}..{} right by {} overflows usize",
                    region.range.start, region.range.end, magnitude
                );
                return;
            };
            let mut head = *region;
            if head.range.start >= range.start {
                // The `checked_add` above already proved this cannot
                // fail, so the result is discarded deliberately rather
                // than re-reported.
                let _ = head.range.checked_push_right(magnitude);
            } else if head.range.end > range.start {
                let mut tail = *region;
                tail.range.start = range.end;
                tail.range.end = shifted_end;
                head.range.end = range.start;
                updated.push(tail);
            }
            // else: fully left of the insertion point, unchanged.
            updated.push(head);
        }
        self.regions.clear();
        self.regions.extend(updated);
    }

    /// Shift every region whose `start > idx` right by `magnitude`.
    /// Regions with `start <= idx` (including straddlers where
    /// `start <= idx < end`) are left untouched — their `end` does
    /// **not** extend to cover the inserted chars. That's the
    /// "replace-and-shift" semantics used by paths that pair each call
    /// with a follow-up `submit_region` for the newly-inserted span,
    /// so the surrounding region deliberately does not absorb the
    /// insertion.
    ///
    /// Callers that want the surrounding region to absorb the insertion
    /// (the text-editor caret, user typing) should instead use
    /// [`Self::insert_regions_at`], which extends straddling regions
    /// to cover the new chars, and shifts on `start >= idx` rather
    /// than `start > idx`. Callers that want neither — new cells that
    /// belong to no existing region, with a region anchored exactly at
    /// `idx` still moving and a straddler splitting around the new
    /// cells rather than swallowing them — want
    /// [`Self::split_and_separate`]. Its doc carries the full
    /// three-primitive table. See [`Self::shrink_regions_after`] for
    /// the delete path.
    ///
    /// **The `start == idx` seam is only defensible when the write
    /// overwrites.** Leaving a region anchored exactly at `idx` in
    /// place is right when the caller is *replacing* the cells that
    /// region covers and is about to re-cover them with its own
    /// `submit_region`. It is wrong when the write is a pure
    /// *insertion* — then nothing at `idx` was displaced, the region's
    /// text has moved right by `magnitude`, and leaving it behind
    /// parks it on cells it never described (and, if the follow-up
    /// `submit_region` uses the same range, evicts it outright, since
    /// the set is keyed on the range alone). This primitive cannot
    /// tell the two apart from `(idx, magnitude)`; the caller can, and
    /// must pick [`Self::split_and_separate`] for the insertion case.
    /// `GlyphMatrix::place_in` does exactly that.
    ///
    /// **The delete path is a companion, not a mirror, at that same
    /// seam.** A region anchored exactly at `idx` survives this shift
    /// untouched, while `shrink_regions_after` drops it, because there
    /// the cut *removed* the text it covered. Both are right for their
    /// direction; a caller pairing the two must not assume one from
    /// the other. `do_region_shift_and_shrink_disagree_at_the_seam`
    /// pins it.
    ///
    /// A shift that would carry a region's `end` past `usize::MAX` is
    /// dropped whole, leaving the set untouched rather than partly
    /// shifted — same posture as [`Self::split_and_separate`].
    ///
    /// **Precondition: the region set is non-degenerate and
    /// non-overlapping** — each region satisfies `start < end`, and no
    /// two cover the same cluster. Like every primitive here this is a
    /// per-region rewrite over a `BTreeSet` keyed on the range alone,
    /// so it cannot detect a violation and will propagate it: two
    /// regions the shift maps onto the same range collapse into one
    /// set slot, silently dropping the loser's color and font. See
    /// [`Self::split_and_separate`], which carries the same
    /// precondition for the same reason.
    ///
    /// Costs: O(n) over existing regions; one full clone of the
    /// BTreeSet to decouple from the iterator.
    pub fn shift_regions_after(&mut self, idx: usize, magnitude: usize) {
        let mut copy_of_regions: Vec<_> = self.regions.iter().copied().collect();
        for region in &mut copy_of_regions {
            if region.range.start > idx && !region.range.checked_push_right(magnitude) {
                warn!(
                    "shift_regions_after dropped: shifting region {}..{} right by {} overflows usize",
                    region.range.start, region.range.end, magnitude
                );
                return;
            }
        }
        self.regions.clear();
        self.regions.extend(copy_of_regions);
    }

    /// Text-edit insertion primitive: `magnitude` chars were inserted
    /// at position `idx` in the backing text; rewrite the region ranges
    /// to reflect that so the inserted chars inherit the surrounding
    /// run's color / `AppFont`. Semantics, per region:
    ///
    /// - **Fully left** (`end < idx`, or `end == idx` with no absorption):
    ///   unchanged.
    /// - **Fully right** (`start >= idx`): both bounds shifted right by
    ///   `magnitude`.
    /// - **Straddling or left-adjacent** (`start < idx` and `end >= idx`):
    ///   `end` grows by `magnitude` so the region absorbs the inserted
    ///   chars. Exactly one such region is extended (the first found).
    ///
    /// Returns `true` if some region absorbed the insertion, `false`
    /// if the new chars are now uncovered (e.g. inserting at `idx == 0`
    /// into a region-less area, or at a position no region touches).
    /// The text-editor caret path uses this return value to decide
    /// whether to `set_or_insert` a fresh region for the caret glyph
    /// so it renders even in an empty-buffer node.
    ///
    /// Contrast with [`Self::shift_regions_after`], whose "replace
    /// and shift" semantics leave straddling regions in place — that
    /// primitive exists for `GlyphMatrix::copy_from`, which explicitly
    /// follows up with a `submit_region` for the inserted span — and
    /// with [`Self::split_and_separate`], which shares this
    /// primitive's `start >= idx` shift but does **not** absorb: a
    /// left-adjacent region stays put and a straddler splits around
    /// the new cells instead of swallowing them. That doc carries the
    /// full three-primitive table. Absorption is right when the new
    /// chars were typed into an existing run and should inherit it; it
    /// is wrong when they are structural filler that belongs to no
    /// run, because the left-adjacent run then grows over cells it
    /// never covered.
    ///
    /// A shift or absorption that would carry a region's `end` past
    /// `usize::MAX` drops the call whole — the set is left untouched
    /// and `false` returned, so the caller treats the insertion as
    /// unabsorbed rather than acting on a half-shifted table. Same
    /// posture as [`Self::split_and_separate`] and
    /// [`Self::shift_regions_after`].
    ///
    /// **Precondition: the region set is non-degenerate and
    /// non-overlapping** — see [`Self::split_and_separate`], which
    /// carries the same precondition for the same reason.
    ///
    /// Costs: O(n) over existing regions; one collect + one extend in
    /// the common case, plus a remove/submit pair when a region
    /// absorbs the insertion.
    pub fn insert_regions_at(&mut self, idx: usize, magnitude: usize) -> bool {
        if magnitude == 0 {
            return false;
        }
        let mut updated: Vec<ColorFontRegion> = Vec::with_capacity(self.regions.len());
        let mut absorbed = false;
        for region in self.regions.iter() {
            let mut r = *region;
            if r.range.start >= idx {
                // Fully right of the insertion — shift both bounds.
                if !r.range.checked_push_right(magnitude) {
                    warn!(
                        "insert_regions_at dropped: shifting region {}..{} right by {} overflows usize",
                        r.range.start, r.range.end, magnitude
                    );
                    return false;
                }
            } else if !absorbed && r.range.end >= idx {
                // First straddling / left-adjacent region — absorb.
                let Some(end) = r.range.end.checked_add(magnitude) else {
                    warn!(
                        "insert_regions_at dropped: growing region {}..{} by {} overflows usize",
                        r.range.start, r.range.end, magnitude
                    );
                    return false;
                };
                r.range.end = end;
                absorbed = true;
            }
            // else: fully left of the insertion, unchanged.
            updated.push(r);
        }
        self.regions.clear();
        self.regions.extend(updated);
        absorbed
    }

    /// Delete-path companion to [`Self::shift_regions_after`] — a
    /// companion rather than a mirror: at the `start == idx` seam the
    /// two deliberately differ, and that doc comment says why.
    /// `magnitude` chars starting at position `idx` have been removed
    /// from the backing text; rewrite the region ranges to reflect
    /// that. Semantics, per region relative to the cut `[idx, idx+magnitude)`:
    ///
    /// - **Fully left** (`end <= idx`): unchanged.
    /// - **Fully right** (`start >= idx + magnitude`): both bounds
    ///   shifted left by `magnitude`.
    /// - **Fully inside** (`idx <= start` and `end <= idx+magnitude`):
    ///   collapses, removed from the set.
    /// - **Spans the cut** (`start < idx` and `end > idx+magnitude`):
    ///   `end` shrinks by `magnitude`; the region absorbs the deletion.
    /// - **Left-partial** (`start < idx` and `idx < end <= idx+magnitude`):
    ///   `end` clamps to `idx`.
    /// - **Right-partial** (`idx <= start < idx+magnitude` and
    ///   `end > idx+magnitude`): `start` clamps to `idx`, `end` shifts
    ///   left by `magnitude` so the region sits flush against the
    ///   remaining-text boundary.
    ///
    /// Used by the text-edit path's backspace / delete handlers (the
    /// app-level `text_edit` module) to keep per-run color and
    /// `AppFont` pins intact across character deletion instead of
    /// rebuilding the region set from a heuristic.
    ///
    /// Costs: O(n) over existing regions; one collect + one extend.
    pub fn shrink_regions_after(&mut self, idx: usize, magnitude: usize) {
        if magnitude == 0 {
            return;
        }
        let end_of_cut = idx + magnitude;
        let mut updated: Vec<ColorFontRegion> = Vec::with_capacity(self.regions.len());
        for region in self.regions.iter() {
            let mut r = *region;
            if r.range.end <= idx {
                updated.push(r);
            } else if r.range.start >= end_of_cut {
                r.range.start -= magnitude;
                r.range.end -= magnitude;
                updated.push(r);
            } else if r.range.start >= idx && r.range.end <= end_of_cut {
                // Fully inside the cut — collapse, drop.
            } else if r.range.start < idx && r.range.end > end_of_cut {
                // Spans the cut — absorb the deletion.
                r.range.end -= magnitude;
                updated.push(r);
            } else if r.range.start < idx {
                // Left-partial — clamp end to idx.
                r.range.end = idx;
                updated.push(r);
            } else {
                // Right-partial — clamp start to idx, shift end left.
                r.range.start = idx;
                r.range.end -= magnitude;
                updated.push(r);
            }
        }
        self.regions.clear();
        self.regions.extend(updated);
    }

    /// Replace the entire region set with a copy of `regions`.
    /// O(n) in `regions.regions.len()`.
    pub fn replace_regions(&mut self, regions: &Self) {
        self.regions.clear();
        for region in &regions.regions {
            self.regions.insert(*region);
        }
    }

    /// Merge `region` into the set: if a region with the same range
    /// already exists, its color / font fields are overwritten by any
    /// `Some(_)` fields in `region` (leaving the rest intact). If no
    /// match exists, the region is inserted as-is.
    pub fn set_or_insert(&mut self, region: &ColorFontRegion) {
        // One lookup rather than a `contains` whose only use was to
        // license the `get` that repeats it.
        if let Some(existing) = self.regions.get(region) {
            let mut new_region = *existing;
            if region.color.is_some() {
                new_region.color = region.color;
            }
            if region.font.is_some() {
                new_region.font = region.font;
            }
            self.submit_region(new_region);
        } else {
            self.regions.insert(*region);
        }
    }

    /// Look up the region whose range exactly matches `range`.
    /// Returns `None` when no such region exists.
    pub fn get(&self, range: Range) -> Option<&ColorFontRegion> {
        self.regions.get(&ColorFontRegion::new_key_only(range))
    }

    /// Test-only convenience: like [`Self::get`] but copies the region
    /// out and panics if the range is not present, with the full region
    /// table dumped to `debug!` first to ease assertion debugging. **Not for
    /// interactive paths** — production callers must use [`Self::get`]
    /// and handle the `None` arm; CODE_CONVENTIONS §7 forbids panics
    /// after the first frame.
    ///
    /// Hidden from public docs because it is a test helper, not part of the
    /// supported API surface.
    #[doc(hidden)]
    pub fn hard_get(&self, range: Range) -> ColorFontRegion {
        debug!("hard_get({}..{}); current regions:", range.start, range.end);
        for r in self.regions.iter() {
            debug!("  {}..{}", r.range.start, r.range.end);
        }
        *self
            .regions
            .get(&ColorFontRegion::new_key_only(range))
            .expect("hard_get: requested range is not present in this region table")
    }

    /// Remove the region keyed on `range`. Returns `true` if a match
    /// was found and removed.
    pub fn remove_range(&mut self, range: Range) -> bool {
        self.remove(&ColorFontRegion::new_key_only(range))
    }

    /// Remove the region whose range matches `region`'s. Color / font
    /// payloads are ignored for the match — only the range is the key.
    pub fn remove(&mut self, region: &ColorFontRegion) -> bool {
        self.regions.remove(region)
    }
}

impl Default for ColorFontRegions {
    fn default() -> Self {
        ColorFontRegions::new_empty()
    }
}

use strum_macros::{Display, EnumString};

/// Selects the arithmetic used when a [`DeltaGlyphArea`](crate::gfx_structs::area::DeltaGlyphArea)
/// is applied to a [`GlyphArea`](crate::gfx_structs::area::GlyphArea).
/// Every field delta in the same `DeltaGlyphArea` shares one
/// `ApplyOperation`, so the caller chooses "add this offset" vs.
/// "replace outright" vs. "remove" once for the whole batch.
#[derive(Clone, Copy, Eq, PartialEq, Debug, EnumString, Display, Serialize, Deserialize)]
pub enum ApplyOperation {
    /// Additive merge: `target += delta`. For numeric fields this is
    /// ordinary addition; for text it is concatenation; for region
    /// sets it submits (merges) each delta region into the existing
    /// set.
    Add,
    /// Wholesale replacement: `target = delta`. The previous value is
    /// discarded entirely.
    Assign,
    /// Reset to default: `target = T::default()`. The delta's payload
    /// is ignored — the semantic is "clear whatever is there."
    Delete,
    /// Subtractive merge: `target -= delta`. For numeric fields this
    /// is ordinary subtraction; for region sets it removes each
    /// matching delta region from the existing set.
    Subtract,
    /// Component-wise multiplication: `target *= delta`. Meaningful
    /// for numeric fields; not currently defined for text or region
    /// sets.
    Multiply,
    /// Identity / skip: do nothing. Useful as a sentinel when a
    /// mutator must carry an operation variant but the caller does
    /// not want any effect.
    Noop,
}

impl ApplyOperation {
    /// Apply this operation to `lhs`, using `rhs` as the delta. `T`
    /// must support all four arithmetic assigns so every variant is
    /// expressible against a single generic bound.
    pub fn apply<T: AddAssign<T> + MulAssign<T> + SubAssign<T> + Default>(&self, lhs: &mut T, rhs: T) {
        match self {
            ApplyOperation::Add => *lhs += rhs,
            ApplyOperation::Assign => *lhs = rhs,
            ApplyOperation::Subtract => *lhs -= rhs,
            ApplyOperation::Multiply => *lhs *= rhs,
            ApplyOperation::Noop => {}
            ApplyOperation::Delete => *lhs = T::default(),
        }
    }

    /// [`Self::apply`] for callers that only hold a *borrowed* delta
    /// payload — the shape every `apply_to(&self, target)` impl is in,
    /// since a delta is applied through a shared reference and can be
    /// applied many times.
    ///
    /// # Costs
    ///
    /// `Noop` and `Delete` ignore the payload entirely, so this
    /// clones **only** on the arms that actually consume `rhs`
    /// (`Add` / `Assign` / `Subtract` / `Multiply`). That matters on
    /// heap-owning payloads such as
    /// [`GlyphMatrix`](crate::gfx_structs::model::GlyphMatrix): a
    /// `Noop` or `Delete` delta used to deep-copy the whole matrix
    /// before throwing it away. Everything else costs exactly one
    /// `T::clone()` — the unavoidable price of moving an owned value
    /// out of a shared borrow.
    pub fn apply_ref<T: AddAssign<T> + MulAssign<T> + SubAssign<T> + Default + Clone>(
        &self,
        lhs: &mut T,
        rhs: &T,
    ) {
        match self {
            ApplyOperation::Noop => {}
            ApplyOperation::Delete => *lhs = T::default(),
            ApplyOperation::Add
            | ApplyOperation::Assign
            | ApplyOperation::Subtract
            | ApplyOperation::Multiply => self.apply(lhs, rhs.clone()),
        }
    }
}

/// Half-open index range `[start, end)`. Used as the key type on
/// [`ColorFontRegion`] and any other span-of-chars payload in the
/// core model. Totally ordered for `BTreeSet` storage.
#[derive(Clone, Copy, PartialOrd, Ord, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Range {
    /// Inclusive start index in grapheme clusters (not bytes, not
    /// chars). User-derived offsets land here.
    pub start: usize,
    /// Exclusive end index in grapheme clusters. `end >= start`
    /// must hold; inverted ranges are dropped with a warning by
    /// [`ColorFontRegions::submit_region`].
    pub end: usize,
}

impl Range {
    /// Construct a range from a `(start, end)` tuple.
    pub fn tup(range: (usize, usize)) -> Self {
        Range {
            start: range.0,
            end: range.1,
        }
    }
    /// Construct a range from explicit `start` and `end` components.
    pub fn new(start: usize, end: usize) -> Self {
        Range { start, end }
    }

    /// Convert to a `std::ops::Range<usize>` so it can be used for
    /// slice indexing and iteration.
    pub fn to_rust_range(&self) -> std::ops::Range<usize> {
        self.start..self.end
    }

    /// `end - start`. Does not check for underflow; an inverted range
    /// will panic in debug and underflow in release.
    pub fn magnitude(&self) -> usize {
        self.end - self.start
    }

    /// Shift both endpoints right by `n`, returning `false` and
    /// leaving the range **untouched** when that would carry `end`
    /// past `usize::MAX`.
    ///
    /// The checked form is the only form. Every caller shifts a range
    /// whose bounds came from a document — grapheme offsets a
    /// hand-authored `.mindmap.json` can set to anything — and the
    /// unchecked predecessor wrapped silently in release (the
    /// workspace sets no `overflow-checks` override), turning a
    /// far-right region into a far-left one. Since `start <= end` is
    /// this type's invariant, checking `end` alone is sufficient.
    ///
    /// Costs: O(1), one `checked_add`, no allocation.
    #[must_use]
    pub fn checked_push_right(&mut self, n: usize) -> bool {
        let Some(end) = self.end.checked_add(n) else {
            return false;
        };
        self.start += n;
        self.end = end;
        true
    }

    /// Returns `true` iff the two half-open ranges share at least one
    /// index. Touching endpoints (`a.end == b.start`) do *not* overlap.
    pub fn overlaps(&self, other: &Self) -> bool {
        if self.start >= other.end || other.start >= self.end {
            return false;
        }
        true
    }
}

/// Helper for types that can have [`Flag`]s toggled on and off.
/// Implementers typically back this with a bitset.
pub trait Flaggable {
    /// `true` iff `flag` is currently set on `self`. O(1) on
    /// hash-set / bitset implementations.
    fn flag_is_set(&self, flag: Flag) -> bool;
    /// Set `flag` on `self`. Idempotent: setting an already-set
    /// flag is a no-op. O(1) on hash-set / bitset implementations.
    fn set_flag(&mut self, flag: Flag);
    /// Clear `flag` from `self`. Idempotent: clearing an unset
    /// flag is a no-op. O(1) on hash-set / bitset implementations.
    fn clear_flag(&mut self, flag: Flag);
}

/// Uniform access to a value's payload-free discriminant tag.
///
/// Every tagged enum in the mutation pipeline — [`GfxElement`](crate::gfx_structs::element::GfxElement),
/// [`GfxMutator`](crate::gfx_structs::mutator::GfxMutator),
/// [`Mutation`](crate::gfx_structs::mutator::Mutation),
/// [`GlyphAreaField`](crate::gfx_structs::area::GlyphAreaField),
/// [`GlyphModelField`](crate::gfx_structs::model::GlyphModelField),
/// [`GlyphAreaCommand`](crate::gfx_structs::area::GlyphAreaCommand),
/// [`GlyphModelCommand`](crate::gfx_structs::model::GlyphModelCommand)
/// — pairs with a `*Type` tag enum generated by strum's
/// `EnumDiscriminants` derive. This trait is the one name the whole
/// crate uses to get from a value to its tag, and it carries the
/// single `same_type` implementation those enums used to each
/// hand-write.
///
/// Implemented by a blanket impl over [`strum::IntoDiscriminant`], so
/// deriving `EnumDiscriminants` (with a tag that is
/// `Copy + Eq + Hash + Debug`) is the *only* step needed — there is
/// no per-enum boilerplate to keep in sync.
///
/// # Costs
///
/// [`Self::variant`] is a single-branch match returning a
/// fieldless enum, forwarding to strum's own `#[inline]`
/// `discriminant()`; no allocation on either method.
/// [`Self::same_type`] is `#[inline]` — it is the tight-loop
/// predicate that carried that attribute before it moved here.
pub trait Discriminated {
    /// The payload-free tag enum listing the same variants as `Self`.
    /// Bounded so it can key an `FxHashMap` (as
    /// [`Delta`](crate::gfx_structs::delta::Delta) does) without
    /// paying the payload's equality or allocation cost.
    type Discriminant: Copy + Eq + Hash + Debug;

    /// This value's tag. O(1), no allocation.
    fn variant(&self) -> Self::Discriminant;

    /// Whether `self` and `other` are the same variant, ignoring
    /// payload. O(1), no allocation.
    #[inline]
    fn same_type(&self, other: &Self) -> bool {
        self.variant() == other.variant()
    }
}

impl<T> Discriminated for T
where
    T: IntoDiscriminant,
    <T as IntoDiscriminant>::Discriminant: Copy + Eq + Hash + Debug,
{
    type Discriminant = <T as IntoDiscriminant>::Discriminant;

    fn variant(&self) -> Self::Discriminant {
        // No `#[inline]` here: this PR deletes two `#[inline]`s for
        // lacking a benchmark that resolves them (§B7), and adding a
        // third on the same terms would be incoherent. The forwardee
        // — strum's generated `discriminant()` — is already
        // `#[inline]`, so there is nothing to reinstate until a quiet
        // machine says otherwise.
        self.discriminant()
    }
}

/// Something that can be applied (merged) into a `T`. The mutation
/// pipeline's dispatch trait — every delta type implements it against
/// its target.
pub trait Applicable<T: Clone> {
    /// Merge `self`'s change into `target` in place. The single
    /// dispatch contract for the entire mutation pipeline: every
    /// `MutatorTree` walk, every `DeltaGlyphArea` / `DeltaGlyphModel`
    /// merge, and every `GfxMutator` evaluation eventually bottoms
    /// out in a call to this method (see CONVENTIONS §B2 — mutate,
    /// don't rebuild). Implementations must keep `target`'s
    /// invariants intact (sorted region sets, region-index
    /// consistency, AABB cache invalidation) and must not allocate
    /// a fresh arena to apply a single field change. Cost varies
    /// by impl; the contract is "cheaper than cloning the target".
    fn apply_to(&self, target: &mut T);
}

/// Element flags used by the UI / interaction layer. Seeded with the
/// minimum needed for focus, mutability, anchoring, and event
/// generation; new flags are added as interactions grow.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub enum Flag {
    /// Element holds keyboard / interaction focus. The renderer
    /// uses this to highlight the focused area, and the input
    /// router routes typed text into the focused node.
    Focused,
    /// Element accepts user-driven mutation. Read-only nodes
    /// (legend chrome, headers) leave this unset so the editing
    /// paths skip them.
    Mutable,
    /// Element is layout-pinned. Carries the [`AnchorBox`]
    /// describing which corners / edges are constrained against
    /// which target.
    Anchored(AnchorBox),
    /// Reserved: when implemented, mutations on this element will
    /// also emit a corresponding event to its subscribers. Not yet
    /// emitted by the walker today.
    MutationEvents,
    /// Marks every element belonging to a *section* subtree — both
    /// the section-area `GlyphArea` and the structural section-model
    /// `GlyphModel` grandchild that sits beneath it (see
    /// [`crate::mindmap::model::MindSection`]). The flag is *not*
    /// limited to the literal "root" of a section despite the
    /// variant name; spreading it across both elements lets
    /// arena-walking consumers (`apply_drag_delta`,
    /// `apply_tree_highlights`) recurse on a flag check without
    /// re-deriving section identity per step. The container area
    /// of a mindmap node and any child mind-node areas inside the
    /// same parent are *not* flagged.
    ///
    /// Click-routing, scene-builder iteration, and per-section
    /// style mutations filter on this flag. Flag-only — payload-
    /// free; the section's index inside its owning node lives on
    /// the document-side
    /// [`crate::mindmap::tree_builder::MindMapTree`] reverse-lookup
    /// tables, not on the flag itself.
    SectionRoot,
}

/// Up to four [`Anchor`]s applied to a single element, describing
/// which corners / edges are pinned. Larger variants imply more
/// constraints on the layout solver.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub enum AnchorBox {
    /// One pin only — element is constrained at a single point;
    /// the rest of its position is free for the layout solver.
    Single(Anchor),
    /// Two pins — sufficient to lock position and one axis of
    /// scale, e.g. left and right edges glued to siblings.
    Dual(Anchor, Anchor),
    /// Three pins — locks position plus one axis of scale and
    /// rotation; one degree of freedom remains.
    Trio(Anchor, Anchor, Anchor),
    /// Four pins, one per corner — the element is fully
    /// constrained; the layout solver has no freedom left.
    Full(Anchor, Anchor, Anchor, Anchor),
}

/// A three-way positional constraint: a target (parent / window /
/// world / …), the point on the target this element pins to, and the
/// point on the element itself that meets it.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct Anchor {
    target: AnchorTarget,
    self_point: AnchorPoint,
    target_point: AnchorPoint,
}

impl Anchor {
    /// Construct an anchor with full control over target, target
    /// point, and self point.
    pub fn new(target: AnchorTarget, target_point: AnchorPoint, self_point: AnchorPoint) -> Self {
        Anchor {
            target,
            self_point,
            target_point,
        }
    }

    /// Pin `self_point` on this element to `parent_point` on its
    /// immediate parent.
    pub fn on_parent(parent_point: AnchorPoint, self_point: AnchorPoint) -> Self {
        Anchor {
            target: AnchorTarget::Parent { generation_offset: 0 },
            self_point,
            target_point: parent_point,
        }
    }

    /// Pin `self_point` on this element to `window_point` in window
    /// coordinates.
    pub fn on_window(window_point: AnchorPoint, self_point: AnchorPoint) -> Self {
        Anchor {
            target: AnchorTarget::Window,
            self_point,
            target_point: window_point,
        }
    }

    /// Pin `self_point` on this element to `world_point` in world
    /// coordinates.
    pub fn in_world(world_point: AnchorPoint, self_point: AnchorPoint) -> Self {
        Anchor {
            target: AnchorTarget::World,
            self_point,
            target_point: world_point,
        }
    }
}

impl Default for Anchor {
    /// Center-on-parent: the element's center is pinned to its
    /// immediate parent's center with zero pixel offset.
    ///
    /// The three-way constraint is:
    /// 1. **Target** — `AnchorTarget::Parent { generation_offset: 0 }`
    ///    (the immediate parent, not a grandparent).
    /// 2. **Target point** — `AnchorPoint::Center(0)` (the parent's
    ///    geometric center, no pixel nudge).
    /// 3. **Self point** — `AnchorPoint::Center(0)` (the element's own
    ///    center, no pixel nudge).
    ///
    /// Together these mean "stack my center on my parent's center" —
    /// the most common starting layout for new tree nodes before the
    /// scene builder repositions them.
    fn default() -> Self {
        Anchor::new(
            AnchorTarget::Parent { generation_offset: 0 },
            AnchorPoint::Center(0),
            AnchorPoint::Center(0),
        )
    }
}

/// What an [`Anchor`] is pinned *to*. `Parent` / `Child` walk the
/// tree by `generation_offset` or `child_num`; `Window` / `Display` /
/// `World` use global coordinate systems.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub enum AnchorTarget {
    /// Pin to an ancestor. `generation_offset == 0` is the
    /// immediate parent, `1` is the grandparent, and so on.
    Parent { generation_offset: usize },
    /// Pin to a specific child by sibling index. Resolved at
    /// layout time against the anchored element's children list.
    Child { child_num: usize },
    /// Pin to the OS window's coordinate space — moves with the
    /// window but ignores camera pan and zoom.
    Window,
    /// Pin to the physical display's coordinate space — survives
    /// window moves on multi-monitor setups.
    Display,
    /// Pin in world (canvas) coordinates — moves with camera pan
    /// and zoom but is independent of any element.
    World,
}

/// Named point on a rectangular region — the nine cardinal positions
/// plus a pixel-offset parameter so callers can nudge a pin without
/// adding a fresh variant. The inner `i16` on every variant is a
/// signed pixel offset whose axis interpretation is the consumer's
/// to choose (no current consumer fixes a convention; the
/// named-trajectory plugin / animation surfaces will pin the
/// semantics when they land).
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub enum AnchorPoint {
    /// Top-left corner of the rectangle, plus `i16` pixel offset.
    TopLeft(i16),
    /// Midpoint of the top edge, plus `i16` pixel offset.
    TopCenter(i16),
    /// Top-right corner of the rectangle, plus `i16` pixel offset.
    TopRight(i16),
    /// Bottom-left corner of the rectangle, plus `i16` pixel offset.
    BotLeft(i16),
    /// Midpoint of the bottom edge, plus `i16` pixel offset.
    BotCenter(i16),
    /// Bottom-right corner of the rectangle, plus `i16` pixel offset.
    BotRight(i16),
    /// Midpoint of the left edge, plus `i16` pixel offset.
    LeftCenter(i16),
    /// Midpoint of the right edge, plus `i16` pixel offset.
    RightCenter(i16),
    /// Geometric center of the rectangle, plus `i16` pixel offset.
    /// The default starting pin for new tree nodes.
    Center(i16),
}
