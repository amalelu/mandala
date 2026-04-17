//! Portal-label geometry: converts between points on a node's
//! rectangular border and the scalar `border_t` parameter used to
//! store a label's position. Also computes the directional default
//! — the `t` that makes a label face its partner endpoint, so an
//! un-dragged portal label always orients toward the node on the
//! other side of the (invisible) edge.
//!
//! The parameter space is the unit square's perimeter, encoded as
//! `t ∈ [0.0, 4.0)` with one unit per side, walked clockwise from
//! the top-left corner:
//!
//! ```text
//!   0 ─────────→ 1
//!   ↑            ↓
//!   3            1
//!   ↑            ↓
//!   3 ←───────── 2
//! ```
//!
//! Using a side-indexed encoding (rather than a single perimeter
//! fraction that stretches with aspect ratio) keeps `border_t`
//! stable when a node is resized: a label at `t = 1.5` always sits
//! at the vertical midpoint of the right edge, regardless of
//! whether the node is tall or wide. That's the behavior the user
//! expects when they've dragged a label to a specific conceptual
//! spot and then resize the node later.
//!
//! All coordinates here are canvas-space — callers pass node
//! positions as the node's top-left corner and size as its extent,
//! matching the `MindNode` field shapes. No screen-space / zoom
//! math lives in this module; that's the renderer's job.

use glam::Vec2;

/// Period of `border_t`: four sides, normalized to one unit each.
/// Exposed so the drag path can wrap values into the canonical
/// range (e.g. after a dragged label would otherwise leave it
/// at `t = 4.2`, we wrap back to `0.2`).
pub const BORDER_T_PERIOD: f32 = 4.0;

/// Wrap a raw `border_t` into the canonical `[0.0, 4.0)` range.
/// Handles both overflow (drag past bottom-left wraps back to
/// top-left) and negative values (e.g. a right-to-left drag that
/// momentarily dips below zero). Pure arithmetic — no Vec2s.
pub fn wrap_border_t(t: f32) -> f32 {
    let m = t.rem_euclid(BORDER_T_PERIOD);
    // `rem_euclid` guarantees `m >= 0` and `m < BORDER_T_PERIOD`
    // for finite inputs, so no extra clamp is needed here.
    m
}

/// Convert a canonical `t ∈ [0.0, 4.0)` to a canvas-space point on
/// the border of the rectangle at `node_pos` with extent
/// `node_size`. Linearly interpolates across each side:
///
/// - `[0, 1)` → top edge, left → right
/// - `[1, 2)` → right edge, top → bottom
/// - `[2, 3)` → bottom edge, right → left
/// - `[3, 4)` → left edge, bottom → top
///
/// Values outside the canonical range are wrapped via
/// [`wrap_border_t`].
pub fn border_point_at(node_pos: Vec2, node_size: Vec2, t: f32) -> Vec2 {
    let t = wrap_border_t(t);
    let side = t.floor() as i32;
    let u = t - t.floor();
    match side {
        0 => Vec2::new(node_pos.x + u * node_size.x, node_pos.y),
        1 => Vec2::new(node_pos.x + node_size.x, node_pos.y + u * node_size.y),
        2 => Vec2::new(node_pos.x + (1.0 - u) * node_size.x, node_pos.y + node_size.y),
        _ => Vec2::new(node_pos.x, node_pos.y + (1.0 - u) * node_size.y),
    }
}

/// Outward unit normal of the border at parameter `t`. Always one
/// of the four axis-aligned unit vectors (±x, ±y). Used to offset
/// a label outside the border so its glyph doesn't overlap the
/// node frame: `label_origin = border_point + outward_normal *
/// outset`.
pub fn border_outward_normal(t: f32) -> Vec2 {
    let t = wrap_border_t(t);
    match t.floor() as i32 {
        0 => Vec2::new(0.0, -1.0), // top → up
        1 => Vec2::new(1.0, 0.0),  // right → right
        2 => Vec2::new(0.0, 1.0),  // bottom → down
        _ => Vec2::new(-1.0, 0.0), // left → left
    }
}

/// Compute the directional default `t` for a portal label whose
/// owning node is at `owner_pos` / `owner_size` and whose partner
/// endpoint's center is `partner_center`. Casts a ray from the
/// owner's center through `partner_center`, finds where it exits
/// the owner's border rectangle, and returns the `t` of that exit
/// point.
///
/// When `partner_center` coincides with the owner's center
/// (degenerate zero-direction case — should be rare, happens only
/// if two nodes are exactly overlapping), falls back to the
/// top-right corner (`t = 1.0`) to match the pre-refactor portal
/// marker placement so a broken map still renders something
/// recognizable.
pub fn default_border_t(owner_pos: Vec2, owner_size: Vec2, partner_center: Vec2) -> f32 {
    let owner_center = Vec2::new(
        owner_pos.x + owner_size.x * 0.5,
        owner_pos.y + owner_size.y * 0.5,
    );
    let dir = partner_center - owner_center;
    if dir.length_squared() < f32::EPSILON {
        return 1.0;
    }
    // Parameterize the ray as `owner_center + s * dir` and find
    // the smallest positive `s` that exits the rectangle. The exit
    // side's four linear equations reduce to the usual
    // axis-aligned ray/box intersection — pick whichever axis
    // bounds fire first.
    let half = owner_size * 0.5;
    // Guard against divide-by-zero on a perfectly axis-aligned
    // direction. `f32::INFINITY` sorts correctly against finite
    // positive values in the `min_by` below, so any infinite
    // axis simply never wins — correct behavior.
    let sx = if dir.x.abs() < f32::EPSILON {
        f32::INFINITY
    } else {
        half.x / dir.x.abs()
    };
    let sy = if dir.y.abs() < f32::EPSILON {
        f32::INFINITY
    } else {
        half.y / dir.y.abs()
    };
    let s = sx.min(sy);
    let exit = owner_center + dir * s;
    nearest_border_t(owner_pos, owner_size, exit)
}

/// Project an arbitrary canvas-space point onto the border of the
/// rectangle at `node_pos` / `node_size` and return the `t` of the
/// nearest border point. Used by drag: the cursor is rarely
/// exactly on the border, so we snap to whichever side it's
/// closest to and slide along that side.
///
/// Points inside the rectangle are handled the same way — we
/// return the `t` of the closest border point (so dragging the
/// label *into* the node snaps it to the nearest edge, matching
/// the user's intent of "put the label on *this* side").
pub fn nearest_border_t(node_pos: Vec2, node_size: Vec2, point: Vec2) -> f32 {
    // Clamp the point onto the rectangle (interior points snap to
    // the nearest edge, exterior points project onto the closest
    // border segment). Four candidate border positions — top,
    // right, bottom, left — one for each edge the cursor could
    // have been snapped to; pick whichever is actually closest.
    let min = node_pos;
    let max = node_pos + node_size;
    let cx = point.x.clamp(min.x, max.x);
    let cy = point.y.clamp(min.y, max.y);
    let top = Vec2::new(cx, min.y);
    let right = Vec2::new(max.x, cy);
    let bottom = Vec2::new(cx, max.y);
    let left = Vec2::new(min.x, cy);

    let d_top = point.distance_squared(top);
    let d_right = point.distance_squared(right);
    let d_bottom = point.distance_squared(bottom);
    let d_left = point.distance_squared(left);

    let best = d_top.min(d_right).min(d_bottom).min(d_left);
    // Guarded division — a zero-size node (shouldn't happen in
    // practice, but not checked at type level) would divide by
    // zero. Treat that as `t = 0.0` rather than NaN.
    let width = if node_size.x.abs() < f32::EPSILON { 1.0 } else { node_size.x };
    let height = if node_size.y.abs() < f32::EPSILON { 1.0 } else { node_size.y };
    if best == d_top {
        ((top.x - min.x) / width).clamp(0.0, 1.0 - f32::EPSILON)
    } else if best == d_right {
        1.0 + ((right.y - min.y) / height).clamp(0.0, 1.0 - f32::EPSILON)
    } else if best == d_bottom {
        2.0 + ((max.x - bottom.x) / width).clamp(0.0, 1.0 - f32::EPSILON)
    } else {
        3.0 + ((max.y - left.y) / height).clamp(0.0, 1.0 - f32::EPSILON)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn wrap_handles_overflow_and_negatives() {
        assert!(approx(wrap_border_t(0.0), 0.0));
        assert!(approx(wrap_border_t(3.9), 3.9));
        assert!(approx(wrap_border_t(4.0), 0.0));
        assert!(approx(wrap_border_t(4.5), 0.5));
        assert!(approx(wrap_border_t(-0.5), 3.5));
    }

    #[test]
    fn border_point_walks_clockwise_from_top_left() {
        let pos = Vec2::new(10.0, 20.0);
        let size = Vec2::new(100.0, 50.0);
        // Top-left corner
        assert_eq!(border_point_at(pos, size, 0.0), Vec2::new(10.0, 20.0));
        // Top-right corner
        assert_eq!(border_point_at(pos, size, 1.0), Vec2::new(110.0, 20.0));
        // Bottom-right corner
        assert_eq!(border_point_at(pos, size, 2.0), Vec2::new(110.0, 70.0));
        // Bottom-left corner
        assert_eq!(border_point_at(pos, size, 3.0), Vec2::new(10.0, 70.0));
        // Midpoints of each side
        assert_eq!(border_point_at(pos, size, 0.5), Vec2::new(60.0, 20.0));
        assert_eq!(border_point_at(pos, size, 1.5), Vec2::new(110.0, 45.0));
        assert_eq!(border_point_at(pos, size, 2.5), Vec2::new(60.0, 70.0));
        assert_eq!(border_point_at(pos, size, 3.5), Vec2::new(10.0, 45.0));
    }

    #[test]
    fn outward_normal_matches_side() {
        assert_eq!(border_outward_normal(0.3), Vec2::new(0.0, -1.0));
        assert_eq!(border_outward_normal(1.3), Vec2::new(1.0, 0.0));
        assert_eq!(border_outward_normal(2.3), Vec2::new(0.0, 1.0));
        assert_eq!(border_outward_normal(3.3), Vec2::new(-1.0, 0.0));
    }

    #[test]
    fn default_t_points_at_partner_north() {
        let pos = Vec2::new(0.0, 0.0);
        let size = Vec2::new(100.0, 50.0);
        // Partner directly north of owner's center (50, 25).
        let partner = Vec2::new(50.0, -500.0);
        let t = default_border_t(pos, size, partner);
        // Top edge midpoint is t = 0.5.
        assert!(approx(t, 0.5), "expected 0.5, got {t}");
    }

    #[test]
    fn default_t_points_at_partner_east() {
        let pos = Vec2::new(0.0, 0.0);
        let size = Vec2::new(100.0, 50.0);
        let partner = Vec2::new(500.0, 25.0);
        let t = default_border_t(pos, size, partner);
        assert!(approx(t, 1.5), "expected 1.5, got {t}");
    }

    #[test]
    fn default_t_points_at_partner_south() {
        let pos = Vec2::new(0.0, 0.0);
        let size = Vec2::new(100.0, 50.0);
        let partner = Vec2::new(50.0, 500.0);
        let t = default_border_t(pos, size, partner);
        assert!(approx(t, 2.5), "expected 2.5, got {t}");
    }

    #[test]
    fn default_t_points_at_partner_west() {
        let pos = Vec2::new(0.0, 0.0);
        let size = Vec2::new(100.0, 50.0);
        let partner = Vec2::new(-500.0, 25.0);
        let t = default_border_t(pos, size, partner);
        assert!(approx(t, 3.5), "expected 3.5, got {t}");
    }

    #[test]
    fn default_t_coincident_partner_falls_back() {
        let pos = Vec2::new(0.0, 0.0);
        let size = Vec2::new(100.0, 50.0);
        let owner_center = Vec2::new(50.0, 25.0);
        assert!(approx(default_border_t(pos, size, owner_center), 1.0));
    }

    #[test]
    fn nearest_t_snaps_to_closest_side() {
        let pos = Vec2::new(0.0, 0.0);
        let size = Vec2::new(100.0, 50.0);
        // Just above the top edge midpoint → snaps to top
        let t = nearest_border_t(pos, size, Vec2::new(50.0, -5.0));
        assert!(approx(t, 0.5));
        // Just to the right of the right edge midpoint → snaps to right
        let t = nearest_border_t(pos, size, Vec2::new(105.0, 25.0));
        assert!(approx(t, 1.5));
        // Inside the node near top edge → still snaps to top
        let t = nearest_border_t(pos, size, Vec2::new(25.0, 5.0));
        assert!(approx(t, 0.25));
    }

    #[test]
    fn border_point_and_nearest_roundtrip() {
        let pos = Vec2::new(10.0, 20.0);
        let size = Vec2::new(80.0, 60.0);
        for raw in [0.1f32, 0.5, 1.0, 1.7, 2.3, 2.99, 3.25, 3.9] {
            let p = border_point_at(pos, size, raw);
            let back = nearest_border_t(pos, size, p);
            assert!(
                (back - raw).abs() < 1e-3,
                "round trip failed: raw={raw}, back={back}"
            );
        }
    }
}
