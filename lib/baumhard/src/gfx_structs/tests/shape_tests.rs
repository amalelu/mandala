//! Tests for [`crate::gfx_structs::shape`] — the `NodeShape` enum
//! and its point-in-shape / shape-vs-AABB primitives.
//!
//! Follows the `do_*()` / `test_*()` split from
//! [`TEST_CONVENTIONS.md §T2.2`] so the criterion bench harness at
//! `lib/baumhard/benches/test_bench.rs` can reuse each body as a
//! micro-benchmark. The shape math sits on the BVH hot path
//! (`bvh_descend`), so the rect-pipeline SDF in the renderer and
//! the lasso / editor hit-tests all pay for it every interaction
//! frame — benching is §B8-mandatory, not optional.

use glam::Vec2;

use crate::gfx_structs::shape::{NodeShape, SHAPE_ID_ELLIPSE, SHAPE_ID_RECTANGLE};

/// Parse every documented shape spelling, plus the `"circle"`
/// alias and the empty-string fallback. Pins the author-facing
/// vocabulary: changing a spelling here would silently change
/// which JSON maps still render as the intended shape.
#[test]
pub fn test_shape_from_style_string_known_names() {
    do_shape_from_style_string_known_names();
}

pub fn do_shape_from_style_string_known_names() {
    assert_eq!(
        NodeShape::from_style_string("rectangle"),
        NodeShape::Rectangle
    );
    assert_eq!(
        NodeShape::from_style_string("Rectangle"),
        NodeShape::Rectangle
    );
    assert_eq!(NodeShape::from_style_string("ellipse"), NodeShape::Ellipse);
    assert_eq!(NodeShape::from_style_string("ELLIPSE"), NodeShape::Ellipse);
    // "circle" is accepted as a convenience alias for the
    // Ellipse variant — a `width == height` ellipse *is* a
    // circle, and authors will often type this.
    assert_eq!(NodeShape::from_style_string("circle"), NodeShape::Ellipse);
}

/// An empty or unknown string falls back to the default
/// `Rectangle` variant (and logs a warning). Mirrors how
/// `tree_builder/node.rs` treats malformed background hex:
/// survive a typo rather than crash the render.
#[test]
pub fn test_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle() {
    do_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle();
}

pub fn do_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle() {
    assert_eq!(NodeShape::from_style_string(""), NodeShape::Rectangle);
    assert_eq!(
        NodeShape::from_style_string("diamond"),
        NodeShape::Rectangle
    );
    assert_eq!(
        NodeShape::from_style_string("zigzag"),
        NodeShape::Rectangle
    );
}

/// Rectangle `contains_local` matches the classic inclusive-AABB
/// predicate: corners and edges hit, anything outside `[0, bounds]`
/// misses. Locks in the happy path for the legacy behaviour that
/// every existing node still depends on.
#[test]
pub fn test_shape_rectangle_contains_local() {
    do_shape_rectangle_contains_local();
}

pub fn do_shape_rectangle_contains_local() {
    let b = Vec2::new(100.0, 50.0);
    assert!(NodeShape::Rectangle.contains_local(Vec2::new(0.0, 0.0), b));
    assert!(NodeShape::Rectangle.contains_local(Vec2::new(100.0, 50.0), b));
    assert!(NodeShape::Rectangle.contains_local(Vec2::new(50.0, 25.0), b));
    assert!(!NodeShape::Rectangle.contains_local(Vec2::new(-0.1, 25.0), b));
    assert!(!NodeShape::Rectangle.contains_local(Vec2::new(100.1, 25.0), b));
}

/// Perfect-circle case: bounds 100×100, radius 50, centre
/// `(50, 50)`. Centre and the four cardinal rim points all count
/// as inside. Pins the "rim is inclusive" edge-case the BVH hit
/// test relies on for click-on-border behaviour.
#[test]
pub fn test_shape_ellipse_contains_centre_and_rim() {
    do_shape_ellipse_contains_centre_and_rim();
}

pub fn do_shape_ellipse_contains_centre_and_rim() {
    let b = Vec2::new(100.0, 100.0);
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 50.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(0.0, 50.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(100.0, 50.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 0.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 100.0), b));
}

/// Bounding-box corners sit at distance `√2 · r` from the centre
/// of an inscribed circle — comfortably outside. This is the
/// exact case the whole refactor exists to reject: under the
/// pre-change AABB-only hit test, a corner click on an ellipse
/// node would select it; post-change it must miss.
#[test]
pub fn test_shape_ellipse_rejects_aabb_corners() {
    do_shape_ellipse_rejects_aabb_corners();
}

pub fn do_shape_ellipse_rejects_aabb_corners() {
    let b = Vec2::new(100.0, 100.0);
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 0.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(100.0, 0.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 100.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(100.0, 100.0), b));
}

/// Stretched conic case: bounds `200 × 50`, radii `(100, 25)`.
/// Centre and cardinal rim points still hit; bounding-box corners
/// still miss. Guards the "ellipse handles wider-than-tall without
/// extra parameters" claim from the shape doc comment.
#[test]
pub fn test_shape_ellipse_handles_stretched_conic() {
    do_shape_ellipse_handles_stretched_conic();
}

pub fn do_shape_ellipse_handles_stretched_conic() {
    let b = Vec2::new(200.0, 50.0);
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(100.0, 25.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(0.0, 25.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(200.0, 25.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 0.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(200.0, 50.0), b));
}

/// Degenerate bounds (zero or negative extent on either axis)
/// never hit — guards the division by `bounds / 2` in the ellipse
/// math and mirrors how the BVH's AABB check skips zero-size areas.
/// Rendering a zero-size node is already a no-op upstream, so
/// counting a click as a miss is the internally consistent answer.
#[test]
pub fn test_shape_degenerate_bounds_never_hit() {
    do_shape_degenerate_bounds_never_hit();
}

pub fn do_shape_degenerate_bounds_never_hit() {
    assert!(!NodeShape::Rectangle.contains_local(Vec2::ZERO, Vec2::ZERO));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::ZERO, Vec2::new(0.0, 100.0)));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::ZERO, Vec2::new(100.0, -1.0)));
}

/// Selection rect tucked fully inside the ellipse: the closest
/// point on the rect to the ellipse centre is the ellipse centre
/// itself, so `distance == 0` and the test registers a hit.
/// Without this branch, the lasso would report "no nodes
/// selected" whenever the user drew a small rectangle inside a
/// circular node — the exact case a user would expect to match.
#[test]
pub fn test_shape_ellipse_intersects_aabb_fully_inside() {
    do_shape_ellipse_intersects_aabb_fully_inside();
}

pub fn do_shape_ellipse_intersects_aabb_fully_inside() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(40.0, 40.0);
    let max = Vec2::new(60.0, 60.0);
    assert!(NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// Selection rect tucked into the AABB corner, outside the
/// ellipse. The pre-change AABB-overlap test would have matched
/// this as "node selected"; the shape-aware test must reject it.
/// This is the case the rect-select refactor exists to fix.
#[test]
pub fn test_shape_ellipse_intersects_aabb_corner_only() {
    do_shape_ellipse_intersects_aabb_corner_only();
}

pub fn do_shape_ellipse_intersects_aabb_corner_only() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(0.0, 0.0);
    let max = Vec2::new(5.0, 5.0);
    assert!(!NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// Selection rect crossing the ellipse's left rim: the clamp
/// lands on the rect's inside edge (x ≈ 0), which is on the
/// ellipse boundary. Conservative (`<= 1.0`) counts this as a
/// hit — the spirit of a lasso is "any overlap selects".
#[test]
pub fn test_shape_ellipse_intersects_aabb_straddling_rim() {
    do_shape_ellipse_intersects_aabb_straddling_rim();
}

pub fn do_shape_ellipse_intersects_aabb_straddling_rim() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(-10.0, 40.0);
    let max = Vec2::new(10.0, 60.0);
    assert!(NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// Selection rect entirely outside the node's bounding box.
/// Early-bails on the AABB–AABB overlap so the shape math isn't
/// even reached. Guards the cheap path every lasso hit-test takes
/// when the user drags far away from any node.
#[test]
pub fn test_shape_ellipse_intersects_aabb_fully_outside() {
    do_shape_ellipse_intersects_aabb_fully_outside();
}

pub fn do_shape_ellipse_intersects_aabb_fully_outside() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(200.0, 200.0);
    let max = Vec2::new(300.0, 300.0);
    assert!(!NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// The `shader_id` values are wire-format: the fragment shader
/// matches on the same integers via `SHAPE_RECT` / `SHAPE_ELLIPSE`
/// WGSL constants. Pinning them here catches the silent-breakage
/// case where a future reorder of the enum variants reassigns the
/// ids and every ellipse in every map quietly renders as a
/// rectangle.
#[test]
pub fn test_shape_shader_ids_are_stable() {
    do_shape_shader_ids_are_stable();
}

pub fn do_shape_shader_ids_are_stable() {
    assert_eq!(NodeShape::Rectangle.shader_id(), SHAPE_ID_RECTANGLE);
    assert_eq!(NodeShape::Ellipse.shader_id(), SHAPE_ID_ELLIPSE);
    // The absolute values are also part of the wire format — the
    // WGSL fragment shader hard-codes `0u` / `1u` in its `switch`
    // arms. Keeping the numeric assertion here means a rename
    // alone can't drift them.
    assert_eq!(NodeShape::Rectangle.shader_id(), 0);
    assert_eq!(NodeShape::Ellipse.shader_id(), 1);
}
