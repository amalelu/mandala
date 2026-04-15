//! Cubic-Bezier math and arc-length sampling. Kept in its own file so
//! the sampling internals are easy to skim without wading through the
//! higher-level `sample_path` / `distance_to_path` surface in
//! [`super`].

use glam::Vec2;

use super::SampledPoint;

/// Number of subdivisions for arc-length approximation on Bezier curves.
pub(super) const ARC_LENGTH_SUBDIVISIONS: usize = 256;

/// Evaluates a cubic Bezier curve at parameter t in [0, 1].
pub(crate) fn cubic_bezier_point(t: f32, p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2) -> Vec2 {
    let u = 1.0 - t;
    let uu = u * u;
    let uuu = uu * u;
    let tt = t * t;
    let ttt = tt * t;
    uuu * p0 + 3.0 * uu * t * p1 + 3.0 * u * tt * p2 + ttt * p3
}

/// Total arc length of a cubic Bezier curve, approximated by walking
/// `ARC_LENGTH_SUBDIVISIONS` straight segments between evenly-spaced
/// parameter samples.
pub(super) fn cubic_bezier_length(
    start: Vec2,
    control1: Vec2,
    control2: Vec2,
    end: Vec2,
) -> f32 {
    let mut length = 0.0f32;
    let mut prev = start;
    for i in 1..=ARC_LENGTH_SUBDIVISIONS {
        let t = i as f32 / ARC_LENGTH_SUBDIVISIONS as f32;
        let pt = cubic_bezier_point(t, start, control1, control2, end);
        length += prev.distance(pt);
        prev = pt;
    }
    length
}

pub(super) fn sample_cubic_bezier(
    start: Vec2,
    control1: Vec2,
    control2: Vec2,
    end: Vec2,
    spacing: f32,
) -> Vec<SampledPoint> {
    // Build arc-length lookup table
    let n = ARC_LENGTH_SUBDIVISIONS;
    let mut arc_lengths = Vec::with_capacity(n + 1);
    arc_lengths.push(0.0f32);
    let mut prev = start;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let pt = cubic_bezier_point(t, start, control1, control2, end);
        let prev_len = arc_lengths[i - 1];
        arc_lengths.push(prev_len + prev.distance(pt));
        prev = pt;
    }
    let total_length = *arc_lengths.last().unwrap();
    if total_length < f32::EPSILON {
        return vec![SampledPoint { position: start }];
    }

    // Sample at even arc-length intervals
    let count = (total_length / spacing).floor() as usize + 1;
    let mut points = Vec::with_capacity(count);
    for i in 0..count {
        let target_len = (i as f32 * spacing).min(total_length);
        let t = arc_length_to_t(&arc_lengths, target_len, n);
        let position = cubic_bezier_point(t, start, control1, control2, end);
        points.push(SampledPoint { position });
    }
    points
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
