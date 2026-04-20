//! Core primitives shared across the Baumhard crate.
//!
//! This module defines the foundational data types that every higher-level
//! abstraction in Baumhard rests on: character-range colour/font regions
//! (`ColorFontRegions`, `ColorFontRegion`), the arithmetic operation
//! enum (`ApplyOperation`), spatial anchoring (`Anchor`, `AnchorPoint`,
//! `AnchorTarget`), element flags (`Flag`), and the `Applicable` trait
//! that the mutation pipeline dispatches through.
//!
//! Nothing in this module touches the GPU, the font system, or the arena
//! — it is pure data + O(n)-or-better algorithms over sorted sets.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use std::hash::{Hash, Hasher};
use std::ops::{AddAssign, MulAssign, SubAssign};
use log::{debug, warn};
use serde::{Deserialize, Serialize};

use crate::font::fonts::AppFont;
use crate::util::color::FloatRgba;

/// One contiguous span of text with an optional colour and font
/// pin. Identified by its [`Range`]; `Eq` / `Hash` only inspect the
/// range so two regions with the same bounds collide in the owning
/// [`ColorFontRegions`] set regardless of colour / font.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ColorFontRegion {
    pub range: Range,
    pub font: Option<AppFont>,
    pub color: Option<FloatRgba>,
}

impl Hash for ColorFontRegion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Only the range is hashed — the set semantics key on it alone
        // so a lookup by bare range can find a stored region with any
        // colour / font payload.
        self.range.hash(state);
    }
}

/// Field selector for a [`ColorFontRegion`]. Used by mutation paths
/// that update a single facet of an existing region.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ColorFontRegionField {
    Range(Range),
    Font(AppFont),
    Color(FloatRgba),
    This,
}

impl PartialOrd for ColorFontRegion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.range.partial_cmp(&other.range)
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
    /// Construct a region with an explicit range, font pin, and colour
    /// pin. All fields may be `None` except the range.
    pub fn new(range: Range, font: Option<AppFont>, color: Option<FloatRgba>) -> Self {
        ColorFontRegion { range, font, color }
    }
    /// Construct a keying-only region — no font, no colour — suitable
    /// for `BTreeSet::get` / `remove` lookups by range alone.
    pub fn new_key_only(range: Range) -> Self {
        ColorFontRegion {
            range,
            font: None,
            color: None,
        }
    }
}

/// Ordered set of [`ColorFontRegion`]s keyed on `Range`. Acts as the
/// styled-span table for a single string; nearly every method on it
/// is a mutation primitive that keeps the set consistent under
/// insertion / deletion of characters in the backing text.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub struct ColorFontRegions {
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

    /// Build a `ColorFontRegions` containing a single region that
    /// covers `[0, char_count)` with the given `color` and `font`
    /// pin. Returns `new_empty()` when `char_count == 0` — the guard
    /// every caller used to write by hand. Used by app-crate
    /// `make_area` / `mk_area` factories that build a `GlyphArea`
    /// from one string and one color, so they don't have to
    /// open-code `new_empty + submit_region` three places.
    ///
    /// Costs: one `BTreeSet::insert`; no walks, no clones.
    pub fn single_span(char_count: usize, color: Option<FloatRgba>, font: Option<AppFont>) -> Self {
        if char_count == 0 {
            return Self::new_empty();
        }
        let mut out = Self::new_empty();
        out.regions.insert(ColorFontRegion::new(
            Range::new(0, char_count),
            font,
            color,
        ));
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

    /// Split any region overlapping `range` at `range.start`, push the
    /// second half forward past `range.end`, and shift every region
    /// strictly right of `range.end` by `range.magnitude()`.
    ///
    /// Costs: O(n) over existing regions plus one full vec clone of
    /// the set.
    pub fn split_and_separate(&mut self, range: Range) {
        let mut copy_of_regions: Vec<_> = self.regions.iter().copied().collect();
        let mut cloned_regions: Vec<ColorFontRegion> = Vec::new();
        for region in &mut copy_of_regions {
            if range.overlaps(&region.range) {
                let mut right_part = region.clone();
                right_part.range.start = range.end;
                right_part.range.end += range.magnitude();
                cloned_regions.push(right_part);
                region.range.end = range.start;
            }
            if region.range.start >= range.end {
                region.range.push_right(range.magnitude());
            }
        }
        self.regions.clear();
        self.regions.extend(copy_of_regions);
        self.regions.extend(cloned_regions);
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
    /// to cover the new chars. See the symmetric
    /// [`Self::shrink_regions_after`] for the delete path.
    ///
    /// Costs: O(n) over existing regions; one full clone of the
    /// BTreeSet to decouple from the iterator.
    pub fn shift_regions_after(&mut self, idx: usize, magnitude: usize) {
        let mut copy_of_regions: Vec<_> = self.regions.iter().copied().collect();
        for region in &mut copy_of_regions {
            if region.range.start > idx {
                region.range.start += magnitude;
                region.range.end += magnitude;
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
    /// follows up with a `submit_region` for the inserted span.
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
                r.range.start += magnitude;
                r.range.end += magnitude;
            } else if !absorbed && r.range.end >= idx {
                // First straddling / left-adjacent region — absorb.
                r.range.end += magnitude;
                absorbed = true;
            }
            // else: fully left of the insertion, unchanged.
            updated.push(r);
        }
        self.regions.clear();
        self.regions.extend(updated);
        absorbed
    }

    /// Symmetric delete-path companion to [`Self::shift_regions_after`].
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
    /// already exists, its colour / font fields are overwritten by any
    /// `Some(_)` fields in `region` (leaving the rest intact). If no
    /// match exists, the region is inserted as-is.
    pub fn set_or_insert(&mut self, region: &ColorFontRegion) {
        if self.regions.contains(region) {
            let mut new_region = self.regions.get(region).unwrap().clone();
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

    /// Remove the region whose range matches `region`'s. Colour / font
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

use strum_macros::{EnumString, Display};
use crate::util::ordered_vec2::OrderedVec2;

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
    pub fn apply<T: AddAssign<T> + MulAssign<T> + SubAssign<T> + Default>(
        &self,
        lhs: &mut T,
        rhs: T,
    ) {
        match self {
            ApplyOperation::Add => *lhs += rhs,
            ApplyOperation::Assign => *lhs = rhs,
            ApplyOperation::Subtract => *lhs -= rhs,
            ApplyOperation::Multiply => *lhs *= rhs,
            ApplyOperation::Noop => {}
            ApplyOperation::Delete => *lhs = T::default(),
        }
    }
}

/// Half-open index range `[start, end)`. Used as the key type on
/// [`ColorFontRegion`] and any other span-of-chars payload in the
/// core model. Totally ordered for `BTreeSet` storage.
#[derive(Clone, Copy, PartialOrd, Ord, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Range {
    pub start: usize,
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

    /// Shift both endpoints right by `n`.
    pub fn push_right(&mut self, n: usize) {
        self.start += n;
        self.end += n;
    }

    /// Shift both endpoints left by `n`.
    pub fn push_left(&mut self, n: usize) {
        self.start -= n;
        self.end -= n;
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
    fn flag_is_set(&self, flag: Flag) -> bool;
    fn set_flag(&mut self, flag: Flag);
    fn clear_flag(&mut self, flag: Flag);
}

/// Something that can be applied (merged) into a `T`. The mutation
/// pipeline's dispatch trait — every delta type implements it against
/// its target.
pub trait Applicable<T: Clone> {
    fn apply_to(&self, target: &mut T);
}

/// Element flags used by the UI / interaction layer. Seeded with the
/// minimum needed for focus, mutability, anchoring, and event
/// generation; new flags are added as interactions grow.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub enum Flag {
    Focused,
    Mutable,
    Anchored(AnchorBox),
    /// If set in an element, all mutations should also create a corresponding event
    MutationEvents,
}

/// Up to four [`Anchor`]s applied to a single element, describing
/// which corners / edges are pinned. Larger variants imply more
/// constraints on the layout solver.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub enum AnchorBox {
    Single(Anchor),
    Dual(Anchor, Anchor),
    Trio(Anchor, Anchor, Anchor),
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
    /// Centre-on-parent: the element's centre is pinned to its
    /// immediate parent's centre with zero pixel offset.
    ///
    /// The three-way constraint is:
    /// 1. **Target** — `AnchorTarget::Parent { generation_offset: 0 }`
    ///    (the immediate parent, not a grandparent).
    /// 2. **Target point** — `AnchorPoint::Center(0)` (the parent's
    ///    geometric centre, no pixel nudge).
    /// 3. **Self point** — `AnchorPoint::Center(0)` (the element's own
    ///    centre, no pixel nudge).
    ///
    /// Together these mean "stack my centre on my parent's centre" —
    /// the most common starting layout for new tree nodes before the
    /// scene builder repositions them.
    fn default() -> Self {
        Anchor::new(
            AnchorTarget::Parent {
                generation_offset: 0,
            },
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
    Parent { generation_offset: usize },
    Child { child_num: usize },
    Window,
    Display,
    World,
}

/// Named point on a rectangular region — the nine cardinal positions
/// plus a pixel-offset parameter so callers can nudge a pin without
/// adding a fresh variant.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub enum AnchorPoint {
    TopLeft(i16),
    TopCenter(i16),
    TopRight(i16),
    BotLeft(i16),
    BotCenter(i16),
    BotRight(i16),
    LeftCenter(i16),
    RightCenter(i16),
    Center(i16),
}

/// Query a 2D position from an element.
pub trait Positioned {
    fn position(&self) -> OrderedVec2;
}

/// Query the bounding-box dimensions (width, height) of an element.
pub trait Bounded {
    fn bounds(&self) -> OrderedVec2;
}