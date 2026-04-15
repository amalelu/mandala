//! Hit-test bounding-rectangle bag carried on every `GlyphModel`. A
//! single `GlyphModel` can occupy multiple rectangles (e.g. a
//! wrapped-line node where each visual line has its own box), so the
//! `HitBox` is a `Vec` rather than a single rect.

use serde::{Deserialize, Serialize};

/// A collection of axis-aligned `BoundingRectangle`s describing the
/// click-sensitive extents of a [`crate::gfx_structs::model::GlyphModel`].
/// Multiple rects let one model carry disjoint hit areas (wrapped
/// lines, multi-part glyphs) without needing a separate wrapper type.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct HitBox {
    pub rectangles: Vec<BoundingRectangle>,
}

impl HitBox {
    /// Construct an empty `HitBox` with no rectangles. O(1); does not
    /// pre-allocate.
    pub fn new() -> Self {
        HitBox { rectangles: vec![] }
    }

    /// Append a single `BoundingRectangle` to the bag. O(1) amortised.
    pub fn add(&mut self, rectangle: BoundingRectangle) {
        self.rectangles.push(rectangle)
    }

    /// Copy every rectangle from `other` into `self`, preserving
    /// order. Performs an `extend_from_slice`, which clones each
    /// `BoundingRectangle` (they are `Copy`, so the clones are
    /// cheap) — O(n) in `other.rectangles.len()`, with one heap
    /// allocation when the internal `Vec` needs to grow.
    pub fn copy_from(&mut self, other: &HitBox) {
        self.rectangles.extend_from_slice(&other.rectangles)
    }

    /// Drop every rectangle but retain the backing allocation so the
    /// next caller's `add` / `copy_from` does not re-allocate. Use
    /// when the same `HitBox` is being re-used across frames.
    pub fn clear(&mut self) {
        self.rectangles.clear()
    }
}

/// A single axis-aligned rectangle described by its top-left offset
/// (`delta_x`, `delta_y`) from the owning `GlyphModel`'s position
/// plus a `length` (x-extent) and `width` (y-extent). The hit test
/// matches a point by adding the model's position and testing against
/// the resulting absolute rect.
#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
pub struct BoundingRectangle {
    pub delta_x: f32,
    pub delta_y: f32,
    pub length: f32,
    pub width: f32,
}

impl BoundingRectangle {
    /// Build a rectangle anchored at the owning model's origin (zero
    /// offset). Used by scene builders that size the rect to fit the
    /// freshly-laid-out glyph extents and anchor it exactly on the
    /// model position.
    pub fn at_origin(length: f32, width: f32) -> Self {
        BoundingRectangle {
            delta_x: 0.0,
            delta_y: 0.0,
            length,
            width,
        }
    }
}
