//! Tests for anchor resolution, path sampling, Bezier math, and edge
//! hit-test. Includes performance regression guards for the long-edge
//! code paths whose drag-frame cost is governed by the invariant
//! "sample count scales linearly with path length, independently of
//! the arc-length subdivision table size".

use super::*;
use crate::mindmap::model::ControlPoint;

#[test]
fn test_anchor_top() {
    let pos = Vec2::new(100.0, 200.0);
    let size = Vec2::new(80.0, 40.0);
    let pt = resolve_anchor_point(pos, size, "top", Vec2::ZERO);
    assert_eq!(pt, Vec2::new(140.0, 200.0));
}

#[test]
fn test_anchor_right() {
    let pos = Vec2::new(100.0, 200.0);
    let size = Vec2::new(80.0, 40.0);
    let pt = resolve_anchor_point(pos, size, "right", Vec2::ZERO);
    assert_eq!(pt, Vec2::new(180.0, 220.0));
}

#[test]
fn test_anchor_bottom() {
    let pos = Vec2::new(100.0, 200.0);
    let size = Vec2::new(80.0, 40.0);
    let pt = resolve_anchor_point(pos, size, "bottom", Vec2::ZERO);
    assert_eq!(pt, Vec2::new(140.0, 240.0));
}

#[test]
fn test_anchor_left() {
    let pos = Vec2::new(100.0, 200.0);
    let size = Vec2::new(80.0, 40.0);
    let pt = resolve_anchor_point(pos, size, "left", Vec2::ZERO);
    assert_eq!(pt, Vec2::new(100.0, 220.0));
}

#[test]
fn test_anchor_auto_picks_nearest() {
    let pos = Vec2::new(0.0, 0.0);
    let size = Vec2::new(100.0, 50.0);
    // Other node is far to the right -- should pick right edge midpoint
    let other = Vec2::new(500.0, 25.0);
    let pt = resolve_anchor_point(pos, size, "auto", other);
    assert_eq!(pt, Vec2::new(100.0, 25.0)); // right edge midpoint
}

#[test]
fn test_anchor_auto_picks_top() {
    let pos = Vec2::new(0.0, 100.0);
    let size = Vec2::new(100.0, 50.0);
    // Other node is above
    let other = Vec2::new(50.0, -500.0);
    let pt = resolve_anchor_point(pos, size, "auto", other);
    assert_eq!(pt, Vec2::new(50.0, 100.0)); // top edge midpoint
}

#[test]
fn test_build_straight_path() {
    let path = build_connection_path(
        Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0), "right",  // from: right anchor
        Vec2::new(200.0, 0.0), Vec2::new(100.0, 50.0), "left", // to: left anchor
        &[],
    );
    match path {
        ConnectionPath::Straight { start, end } => {
            assert_eq!(start, Vec2::new(100.0, 25.0));
            assert_eq!(end, Vec2::new(200.0, 25.0));
        }
        _ => panic!("Expected Straight path"),
    }
}

#[test]
fn test_build_cubic_path() {
    let cps = vec![
        ControlPoint { x: 50.0, y: 0.0 },
        ControlPoint { x: -50.0, y: 0.0 },
    ];
    let path = build_connection_path(
        Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0), "right",
        Vec2::new(300.0, 0.0), Vec2::new(100.0, 50.0), "left",
        &cps,
    );
    match path {
        ConnectionPath::CubicBezier { start, control1, control2, end } => {
            assert_eq!(start, Vec2::new(100.0, 25.0));
            assert_eq!(end, Vec2::new(300.0, 25.0));
            // control1 = from_center + offset = (50,25) + (50,0) = (100, 25)
            assert_eq!(control1, Vec2::new(100.0, 25.0));
            // control2 = to_center + offset = (350,25) + (-50,0) = (300, 25)
            assert_eq!(control2, Vec2::new(300.0, 25.0));
        }
        _ => panic!("Expected CubicBezier path"),
    }
}

#[test]
fn test_straight_path_length() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let len = path_length(&path);
    assert!((len - 100.0).abs() < 0.01);
}

#[test]
fn test_straight_sampling() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 10.0);
    assert_eq!(points.len(), 11); // 0, 10, 20, ..., 100
    // First point at start
    assert!((points[0].position.x - 0.0).abs() < 0.01);
    // Last point at or near 100
    assert!((points[10].position.x - 100.0).abs() < 0.01);
    // All y should be 0
    for p in &points {
        assert!((p.position.y).abs() < 0.01);
    }
}

#[test]
fn test_straight_sampling_diagonal() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(30.0, 40.0), // length = 50
    };
    let points = sample_path(&path, 10.0);
    assert_eq!(points.len(), 6); // 0, 10, 20, 30, 40, 50
}

#[test]
fn test_collinear_bezier_length() {
    // Control points on the line -> arc length should equal straight distance
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.33, 0.0),
        control2: Vec2::new(66.67, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let len = path_length(&path);
    assert!((len - 100.0).abs() < 0.5, "Expected ~100, got {}", len);
}

#[test]
fn test_curved_bezier_longer_than_straight() {
    // Control points perpendicular -> arc length > straight distance
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.0, 100.0),
        control2: Vec2::new(67.0, -100.0),
        end: Vec2::new(100.0, 0.0),
    };
    let straight_dist = 100.0f32;
    let arc_len = path_length(&path);
    assert!(arc_len > straight_dist, "Arc length {} should exceed straight {}", arc_len, straight_dist);
}

#[test]
fn test_curved_bezier_sampling() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.0, 100.0),
        control2: Vec2::new(67.0, -100.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 10.0);
    // Curved path is longer than 100, so should have more than 11 points
    assert!(points.len() > 11, "Expected >11 points, got {}", points.len());
    // First point near start
    assert!(points[0].position.distance(Vec2::ZERO) < 1.0);
}

#[test]
fn test_sample_path_zero_spacing() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 0.0);
    assert!(points.is_empty());
}

#[test]
fn test_sample_path_degenerate() {
    // Zero-length path
    let path = ConnectionPath::Straight {
        start: Vec2::new(50.0, 50.0),
        end: Vec2::new(50.0, 50.0),
    };
    let points = sample_path(&path, 10.0);
    assert_eq!(points.len(), 1);
}

#[test]
fn test_distance_to_straight_on_path() {
    // Point lying exactly on a horizontal segment -> distance 0
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(50.0, 0.0), &path);
    assert!(d.abs() < 0.01);
}

#[test]
fn test_distance_to_straight_perpendicular() {
    // Perpendicular offset of 5 above the path midpoint
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(50.0, 5.0), &path);
    assert!((d - 5.0).abs() < 0.01, "expected ~5, got {}", d);
}

#[test]
fn test_distance_to_straight_past_endpoint() {
    // Point beyond `end`: distance should be to the end, not the
    // infinite line.
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(110.0, 0.0), &path);
    assert!((d - 10.0).abs() < 0.01, "expected ~10, got {}", d);
}

#[test]
fn test_distance_to_straight_diagonal() {
    // Diagonal segment (0,0)->(30,40), length 50.
    // Point at (0,50): expected distance from segment ~ ?
    // Perpendicular foot on segment is at t = (0*30 + 50*40)/2500 = 0.8
    // -> closest = (24, 32), distance = sqrt(24^2 + 18^2) = sqrt(576+324) = 30
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(30.0, 40.0),
    };
    let d = distance_to_path(Vec2::new(0.0, 50.0), &path);
    assert!((d - 30.0).abs() < 0.01, "expected 30, got {}", d);
}

#[test]
fn test_distance_to_zero_length_path() {
    // Degenerate (zero-length) segment: distance is point-to-point
    let path = ConnectionPath::Straight {
        start: Vec2::new(50.0, 50.0),
        end: Vec2::new(50.0, 50.0),
    };
    let d = distance_to_path(Vec2::new(50.0, 60.0), &path);
    assert!((d - 10.0).abs() < 0.01);
}

#[test]
fn test_distance_to_cubic_bezier_on_curve() {
    // A straight-ish cubic: control points collinear with endpoints
    // means the "curve" is effectively a line from (0,0) to (100,0).
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.33, 0.0),
        control2: Vec2::new(66.67, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(50.0, 0.0), &path);
    assert!(d < 0.5, "expected ~0, got {}", d);
}

#[test]
fn test_distance_to_cubic_bezier_perpendicular() {
    // Point 5 units above the midpoint of a collinear bezier (straight
    // line in practice): distance should be ~5
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.33, 0.0),
        control2: Vec2::new(66.67, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(50.0, 5.0), &path);
    assert!((d - 5.0).abs() < 0.5, "expected ~5, got {}", d);
}

#[test]
fn test_build_quadratic_promotion() {
    let cps = vec![ControlPoint { x: 0.0, y: 100.0 }];
    let path = build_connection_path(
        Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0), "auto",
        Vec2::new(200.0, 0.0), Vec2::new(100.0, 50.0), "auto",
        &cps,
    );
    match path {
        ConnectionPath::CubicBezier { .. } => { /* promoted correctly */ }
        _ => panic!("Expected CubicBezier from quadratic promotion"),
    }
}

// -----------------------------------------------------------------
// Performance regression guards
//
// These tests do not assert wall-clock timings (flaky under CI load).
// They assert the behavioural invariant the drag-frame sampler
// relies on: long paths must emit a sample count proportional to
// `length / spacing`, not capped at the arc-length subdivision
// table size. Breaking this reintroduces the long-connection drag
// stutter the sampler was tuned to avoid.
// -----------------------------------------------------------------

/// A 20,000-unit straight path sampled at spacing 15 must produce a
/// sample count proportional to length/spacing, not capped at
/// `ARC_LENGTH_SUBDIVISIONS` (256). Guards against a regression that
/// clamped sample count to the arc-length table size.
#[test]
fn test_sample_long_straight_scales_linearly_with_length() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(20_000.0, 0.0),
    };
    let points = sample_path(&path, 15.0);
    // Expected: floor(20000/15) + 1 = 1334.
    assert_eq!(points.len(), 1334);
    // Way above the 256-subdivision table size — proves no clamp.
    assert!(points.len() > 1000,
        "sample count {} should scale with length, not subdivisions",
        points.len());
}

/// A long cubic bezier's sample count is linear in path length, not in
/// the subdivision table size. If the arc-length lookup regressed to
/// walking the table per sample (O(N·subdivisions)) instead of binary
/// search, the test would still pass — the invariant we care about
/// here is that sample count itself tracks length, and that's what
/// this guards.
#[test]
fn test_sample_long_bezier_count_bounded_by_length() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(5_000.0, 800.0),
        control2: Vec2::new(15_000.0, -800.0),
        end: Vec2::new(20_000.0, 0.0),
    };
    let length = path_length(&path);
    let spacing = 15.0;
    let points = sample_path(&path, spacing);
    let expected_floor = (length / spacing) as usize;
    // Sampler emits `floor(length/spacing) + 1` points. Allow a window
    // of ±2 to tolerate FP drift at the endpoint.
    assert!(points.len() >= expected_floor,
        "expected at least {}, got {}", expected_floor, points.len());
    assert!(points.len() <= expected_floor + 2,
        "expected at most {}, got {}", expected_floor + 2, points.len());
    // Sanity: we're in the "long edge" regime the sample-count
    // invariant targets.
    assert!(points.len() > 1000);
}

/// On a straight path, successive samples must be ordered along the
/// path direction. Catches an off-by-one or reversed loop in the arc
/// length → t conversion.
#[test]
fn test_sample_path_monotonic_along_straight() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(1000.0, 0.0),
    };
    let points = sample_path(&path, 10.0);
    assert!(points.len() > 2);
    for pair in points.windows(2) {
        assert!(pair[1].position.x >= pair[0].position.x - 1e-4,
            "samples not monotonic: {:?} -> {:?}",
            pair[0].position, pair[1].position);
    }
}

/// Consecutive sample distances on a straight path should match the
/// requested spacing within floating-point tolerance. Catches any
/// accumulated FP drift regression from a naive refactor (e.g.
/// `current += spacing` instead of `i * spacing`).
#[test]
fn test_sample_path_even_spacing_within_tolerance() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(500.0, 0.0),
    };
    let spacing = 10.0;
    let points = sample_path(&path, spacing);
    // All pairs except possibly the last must be within tolerance of
    // the requested spacing. The last pair can be shorter because the
    // tail is clamped to t=1.
    let n = points.len();
    assert!(n >= 3);
    for i in 0..(n - 2) {
        let d = points[i + 1].position.distance(points[i].position);
        assert!((d - spacing).abs() < 0.01,
            "sample spacing {} at i={} deviates from {}", d, i, spacing);
    }
}

/// Negative spacing must not produce an infinite loop or a panic.
/// Current behaviour: empty Vec (matches the existing zero-spacing
/// behaviour). WASM crash guard.
#[test]
fn test_sample_path_rejects_negative_spacing() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, -1.0);
    assert!(points.is_empty(),
        "negative spacing must return empty, got {} points", points.len());
}

/// NaN spacing must not panic (NaN comparisons are always false, so
/// `spacing <= 0.0` is false — we rely on downstream guards to still
/// produce a sane result). WASM crash guard.
#[test]
fn test_sample_path_rejects_nan_spacing() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    // Must not panic — we do not care about the exact return value,
    // only that we get back to this line without an abort. A non-panic
    // outcome is the WASM-reliability invariant.
    let _ = sample_path(&path, f32::NAN);
}

/// Spacing larger than the path length should return exactly one
/// sample (the start point). This guards the `count = 0` edge case.
#[test]
fn test_sample_path_huge_spacing_returns_start_only() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 10_000.0);
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].position, Vec2::new(0.0, 0.0));
}

/// Two calls to `sample_path` with the same inputs must produce
/// bit-identical output. Guards against a future accidental
/// randomisation (jitter, thread-local state, HashMap iteration
/// order leaking into the output).
#[test]
fn test_sample_path_deterministic_across_calls() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(100.0, 200.0),
        control2: Vec2::new(300.0, -200.0),
        end: Vec2::new(400.0, 0.0),
    };
    let a = sample_path(&path, 5.0);
    let b = sample_path(&path, 5.0);
    assert_eq!(a.len(), b.len());
    for (pa, pb) in a.iter().zip(b.iter()) {
        assert_eq!(pa.position, pb.position);
    }
}

/// `distance_to_path` on a long cubic bezier must return a finite,
/// non-NaN value. Guards against a hypothetical exponential
/// subdivision regression or NaN propagation from the sampler.
#[test]
fn test_distance_to_path_on_long_bezier_is_finite() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(25_000.0, 10_000.0),
        control2: Vec2::new(75_000.0, -10_000.0),
        end: Vec2::new(100_000.0, 0.0),
    };
    let d = distance_to_path(Vec2::new(50_000.0, 50_000.0), &path);
    assert!(d.is_finite(), "distance should be finite, got {}", d);
    assert!(d >= 0.0, "distance should be non-negative, got {}", d);
    // And the point is visibly off the curve, so non-zero.
    assert!(d > 1.0);
}

// point_at_t for label positioning along edges.

#[test]
fn point_at_t_straight_endpoints_and_midpoint() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 50.0),
    };
    assert_eq!(point_at_t(&path, 0.0), Vec2::new(0.0, 0.0));
    assert_eq!(point_at_t(&path, 1.0), Vec2::new(100.0, 50.0));
    let mid = point_at_t(&path, 0.5);
    assert!((mid.x - 50.0).abs() < 1e-5);
    assert!((mid.y - 25.0).abs() < 1e-5);
}

#[test]
fn point_at_t_cubic_bezier_endpoints() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(25.0, 100.0),
        control2: Vec2::new(75.0, 100.0),
        end: Vec2::new(100.0, 0.0),
    };
    // A cubic curve at t = 0 hits the start, at t = 1 hits the end.
    let p0 = point_at_t(&path, 0.0);
    let p1 = point_at_t(&path, 1.0);
    assert!((p0.x - 0.0).abs() < 1e-5 && (p0.y - 0.0).abs() < 1e-5);
    assert!((p1.x - 100.0).abs() < 1e-5 && (p1.y - 0.0).abs() < 1e-5);
    // And t = 0.5 sits between the control points vertically, well
    // above the straight-line midpoint.
    let mid = point_at_t(&path, 0.5);
    assert!(mid.y > 50.0, "midpoint y={} should be curved above", mid.y);
}

#[test]
fn point_at_t_clamps_out_of_range() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    // Values outside [0, 1] are clamped.
    assert_eq!(point_at_t(&path, -10.0), Vec2::new(0.0, 0.0));
    assert_eq!(point_at_t(&path, 99.0), Vec2::new(100.0, 0.0));
}

// ── Bezier math (bezier.rs) ────────────────────────────────────────

use super::bezier::{cubic_bezier_point, cubic_bezier_length, sample_cubic_bezier};
use crate::util::geometry::almost_equal;

#[test]
fn test_bezier_point_at_endpoints() {
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(10.0, 50.0);
    let p2 = Vec2::new(90.0, 50.0);
    let p3 = Vec2::new(100.0, 0.0);

    let start = cubic_bezier_point(0.0, p0, p1, p2, p3);
    assert!(almost_equal(start.x, 0.0), "t=0 should return p0.x");
    assert!(almost_equal(start.y, 0.0), "t=0 should return p0.y");

    let end = cubic_bezier_point(1.0, p0, p1, p2, p3);
    assert!(almost_equal(end.x, 100.0), "t=1 should return p3.x");
    assert!(almost_equal(end.y, 0.0), "t=1 should return p3.y");
}

#[test]
fn test_bezier_point_at_midpoint_is_influenced_by_controls() {
    let p0 = Vec2::new(0.0, 0.0);
    let p3 = Vec2::new(100.0, 0.0);

    // Straight line (controls on the segment)
    let mid_straight = cubic_bezier_point(0.5, p0, p0, p3, p3);
    assert!(almost_equal(mid_straight.x, 50.0), "straight midpoint x");
    assert!(almost_equal(mid_straight.y, 0.0), "straight midpoint y");

    // Curved (controls above the line)
    let p1 = Vec2::new(10.0, 80.0);
    let p2 = Vec2::new(90.0, 80.0);
    let mid_curved = cubic_bezier_point(0.5, p0, p1, p2, p3);
    assert!(mid_curved.y > 30.0, "curved midpoint should be pulled up by control points; got {}", mid_curved.y);
}

#[test]
fn test_bezier_length_straight_line() {
    let a = Vec2::new(0.0, 0.0);
    let b = Vec2::new(100.0, 0.0);
    // Controls on the line make it degenerate into a straight segment
    let length = cubic_bezier_length(a, a, b, b);
    assert!(
        (length - 100.0).abs() < 1.0,
        "straight-line bezier should have length ~100; got {}",
        length,
    );
}

#[test]
fn test_bezier_sample_produces_points() {
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(10.0, 50.0);
    let p2 = Vec2::new(90.0, 50.0);
    let p3 = Vec2::new(100.0, 0.0);
    let spacing = 10.0;

    let samples = sample_cubic_bezier(p0, p1, p2, p3, spacing);
    assert!(samples.len() > 5, "a 100-unit curve at spacing 10 should produce >5 samples; got {}", samples.len());

    // First sample should be at p0
    assert!(almost_equal(samples[0].position.x, 0.0));
    assert!(almost_equal(samples[0].position.y, 0.0));
}

#[test]
fn test_bezier_sample_degenerate_returns_single_point() {
    // A zero-length curve (all points identical)
    let pt = Vec2::new(42.0, 42.0);
    let samples = sample_cubic_bezier(pt, pt, pt, pt, 10.0);
    assert_eq!(samples.len(), 1, "degenerate curve should produce single sample");
    assert!(almost_equal(samples[0].position.x, 42.0));
}

// ---- tangent / normal helpers ----

#[test]
fn tangent_at_t_straight_path_returns_endpoint_direction() {
    // For a straight path, the tangent is the normalised
    // end-minus-start vector regardless of `t`.
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(10.0, 0.0),
    };
    for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let tangent = tangent_at_t(&path, t);
        assert!(almost_equal(tangent.x, 1.0), "t={t} x should be 1");
        assert!(almost_equal(tangent.y, 0.0), "t={t} y should be 0");
    }
}

#[test]
fn tangent_at_t_zero_length_straight_path_falls_back_to_x_axis() {
    // Coincident endpoints produce a zero-length raw tangent;
    // the fallback keeps callers from dividing by zero.
    let pt = Vec2::new(5.0, 5.0);
    let path = ConnectionPath::Straight { start: pt, end: pt };
    let tangent = tangent_at_t(&path, 0.5);
    assert_eq!(tangent, Vec2::X);
}

#[test]
fn tangent_at_t_cubic_bezier_at_endpoints_uses_analytical_derivative() {
    // At t = 0: derivative = 3(p1 - p0). At t = 1: derivative =
    // 3(p3 - p2). Normalised.
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(10.0, 0.0);
    let p2 = Vec2::new(20.0, 10.0);
    let p3 = Vec2::new(30.0, 10.0);
    let path = ConnectionPath::CubicBezier {
        start: p0,
        control1: p1,
        control2: p2,
        end: p3,
    };
    // t = 0: tangent ∝ (p1 - p0) = (10, 0) → normalised (1, 0).
    let t0 = tangent_at_t(&path, 0.0);
    assert!(almost_equal(t0.x, 1.0));
    assert!(almost_equal(t0.y, 0.0));
    // t = 1: tangent ∝ (p3 - p2) = (10, 0) → (1, 0).
    let t1 = tangent_at_t(&path, 1.0);
    assert!(almost_equal(t1.x, 1.0));
    assert!(almost_equal(t1.y, 0.0));
}

#[test]
fn normal_at_t_is_orthogonal_to_tangent() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(7.0, 3.0),
    };
    let tangent = tangent_at_t(&path, 0.5);
    let normal = normal_at_t(&path, 0.5);
    // Dot product of orthogonal unit vectors is 0.
    assert!(tangent.dot(normal).abs() < 1.0e-5);
    // And length is 1.
    assert!((normal.length() - 1.0).abs() < 1.0e-5);
}

#[test]
fn normal_at_t_rotates_canvas_90_clockwise_into_screen_space() {
    // A tangent pointing +X in canvas space rotates to (-0, +1)
    // by the `(x, y) → (-y, x)` formula, i.e. +Y — which on a
    // Y-down canvas lands *below* the path (the right-hand side
    // of travel in screen space). Pin the behaviour so a future
    // flip of the formula or a coordinate-system change breaks
    // this test instead of silently inverting label positioning.
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(10.0, 0.0),
    };
    let normal = normal_at_t(&path, 0.5);
    assert!(almost_equal(normal.x, 0.0));
    assert!(almost_equal(normal.y, 1.0));
}

// ---- cubic_bezier_tangent analytical derivative ----

#[test]
fn cubic_bezier_tangent_matches_finite_difference() {
    // Spot-check the analytical derivative against a finite
    // difference — a single bug in the coefficients (e.g. writing
    // `2.0 * u * t` instead of `6.0 * u * t` for the middle term)
    // would break this test.
    use crate::mindmap::connection::bezier::{cubic_bezier_point, cubic_bezier_tangent};
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(1.0, 5.0);
    let p2 = Vec2::new(4.0, -2.0);
    let p3 = Vec2::new(6.0, 3.0);
    // h = 1e-3 balances truncation error (O(h² · |f‴|) ≈ 1e-4
    // on this cubic) against f32 cancellation in the central
    // difference (rounding amplification ≈ 1/(2h)). Smaller h
    // would sink under cancellation noise; larger h would let
    // truncation dominate.
    let h = 1.0e-3;
    for t in [0.1, 0.3, 0.5, 0.7, 0.9] {
        let analytical = cubic_bezier_tangent(t, p0, p1, p2, p3);
        let fwd = cubic_bezier_point(t + h, p0, p1, p2, p3);
        let back = cubic_bezier_point(t - h, p0, p1, p2, p3);
        let fd = (fwd - back) / (2.0 * h);
        // Tolerance 1e-3 sits comfortably above the combined
        // truncation + f32 cancellation floor while still
        // catching a single-coefficient bug — e.g. a missing
        // factor of 2 or a `u*t` → `u+t` typo produces errors
        // of order 1-10.
        assert!(
            (analytical.x - fd.x).abs() < 1.0e-3,
            "t={t} x analytical {} vs fd {}",
            analytical.x,
            fd.x
        );
        assert!(
            (analytical.y - fd.y).abs() < 1.0e-3,
            "t={t} y analytical {} vs fd {}",
            analytical.y,
            fd.y
        );
    }
}
