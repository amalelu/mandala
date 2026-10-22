// SPDX-License-Identifier: MPL-2.0

//! Cubic-Bezier math and arc-length sampling. Kept in its own file so
//! the sampling internals are easy to skim without wading through the
//! higher-level `sample_path` / `distance_to_path` surface in
//! [`super`].

use glam::Vec2;

/// Number of subdivisions for arc-length approximation on Bezier curves.
pub(super) const ARC_LENGTH_SUBDIVISIONS: usize = 256;

/// Cumulative arc-length table for one cubic Bezier — one entry per
/// subdivision boundary, so `ARC_LENGTH_SUBDIVISIONS + 1` of them.
///
/// A fixed-size array rather than a `Vec`: the length is a
/// compile-time constant, so the table can live in the frame of
/// whoever asked for it and the sampler owns no heap allocation at
/// all (§B1, §B7).
type ArcLengthTable = [f32; ARC_LENGTH_SUBDIVISIONS + 1];

/// Evaluates a cubic Bezier curve at parameter t in [0, 1].
pub(crate) fn cubic_bezier_point(t: f32, p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2) -> Vec2 {
    let u = 1.0 - t;
    let uu = u * u;
    let uuu = uu * u;
    let tt = t * t;
    let ttt = tt * t;
    uuu * p0 + 3.0 * uu * t * p1 + 3.0 * u * tt * p2 + ttt * p3
}

/// Analytical derivative of a cubic Bezier curve at parameter t.
/// Used to compute the path tangent (and thus the normal) for
/// label positioning. Returns an unnormalized tangent vector; the
/// caller normalizes. Degenerate paths (coincident control points)
/// can produce a zero-length tangent — callers handle that by
/// falling back to the straight-segment direction.
pub(crate) fn cubic_bezier_tangent(t: f32, p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2) -> Vec2 {
    let u = 1.0 - t;
    // d/dt [ (1-t)^3 p0 + 3(1-t)^2 t p1 + 3(1-t) t^2 p2 + t^3 p3 ]
    //     = 3(1-t)^2 (p1 - p0) + 6(1-t)t (p2 - p1) + 3 t^2 (p3 - p2)
    3.0 * u * u * (p1 - p0) + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (p3 - p2)
}

/// Analytical second derivative of a cubic Bezier curve at
/// parameter t. Backs the Newton refinement inside
/// [`super::closest_point_on_path`]: Newton needs `f'(t)` where
/// `f(t) = (B(t) - cursor) · B'(t)`, which expands to include
/// `B''(t)`. Returns the unnormalized second-derivative vector.
pub(crate) fn cubic_bezier_second_derivative(t: f32, p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2) -> Vec2 {
    // d²/dt² [ (1-t)^3 p0 + 3(1-t)^2 t p1 + 3(1-t) t^2 p2 + t^3 p3 ]
    //      = 6(1-t)(p2 - 2 p1 + p0) + 6 t (p3 - 2 p2 + p1)
    let u = 1.0 - t;
    6.0 * u * (p2 - 2.0 * p1 + p0) + 6.0 * t * (p3 - 2.0 * p2 + p1)
}

/// Build the cumulative arc-length table for a cubic Bezier — one
/// entry per subdivision boundary, monotonically non-decreasing.
/// `table[0] == 0.0`, `table[ARC_LENGTH_SUBDIVISIONS]` is the total
/// polyline length.
///
/// The table backs [`plan_cubic_samples`], which binary-searches it
/// to invert arc-length → `t`. [`cubic_bezier_length`] does **not**
/// go through here: it wants only the final entry, and a running sum
/// reaches that without a table.
///
/// Cost: `ARC_LENGTH_SUBDIVISIONS` curve evaluations and as many
/// distances. No allocation — the table is an [`ArcLengthTable`],
/// which is an array.
fn build_arc_length_table(start: Vec2, control1: Vec2, control2: Vec2, end: Vec2) -> ArcLengthTable {
    let n = ARC_LENGTH_SUBDIVISIONS;
    let mut arc_lengths: ArcLengthTable = [0.0; ARC_LENGTH_SUBDIVISIONS + 1];
    let mut prev = start;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let pt = cubic_bezier_point(t, start, control1, control2, end);
        arc_lengths[i] = arc_lengths[i - 1] + prev.distance(pt);
        prev = pt;
    }
    arc_lengths
}

/// Total arc length of a cubic Bezier curve, approximated by
/// walking `ARC_LENGTH_SUBDIVISIONS` straight segments between
/// evenly-spaced parameter samples.
///
/// Accumulated as a running sum rather than through
/// [`build_arc_length_table`]: the intermediate partial sums are
/// what a table stores, and nothing here reads them. The additions
/// happen in the same left-to-right order the table's recurrence
/// used, so the answer is the same float and not merely a close one
/// — `test_bezier_length_equals_the_arc_length_table_total` holds
/// that against an independently built table.
///
/// Cost: `ARC_LENGTH_SUBDIVISIONS` curve evaluations and as many
/// distances, in `O(1)` space. No allocation.
pub(super) fn cubic_bezier_length(start: Vec2, control1: Vec2, control2: Vec2, end: Vec2) -> f32 {
    let n = ARC_LENGTH_SUBDIVISIONS;
    let mut total = 0.0f32;
    let mut prev = start;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let pt = cubic_bezier_point(t, start, control1, control2, end);
        total += prev.distance(pt);
        prev = pt;
    }
    total
}

/// The arc-length-uniform sample sequence of one cubic Bezier,
/// planned but not materialized.
///
/// Holds the curve, its arc-length table and the sample count, and
/// answers [`position`](CubicSamples::position) for each index in
/// `0..len()`. Two callers want the same sequence for different
/// reasons and this is what keeps one copy of the math between them:
/// [`extend_cubic_samples`] appends it to a buffer the connection
/// pass owns, because that pass needs the points to outlive the
/// walk, while [`super::distance_to_path`] consumes each point as it
/// arrives and needs no buffer at all.
///
/// The struct is a plain value carrying an [`ArcLengthTable`]; it
/// owns no heap allocation and is meant to live on the stack of the
/// function that plans it.
pub(super) struct CubicSamples {
    start: Vec2,
    control1: Vec2,
    control2: Vec2,
    end: Vec2,
    arc_lengths: ArcLengthTable,
    total_length: f32,
    spacing: f32,
    count: usize,
}

/// Plan the sample sequence for one cubic Bezier at `spacing`,
/// capped at `cap` points.
///
/// Cost: one [`build_arc_length_table`] — `ARC_LENGTH_SUBDIVISIONS`
/// curve evaluations — and no allocation. Each subsequent
/// [`CubicSamples::position`] is one binary search over the table
/// plus one curve evaluation.
pub(super) fn plan_cubic_samples(
    start: Vec2,
    control1: Vec2,
    control2: Vec2,
    end: Vec2,
    spacing: f32,
    cap: usize,
) -> CubicSamples {
    let arc_lengths = build_arc_length_table(start, control1, control2, end);
    let total_length = arc_lengths[ARC_LENGTH_SUBDIVISIONS];
    // A curve with no measurable length is one point, whatever the
    // spacing says. Guarding here rather than leaning on
    // `sample_count` is deliberate: a sub-epsilon length divided by a
    // sub-epsilon spacing is a large count over a curve that has
    // nowhere to put the points.
    let count = if total_length < f32::EPSILON {
        1
    } else {
        super::sample_count(total_length, spacing, cap)
    };
    CubicSamples {
        start,
        control1,
        control2,
        end,
        arc_lengths,
        total_length,
        spacing,
        count,
    }
}

impl CubicSamples {
    /// How many points the sequence has. Never zero — `sample_count`
    /// floors at one and the degenerate branch above sets one — so a
    /// caller may always ask for `position(0)`.
    pub(super) fn len(&self) -> usize {
        self.count
    }

    /// The `index`-th sampled point, in canvas space.
    ///
    /// Cost: one binary search of the arc-length table plus one curve
    /// evaluation. No allocation.
    pub(super) fn position(&self, index: usize) -> Vec2 {
        if self.total_length < f32::EPSILON {
            return self.start;
        }
        let target_len = (index as f32 * self.spacing).min(self.total_length);
        let t = arc_length_to_t(&self.arc_lengths, target_len, ARC_LENGTH_SUBDIVISIONS);
        cubic_bezier_point(t, self.start, self.control1, self.control2, self.end)
    }
}

/// Append the whole sample sequence of a cubic Bezier to `out`, for
/// callers that need the points to outlive the walk.
///
/// Writes into a caller-owned buffer rather than returning one: the
/// connection pass refills the same edge's buffer every resampling
/// frame, and a buffer that already has the capacity does not
/// allocate at all. `out` is reserved for the exact number of points
/// about to be pushed, so a fresh buffer takes one allocation and no
/// growth sequence.
///
/// Cost: one [`plan_cubic_samples`] — `ARC_LENGTH_SUBDIVISIONS`
/// curve evaluations — plus one binary search and one curve
/// evaluation per point. A caller that only reads each point once
/// should plan the sequence and walk it instead of buffering it.
pub(super) fn extend_cubic_samples(
    start: Vec2,
    control1: Vec2,
    control2: Vec2,
    end: Vec2,
    spacing: f32,
    cap: usize,
    out: &mut Vec<Vec2>,
) {
    let plan = plan_cubic_samples(start, control1, control2, end, spacing, cap);
    out.reserve(plan.len());
    for index in 0..plan.len() {
        out.push(plan.position(index));
    }
}

/// Binary search the arc-length table to find the t value for a given arc length.
fn arc_length_to_t(arc_lengths: &[f32], target_len: f32, n: usize) -> f32 {
    let mut lo = 0usize;
    let mut hi = n;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if arc_lengths[mid] < target_len {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        return 0.0;
    }
    let seg_start = arc_lengths[lo - 1];
    let seg_end = arc_lengths[lo];
    let seg_len = seg_end - seg_start;
    let frac = if seg_len > f32::EPSILON {
        (target_len - seg_start) / seg_len
    } else {
        0.0
    };
    ((lo - 1) as f32 + frac) / n as f32
}
