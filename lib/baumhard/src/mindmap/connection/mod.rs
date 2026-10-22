// SPDX-License-Identifier: MPL-2.0

//! Connection-path geometry: anchor resolution, straight/cubic Bezier
//! path construction, arc-length sampling, and point-to-path distance.
//!
//! - `build_connection_path` turns an edge's anchors + control points
//!   into a `ConnectionPath` (straight or cubic Bezier).
//! - `sample_path_into` walks evenly-spaced points along a path into
//!   a buffer the caller owns — the connection pass uses these to
//!   place per-glyph anchors along a rendered connection, and the
//!   buffer it passes is the one the scene cache already holds for
//!   that edge, so a resampling frame refills an allocation rather
//!   than making one. `sample_path` is the allocating wrapper, for
//!   callers with no buffer to reuse.
//! - `distance_to_path` measures a point against a path;
//!   `distance_to_path_within` is the tolerance-bounded form the edge
//!   hit-test uses, which rejects a far-away path from its bounding
//!   box without sampling it.
//!
//! The cubic-Bezier internals (arc-length table, parameter binary
//! search) live in the sibling `bezier` module; the tests live in
//! `tests.rs` so the public surface here stays skimmable.

/// Cubic-Bezier math and arc-length sampling — the internals
/// behind `sample_path` and `distance_to_path`.
pub mod bezier;
#[cfg(test)]
mod tests;

use glam::Vec2;

use crate::mindmap::model::ControlPoint;
use crate::util::geometry::aabb_center;

use self::bezier::{
    cubic_bezier_length, cubic_bezier_point, cubic_bezier_second_derivative, cubic_bezier_tangent,
    extend_cubic_samples, plan_cubic_samples,
};

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
pub fn resolve_anchor_point(node_pos: Vec2, node_size: Vec2, anchor: &str, other_center: Vec2) -> Vec2 {
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
            candidates
                .into_iter()
                .min_by(|a, b| {
                    let da = a.distance_squared(other_center);
                    let db = b.distance_squared(other_center);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("connection invariant: a box has four edge midpoints, so the candidate array is never empty")
        }
    }
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
    let from_center = aabb_center(from_pos, from_size);
    let to_center = aabb_center(to_pos, to_size);
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
            ConnectionPath::CubicBezier {
                start,
                control1: c1,
                control2: c2,
                end,
            }
        }
        _ => {
            // Cubic Bezier: control points are offsets from respective node centers
            let c1 = from_center + Vec2::new(control_points[0].x as f32, control_points[0].y as f32);
            let c2 = to_center + Vec2::new(control_points[1].x as f32, control_points[1].y as f32);
            ConnectionPath::CubicBezier {
                start,
                control1: c1,
                control2: c2,
                end,
            }
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
        ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } => cubic_bezier_point(t, *start, *control1, *control2, *end),
    }
}

/// Return the unit tangent direction of `path` at parameter `t`,
/// clamped to `[0, 1]`. Straight paths return the normalized
/// (end - start) vector for every `t`; cubic Bezier paths evaluate
/// the analytical derivative at `t`. If the path is degenerate
/// (zero length or coincident controls) the returned vector is
/// [`Vec2::X`] — a deterministic fallback so callers computing a
/// normal (by rotating 90°) still get a well-defined perpendicular.
pub fn tangent_at_t(path: &ConnectionPath, t: f32) -> Vec2 {
    let t = t.clamp(0.0, 1.0);
    let raw = match path {
        ConnectionPath::Straight { start, end } => *end - *start,
        ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } => cubic_bezier_tangent(t, *start, *control1, *control2, *end),
    };
    let len = raw.length();
    if len < f32::EPSILON {
        Vec2::X
    } else {
        raw / len
    }
}

// ---- tuning constants for the cubic closest-point search ----

/// Uniform-t sample count for the cubic-Bezier closest-point
/// search. 32 keeps the sweep well under 1µs at f32 and is
/// sufficient to seed the Newton refiner in the neighborhood of
/// the true minimum for labels on typical mindmap curvatures.
const CLOSEST_POINT_SAMPLES: usize = 32;

/// Newton iterations applied after the sampling sweep. 6 is more
/// than enough for quadratic convergence to f32 epsilon on
/// well-conditioned curves; caps the cost.
const CLOSEST_POINT_NEWTON_ITERS: usize = 6;

/// Minimum `|f'(t)|` before the Newton iteration bails. Below
/// this, `numer / denom` would produce a step that either flips
/// sign (overshoot the minimum) or flies off the [0, 1] range on
/// near-inflection cubics. The seed-vs-refined fallback below
/// catches the cases this early-break misses.
const CLOSEST_POINT_NEWTON_DENOM_EPSILON: f32 = f32::EPSILON;

/// Step-size below which Newton has converged. Each iteration
/// after this produces noise at f32 precision; bailing avoids
/// burning cycles on bit-level oscillation.
const CLOSEST_POINT_NEWTON_STEP_EPSILON: f32 = 1.0e-5;

/// Project `cursor` onto the closest point of `path` and return
/// `(t, perpendicular_offset)` — the path parameter at the
/// projection plus the signed normal-component offset from the path
/// to the cursor. Straight segments are projected directly; cubic
/// Bezier segments use the uniform-sample seed plus the Newton
/// refinement described on the constants above. The signed perp
/// is `to_cursor · normal_at_t(t)`; for cursors past `t = 0` or
/// `t = 1` the tangential component is discarded (right shape for
/// the edge-label drag that drives the caller — see
/// [`crate::mindmap::model::EdgeLabelConfig`]).
///
/// Cost: straight is O(1); cubic is O(`CLOSEST_POINT_SAMPLES`) for
/// the seed plus up to `CLOSEST_POINT_NEWTON_ITERS` Newton steps,
/// no allocation.
pub fn closest_point_on_path(path: &ConnectionPath, cursor: Vec2) -> (f32, f32) {
    match path {
        ConnectionPath::Straight { start, end } => {
            let ab = *end - *start;
            let len_sq = ab.length_squared();
            if len_sq < f32::EPSILON {
                // Degenerate segment — cursor projects to `start`
                // with zero perpendicular offset by convention.
                return (0.0, 0.0);
            }
            let t = ((cursor - *start).dot(ab) / len_sq).clamp(0.0, 1.0);
            let closest = *start + ab * t;
            let to_cursor = cursor - closest;
            let tangent = ab.normalize_or_zero();
            // Rotate tangent 90° in canvas coords (same rotation
            // `normal_at_t` uses) — matches the display semantics
            // of `EdgeLabelConfig::perpendicular_offset`.
            let normal = Vec2::new(-tangent.y, tangent.x);
            let perp = to_cursor.dot(normal);
            (t, perp)
        }
        ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } => {
            let p0 = *start;
            let p1 = *control1;
            let p2 = *control2;
            let p3 = *end;
            // Uniform t-sample sweep to find the neighborhood of
            // the closest point.
            let mut best_t = 0.0f32;
            let mut best_dist_sq = f32::MAX;
            for i in 0..=CLOSEST_POINT_SAMPLES {
                let t = i as f32 / CLOSEST_POINT_SAMPLES as f32;
                let point = cubic_bezier_point(t, p0, p1, p2, p3);
                let d = (point - cursor).length_squared();
                if d < best_dist_sq {
                    best_dist_sq = d;
                    best_t = t;
                }
            }
            // Newton refinement on f(t) = (B(t) - cursor) · B'(t).
            // f'(t) = B'(t) · B'(t) + (B(t) - cursor) · B''(t).
            // Bracket into [0, 1] after each step.
            let mut t = best_t;
            for _ in 0..CLOSEST_POINT_NEWTON_ITERS {
                let b = cubic_bezier_point(t, p0, p1, p2, p3);
                let bp = cubic_bezier_tangent(t, p0, p1, p2, p3);
                let bpp = cubic_bezier_second_derivative(t, p0, p1, p2, p3);
                let numer = (b - cursor).dot(bp);
                let denom = bp.dot(bp) + (b - cursor).dot(bpp);
                if denom.abs() < CLOSEST_POINT_NEWTON_DENOM_EPSILON {
                    break;
                }
                let next = (t - numer / denom).clamp(0.0, 1.0);
                if (next - t).abs() < CLOSEST_POINT_NEWTON_STEP_EPSILON {
                    t = next;
                    break;
                }
                t = next;
            }
            // Divergence guard: if Newton wandered worse than the
            // sampling seed (possible near inflection points
            // where `B''` flips sign and the step overshoots),
            // fall back to the seed. Without this the caller can
            // see `t` oscillate between 0 and 1 under a slow
            // drag even though the geometric closest point is
            // mid-curve.
            let refined_point = cubic_bezier_point(t, p0, p1, p2, p3);
            let refined_dist_sq = (refined_point - cursor).length_squared();
            let (final_t, closest) = if refined_dist_sq <= best_dist_sq {
                (t, refined_point)
            } else {
                (best_t, cubic_bezier_point(best_t, p0, p1, p2, p3))
            };
            let to_cursor = cursor - closest;
            let tangent = cubic_bezier_tangent(final_t, p0, p1, p2, p3).normalize_or_zero();
            let normal = Vec2::new(-tangent.y, tangent.x);
            let perp = to_cursor.dot(normal);
            (final_t, perp)
        }
    }
}

// closest-point tuning constants live alongside their user
// (`closest_point_on_path`). Kept `const` rather than inlined so a
// future reviewer sees the tuning knobs listed together.

/// Unit normal of `path` at `t`. Computed as the tangent rotated
/// 90° in canvas coordinates via `(x, y) → (-y, x)`.
///
/// **Orientation note** — mandala uses a Y-grows-down canvas
/// (`"top"` anchor has a smaller `y` than `"bottom"`, see
/// [`resolve_anchor_point`]). `(x, y) → (-y, x)` is
/// counter-clockwise in math coordinates but lands on the
/// **right-hand side of the direction of travel** from `start`
/// to `end` on screen. Downstream callers only need a stable
/// perpendicular — a positive
/// [`crate::mindmap::model::EdgeLabelConfig::perpendicular_offset`]
/// pushes the label in the returned direction, a negative one
/// pushes it the opposite way; the side is determined by the
/// caller's sign. The app's curve-straight-edge gesture (in the
/// mandala crate) also routes through this helper, so keyboard
/// and mouse path-bending agree on the same side.
pub fn normal_at_t(path: &ConnectionPath, t: f32) -> Vec2 {
    let tangent = tangent_at_t(path, t);
    Vec2::new(-tangent.y, tangent.x)
}

/// Total arc length of a connection path in canvas units. Straight
/// paths return the exact endpoint distance; cubic Bezier paths
/// approximate the length by walking `ARC_LENGTH_SUBDIVISIONS`
/// straight segments, so cost is O(subdivisions) with no allocation.
pub fn path_length(path: &ConnectionPath) -> f32 {
    match path {
        ConnectionPath::Straight { start, end } => start.distance(*end),
        ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } => cubic_bezier_length(*start, *control1, *control2, *end),
    }
}

/// Append evenly-spaced canvas-space points along `path` to `out`,
/// including the start point. The last point may be slightly before
/// the path endpoint if the remaining distance is less than
/// `spacing`. A non-positive `spacing` appends nothing.
///
/// **`out` is not cleared.** A caller refilling a buffer clears it
/// first; a caller concatenating several paths does not, and that is
/// the reason the choice is the caller's rather than baked in here.
///
/// This is the shape the connection pass uses, because the buffer it
/// fills is the one the scene cache already owns for that edge: a
/// resampling frame reuses the allocation instead of making a new
/// vector and copying into the cache. [`sample_path`] is the
/// allocating convenience wrapper over it.
///
/// # Costs
///
/// One curve evaluation per point, plus — for a cubic — the
/// `ARC_LENGTH_SUBDIVISIONS` evaluations of the arc-length table and
/// one binary search of it per point. `out` is reserved once for the
/// exact number of points about to be pushed, so an empty buffer
/// takes one allocation and no growth sequence, and a buffer with
/// room takes none.
pub fn sample_path_into(path: &ConnectionPath, spacing: f32, cap: usize, out: &mut Vec<Vec2>) {
    if spacing <= 0.0 {
        return;
    }

    match path {
        ConnectionPath::Straight { start, end } => extend_straight_samples(*start, *end, spacing, cap, out),
        ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } => extend_cubic_samples(*start, *control1, *control2, *end, spacing, cap, out),
    }
}

/// [`sample_path_into`] into a freshly allocated vector, for callers
/// with no buffer to reuse.
///
/// Cost: as [`sample_path_into`], plus the one allocation it makes.
pub fn sample_path(path: &ConnectionPath, spacing: f32, cap: usize) -> Vec<Vec2> {
    let mut out = Vec::new();
    sample_path_into(path, spacing, cap, &mut out);
    out
}

/// Hard ceiling on the number of glyph samples one connection path
/// may produce.
///
/// The count is `path length / spacing`, and **both terms come out
/// of the document**: the endpoints are node positions and the
/// spacing derives from an authored font size. A `.mindmap.json` is
/// untrusted input, so the quotient is attacker-controlled — and it
/// lands in `Vec::with_capacity`, where an over-large request is an
/// allocator abort rather than a catchable panic. The loader's
/// numeric-domain check (`model::validate`) already rejects the
/// coordinates and sizes that make this quotient absurd; this is
/// the second wall, so a future path that reaches the sampler
/// without passing the loader still cannot ask for terabytes.
///
/// **The sample vector is not what this bounds.** Each
/// sample becomes a repeat of the connection's body glyph
/// downstream: an owned clone of that string, a grapheme walk over
/// it, a `GlyphArea` in the arena, and a shaped cosmic-text buffer.
/// The real cost per sample is kilobytes, not the eight bytes the
/// point itself occupies, so a ceiling budgeted against the vector
/// would be wrong by three orders of magnitude — and the body's own
/// length is a second multiplier, capped separately by
/// [`MAX_CONNECTION_GLYPH_GRAPHEMES`](crate::mindmap::model::validate::MAX_CONNECTION_GLYPH_GRAPHEMES).
///
/// The number is a **canvas-space** budget, and has to be read that
/// way: `sample_count` divides a canvas-space length by a
/// canvas-space step, with no camera in the expression. At a typical
/// authored body size the step is a few canvas units, so this covers
/// a path of tens of thousands of units — far longer than any edge
/// in the canonical fixture — and beyond that the rail is silently
/// short rather than absent. A cap that could not be hit without an
/// absurd length or a sub-unit step is the trade being made; a
/// screen-space budget would need the zoom, which this layer does
/// not have.
///
/// It bounds one path. The aggregate across a map is bounded
/// separately by [`MAX_TOTAL_PATH_SAMPLES`], which is the ceiling that
/// actually matters: this one alone let a 73 KB file with 200 edges
/// ask for two million glyph areas.
pub const MAX_PATH_SAMPLES: usize = 10_000;

/// Ceiling on the connection glyphs one scene may emit, across every
/// edge.
///
/// **[`MAX_PATH_SAMPLES`] bounds a path and bounded nothing.** Each
/// sample becomes a glyph area in the scene arena, and an edge costs
/// about 120 bytes in the file — so a 73 KB document with 200 edges
/// reached 2 000 000 samples, a 2 000 201-node arena and 1 642 MiB
/// resident, from a file smaller than this source tree's smallest map.
/// A megabyte of the same shape would ask for tens of gigabytes. The
/// per-path cap was documented as bounding one path and not the
/// aggregate; this closes that.
///
/// **Spent as an equal share per edge rather than first-come.** The
/// budget divided by the edge count gives every edge the same
/// allowance, so the result does not depend on iteration order and no
/// edge renders while a later one vanishes. Density degrades across
/// the whole map at once, which is the graceful version of this
/// failure: fewer glyphs along each connection rather than some
/// connections missing.
///
/// Sized against real maps rather than against a guess.
/// `maps/testament.mindmap.json` uses 16 259 samples over 258 edges,
/// and `maps/stress_long_edges.mindmap.json` — the repository's own
/// worst case — uses 46 290 over 124. This is an order of magnitude
/// above the heavier of the two, so a map that is merely large is
/// untouched and only a pathological one is thinned.
pub const MAX_TOTAL_PATH_SAMPLES: usize = 500_000;

/// The per-path allowance when a scene has `edge_count` edges to draw.
///
/// Never more than [`MAX_PATH_SAMPLES`], never more than an equal
/// share of [`MAX_TOTAL_PATH_SAMPLES`], and never zero — a connection
/// that renders one glyph is still visible, and a connection that
/// renders none looks like a missing edge.
pub fn per_path_sample_budget(edge_count: usize) -> usize {
    let share = MAX_TOTAL_PATH_SAMPLES / edge_count.max(1);
    share.clamp(1, MAX_PATH_SAMPLES)
}

/// How many samples a path of `total_length` yields at `spacing`,
/// clamped to [`MAX_PATH_SAMPLES`] and total over hostile inputs.
///
/// Float-to-integer casts in Rust *saturate*, which is what makes
/// the naive form dangerous rather than merely wrong: an infinite
/// length reaches `usize::MAX` and the `+ 1` overflows it, while a
/// `NaN` casts to zero. Both are folded into the clamp here so
/// neither reaches an allocation.
///
/// Cost: a few float ops, no allocation.
fn sample_count(total_length: f32, spacing: f32, cap: usize) -> usize {
    if !total_length.is_finite() || !spacing.is_finite() || spacing <= 0.0 {
        return 1;
    }
    let count = (total_length / spacing).floor();
    if !count.is_finite() || count <= 0.0 {
        return 1;
    }
    (count as usize).saturating_add(1).min(cap.max(1))
}

fn extend_straight_samples(start: Vec2, end: Vec2, spacing: f32, cap: usize, out: &mut Vec<Vec2>) {
    let total_length = start.distance(end);
    if total_length < f32::EPSILON {
        out.push(start);
        return;
    }
    let count = sample_count(total_length, spacing, cap);
    out.reserve(count);
    for i in 0..count {
        let t = (i as f32 * spacing) / total_length;
        let t = t.min(1.0);
        out.push(start.lerp(end, t));
    }
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

/// Sample spacing, in canvas units, used when measuring a cubic
/// Bezier's distance to a point. Finer than the spacing the
/// connection pass renders at, because this polyline stands in for
/// the curve in a comparison rather than carrying glyphs.
const DISTANCE_SAMPLE_SPACING: f32 = 4.0;

/// Returns the minimum distance from `point` to the given connection path.
///
/// - `Straight`: exact point-to-segment distance.
/// - `CubicBezier`: walks the curve's sample sequence and returns the
///   minimum distance over all resulting polyline segments. This is an
///   approximation; at the module-private `DISTANCE_SAMPLE_SPACING`
///   the error is below one canvas unit for typical connection paths
///   — well within a click tolerance.
///
/// Cost: straight is O(1). Cubic is one arc-length table plus one
/// curve evaluation and one point-to-segment test per sample, and
/// **allocates nothing** — the samples are consumed as they are
/// produced rather than collected (contrast [`sample_path`], whose
/// caller needs the points afterwards).
///
/// A caller that only wants to know whether the path is within some
/// radius should ask [`distance_to_path_within`], which can answer
/// "no" from the path's bounding box without sampling at all.
pub fn distance_to_path(point: Vec2, path: &ConnectionPath) -> f32 {
    match path {
        ConnectionPath::Straight { start, end } => {
            point_to_segment_distance_squared(point, *start, *end).sqrt()
        }
        ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } => {
            let plan = plan_cubic_samples(
                *start,
                *control1,
                *control2,
                *end,
                DISTANCE_SAMPLE_SPACING,
                MAX_PATH_SAMPLES,
            );
            // `CubicSamples::len` is never zero, so index 0 always
            // resolves; a one-point sequence has no segment to
            // measure against and answers with the point itself.
            let mut prev = plan.position(0);
            if plan.len() == 1 {
                return point.distance(prev);
            }
            let mut min_sq = f32::INFINITY;
            for index in 1..plan.len() {
                let next = plan.position(index);
                let d = point_to_segment_distance_squared(point, prev, next);
                if d < min_sq {
                    min_sq = d;
                }
                prev = next;
            }
            min_sq.sqrt()
        }
    }
}

/// Axis-aligned bounding box of `path`'s control polygon, as
/// `(min, max)`.
///
/// **The box contains the path.** A cubic Bezier lies within the
/// convex hull of its four control points — its Bernstein basis is
/// non-negative and sums to one over `[0, 1]`, so every point of the
/// curve is a convex combination of `p0…p3` — and a convex hull lies
/// within the axis-aligned box of the points that generate it. A
/// straight path's two endpoints bound it the same way, with the
/// hull degenerated to the segment itself.
///
/// **It also contains what [`distance_to_path`] measures against**,
/// which is the stronger statement an early-out needs: that
/// function's cubic branch answers with the distance to a *polyline*
/// through sampled curve points, and a chord between two points of a
/// convex set stays inside it. So no segment it tests can leave this
/// box.
///
/// **Both statements are exact in real arithmetic and approximate in
/// `f32`, and the difference is load-bearing.** `cubic_bezier_point`
/// evaluates the Bernstein form, whose coefficients sum to one only
/// exactly, so a sampled point is a *nearly* convex combination and
/// can land outside this box by about `|coordinate| × f32::EPSILON`.
/// An axis-aligned control polygon — every control point on one x —
/// makes the box zero-width on that axis and every sample escape it.
/// A caller comparing a distance against this box must therefore
/// allow for that; `distance_to_path_within` does, through
/// `HULL_ESCAPE_SLACK`, and is the reason to reach for it rather than
/// to hand-roll the comparison.
///
/// The box is not tight — an S-curve's control points can sit well
/// outside the curve's own extent — which is the trade: four
/// component-wise `min`/`max` pairs and no root-finding on the
/// derivative.
///
/// Cost: O(1), no allocation. A non-finite control point is not
/// screened here, and what comes back for one is deliberately not
/// promised: `Vec2::min` returns whichever operand its comparison
/// falls through to, so whether a `NaN` survives into the bound
/// depends on which side of the fold it lands, while an infinity
/// always survives. Callers must not read a finite box as evidence
/// that the path is finite. `distance_to_path_within` does not — it
/// answers `None` either way, because the measured distance is
/// non-finite and fails its `<=` test —  and
/// `test_distance_to_path_within_holds_over_non_finite_geometry`
/// drives every placement of every non-finite value across all four
/// control points.
pub fn path_bounds(path: &ConnectionPath) -> (Vec2, Vec2) {
    match path {
        ConnectionPath::Straight { start, end } => (start.min(*end), start.max(*end)),
        ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } => (
            start.min(*control1).min(control2.min(*end)),
            start.max(*control1).max(control2.max(*end)),
        ),
    }
}

/// Whether `point` sits further than `tolerance` outside the box
/// `[min, max]` on at least one axis.
///
/// Written as a subtraction compared against `tolerance` rather than
/// as a containment test against a box inflated by `tolerance`, and
/// the reason is float precision rather than style. Inflating rounds
/// `max + tolerance` at the magnitude of a *canvas coordinate*,
/// which on a large map is a coarse place to round; subtracting
/// rounds at the magnitude of the *result*, which is the tolerance
/// itself. Both forms decide the same thing in exact arithmetic; the
/// second keeps the rounding error small relative to the quantity
/// being compared.
///
/// A `NaN` anywhere makes every comparison false, so the answer is
/// "not outside" — the safe direction, since the caller then does
/// the full computation instead of trusting this.
///
/// `margin` is per-axis because the slack the caller needs is not:
/// one term of it scales with the coordinate magnitude, and a path
/// can be a thousand units from the origin on one axis and a million
/// on the other.
fn outside_bounds_by(point: Vec2, min: Vec2, max: Vec2, margin: Vec2) -> bool {
    min.x - point.x > margin.x
        || point.x - max.x > margin.x
        || min.y - point.y > margin.y
        || point.y - max.y > margin.y
}

/// Relative slack added to [`distance_to_path_within`]'s reject
/// threshold, so the early-out stays conservative **after** float
/// rounding and not only in exact arithmetic.
///
/// The reject compares an axis overhang — one correctly-rounded
/// subtraction — against `tolerance`, while the value it is
/// protecting comes out of [`distance_to_path`]'s longer chain of
/// dot products, a clamp, and a square root. Each sits within a few
/// ulps of the exact quantity it approximates, and a few ulps on
/// either side of a strict `>` is enough for the two to disagree
/// about a point lying exactly `tolerance` from the path. Widening
/// the reject by more ulps than either chain can lose removes the
/// disagreement rather than making it unlikely: a rejected point's
/// overhang then exceeds `tolerance` by more than the combined
/// rounding of both computations, so its measured distance does too.
///
/// 32 × [`f32::EPSILON`] is 2⁻¹⁸ — a relative 4 × 10⁻⁶, orders of
/// magnitude above the handful of ulps in play and orders below
/// anything a canvas-space click radius can tell apart. The points
/// it changes the outcome for are exactly those whose distance falls
/// inside that band around `tolerance`, and for those the full
/// computation runs and decides.
///
/// **This margin is argued, not observed.** No input is known that
/// needs it: `test_distance_to_path_within_agrees_with_the_unbounded_form`
/// drives the corpus at `tolerance` exactly equal to the measured
/// distance — the knife edge — and passes with the margin removed.
/// What the margin buys is that the soundness argument above stops
/// depending on that: without it the argument holds in real
/// arithmetic and is merely very likely in `f32`, and "very likely"
/// is not what a hit test should rest on. The same test fails when
/// the margin is *inverted*, which is what says the sweep can
/// resolve a shift of this size at all.
///
/// **It covers the comparison and nothing else.** The larger error —
/// the curve evaluation leaving the control-point box at all — does
/// not scale with `tolerance` and is covered by
/// [`HULL_ESCAPE_SLACK`], which is a separate term for a separate
/// reason. Scaling one margin to `tolerance` and expecting it to
/// absorb both is the defect that shipped in the first version of
/// this function.
const BOUNDS_REJECT_SLACK: f32 = 32.0 * f32::EPSILON;

/// Slack, relative to the path's **coordinate magnitude**, covering
/// how far a sampled point can fall outside [`path_bounds`]'s box.
///
/// [`path_bounds`]'s containment claim is exact in real arithmetic
/// and false in `f32`, and the reason is in `cubic_bezier_point`: the
/// Bernstein coefficients sum to one only exactly, so the evaluated
/// point is a *nearly* convex combination and can land a rounding
/// step outside the box its control points span. The error is
/// relative to the coordinate — around `|coordinate| × f32::EPSILON`
/// — and so has nothing to do with `tolerance`.
///
/// **The shape that makes it visible is an axis-aligned control
/// polygon**, which is ordinary content rather than a corner case:
/// two nodes stacked vertically with control offsets whose x is zero
/// — "curve this edge straight up" — puts all four control points on
/// one x, and the box is then zero-width on that axis with nowhere
/// for the rounding to hide. Every sample escapes, by
/// `|x| × f32::EPSILON`, and at a canvas x of 10⁴ that is already
/// larger than a click tolerance at high zoom.
///
/// **The term applies to `Straight` paths as well, and there its
/// status is different — say so rather than bank it.**
/// [`path_bounds`] is *exact* for a straight segment (it is the two
/// endpoints), so no hull escape exists there. What remains is
/// [`distance_to_path`]'s straight branch, which reaches its answer
/// through `point_to_segment_distance_squared`: the accept side
/// compares that computed distance against `tolerance` while the
/// reject side compares an exact axis overhang, and the projection
/// `a + ab * t` rounds relative to the coordinate. So the same
/// disagreement is *available* in principle.
///
/// It has not been exhibited. The window is narrow by construction —
/// a correctly-rounded `a + ab * t` cannot put the closest point more
/// than half an ulp of the coordinate outside the box, so a
/// disagreement needs the true distance to sit inside that half-ulp
/// of `tolerance` — and a search aimed straight at it found nothing:
/// 96 000 000 probes over five magnitudes (10³–10⁷), six tolerances
/// from 0.02 to 12, segment orientations swept from near-vertical to
/// near-horizontal, and offsets stepped finely through the tolerance
/// boundary on the escaping face, produced **zero** disagreements
/// against a tolerance-only margin. The straight corpus entries added
/// alongside this note likewise pass with the term removed.
///
/// So this paragraph is an argument, not a measurement, and it is
/// labeled as one on purpose: promoting it to a demonstrated effect
/// would be the same move — a claim outrunning its evidence — that
/// put the defect this constant fixes into the tree. The term covers
/// straight paths because it is cheaper to apply it uniformly than to
/// reason about which branch needs it, and that is the whole of the
/// claim.
///
/// **What 32 is chosen against, and what that does and does not
/// mean.** No closed-form bound on the escape is derived anywhere in
/// this tree, so every figure available is a measured maximum over a
/// finite sweep — a sample, not a ceiling. Successive denser sweeps
/// have each found a larger number than the one before, which is the
/// honest shape of that situation rather than a reason to distrust
/// any of them. The largest figure any sweep has produced is
/// recorded once, in `MEASURED_WORST_ESCAPE_ULPS` beside
/// `test_path_bounds_slack_covers_the_sampler_escape`, together with
/// the methods that produced it; it is deliberately not restated
/// here, because a figure copied into a second place is a figure
/// that goes stale silently. That test asserts in both directions —
/// no sweep may exceed the record, and this constant must stay at
/// least four times above it — so the two cannot drift apart.
///
/// **What the sweeps establish that a number could not** is that the
/// escape is *magnitude-invariant* in these units: the worst per
/// decade varies by well under a factor of two from 10⁰ to 10²⁰.
/// That is the finding this constant actually rests on, because it
/// is what makes `|coordinate|` the right quantity to scale by at
/// all; the particular ulp count only sets where to put the ceiling.
/// The invariance is asserted rather than quoted, in the same test.
///
/// The asymmetry is what makes generous the right side to err on:
/// over-covering costs a failure to reject, which spends the full
/// computation and returns the right answer, while under-covering
/// drops a click.
const HULL_ESCAPE_SLACK: f32 = 32.0 * f32::EPSILON;

/// [`distance_to_path`], answered only when the answer is within
/// `tolerance` — the shape a hit test wants.
///
/// **Contract:** returns `Some(d)` exactly when
/// `distance_to_path(point, path)` is a `d` satisfying
/// `d <= tolerance`, and `None` otherwise. The bounding-box test it
/// opens with is an optimization inside that contract, not a
/// relaxation of it.
///
/// **Why the early-out cannot reject a true hit.**
/// [`path_bounds`] returns a box containing every segment
/// [`distance_to_path`] measures against (see its doc for why). If
/// `point` lies more than `tolerance` outside that box on some axis,
/// then its distance to every point of the box — and so to every
/// segment inside it — exceeds `tolerance` on that axis alone, so the
/// full computation could only have returned a value the `<=` test
/// would reject. The reject is therefore sound for *any* box that
/// contains the path, which is what makes a loose one safe to use.
/// Two module-private slacks carry that argument across float
/// rounding, and they are separate because the two errors scale with
/// different quantities. `BOUNDS_REJECT_SLACK` is relative to
/// `tolerance` and covers the comparison itself, so the two routes
/// cannot disagree on a point sitting exactly `tolerance` away.
/// `HULL_ESCAPE_SLACK` is relative to the path's coordinate
/// magnitude and covers the sampled points that fall outside the box
/// — which they do, because the containment above is a
/// real-arithmetic statement. Folding both into one `tolerance`-
/// scaled margin is what the first version of this function did, and
/// it dropped clicks on axis-aligned curves far from the origin.
///
/// A `NaN` in `point` or in the path falls out of the contract
/// rather than needing a case: `distance_to_path` is then `NaN`,
/// `NaN <= tolerance` is false, and `None` is the answer both routes
/// give. That differs from a caller that spelled its own filter
/// `distance > tolerance` — `NaN > tolerance` is *also* false, so
/// such a caller kept the path as a candidate at an unordered
/// distance.
///
/// Cost: O(1) to reject. Otherwise [`distance_to_path`]'s cost plus
/// that O(1). No allocation on either route.
pub fn distance_to_path_within(point: Vec2, path: &ConnectionPath, tolerance: f32) -> Option<f32> {
    let (min, max) = path_bounds(path);
    // Two independent error terms, and they scale with different
    // quantities: the comparison's own rounding with `tolerance`, the
    // sampler's escape from the box with the coordinate magnitude.
    let comparison = tolerance + tolerance.abs() * BOUNDS_REJECT_SLACK;
    let magnitude = Vec2::new(min.x.abs().max(max.x.abs()), min.y.abs().max(max.y.abs()));
    let margin = Vec2::splat(comparison) + magnitude * HULL_ESCAPE_SLACK;
    if outside_bounds_by(point, min, max, margin) {
        return None;
    }
    let distance = distance_to_path(point, path);
    (distance <= tolerance).then_some(distance)
}
