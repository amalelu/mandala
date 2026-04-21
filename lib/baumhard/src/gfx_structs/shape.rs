//! Per-node background / hit-test shapes.
//!
//! [`NodeShape`] is the single source of truth for "what shape does
//! this node occupy?". Both the renderer (SDF fragment path) and the
//! BVH hit test (point-in-shape check) consult the same enum, so
//! adding a new shape never drifts between visuals and input.
//!
//! Extending the set is deliberately local:
//!
//! 1. Add a variant to the enum below.
//! 2. Add a `SHAPE_*` constant + a `case` arm to the rect pipeline's
//!    fragment shader (`src/application/renderer/mod.rs`,
//!    `RECT_SHADER_WGSL`).
//! 3. Add a branch in [`NodeShape::contains_local`] and
//!    [`NodeShape::intersects_local_aabb`].
//!
//! No new structs, no new mutation surfaces, no new mesh builders.

use glam::Vec2;
use serde::{Deserialize, Serialize};

/// The background / hit shape of a node. Stored on
/// [`crate::gfx_structs::area::GlyphArea`] next to `background_color`
/// and read by both the renderer and the BVH hit test.
///
/// The variant is copied out of the area in the hot paths, so it is
/// intentionally `Copy` and allocation-free.
///
/// # Costs
/// O(1) to copy, hash, compare. No heap allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum NodeShape {
    /// Fills the bounding box exactly — the legacy behaviour and the
    /// default for any node that doesn't opt in to a different shape.
    #[default]
    Rectangle,
    /// Axis-aligned ellipse inscribed in the bounding box. A perfect
    /// circle is expressed as an `Ellipse` with `width == height`;
    /// the same variant handles stretched / "conical" cases where
    /// the box is wider than it is tall (or vice versa) without any
    /// extra parameters.
    Ellipse,
}

/// Shader-side id for [`NodeShape::Rectangle`]. Must match the
/// `SHAPE_RECT` constant in the rect pipeline's WGSL fragment shader.
pub const SHAPE_ID_RECTANGLE: u32 = 0;
/// Shader-side id for [`NodeShape::Ellipse`]. Must match the
/// `SHAPE_ELLIPSE` constant in the rect pipeline's WGSL fragment
/// shader.
pub const SHAPE_ID_ELLIPSE: u32 = 1;

impl NodeShape {
    /// Stable id fed to the fragment shader. Must stay in lock-step
    /// with the `SHAPE_*` constants in
    /// `src/application/renderer/mod.rs` — adding a variant without
    /// adding its shader case would render the new shape as a
    /// rectangle. O(1).
    #[inline]
    pub const fn shader_id(self) -> u32 {
        match self {
            NodeShape::Rectangle => SHAPE_ID_RECTANGLE,
            NodeShape::Ellipse => SHAPE_ID_ELLIPSE,
        }
    }

    /// Parse the format-level `NodeStyle.shape` string. Recognised
    /// values (case-insensitive) map to a variant; anything else
    /// (including the empty string) falls back to
    /// [`NodeShape::Rectangle`] and a `log::warn!`, mirroring how
    /// the tree builder treats malformed background hex colors.
    ///
    /// The format doc at `format/enums.md` lists the canonical
    /// spellings; unknown values stay on disk untouched so a
    /// round-trip through `maptool convert` doesn't lose them.
    ///
    /// O(n) in `s.len()` for the ASCII-lowercase compare; no
    /// allocation for recognised spellings.
    pub fn from_style_string(s: &str) -> Self {
        if s.is_empty() {
            return NodeShape::Rectangle;
        }
        if s.eq_ignore_ascii_case("rectangle") {
            NodeShape::Rectangle
        } else if s.eq_ignore_ascii_case("ellipse") || s.eq_ignore_ascii_case("circle") {
            // "circle" isn't one of the canonical named-enum
            // spellings, but accepting it is free and matches
            // common-sense author expectation — a `width == height`
            // ellipse *is* a circle. The round-trip stays correct
            // because `NodeStyle.shape` is a free-form String at
            // the format layer; we never write this value back from
            // here.
            NodeShape::Ellipse
        } else {
            log::warn!(
                "NodeShape::from_style_string: unknown shape {s:?}, \
                 falling back to Rectangle"
            );
            NodeShape::Rectangle
        }
    }

    /// Point-in-shape test in the node's **local** coordinate space,
    /// where the bounding box runs from `(0, 0)` to `bounds`.
    /// Callers pre-translate `local = world_point - area.position`.
    ///
    /// A degenerate `bounds` (either dimension `<= 0`) always
    /// reports `false`, matching how the BVH skips zero-size areas
    /// at the AABB stage.
    ///
    /// O(1). No allocation.
    #[inline]
    pub fn contains_local(self, local: Vec2, bounds: Vec2) -> bool {
        if bounds.x <= 0.0 || bounds.y <= 0.0 {
            return false;
        }
        match self {
            NodeShape::Rectangle => {
                local.x >= 0.0
                    && local.x <= bounds.x
                    && local.y >= 0.0
                    && local.y <= bounds.y
            }
            NodeShape::Ellipse => {
                // Normalised coordinates in [-1, 1] relative to the
                // ellipse centre. A perfect circle is `bounds.x ==
                // bounds.y`; a stretched conic is anything else.
                let rx = bounds.x * 0.5;
                let ry = bounds.y * 0.5;
                let nx = (local.x - rx) / rx;
                let ny = (local.y - ry) / ry;
                nx * nx + ny * ny <= 1.0
            }
        }
    }

    /// Does the shape's filled area overlap the AABB
    /// `[min, max]` (in local coordinates, same frame as
    /// [`Self::contains_local`])? Conservative — false positives
    /// are tolerated, false negatives are not. Used by rect-select.
    ///
    /// For the ellipse variant this clamps the AABB to the
    /// ellipse's bounding box, then checks whether the closest
    /// clamped point sits inside the ellipse. That's conservative
    /// in the "selection rect fully inside the ellipse" corner
    /// (the closest-point test returns true when any corner of the
    /// rect is inside the ellipse, *or* when the ellipse-centre is
    /// inside the rect) — which is what we want for a lasso.
    ///
    /// O(1). No allocation.
    #[inline]
    pub fn intersects_local_aabb(self, min: Vec2, max: Vec2, bounds: Vec2) -> bool {
        if bounds.x <= 0.0 || bounds.y <= 0.0 {
            return false;
        }
        // First, AABB–AABB overlap. Bails on any shape whose bounds
        // don't touch the selection rectangle at all.
        if max.x < 0.0 || min.x > bounds.x || max.y < 0.0 || min.y > bounds.y {
            return false;
        }
        match self {
            NodeShape::Rectangle => true,
            NodeShape::Ellipse => {
                let rx = bounds.x * 0.5;
                let ry = bounds.y * 0.5;
                let cx = rx;
                let cy = ry;
                // Closest point on the AABB to the ellipse centre.
                let clamped_x = cx.clamp(min.x, max.x);
                let clamped_y = cy.clamp(min.y, max.y);
                let nx = (clamped_x - cx) / rx;
                let ny = (clamped_y - cy) / ry;
                nx * nx + ny * ny <= 1.0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_style_string_known_names() {
        assert_eq!(NodeShape::from_style_string("rectangle"), NodeShape::Rectangle);
        assert_eq!(NodeShape::from_style_string("Rectangle"), NodeShape::Rectangle);
        assert_eq!(NodeShape::from_style_string("ellipse"), NodeShape::Ellipse);
        assert_eq!(NodeShape::from_style_string("ELLIPSE"), NodeShape::Ellipse);
        // "circle" is accepted as an alias for the Ellipse variant.
        assert_eq!(NodeShape::from_style_string("circle"), NodeShape::Ellipse);
    }

    #[test]
    fn from_style_string_empty_and_unknown_fall_back_to_rectangle() {
        assert_eq!(NodeShape::from_style_string(""), NodeShape::Rectangle);
        assert_eq!(NodeShape::from_style_string("diamond"), NodeShape::Rectangle);
        assert_eq!(NodeShape::from_style_string("zigzag"), NodeShape::Rectangle);
    }

    #[test]
    fn rectangle_contains_local() {
        let b = Vec2::new(100.0, 50.0);
        assert!(NodeShape::Rectangle.contains_local(Vec2::new(0.0, 0.0), b));
        assert!(NodeShape::Rectangle.contains_local(Vec2::new(100.0, 50.0), b));
        assert!(NodeShape::Rectangle.contains_local(Vec2::new(50.0, 25.0), b));
        assert!(!NodeShape::Rectangle.contains_local(Vec2::new(-0.1, 25.0), b));
        assert!(!NodeShape::Rectangle.contains_local(Vec2::new(100.1, 25.0), b));
    }

    #[test]
    fn ellipse_contains_centre_and_rim() {
        // Perfect circle: bounds 100x100, radius 50, centre (50, 50).
        let b = Vec2::new(100.0, 100.0);
        assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 50.0), b));
        assert!(NodeShape::Ellipse.contains_local(Vec2::new(0.0, 50.0), b)); // left rim
        assert!(NodeShape::Ellipse.contains_local(Vec2::new(100.0, 50.0), b)); // right rim
        assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 0.0), b)); // top rim
        assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 100.0), b)); // bottom rim
    }

    #[test]
    fn ellipse_rejects_aabb_corners() {
        // Corners of the bounding box sit at distance √2 · r from
        // the centre, well outside the circle.
        let b = Vec2::new(100.0, 100.0);
        assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 0.0), b));
        assert!(!NodeShape::Ellipse.contains_local(Vec2::new(100.0, 0.0), b));
        assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 100.0), b));
        assert!(!NodeShape::Ellipse.contains_local(Vec2::new(100.0, 100.0), b));
    }

    #[test]
    fn ellipse_handles_stretched_conic() {
        // Wider than tall: 200 × 50, radii (100, 25). The point
        // (100, 50) is the bottom rim; (100, 0) is the top rim;
        // (0, 25) / (200, 25) are the left/right rims. Corners
        // are still outside.
        let b = Vec2::new(200.0, 50.0);
        assert!(NodeShape::Ellipse.contains_local(Vec2::new(100.0, 25.0), b)); // centre
        assert!(NodeShape::Ellipse.contains_local(Vec2::new(0.0, 25.0), b));
        assert!(NodeShape::Ellipse.contains_local(Vec2::new(200.0, 25.0), b));
        assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 0.0), b));
        assert!(!NodeShape::Ellipse.contains_local(Vec2::new(200.0, 50.0), b));
    }

    #[test]
    fn degenerate_bounds_never_hit() {
        assert!(!NodeShape::Rectangle.contains_local(Vec2::ZERO, Vec2::ZERO));
        assert!(!NodeShape::Ellipse.contains_local(Vec2::ZERO, Vec2::new(0.0, 100.0)));
        assert!(!NodeShape::Ellipse.contains_local(Vec2::ZERO, Vec2::new(100.0, -1.0)));
    }

    #[test]
    fn ellipse_intersects_aabb_fully_inside_ellipse() {
        // Selection rect fully inside a 100×100 circle at centre.
        let b = Vec2::new(100.0, 100.0);
        let min = Vec2::new(40.0, 40.0);
        let max = Vec2::new(60.0, 60.0);
        assert!(NodeShape::Ellipse.intersects_local_aabb(min, max, b));
    }

    #[test]
    fn ellipse_intersects_aabb_corner_only() {
        // Selection rect tucked into the top-left corner of the
        // bounding box (outside the ellipse). Should NOT intersect.
        let b = Vec2::new(100.0, 100.0);
        let min = Vec2::new(0.0, 0.0);
        let max = Vec2::new(5.0, 5.0);
        assert!(!NodeShape::Ellipse.intersects_local_aabb(min, max, b));
    }

    #[test]
    fn ellipse_intersects_aabb_straddling_rim() {
        // Selection rect whose left edge crosses the ellipse's
        // left rim — must count as an overlap.
        let b = Vec2::new(100.0, 100.0);
        let min = Vec2::new(-10.0, 40.0);
        let max = Vec2::new(10.0, 60.0);
        assert!(NodeShape::Ellipse.intersects_local_aabb(min, max, b));
    }

    #[test]
    fn ellipse_intersects_aabb_fully_outside() {
        let b = Vec2::new(100.0, 100.0);
        let min = Vec2::new(200.0, 200.0);
        let max = Vec2::new(300.0, 300.0);
        assert!(!NodeShape::Ellipse.intersects_local_aabb(min, max, b));
    }

    #[test]
    fn shader_ids_are_stable() {
        // These values are wire-format: the fragment shader
        // matches on the same integers. Lock them in so a
        // reordering of the enum doesn't silently reassign them.
        assert_eq!(NodeShape::Rectangle.shader_id(), 0);
        assert_eq!(NodeShape::Ellipse.shader_id(), 1);
    }
}
