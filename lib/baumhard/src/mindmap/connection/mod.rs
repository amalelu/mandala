//! Connection-path geometry: anchor resolution, straight/cubic Bezier
//! path construction, arc-length sampling, and point-to-path distance.
//!
//! - [`build_connection_path`] turns an edge's anchors + control points
//!   into a [`ConnectionPath`] (straight or cubic Bezier).
//! - [`sample_path`] walks evenly-spaced points along a path —
//!   `scene_builder` uses these to place per-glyph anchors along a
//!   rendered connection.
//! - [`distance_to_path`] backs the edge hit-test.
//!
//! The cubic-Bezier internals (arc-length table, parameter binary
//! search) live in the sibling [`bezier`] module; the tests live in
//! `tests.rs` so the public surface here stays skimmable.

pub mod bezier;
#[cfg(test)]
mod tests;

use glam::Vec2;

use crate::mindmap::model::ControlPoint;

use self::bezier::{cubic_bezier_length, cubic_bezier_point, sample_cubic_bezier};

/// A single sampled point along a connection path, produced by
/// [`sample_path`] in canvas-space coordinates. Plain data; no
/// runtime cost beyond the `Vec2` copy.
#[derive(Debug, Clone)]
pub struct SampledPoint {
    pub position: Vec2,
}

/// Geometric shape of a connection between two nodes, returned by
/// [`build_connection_path`]. Either a straight segment (no control
/// points) or a cubic Bezier (one or two control points — a quadratic
/// Bezier is promoted to cubic by the builder so the downstream
/// sampler only has to handle one curved shape). Plain data.
#[derive(Debug, Clone)]
pub enum ConnectionPath {
    Straight {
        start: Vec2,
        end: Vec2,
    },
    CubicBezier {
        start: Vec2,
        control1: Vec2,
        control2: Vec2,
        end: Vec2,
    },
}

/// Resolves the anchor point on a node's bounding box.
///
/// - `node_pos`: top-left corner of the node
/// - `node_size`: (width, height) of the node
/// - `anchor`: "auto", "top", "right", "bottom", "left"
/// - `other_center`: center of the other node (used for auto resolution)
pub fn resolve_anchor_point(
    node_pos: Vec2,
    node_size: Vec2,
    anchor: &str,
    other_center: Vec2,
) -> Vec2 {
    let half_w = node_size.x * 0.5;
    let half_h = node_size.y * 0.5;

    match anchor {
        "top" => Vec2::new(node_pos.x + half_w, node_pos.y),
        "right" => Vec2::new(node_pos.x + node_size.x, node_pos.y + half_h),
        "bottom" => Vec2::new(node_pos.x + half_w, node_pos.y + node_size.y),
        "left" => Vec2::new(node_pos.x, node_pos.y + half_h),
        _ => {
            // Auto: pick the edge midpoint closest to the other node's center
            let candidates = [
                Vec2::new(node_pos.x + half_w, node_pos.y),
                Vec2::new(node_pos.x + node_size.x, node_pos.y + half_h),
                Vec2::new(node_pos.x + half_w, node_pos.y + node_size.y),
                Vec2::new(node_pos.x, node_pos.y + half_h),
            ];
            candidates.into_iter()
                .min_by(|a, b| {
                    let da = a.distance_squared(other_center);
                    let db = b.distance_squared(other_center);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap()
        }
    }
}

/// Returns the center of a node given its top-left position and size.
fn node_center(pos: Vec2, size: Vec2) -> Vec2 {
    Vec2::new(pos.x + size.x * 0.5, pos.y + size.y * 0.5)
}

/// Builds a connection path from edge data.
///
/// Control points are interpreted as offsets from the respective node centers:
/// - 0 control points: straight line between anchors
/// - 1 control point: quadratic Bezier (promoted to cubic), offset from source node center
/// - 2 control points: cubic Bezier, offsets from source and target node centers respectively
pub fn build_connection_path(
    from_pos: Vec2,
    from_size: Vec2,
    anchor_from: &str,
    to_pos: Vec2,
    to_size: Vec2,
    anchor_to: &str,
    control_points: &[ControlPoint],
) -> ConnectionPath {
    let from_center = node_center(from_pos, from_size);
    let to_center = node_center(to_pos, to_size);
    let start = resolve_anchor_point(from_pos, from_size, anchor_from, to_center);
    let end = resolve_anchor_point(to_pos, to_size, anchor_to, from_center);

    match control_points.len() {
        0 => ConnectionPath::Straight { start, end },
        1 => {
            // Quadratic Bezier: promote to cubic
            // Control point is offset from source node center
            let qp = from_center + Vec2::new(control_points[0].x as f32, control_points[0].y as f32);
            // Quadratic -> Cubic: C1 = P0 + 2/3*(Q - P0), C2 = P2 + 2/3*(Q - P2)
            let c1 = start + (2.0 / 3.0) * (qp - start);
            let c2 = end + (2.0 / 3.0) * (qp - end);
            ConnectionPath::CubicBezier { start, control1: c1, control2: c2, end }
        }
        _ => {
            // Cubic Bezier: control points are offsets from respective node centers
            let c1 = from_center + Vec2::new(control_points[0].x as f32, control_points[0].y as f32);
            let c2 = to_center + Vec2::new(control_points[1].x as f32, control_points[1].y as f32);
            ConnectionPath::CubicBezier { start, control1: c1, control2: c2, end }
        }
    }
}

/// Return a point on `path` at the parameter value `t`, clamped to
/// `[0.0, 1.0]`. Straight paths lerp linearly between endpoints; cubic
/// Bezier paths evaluate the curve at `t` directly. Used for label
/// positioning along a connection — `t = 0.0` sits at the
/// from-anchor, `t = 0.5` at the midpoint, `t = 1.0` at the to-anchor.
///
/// Parameter-space positioning is fine for the Start/Middle/End label
/// presets the palette exposes; arc-length uniformity is not needed
/// because the three preset values all correspond to the same t values
/// regardless of curvature.
pub fn point_at_t(path: &ConnectionPath, t: f32) -> Vec2 {
    let t = t.clamp(0.0, 1.0);
    match path {
        ConnectionPath::Straight { start, end } => start.lerp(*end, t),
        ConnectionPath::CubicBezier { start, control1, control2, end } => {
            cubic_bezier_point(t, *start, *control1, *control2, *end)
        }
    }
}

/// Total arc length of a connection path in canvas units. Straight
/// paths return the exact endpoint distance; cubic Bezier paths
/// approximate the length by walking `ARC_LENGTH_SUBDIVISIONS`
/// straight segments, so cost is O(subdivisions) with no allocation.
pub fn path_length(path: &ConnectionPath) -> f32 {
    match path {
        ConnectionPath::Straight { start, end } => start.distance(*end),
        ConnectionPath::CubicBezier { start, control1, control2, end } => {
            cubic_bezier_length(*start, *control1, *control2, *end)
        }
    }
}

/// Samples points along a connection path at the given spacing.
///
/// Returns evenly-spaced points including the start point. The last point
/// may be slightly before the path endpoint if the remaining distance is
/// less than `spacing`.
pub fn sample_path(path: &ConnectionPath, spacing: f32) -> Vec<SampledPoint> {
    if spacing <= 0.0 {
        return Vec::new();
    }

    match path {
        ConnectionPath::Straight { start, end } => {
            sample_straight(*start, *end, spacing)
        }
        ConnectionPath::CubicBezier { start, control1, control2, end } => {
            sample_cubic_bezier(*start, *control1, *control2, *end, spacing)
        }
    }
}

fn sample_straight(start: Vec2, end: Vec2, spacing: f32) -> Vec<SampledPoint> {
    let total_length = start.distance(end);
    if total_length < f32::EPSILON {
        return vec![SampledPoint { position: start }];
    }
    let count = (total_length / spacing).floor() as usize + 1;
    let mut points = Vec::with_capacity(count);
    for i in 0..count {
        let t = (i as f32 * spacing) / total_length;
        let t = t.min(1.0);
        let position = start.lerp(end, t);
        points.push(SampledPoint { position });
    }
    points
}

/// Returns the squared distance from `point` to the line segment `a`—`b`.
fn point_to_segment_distance_squared(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < f32::EPSILON {
        return point.distance_squared(a);
    }
    let t = ((point - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    let closest = a + ab * t;
    point.distance_squared(closest)
}

/// Returns the minimum distance from `point` to the given connection path.
///
/// - `Straight`: exact point-to-segment distance.
/// - `CubicBezier`: samples the curve and returns the minimum distance over
///   all resulting polyline segments. This is an approximation; at default
///   sampling density (4.0 canvas units) the error is below one canvas unit
///   for typical connection paths — well within a click tolerance.
pub fn distance_to_path(point: Vec2, path: &ConnectionPath) -> f32 {
    match path {
        ConnectionPath::Straight { start, end } => {
            point_to_segment_distance_squared(point, *start, *end).sqrt()
        }
        ConnectionPath::CubicBezier { .. } => {
            let samples = sample_path(path, 4.0);
            if samples.is_empty() {
                return f32::INFINITY;
            }
            if samples.len() == 1 {
                return point.distance(samples[0].position);
            }
            let mut min_sq = f32::INFINITY;
            for pair in samples.windows(2) {
                let d = point_to_segment_distance_squared(
                    point, pair[0].position, pair[1].position,
                );
                if d < min_sq {
                    min_sq = d;
                }
            }
            min_sq.sqrt()
        }
    }
}
