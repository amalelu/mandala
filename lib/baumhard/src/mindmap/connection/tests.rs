// SPDX-License-Identifier: MPL-2.0

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
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 50.0),
        "right", // from: right anchor
        Vec2::new(200.0, 0.0),
        Vec2::new(100.0, 50.0),
        "left", // to: left anchor
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
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 50.0),
        "right",
        Vec2::new(300.0, 0.0),
        Vec2::new(100.0, 50.0),
        "left",
        &cps,
    );
    match path {
        ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } => {
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
    let points = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
    assert_eq!(points.len(), 11); // 0, 10, 20, ..., 100
                                  // First point at start
    assert!((points[0].x - 0.0).abs() < 0.01);
    // Last point at or near 100
    assert!((points[10].x - 100.0).abs() < 0.01);
    // All y should be 0
    for p in &points {
        assert!((p.y).abs() < 0.01);
    }
}

#[test]
fn test_straight_sampling_diagonal() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(30.0, 40.0), // length = 50
    };
    let points = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
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
    assert!(
        arc_len > straight_dist,
        "Arc length {} should exceed straight {}",
        arc_len,
        straight_dist
    );
}

#[test]
fn test_curved_bezier_sampling() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(33.0, 100.0),
        control2: Vec2::new(67.0, -100.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
    // Curved path is longer than 100, so should have more than 11 points
    assert!(points.len() > 11, "Expected >11 points, got {}", points.len());
    // First point near start
    assert!(points[0].distance(Vec2::ZERO) < 1.0);
}

#[test]
fn test_sample_path_zero_spacing() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 0.0, super::MAX_PATH_SAMPLES);
    assert!(points.is_empty());
}

#[test]
fn test_sample_path_degenerate() {
    // Zero-length path
    let path = ConnectionPath::Straight {
        start: Vec2::new(50.0, 50.0),
        end: Vec2::new(50.0, 50.0),
    };
    let points = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
    assert_eq!(points.len(), 1);
}

/// `sample_path_into` appends. It does not clear, and the doc says
/// so, because a caller refilling a buffer clears it and a caller
/// concatenating paths must not.
///
/// Input that makes it fail: adding a `clear()` inside — the seeded
/// point vanishes and the length drops to the sample count.
#[test]
fn test_sample_path_into_appends_to_what_the_caller_already_has() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let fresh = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
    assert!(
        fresh.len() > 5,
        "precondition: this path samples to several points"
    );

    let seed = Vec2::new(-999.0, -999.0);
    let mut out = vec![seed];
    super::sample_path_into(&path, 10.0, super::MAX_PATH_SAMPLES, &mut out);
    assert_eq!(out.len(), fresh.len() + 1, "the seeded point must survive");
    assert_eq!(out[0], seed);
    assert_eq!(
        &out[1..],
        &fresh[..],
        "and the appended run must be the whole sequence"
    );
}

/// Filling an empty buffer takes one allocation of exactly the right
/// size, not a growth sequence.
///
/// The observable is slack: `Vec` grows by doubling, so pushing `n`
/// points into an empty buffer without reserving first lands on the
/// next power of two above `n`, while one exact reserve lands on `n`.
/// Input that makes it fail: dropping the `reserve` inside
/// `sample_path_into`'s two arms.
#[test]
fn test_sample_path_into_reserves_the_exact_count_rather_than_growing() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(1_000.0, 0.0),
    };
    let mut out: Vec<Vec2> = Vec::new();
    super::sample_path_into(&path, 7.0, super::MAX_PATH_SAMPLES, &mut out);
    assert!(
        !out.len().is_power_of_two(),
        "precondition: pick a count doubling would not land on anyway; got {}",
        out.len()
    );
    assert_eq!(
        out.capacity(),
        out.len(),
        "an exact reserve leaves no slack; a growth sequence from empty would not"
    );
}

/// A refilled buffer produces the same points as a fresh one, and
/// reuses its allocation to do it.
///
/// This is the contract the connection pass's resample path rests on:
/// it clears the buffer the scene cache handed back and samples into
/// it. Input that makes the first half fail: the pass forgetting the
/// clear, which is the defect this pairing exists to catch.
#[test]
fn test_sample_path_into_refills_a_reused_buffer_without_reallocating() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(30.0, 80.0),
        control2: Vec2::new(70.0, -80.0),
        end: Vec2::new(100.0, 0.0),
    };
    let mut buffer: Vec<Vec2> = Vec::new();
    super::sample_path_into(&path, 6.0, super::MAX_PATH_SAMPLES, &mut buffer);
    let first = buffer.clone();
    let capacity = buffer.capacity();
    assert!(
        first.len() > 5,
        "precondition: this curve samples to several points"
    );

    buffer.clear();
    super::sample_path_into(&path, 6.0, super::MAX_PATH_SAMPLES, &mut buffer);

    assert_eq!(buffer, first, "a refill must reproduce the sequence exactly");
    assert_eq!(
        buffer.capacity(),
        capacity,
        "refilling a buffer that already has the room must not reallocate"
    );
}

/// A non-positive spacing appends nothing and leaves what was there
/// alone — the guard the pass leans on to detect "this edge produces
/// no samples" by asking whether the buffer is still empty.
#[test]
fn test_sample_path_into_appends_nothing_for_a_non_positive_spacing() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    for spacing in [0.0f32, -1.0, -0.0] {
        let mut out: Vec<Vec2> = Vec::new();
        super::sample_path_into(&path, spacing, super::MAX_PATH_SAMPLES, &mut out);
        assert!(out.is_empty(), "spacing {spacing} must append nothing");
    }
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
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 50.0),
        "auto",
        Vec2::new(200.0, 0.0),
        Vec2::new(100.0, 50.0),
        "auto",
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
// They assert the behavioral invariant the drag-frame sampler
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
    let points = sample_path(&path, 15.0, super::MAX_PATH_SAMPLES);
    // Expected: floor(20000/15) + 1 = 1334.
    assert_eq!(points.len(), 1334);
    // Way above the 256-subdivision table size — proves no clamp.
    assert!(
        points.len() > 1000,
        "sample count {} should scale with length, not subdivisions",
        points.len()
    );
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
    let points = sample_path(&path, spacing, super::MAX_PATH_SAMPLES);
    let expected_floor = (length / spacing) as usize;
    // Sampler emits `floor(length/spacing) + 1` points. Allow a window
    // of ±2 to tolerate FP drift at the endpoint.
    assert!(
        points.len() >= expected_floor,
        "expected at least {}, got {}",
        expected_floor,
        points.len()
    );
    assert!(
        points.len() <= expected_floor + 2,
        "expected at most {}, got {}",
        expected_floor + 2,
        points.len()
    );
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
    let points = sample_path(&path, 10.0, super::MAX_PATH_SAMPLES);
    assert!(points.len() > 2);
    for pair in points.windows(2) {
        assert!(
            pair[1].x >= pair[0].x - 1e-4,
            "samples not monotonic: {:?} -> {:?}",
            pair[0],
            pair[1]
        );
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
    let points = sample_path(&path, spacing, super::MAX_PATH_SAMPLES);
    // All pairs except possibly the last must be within tolerance of
    // the requested spacing. The last pair can be shorter because the
    // tail is clamped to t=1.
    let n = points.len();
    assert!(n >= 3);
    for i in 0..(n - 2) {
        let d = points[i + 1].distance(points[i]);
        assert!(
            (d - spacing).abs() < 0.01,
            "sample spacing {} at i={} deviates from {}",
            d,
            i,
            spacing
        );
    }
}

/// Negative spacing must not produce an infinite loop or a panic.
/// Current behavior: empty Vec (matches the existing zero-spacing
/// behavior). WASM crash guard.
#[test]
fn test_sample_path_rejects_negative_spacing() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, -1.0, super::MAX_PATH_SAMPLES);
    assert!(
        points.is_empty(),
        "negative spacing must return empty, got {} points",
        points.len()
    );
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
    let _ = sample_path(&path, f32::NAN, super::MAX_PATH_SAMPLES);
}

/// Spacing larger than the path length should return exactly one
/// sample (the start point). This guards the `count = 0` edge case.
#[test]
fn test_sample_path_huge_spacing_returns_start_only() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let points = sample_path(&path, 10_000.0, super::MAX_PATH_SAMPLES);
    assert_eq!(points.len(), 1);
    assert_eq!(points[0], Vec2::new(0.0, 0.0));
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
    let a = sample_path(&path, 5.0, super::MAX_PATH_SAMPLES);
    let b = sample_path(&path, 5.0, super::MAX_PATH_SAMPLES);
    assert_eq!(a.len(), b.len());
    for (pa, pb) in a.iter().zip(b.iter()) {
        assert_eq!(pa, pb);
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

use super::bezier::{cubic_bezier_length, cubic_bezier_point, extend_cubic_samples};
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
    assert!(
        mid_curved.y > 30.0,
        "curved midpoint should be pulled up by control points; got {}",
        mid_curved.y
    );
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

    let mut samples: Vec<Vec2> = Vec::new();
    extend_cubic_samples(p0, p1, p2, p3, spacing, super::MAX_PATH_SAMPLES, &mut samples);
    assert!(
        samples.len() > 5,
        "a 100-unit curve at spacing 10 should produce >5 samples; got {}",
        samples.len()
    );

    // First sample should be at p0
    assert!(almost_equal(samples[0].x, 0.0));
    assert!(almost_equal(samples[0].y, 0.0));
}

#[test]
fn test_bezier_sample_degenerate_returns_single_point() {
    // A zero-length curve (all points identical)
    let pt = Vec2::new(42.0, 42.0);
    let mut samples: Vec<Vec2> = Vec::new();
    extend_cubic_samples(pt, pt, pt, pt, 10.0, super::MAX_PATH_SAMPLES, &mut samples);
    assert_eq!(samples.len(), 1, "degenerate curve should produce single sample");
    assert!(almost_equal(samples[0].x, 42.0));
}

// ---- tangent / normal helpers ----

#[test]
fn tangent_at_t_straight_path_returns_endpoint_direction() {
    // For a straight path, the tangent is the normalized
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
    // t = 0: tangent ∝ (p1 - p0) = (10, 0) → normalized (1, 0).
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
    // of travel in screen space). Pin the behavior so a future
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

// ---- closest_point_on_path ----

#[test]
fn closest_point_on_path_straight_cursor_above_midpoint() {
    // Horizontal segment; cursor directly above the midpoint at
    // y = -5. Expected: t = 0.5, perp_offset = -5 (perp direction
    // is `(-y, x)` rotation of `(1, 0)` tangent = `(0, 1)`, so
    // cursor at `(x_mid, -5)` relative to `(x_mid, 0)` has
    // `to_cursor = (0, -5)`, perp = dot((0,-5), (0,1)) = -5).
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let (t, perp) = closest_point_on_path(&path, Vec2::new(50.0, -5.0));
    assert!((t - 0.5).abs() < 1.0e-5, "t={t}");
    assert!((perp - -5.0).abs() < 1.0e-5, "perp={perp}");
}

#[test]
fn closest_point_on_path_straight_cursor_beyond_end_clamps_to_1() {
    // Cursor past the `end` endpoint clamps `t` into `[0, 1]`.
    let path = ConnectionPath::Straight {
        start: Vec2::new(0.0, 0.0),
        end: Vec2::new(100.0, 0.0),
    };
    let (t, _perp) = closest_point_on_path(&path, Vec2::new(200.0, 0.0));
    assert!((t - 1.0).abs() < 1.0e-5, "t={t}");
}

#[test]
fn closest_point_on_path_straight_cursor_behind_start_clamps_to_0() {
    let path = ConnectionPath::Straight {
        start: Vec2::new(10.0, 10.0),
        end: Vec2::new(50.0, 10.0),
    };
    let (t, _perp) = closest_point_on_path(&path, Vec2::new(-40.0, 10.0));
    assert!(t < 1.0e-5, "t={t}");
}

#[test]
fn closest_point_on_path_straight_zero_length_returns_defaults() {
    // Coincident endpoints: no meaningful direction. Contract is
    // `(0, 0)` so the caller can rely on the tuple without a
    // `NaN` check.
    let pt = Vec2::new(5.0, 5.0);
    let path = ConnectionPath::Straight { start: pt, end: pt };
    let (t, perp) = closest_point_on_path(&path, Vec2::new(20.0, 20.0));
    assert_eq!(t, 0.0);
    assert_eq!(perp, 0.0);
}

#[test]
fn closest_point_on_path_cubic_cursor_on_curve_returns_zero_perp() {
    // A cursor sitting exactly on the curve at a known `t` must
    // recover `t` (approximately) with near-zero perp.
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(33.0, 50.0);
    let p2 = Vec2::new(66.0, 50.0);
    let p3 = Vec2::new(100.0, 0.0);
    let path = ConnectionPath::CubicBezier {
        start: p0,
        control1: p1,
        control2: p2,
        end: p3,
    };
    // Pick a known t, evaluate the curve there, and ask the
    // closest-point solver for that point.
    let true_t = 0.37;
    let on_curve = crate::mindmap::connection::bezier::cubic_bezier_point(true_t, p0, p1, p2, p3);
    let (t, perp) = closest_point_on_path(&path, on_curve);
    assert!(
        (t - true_t).abs() < 1.0e-3,
        "Newton should recover t within 1e-3; got {t} vs {true_t}"
    );
    assert!(
        perp.abs() < 1.0e-3,
        "perp should be ~0 when cursor is on the curve; got {perp}"
    );
}

#[test]
fn closest_point_on_path_cubic_offset_cursor_produces_signed_perp() {
    // Offset the on-curve point along the path normal by a known
    // signed distance; the solver should recover the same signed
    // perp value.
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(33.0, 50.0);
    let p2 = Vec2::new(66.0, 50.0);
    let p3 = Vec2::new(100.0, 0.0);
    let path = ConnectionPath::CubicBezier {
        start: p0,
        control1: p1,
        control2: p2,
        end: p3,
    };
    let true_t = 0.42;
    let on_curve = crate::mindmap::connection::bezier::cubic_bezier_point(true_t, p0, p1, p2, p3);
    let normal = normal_at_t(&path, true_t);
    let offset = 12.0_f32;
    let cursor = on_curve + normal * offset;
    let (t, perp) = closest_point_on_path(&path, cursor);
    assert!((t - true_t).abs() < 1.0e-2, "t={t} vs {true_t}");
    assert!((perp - offset).abs() < 1.0e-1, "perp={perp} vs {offset}");
}

#[test]
fn closest_point_on_path_cubic_degenerate_all_coincident() {
    // All four control points coincident: every evaluation of
    // B(t) returns the same point, tangent is zero everywhere.
    // Contract: `t=0`, `perp=0` — callers get a deterministic
    // fallback rather than NaN.
    let pt = Vec2::new(7.0, 11.0);
    let path = ConnectionPath::CubicBezier {
        start: pt,
        control1: pt,
        control2: pt,
        end: pt,
    };
    let (t, perp) = closest_point_on_path(&path, Vec2::new(50.0, 50.0));
    assert!(t.is_finite());
    assert!(perp.is_finite());
    // The closest point on a degenerate curve is the coincident
    // point; perpendicular from cursor to it projected on the
    // zero tangent is zero by convention.
    assert!(
        (0.0..=1.0).contains(&t),
        "t stayed in [0,1] under degenerate curve"
    );
    assert_eq!(perp, 0.0);
}

#[test]
fn closest_point_on_path_cubic_near_inflection_never_worse_than_seed() {
    // Cubic with an inflection point — B''(t) changes sign near
    // the middle of the curve, which can send a naive Newton
    // step past the true minimum. The divergence guard compares
    // Newton's refined dist² against the sampling seed's and
    // falls back when Newton diverged. Any cursor position
    // must produce a path point whose distance is ≤ the best
    // sample's distance.
    let p0 = Vec2::new(0.0, 0.0);
    let p1 = Vec2::new(10.0, 100.0);
    let p2 = Vec2::new(90.0, -100.0); // inflection between p1 and p2
    let p3 = Vec2::new(100.0, 0.0);
    let path = ConnectionPath::CubicBezier {
        start: p0,
        control1: p1,
        control2: p2,
        end: p3,
    };
    // Cursor at a position that exercises the inflection region.
    let cursors = [
        Vec2::new(50.0, 0.0),
        Vec2::new(50.0, 10.0),
        Vec2::new(50.0, -10.0),
        Vec2::new(25.0, 30.0),
        Vec2::new(75.0, -30.0),
    ];
    for cursor in cursors {
        // Compute the sampling-only seed distance for the same
        // cursor by replicating the 32-sample sweep.
        let mut seed_best = f32::MAX;
        for i in 0..=32 {
            let t = i as f32 / 32.0;
            let d = (crate::mindmap::connection::bezier::cubic_bezier_point(t, p0, p1, p2, p3) - cursor)
                .length_squared();
            if d < seed_best {
                seed_best = d;
            }
        }
        let (t, _perp) = closest_point_on_path(&path, cursor);
        let point = crate::mindmap::connection::bezier::cubic_bezier_point(t, p0, p1, p2, p3);
        let refined_dist_sq = (point - cursor).length_squared();
        assert!(
            refined_dist_sq <= seed_best + 1.0e-3,
            "Newton must never be worse than seed: refined={} seed_best={} t={} cursor={:?}",
            refined_dist_sq,
            seed_best,
            t,
            cursor
        );
    }
}

/// **The allocation the sampler used to commit to unconditionally.**
///
/// The sample count is `path length / spacing`, and both terms come
/// out of the document — endpoints are node positions, spacing
/// derives from an authored font size. Uncapped, a long path at a
/// fine spacing asked `Vec::with_capacity` for billions of points,
/// which fails as an allocator abort rather than a catchable panic.
///
/// The loader now rejects the coordinates that get here in the first
/// place; this pins the second wall, so a path reaching the sampler
/// by some other route still cannot ask for terabytes.
#[test]
fn test_sample_path_caps_hostile_geometry() {
    let far = ConnectionPath::Straight {
        start: Vec2::ZERO,
        end: Vec2::new(1.0e9, 0.0),
    };
    // Equality, not `<=`: the cap must *bind* here. A sampler that
    // bailed to a single point on any large quotient — an easy
    // mistake, since `sample_count` already returns 1 on four guard
    // paths — would satisfy `<=` while silently drawing nothing.
    let samples = sample_path(&far, 0.001, super::MAX_PATH_SAMPLES);
    assert_eq!(
        samples.len(),
        MAX_PATH_SAMPLES,
        "a long path at fine spacing must saturate the cap, not fall through it"
    );

    let curved = ConnectionPath::CubicBezier {
        start: Vec2::ZERO,
        control1: Vec2::new(1.0e8, 1.0e8),
        control2: Vec2::new(2.0e8, -1.0e8),
        end: Vec2::new(1.0e9, 0.0),
    };
    assert_eq!(
        sample_path(&curved, 0.001, super::MAX_PATH_SAMPLES).len(),
        MAX_PATH_SAMPLES
    );
}

/// A non-finite length or spacing must not reach the allocation
/// either. Float-to-integer casts saturate, so an infinite quotient
/// would land on `usize::MAX` and overflow the `+ 1`; a `NaN` casts
/// to zero. Both resolve to a single point instead.
#[test]
fn test_sample_path_survives_non_finite_geometry() {
    let infinite = ConnectionPath::Straight {
        start: Vec2::ZERO,
        end: Vec2::new(f32::INFINITY, 0.0),
    };
    // Asserted as an exact count, not `<= MAX_PATH_SAMPLES`. The
    // bound is satisfied by *any* result including an empty one, so
    // it holds whether or not `sample_count`'s guards exist and
    // certifies nothing. One point is the documented outcome.
    assert_eq!(
        sample_path(&infinite, 1.0, super::MAX_PATH_SAMPLES).len(),
        1,
        "an infinite length must resolve to a single point"
    );

    let nan = ConnectionPath::Straight {
        start: Vec2::ZERO,
        end: Vec2::new(f32::NAN, 0.0),
    };
    assert_eq!(
        sample_path(&nan, 1.0, super::MAX_PATH_SAMPLES).len(),
        1,
        "a NaN length must resolve to a single point"
    );

    // A non-finite spacing is the same hazard from the other side.
    let ordinary = ConnectionPath::Straight {
        start: Vec2::ZERO,
        end: Vec2::new(100.0, 0.0),
    };
    assert_eq!(
        sample_path(&ordinary, f32::NAN, super::MAX_PATH_SAMPLES).len(),
        1,
        "a NaN spacing must resolve to a single point"
    );
    assert_eq!(
        sample_path(&ordinary, f32::INFINITY, super::MAX_PATH_SAMPLES).len(),
        1,
        "an infinite spacing must resolve to a single point"
    );
    // A non-positive spacing is refused by `sample_path` itself,
    // before `sample_count` is consulted — so the answer is no
    // samples at all rather than the single point the non-finite
    // cases resolve to. Asserted because the two guards are easy to
    // conflate: `sample_count`'s own `spacing <= 0.0` arm is
    // unreachable through this entry point.
    assert!(
        sample_path(&ordinary, 0.0, super::MAX_PATH_SAMPLES).is_empty(),
        "a zero spacing yields no samples, not one"
    );
    assert!(
        sample_path(&ordinary, -5.0, super::MAX_PATH_SAMPLES).is_empty(),
        "a negative spacing yields no samples, not one"
    );

    // The ordinary case still samples the way it always did.
    assert_eq!(sample_path(&ordinary, 10.0, super::MAX_PATH_SAMPLES).len(), 11);
}

/// **The clamp that must not panic on a window the cascade built
/// backwards.**
///
/// `f32::clamp` panics when its bounds cross, and a size window is
/// not assembled from one place: a label may set only
/// `min_font_size_pt` and inherit `max` from its edge. Validating
/// each struct's own pair at load therefore cannot see every window
/// the cascade can produce — the halves are individually valid and
/// jointly inverted — so the ordering is enforced where the window
/// is used.
#[test]
fn test_font_window_tolerates_an_inverted_cascade() {
    use crate::font::fonts::{clamp_to_font_window, MAX_FONT_SIZE_PT, MIN_FONT_SIZE_PT};

    // Inverted: read as the window the author described.
    assert_eq!(clamp_to_font_window(50.0, 40.0, 8.0), 40.0);
    assert_eq!(clamp_to_font_window(4.0, 40.0, 8.0), 8.0);
    assert_eq!(clamp_to_font_window(20.0, 40.0, 8.0), 20.0);

    // Ordinary window is untouched.
    assert_eq!(clamp_to_font_window(20.0, 8.0, 40.0), 20.0);
    assert_eq!(clamp_to_font_window(100.0, 8.0, 40.0), 40.0);

    // Non-finite bounds are pulled into the shaper's domain first,
    // so neither the clamp nor the shaper ever sees one. A window
    // with nothing usable in it widens to the whole domain rather
    // than collapsing, so an ordinary size passes through.
    assert_eq!(clamp_to_font_window(20.0, f32::NAN, f32::NAN), 20.0);
    assert_eq!(clamp_to_font_window(0.0, f32::NAN, f32::NAN), MIN_FONT_SIZE_PT);
    assert_eq!(clamp_to_font_window(1.0e9, f32::NAN, f32::NAN), MAX_FONT_SIZE_PT);
    assert!(clamp_to_font_window(1.0e9, 8.0, f32::INFINITY) <= MAX_FONT_SIZE_PT);

    // A NaN size takes the floor rather than propagating.
    assert_eq!(clamp_to_font_window(f32::NAN, 8.0, 40.0), 8.0);
}

/// **The per-path cap bounded a path and bounded nothing.**
///
/// Each sample becomes a glyph area in the scene arena, and an edge
/// costs about 120 bytes in the file. A 73 KB document with 200 edges
/// reached 2 000 000 samples, a 2 000 201-node arena and 1 642 MiB
/// resident; with the budget it reaches 500 000 and 416 MiB, and no
/// document can exceed that however many edges it declares. The
/// per-path constant's own doc had said the aggregate was not bounded.
///
/// The share is equal per edge rather than first-come, so the outcome
/// cannot depend on iteration order and no edge renders while a later
/// one vanishes.
#[test]
fn test_the_scene_wide_glyph_budget_is_shared_equally_and_never_zero() {
    use super::{per_path_sample_budget, MAX_PATH_SAMPLES, MAX_TOTAL_PATH_SAMPLES};

    // Few edges: the per-path cap still governs, unchanged behavior.
    assert_eq!(per_path_sample_budget(1), MAX_PATH_SAMPLES);
    assert_eq!(
        per_path_sample_budget(0),
        MAX_PATH_SAMPLES,
        "no edges must not divide by zero"
    );
    assert_eq!(
        per_path_sample_budget(MAX_TOTAL_PATH_SAMPLES / MAX_PATH_SAMPLES),
        MAX_PATH_SAMPLES,
        "at the crossover the two ceilings agree"
    );

    // Many edges: the aggregate governs, and the product stays inside
    // the budget — which is the whole property.
    for edges in [200usize, 1_000, 50_000, 5_000_000] {
        let each = per_path_sample_budget(edges);
        assert!(
            each >= 1,
            "{edges} edges: an edge that renders nothing looks like a missing edge"
        );
        assert!(
            each <= MAX_PATH_SAMPLES,
            "{edges} edges: the per-path cap must still apply"
        );
        // `each * edges` can exceed the budget only through the
        // never-zero floor, which is the deliberate trade.
        if each > 1 {
            assert!(
                each.saturating_mul(edges) <= MAX_TOTAL_PATH_SAMPLES,
                "{edges} edges x {each} samples exceeds the scene budget"
            );
        }
    }

    // Real maps must be untouched by this: the repository's heaviest is
    // `stress_long_edges` at 46 290 samples over 124 edges.
    assert!(
        per_path_sample_budget(124) > 46_290 / 124,
        "the budget must not thin the repository's own worst-case map"
    );
}

// ── Allocation-free sampling, and the bounding-box early-out ───────
//
// The three rewrites this section covers — an arc-length table that
// is an array, a length that is a running sum, and a
// `distance_to_path` that consumes samples instead of collecting
// them — are all invisible to a behavioral test: each produces the
// same numbers it produced before. What the tests below hold is
// therefore *equivalence* against an independently written form of
// the old computation, and, for the parts no runtime assertion can
// see at all, the source text itself. The new surface
// (`path_bounds`, `distance_to_path_within`) is the exception: its
// contract is a claim about behavior and is tested as one.

/// A 32-bit linear congruential generator, so the corpora below are
/// a fixed sequence rather than a fresh one per run. The constants
/// are Numerical Recipes'; the high bits are the ones used.
struct Lcg(u32);

impl Lcg {
    fn next_f32(&mut self, lo: f32, hi: f32) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let unit = (self.0 >> 8) as f32 / (1u32 << 24) as f32;
        lo + unit * (hi - lo)
    }

    fn next_vec2(&mut self, lo: f32, hi: f32) -> Vec2 {
        Vec2::new(self.next_f32(lo, hi), self.next_f32(lo, hi))
    }
}

/// Cubics whose control polygon is **axis-aligned**: all four
/// control points sharing one x, or one y, at a range of canvas
/// magnitudes.
///
/// This is the shape [`cubic_corpus`]'s generated half cannot
/// produce, and it is the one that matters: with every control point
/// on one x, the control-polygon box is zero-width on that axis, so
/// any rounding in the curve evaluation leaves the box rather than
/// hiding inside it. It is also entirely ordinary content — two
/// nodes stacked vertically with control offsets whose x is 0 is
/// what "curve this edge straight up" authors.
///
/// The magnitudes matter for the same reason: the escape scales with
/// the coordinate, so a corpus centered on the origin cannot see it.
fn axis_aligned_corpus() -> Vec<ConnectionPath> {
    let mut out = Vec::new();
    for magnitude in [1.0e2f32, 1.1e3, 1.0e4, 1.0e5, 1.0e6] {
        // "Curve straight up": a zero-width box at x = magnitude.
        out.push(ConnectionPath::CubicBezier {
            start: Vec2::new(magnitude, 0.0),
            control1: Vec2::new(magnitude, 120.0),
            control2: Vec2::new(magnitude, 240.0),
            end: Vec2::new(magnitude, 360.0),
        });
        // Transposed: a zero-height box at y = magnitude.
        out.push(ConnectionPath::CubicBezier {
            start: Vec2::new(0.0, magnitude),
            control1: Vec2::new(120.0, magnitude),
            control2: Vec2::new(240.0, magnitude),
            end: Vec2::new(360.0, magnitude),
        });
        // Negative, because the magnitude term has to read |coord|.
        out.push(ConnectionPath::CubicBezier {
            start: Vec2::new(-magnitude, 0.0),
            control1: Vec2::new(-magnitude, -120.0),
            control2: Vec2::new(-magnitude, -240.0),
            end: Vec2::new(-magnitude, -360.0),
        });
    }
    out
}

/// Cubic paths spanning the shapes a document can author: a
/// degenerate point, a curve whose controls lie on the chord, a
/// single hump, an S-curve, one whose control points sit far outside
/// the curve's own extent (the case a control-polygon box bounds
/// most loosely), a long one, and thirty generated ones.
fn cubic_corpus() -> Vec<ConnectionPath> {
    let cubic = |s: Vec2, c1: Vec2, c2: Vec2, e: Vec2| ConnectionPath::CubicBezier {
        start: s,
        control1: c1,
        control2: c2,
        end: e,
    };
    let mut paths = vec![
        // Degenerate: every control point coincident.
        cubic(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::ZERO),
        // Collinear controls — a curve that draws as a straight line.
        cubic(
            Vec2::new(0.0, 0.0),
            Vec2::new(33.0, 0.0),
            Vec2::new(66.0, 0.0),
            Vec2::new(100.0, 0.0),
        ),
        // Single hump.
        cubic(
            Vec2::new(0.0, 0.0),
            Vec2::new(25.0, 100.0),
            Vec2::new(75.0, 100.0),
            Vec2::new(100.0, 0.0),
        ),
        // S-curve: the controls straddle the chord.
        cubic(
            Vec2::new(0.0, 0.0),
            Vec2::new(30.0, 80.0),
            Vec2::new(70.0, -80.0),
            Vec2::new(100.0, 0.0),
        ),
        // Controls far outside the curve's own extent.
        cubic(
            Vec2::new(0.0, 0.0),
            Vec2::new(-400.0, 900.0),
            Vec2::new(500.0, -900.0),
            Vec2::new(100.0, 0.0),
        ),
        // Long, the shape `stress_long_edges` is made of.
        cubic(
            Vec2::new(0.0, 0.0),
            Vec2::new(25_000.0, 10_000.0),
            Vec2::new(75_000.0, -10_000.0),
            Vec2::new(100_000.0, 0.0),
        ),
    ];
    let mut rng = Lcg(0x5eed_1234);
    for _ in 0..30 {
        paths.push(cubic(
            rng.next_vec2(-500.0, 500.0),
            rng.next_vec2(-500.0, 500.0),
            rng.next_vec2(-500.0, 500.0),
            rng.next_vec2(-500.0, 500.0),
        ));
    }
    paths.extend(axis_aligned_corpus());
    paths
}

/// The corpus above plus straight paths, for the tests whose subject
/// is the whole `ConnectionPath` surface rather than the Bezier math.
fn path_corpus() -> Vec<ConnectionPath> {
    let mut paths = vec![
        ConnectionPath::Straight {
            start: Vec2::new(0.0, 0.0),
            end: Vec2::new(100.0, 0.0),
        },
        ConnectionPath::Straight {
            start: Vec2::new(-40.0, 60.0),
            end: Vec2::new(-40.0, 60.0),
        },
        ConnectionPath::Straight {
            start: Vec2::new(-250.0, 700.0),
            end: Vec2::new(900.0, -30.0),
        },
    ];
    // Straight segments far from the origin. `path_bounds` is exact
    // for these — it is the two endpoints — so the hull escape is not
    // what they exercise. What they exercise is the *other* half of
    // the magnitude term: `point_to_segment_distance_squared` computes
    // its projection through a chain whose rounding also scales with
    // the coordinate, so the distance the accept side compares can sit
    // that far from the one the reject side implies. The corpus had
    // nothing above ~900 units and so could not see it.
    for magnitude in [1.0e4f32, 1.0e5, 1.0e6] {
        paths.push(ConnectionPath::Straight {
            start: Vec2::new(magnitude, -magnitude),
            end: Vec2::new(magnitude + 700.0, magnitude),
        });
        paths.push(ConnectionPath::Straight {
            start: Vec2::new(-magnitude, magnitude),
            end: Vec2::new(magnitude, magnitude + 400.0),
        });
    }
    paths.extend(cubic_corpus());
    paths
}

/// The cumulative arc-length table exactly as it was built before
/// this change: a `Vec` grown by pushing
/// `table[i - 1] + prev.distance(pt)`. Written out here rather than
/// called, so the tests below compare the new shape against the old
/// computation instead of against itself.
fn reference_arc_length_table(start: Vec2, control1: Vec2, control2: Vec2, end: Vec2) -> Vec<f32> {
    let n = super::bezier::ARC_LENGTH_SUBDIVISIONS;
    let mut arc_lengths = Vec::with_capacity(n + 1);
    arc_lengths.push(0.0f32);
    let mut prev = start;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let pt = cubic_bezier_point(t, start, control1, control2, end);
        arc_lengths.push(arc_lengths[i - 1] + prev.distance(pt));
        prev = pt;
    }
    arc_lengths
}

/// The pre-change arc-length inversion, likewise written out rather
/// than called.
fn reference_arc_length_to_t(arc_lengths: &[f32], target_len: f32, n: usize) -> f32 {
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

/// The pre-change cubic sampler: `Vec` table, total read off its
/// last entry, one binary search per point.
///
/// The one thing it borrows from the code under test is
/// `sample_count`, which this change does not touch — reimplementing
/// its saturation rules here would put a second, unrelated
/// derivation in the test's way.
fn reference_cubic_samples(
    start: Vec2,
    control1: Vec2,
    control2: Vec2,
    end: Vec2,
    spacing: f32,
    cap: usize,
) -> Vec<Vec2> {
    let arc_lengths = reference_arc_length_table(start, control1, control2, end);
    let total_length = *arc_lengths.last().expect("the table carries its leading 0.0");
    if total_length < f32::EPSILON {
        return vec![start];
    }
    let n = super::bezier::ARC_LENGTH_SUBDIVISIONS;
    let count = sample_count(total_length, spacing, cap);
    let mut points = Vec::with_capacity(count);
    for i in 0..count {
        let target_len = (i as f32 * spacing).min(total_length);
        let t = reference_arc_length_to_t(&arc_lengths, target_len, n);
        points.push(cubic_bezier_point(t, start, control1, control2, end));
    }
    points
}

/// `cubic_bezier_length` no longer builds the cumulative table it
/// used to read the last entry of. The claim that replaces the table
/// is that a running sum reaches the *same float*, not merely a
/// close one — the additions happen in the order the table's
/// recurrence used them.
///
/// The reference is [`reference_arc_length_table`], the pre-change
/// body. Nothing it uses comes from the code under test except
/// `cubic_bezier_point`, which this change did not touch.
///
/// The input that makes this fail is any reassociation of the sum —
/// accumulating from the far end, or summing in pairs — which is
/// exactly the kind of "equivalent" rewrite that would silently move
/// every label along every curved edge by a fraction of a unit.
#[test]
fn test_bezier_length_equals_the_arc_length_table_total() {
    let corpus = cubic_corpus();
    assert!(corpus.len() > 30, "the corpus must not have shrunk to nothing");
    let mut curved = 0usize;
    for path in &corpus {
        let ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } = path
        else {
            unreachable!("cubic_corpus yields cubics only");
        };
        let table = reference_arc_length_table(*start, *control1, *control2, *end);
        let expected = *table.last().expect("the table carries its leading 0.0");
        let actual = cubic_bezier_length(*start, *control1, *control2, *end);
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "running sum must be bit-identical to the table's last entry for {path:?}: \
             got {actual}, table says {expected}"
        );
        if expected > 1.0 {
            curved += 1;
        }
    }
    // Without this the assertion above would be satisfied by a corpus
    // of degenerate zero-length curves, where both sides are 0.0.
    assert!(
        curved >= 30,
        "the corpus must contain curves with real length, not just degenerate ones; found {curved}"
    );
}

/// The sample sequence the new plan produces is the sequence the old
/// `Vec`-table sampler produced — same count, same points, in the
/// same order, bit for bit.
///
/// The reference is [`reference_cubic_samples`], the pre-change body.
/// Comparing the *planned* walk against `extend_cubic_samples` would
/// prove nothing: the collector is now written in terms of the plan,
/// so the two share every line that could be wrong. That mirror was
/// the first draft of this test, and an off-by-one planted in
/// `CubicSamples::position` left it green.
///
/// Both forms are checked against the reference, because the
/// connection pass still calls the collecting one.
#[test]
fn test_cubic_sampling_matches_the_pre_plan_reference() {
    use super::bezier::{extend_cubic_samples, plan_cubic_samples};

    let mut compared = 0usize;
    for path in cubic_corpus() {
        let ConnectionPath::CubicBezier {
            start,
            control1,
            control2,
            end,
        } = path
        else {
            unreachable!("cubic_corpus yields cubics only");
        };
        for spacing in [0.5f32, 4.0, 17.5, 1_000.0] {
            let expected = reference_cubic_samples(start, control1, control2, end, spacing, MAX_PATH_SAMPLES);
            let mut collected: Vec<Vec2> = Vec::new();
            extend_cubic_samples(
                start,
                control1,
                control2,
                end,
                spacing,
                MAX_PATH_SAMPLES,
                &mut collected,
            );
            let planned = plan_cubic_samples(start, control1, control2, end, spacing, MAX_PATH_SAMPLES);
            assert_eq!(
                collected.len(),
                expected.len(),
                "collected sample count diverged at spacing {spacing} for {path:?}"
            );
            assert_eq!(
                planned.len(),
                expected.len(),
                "planned sample count diverged at spacing {spacing} for {path:?}"
            );
            for (index, want) in expected.iter().enumerate() {
                for (label, got) in [
                    ("collected", collected[index]),
                    ("planned", planned.position(index)),
                ] {
                    assert_eq!(
                        got.x.to_bits(),
                        want.x.to_bits(),
                        "{label} sample {index} x diverged at spacing {spacing} for {path:?}: \
                         got {}, reference says {}",
                        got.x,
                        want.x
                    );
                    assert_eq!(
                        got.y.to_bits(),
                        want.y.to_bits(),
                        "{label} sample {index} y diverged at spacing {spacing} for {path:?}: \
                         got {}, reference says {}",
                        got.y,
                        want.y
                    );
                }
                compared += 1;
            }
        }
    }
    // A reference that reported zero samples everywhere would satisfy
    // the loop above without comparing anything.
    assert!(
        compared > 10_000,
        "expected the corpus to produce a substantial number of samples; compared {compared}"
    );
}

/// `distance_to_path` now walks the samples instead of collecting
/// them into a `Vec`. The answer must not have moved by a bit.
///
/// The reference collects, exactly as the previous body did, and
/// reduces with the same private segment helper — which this change
/// did not touch. What is being compared is therefore the walk, not
/// the arithmetic: a dropped first or last segment, a stale `prev`,
/// or a `min` taken over the wrong pairs all show up here.
#[test]
fn test_distance_to_path_matches_a_collected_sample_walk() {
    fn reference_distance(point: Vec2, path: &ConnectionPath) -> f32 {
        let samples = sample_path(path, DISTANCE_SAMPLE_SPACING, MAX_PATH_SAMPLES);
        if samples.is_empty() {
            return f32::INFINITY;
        }
        if samples.len() == 1 {
            return point.distance(samples[0]);
        }
        let mut min_sq = f32::INFINITY;
        for pair in samples.windows(2) {
            let d = point_to_segment_distance_squared(point, pair[0], pair[1]);
            if d < min_sq {
                min_sq = d;
            }
        }
        min_sq.sqrt()
    }

    let mut rng = Lcg(0x1234_5eed);
    let mut checked = 0usize;
    for path in cubic_corpus() {
        for _ in 0..12 {
            let point = rng.next_vec2(-800.0, 800.0);
            let expected = reference_distance(point, &path);
            let actual = distance_to_path(point, &path);
            assert_eq!(
                actual.to_bits(),
                expected.to_bits(),
                "streamed distance {actual} != collected {expected} at {point:?} on {path:?}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 400, "the sweep must actually run; checked {checked}");
}

/// The convex-hull property `path_bounds` rests on, checked against
/// the points `distance_to_path` actually measures: every sample of
/// every corpus path lies inside the box of that path's control
/// points, **to within the slack the code itself allows**.
///
/// That slack is `HULL_ESCAPE_SLACK × |coordinate|`, not one ulp.
/// The first version of this test allowed one ulp of the coordinate
/// and said the escape was "many orders below the tolerance margin",
/// which is false: the escape is around `|coordinate| ×
/// f32::EPSILON`, so it passes a canvas-space click tolerance
/// somewhere around x = 200 and keeps going. That sentence was the
/// premise that let a real defect ship, so the assertion is now
/// written against the constant the production code compares with —
/// if the two ever disagree, this test is what says so.
///
/// **The control is inside the test.** The same sweep run against the
/// *endpoint* box — the wrong box, which a hump escapes — must fail
/// for at least one corpus path. Without it, a `path_bounds`
/// returning an infinite box would pass.
#[test]
fn test_path_bounds_contains_every_sampled_point() {
    fn contains_with_escape_slack(point: Vec2, min: Vec2, max: Vec2) -> bool {
        let slack = |lo: f32, hi: f32| lo.abs().max(hi.abs()).max(1.0) * HULL_ESCAPE_SLACK;
        let sx = slack(min.x, max.x);
        let sy = slack(min.y, max.y);
        point.x >= min.x - sx && point.x <= max.x + sx && point.y >= min.y - sy && point.y <= max.y + sy
    }

    let mut escaped_the_endpoint_box = 0usize;
    let mut sampled = 0usize;
    for path in path_corpus() {
        let (min, max) = path_bounds(&path);
        let (chord_min, chord_max) = match path {
            ConnectionPath::Straight { start, end } => (start.min(end), start.max(end)),
            ConnectionPath::CubicBezier { start, end, .. } => (start.min(end), start.max(end)),
        };
        for sample in sample_path(&path, DISTANCE_SAMPLE_SPACING, MAX_PATH_SAMPLES) {
            let p = sample;
            assert!(
                contains_with_escape_slack(p, min, max),
                "sample {p:?} escaped the control-point box {min:?}..{max:?} of {path:?} \
                 by more than HULL_ESCAPE_SLACK allows"
            );
            if !contains_with_escape_slack(p, chord_min, chord_max) {
                escaped_the_endpoint_box += 1;
            }
            sampled += 1;
        }
    }
    assert!(sampled > 1_000, "the sweep must actually run; sampled {sampled}");
    assert!(
        escaped_the_endpoint_box > 100,
        "the containment assertion has no teeth unless a wrong box fails it: \
         {escaped_the_endpoint_box} samples escaped the endpoint box"
    );
}

/// The largest escape **any** sweep has measured, in units of
/// `max|coordinate| × f32::EPSILON`.
///
/// **This is a record of measurements, not a bound**, and the
/// distinction is the reason it is written down this way. Three
/// sweeps have produced a figure, by three different methods, and
/// they disagree in the direction that matters — each denser probe
/// found a larger number than the last:
///
/// | sweep | method | worst |
/// |---|---|---|
/// | the corpus walk below | `path_corpus`, sampled as `distance_to_path` samples | ~1.86 |
/// | the dense `t` sweep below | 11 magnitudes × 26 polygons × 20 001 `t` values | ~2.46 |
/// | a wider off-tree sweep | 21 magnitudes × ~80 polygons × 200 001 `t` values | 2.77 |
/// | review's polygon sweep | 64 800 polygons | 2.88 |
///
/// So the constant is set to the largest of them, and no claim is
/// made that a denser sweep could not find more. The escape has no
/// closed-form bound written down anywhere in this tree; if one is
/// ever derived it belongs beside `HULL_ESCAPE_SLACK` and this
/// constant becomes a cross-check on it.
///
/// The two-directional assertions in
/// `test_path_bounds_slack_covers_the_sampler_escape` are what keep
/// this honest: no sweep may exceed this value, and
/// `HULL_ESCAPE_SLACK` must stay at least four times above it. The
/// number lives here and nowhere else — prose that wants it names
/// this constant rather than restating the digits, because a figure
/// copied into a doc comment is a figure that goes stale silently.
/// That is exactly how the first version of this area shipped a
/// defect.
const MEASURED_WORST_ESCAPE_ULPS: f32 = 2.88;

/// The escape `HULL_ESCAPE_SLACK` covers, re-measured on every run,
/// and the record of the largest measurement kept where exactly one
/// copy of it exists.
///
/// **No closed-form bound is offered here, and none is implied.**
/// Every figure below is a measured maximum over a finite sweep — a
/// sample, not a ceiling. That distinction is the whole reason this
/// test exists in this shape: the first version of
/// `test_path_bounds_contains_every_sampled_point` generalised a
/// sample ("many orders below the tolerance margin") into a bound,
/// and it cost a shipping defect. An operation count over the
/// Bernstein form — four coefficient products, four term products,
/// three sums, so roughly eight roundings each relative to
/// `max|coordinate|` — puts the error in the same handful-of-ulps
/// range the sweeps find, which is why the numbers cluster where they
/// do. That is an order-of-magnitude sanity check on the sweeps, not
/// a proof, and no proof is offered.
///
/// Two sweeps, because they answer different questions:
///
/// 1. **The corpus walk** — every path `path_corpus` yields, sampled
///    the way `distance_to_path` samples it. This is the escape as the
///    production path actually produces it, and it is the smaller
///    number: `sample_path` visits arc-length-uniform `t` values,
///    which is a sparse and shape-dependent subset of `[0, 1]`.
/// 2. **A dense `t` sweep** across many magnitudes, over the polygon
///    families the escape is largest for. The worst escape lives at
///    particular `t`, so sweeping `t` densely at fixed control points
///    is a strictly better probe of it than sweeping shapes and taking
///    whatever `t` the sampler happened to visit. This is where
///    [`MEASURED_WORST_ESCAPE_ULPS`] comes from.
///
/// **The measurement is not circular.** The expectation is
/// `path_bounds`'s `min`/`max` over four given control points, which
/// is exact — no rounding to speak of and nothing from the code under
/// test. Measuring the sampler's rounding *with* the sampler is the
/// measurement; a mirror would recompute the escape and compare it to
/// itself.
///
/// Three assertions, and each has a direction:
///
/// - the escape is **real** — a zero worst would mean the slack is
///   dead weight, and this is where that would be noticed;
/// - the record is **not stale** — a sweep that exceeds
///   [`MEASURED_WORST_ESCAPE_ULPS`] fails here rather than being
///   quietly absorbed, which is what keeps the recorded figure and
///   the sweeps that produced it from drifting apart;
/// - the constant still **covers the record with room** — a headroom
///   under four is a warning shot fired here rather than a dropped
///   click found by a user.
#[test]
fn test_path_bounds_slack_covers_the_sampler_escape() {
    use super::bezier::cubic_bezier_point;

    fn escape_ulps(value: f32, lo: f32, hi: f32, scale: f32) -> Option<f32> {
        if scale <= 0.0 || !scale.is_finite() {
            return None;
        }
        Some((lo - value).max(value - hi).max(0.0) / (scale * f32::EPSILON))
    }

    // 1. The corpus walk, through the production sampler.
    let mut corpus_worst = 0.0f32;
    let mut corpus_measured = 0usize;
    for path in path_corpus() {
        let (min, max) = path_bounds(&path);
        let magnitude = Vec2::new(min.x.abs().max(max.x.abs()), min.y.abs().max(max.y.abs()));
        for sample in sample_path(&path, DISTANCE_SAMPLE_SPACING, MAX_PATH_SAMPLES) {
            let p = sample;
            for (value, lo, hi, scale) in [(p.x, min.x, max.x, magnitude.x), (p.y, min.y, max.y, magnitude.y)]
            {
                if let Some(ratio) = escape_ulps(value, lo, hi, scale) {
                    corpus_worst = corpus_worst.max(ratio);
                    corpus_measured += 1;
                }
            }
        }
    }

    // 2. The dense `t` sweep. The escape lives at particular `t`, so
    //    sweeping `t` finely at fixed control points probes it far
    //    better than sweeping shapes and reading whatever `t` the
    //    arc-length sampler happened to land on. Eleven magnitudes
    //    from 1 to 10^20, and the families the escape is largest for:
    //    the degenerate box (every control point equal, so the box is
    //    a point and nothing can hide) and thin boxes made by nudging
    //    one control point off it.
    const T_STEPS: usize = 20_000;
    let mut sweep_worst = 0.0f32;
    let mut sweep_worst_case = String::new();
    let mut sweep_measured = 0usize;
    let mut per_magnitude: Vec<(f32, f32)> = Vec::new();
    for exponent in (0..=20i32).step_by(2) {
        let m = 10f32.powi(exponent);
        let mut families: Vec<[f32; 4]> = vec![[m, m, m, m], [-m, -m, -m, -m]];
        for rel in [1.0e-6f32, 1.0e-3, 0.1] {
            for slot in 0..4 {
                let mut up = [m; 4];
                up[slot] = m * (1.0 + rel);
                families.push(up);
                let mut down = [m; 4];
                down[slot] = m * (1.0 - rel);
                families.push(down);
            }
        }
        let mut worst_here = 0.0f32;
        for family in &families {
            let lo = family.iter().copied().fold(f32::INFINITY, f32::min);
            let hi = family.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let scale = family.iter().map(|v| v.abs()).fold(0.0f32, f32::max);
            for step in 0..=T_STEPS {
                let t = step as f32 / T_STEPS as f32;
                // One axis at a time: a y component cannot affect an
                // x-axis escape, so the same four numbers go through
                // both slots and only the x is read.
                let value = cubic_bezier_point(
                    t,
                    Vec2::new(family[0], family[0]),
                    Vec2::new(family[1], family[1]),
                    Vec2::new(family[2], family[2]),
                    Vec2::new(family[3], family[3]),
                )
                .x;
                if let Some(ratio) = escape_ulps(value, lo, hi, scale) {
                    if ratio > sweep_worst {
                        sweep_worst = ratio;
                        sweep_worst_case = format!("{family:?} at t={t}");
                    }
                    worst_here = worst_here.max(ratio);
                    sweep_measured += 1;
                }
            }
        }
        per_magnitude.push((m, worst_here));
    }

    assert!(
        corpus_measured > 2_000 && sweep_measured > 400_000,
        "both sweeps must actually run; {corpus_measured} corpus / {sweep_measured} dense"
    );
    assert!(
        corpus_worst > 0.5 && sweep_worst > 0.5,
        "no sample left the control-point box at all (corpus {corpus_worst}, dense \
         {sweep_worst}); either the corpus lost its axis-aligned entries or \
         HULL_ESCAPE_SLACK guards nothing"
    );

    // The escape does not grow with the coordinate — that is *why*
    // scaling the margin by the coordinate is the right move, and it
    // is a property rather than a number, so it is asserted rather
    // than quoted.
    let spread = per_magnitude
        .iter()
        .map(|(_, w)| *w)
        .fold(f32::NEG_INFINITY, f32::max)
        / per_magnitude
            .iter()
            .map(|(_, w)| *w)
            .fold(f32::INFINITY, f32::min);
    assert!(
        spread < 4.0,
        "the escape must be magnitude-invariant in these units, or coordinate-relative is \
         the wrong quantity to scale by; worst-per-magnitude spread {spread}x over \
         {per_magnitude:?}"
    );

    let worst = corpus_worst.max(sweep_worst);
    assert!(
        worst <= MEASURED_WORST_ESCAPE_ULPS,
        "a sweep found {worst} ulps of escape, above the recorded worst of \
         {MEASURED_WORST_ESCAPE_ULPS}. Raise the record (and re-check the headroom below) \
         rather than lowering the sweep. Worst: {sweep_worst_case}"
    );
    let headroom = (HULL_ESCAPE_SLACK / f32::EPSILON) / MEASURED_WORST_ESCAPE_ULPS;
    assert!(
        headroom >= 4.0,
        "HULL_ESCAPE_SLACK is {} ulps against a recorded worst of \
         {MEASURED_WORST_ESCAPE_ULPS} ({headroom}x headroom) — raise the constant",
        HULL_ESCAPE_SLACK / f32::EPSILON
    );
}

/// `distance_to_path_within`'s contract: `Some(d)` exactly when
/// `distance_to_path` returns a `d` with `d <= tolerance`.
///
/// The sweep drives every corpus path against three families of
/// points, and the third is the one that earns its place:
///
/// 1. random points across the canvas, which mostly sit far away and
///    exercise the reject;
/// 2. `tolerance` chosen *relative to the answer* — including equal
///    to the distance itself, the knife edge the reject's rounding
///    slack exists for;
/// 3. **points taken off the path itself**, nudged by a fraction of
///    the tolerance. A point that is genuinely on an edge is the only
///    input that can expose a reject which is too tight, and family 1
///    almost never produces one: a random point in a 3000-unit square
///    is not within 12 units of a specific curve. Without this family
///    the sweep agreed with itself while the early-out silently
///    dropped hits — see `axis_aligned_corpus` for the shape that
///    made that visible.
#[test]
fn test_distance_to_path_within_agrees_with_the_unbounded_form() {
    let mut rng = Lcg(0x0bad_f00d);
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut on_path = 0usize;
    for path in path_corpus() {
        let mut points: Vec<Vec2> = (0..16).map(|_| rng.next_vec2(-1_500.0, 1_500.0)).collect();

        // Family 3: real hits, and near-misses beside them.
        let samples = sample_path(&path, DISTANCE_SAMPLE_SPACING, MAX_PATH_SAMPLES);
        let step = (samples.len() / 8).max(1);
        for &sample in samples.iter().step_by(step).take(9) {
            points.push(sample);
            for nudge in [
                Vec2::new(3.0, 0.0),
                Vec2::new(-3.0, 0.0),
                Vec2::new(0.0, 3.0),
                Vec2::new(0.0, -7.5),
            ] {
                points.push(sample + nudge);
            }
            on_path += 1;
        }

        for point in points {
            let distance = distance_to_path(point, &path);
            let tolerances = [
                0.0,
                distance * 0.5,
                distance,
                distance * 1.000_01,
                distance + 1.0,
                1.0,
                8.0,
                12.0,
                1.0e9,
            ];
            for tolerance in tolerances {
                let expected = (distance <= tolerance).then_some(distance);
                let actual = distance_to_path_within(point, &path, tolerance);
                assert_eq!(
                    actual, expected,
                    "within({point:?}, {tolerance}) on {path:?}: got {actual:?}, \
                     unbounded form says {distance}"
                );
                match expected {
                    Some(_) => accepted += 1,
                    None => rejected += 1,
                }
            }
        }
    }
    // All three families have to have run, or the sweep is testing one
    // branch of the function and calling it two.
    assert!(accepted > 200, "no accepting case exercised; accepted {accepted}");
    assert!(rejected > 200, "no rejecting case exercised; rejected {rejected}");
    assert!(
        on_path > 200,
        "no on-path point exercised; {on_path} sample anchors"
    );
}

/// The contract holds over non-finite geometry too, which is the one/// The contract holds over non-finite geometry too, which is the one
/// place the early-out and the full computation could plausibly
/// disagree by construction rather than by rounding.
///
/// Both routes have to answer `None`, and they get there differently:
/// `outside_bounds_by`'s comparisons are all false against a `NaN`, so
/// nothing rejects early and the full computation runs; that
/// computation yields `NaN`, and `NaN <= tolerance` is false. The
/// interesting part is that `path_bounds` does **not** reliably become
/// `NaN` — `Vec2::min` returns whichever operand its comparison falls
/// through to, so whether a `NaN` survives into the bound depends on
/// which side it is written — and the agreement therefore has to hold
/// for a finite box over poisoned geometry as well as for a poisoned
/// one. Both occur here and both are asserted.
///
/// A related thing I had wrong in prose before this test existed: the
/// sample plan does not collapse to `B(0) == start`. It collapses to
/// `NaN` whenever the non-finite component is in `control1`,
/// `control2` or `end`, because the Bernstein form multiplies those by
/// a literal `0.0` at `t = 0` and `0.0 * NaN` is `NaN`. The conclusion
/// was right and the reason was not, which is exactly the kind of
/// claim that should be a test instead of a sentence.
#[test]
fn test_distance_to_path_within_holds_over_non_finite_geometry() {
    const POISONS: [f32; 3] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
    let base = ConnectionPath::CubicBezier {
        start: Vec2::new(10.0, 20.0),
        control1: Vec2::new(40.0, 120.0),
        control2: Vec2::new(90.0, 120.0),
        end: Vec2::new(130.0, 20.0),
    };
    let probes = [
        Vec2::new(70.0, 80.0),
        Vec2::new(10.0, 20.0),
        Vec2::new(-4_000.0, 9_000.0),
        Vec2::new(f32::NAN, 0.0),
        Vec2::splat(f32::INFINITY),
    ];

    let mut checked = 0usize;
    let mut finite_boxes = 0usize;
    let mut non_finite_boxes = 0usize;
    let mut non_finite_answers = 0usize;
    for slot in 0..4usize {
        for component in 0..2usize {
            for poison in POISONS {
                let ConnectionPath::CubicBezier {
                    mut start,
                    mut control1,
                    mut control2,
                    mut end,
                } = base
                else {
                    unreachable!("base is a cubic");
                };
                {
                    let target = match slot {
                        0 => &mut start,
                        1 => &mut control1,
                        2 => &mut control2,
                        _ => &mut end,
                    };
                    if component == 0 {
                        target.x = poison;
                    } else {
                        target.y = poison;
                    }
                }
                let path = ConnectionPath::CubicBezier {
                    start,
                    control1,
                    control2,
                    end,
                };
                let (min, max) = path_bounds(&path);
                if min.is_finite() && max.is_finite() {
                    finite_boxes += 1;
                } else {
                    non_finite_boxes += 1;
                }
                for probe in probes {
                    let distance = distance_to_path(probe, &path);
                    if !distance.is_finite() {
                        non_finite_answers += 1;
                    }
                    for tolerance in [0.0f32, 1.0, 12.0, 1.0e9, f32::INFINITY] {
                        let expected = (distance <= tolerance).then_some(distance);
                        let actual = distance_to_path_within(probe, &path, tolerance);
                        assert_eq!(
                            actual, expected,
                            "slot {slot} component {component} poison {poison} probe {probe:?} \
                             tolerance {tolerance}: got {actual:?}, unbounded form says {distance}"
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 500, "the sweep must actually run; checked {checked}");
    // Both kinds of box have to occur, because the agreement has to
    // hold for both. Which poison produces which is not something to
    // assert: `Vec2::min` returns whichever operand its comparison
    // falls through to, so a `NaN` survives into the bound or does not
    // depending on which side of the fold it lands, and this sweep
    // produces both from `NaN` alone. If only one kind appeared the
    // sweep would be agreeing about one shape while claiming two.
    assert!(
        finite_boxes > 0,
        "no poisoned path kept a finite box; the agreement is only being checked against a \
         bound that is itself non-finite"
    );
    assert!(
        non_finite_boxes > 0,
        "no poisoned path produced a non-finite box; the non-finite bound is unexercised"
    );
    assert!(
        non_finite_answers > 50,
        "the geometry must actually poison the measured distance; {non_finite_answers} did"
    );
}

/// The reject must actually reject: a point far outside a path's box
/// answers `None` without the sampling walk running at all.
///
/// A behavioral test cannot see "the walk did not run", so what this
/// pins is the observable half — the answer — plus the arrangement
/// that makes the early-out the only way to reach it: a path whose
/// sample count is in the thousands, and a point a hundred thousand
/// units away from it.
#[test]
fn test_distance_to_path_within_rejects_a_point_outside_the_box() {
    let path = ConnectionPath::CubicBezier {
        start: Vec2::new(0.0, 0.0),
        control1: Vec2::new(25_000.0, 10_000.0),
        control2: Vec2::new(75_000.0, -10_000.0),
        end: Vec2::new(100_000.0, 0.0),
    };
    let (min, max) = path_bounds(&path);
    let far = Vec2::new(max.x + 100_000.0, min.y - 100_000.0);
    assert_eq!(distance_to_path_within(far, &path, 12.0), None);
    // And the same point is not rejected once the tolerance covers
    // the gap, which is what says the reject is tolerance-driven
    // rather than unconditional.
    let generous = distance_to_path_within(far, &path, 1.0e9);
    assert!(
        generous.is_some(),
        "a tolerance larger than the whole canvas must not reject"
    );
}

/// The rewrites in this section remove work that no runtime
/// assertion can observe, so the removal is pinned in the source
/// text: the arc-length table is an array, the length path does not
/// build a table at all, the sampling file allocates no vector at
/// all, and the hit-test path does not collect its samples.
///
/// Each clause is scoped to one function body through
/// `braced_block_after`, and each is paired with the assertion that
/// the token it forbids *does* still occur elsewhere in the same
/// file. Without that pairing a scan that resolved to the empty
/// string would report all three as clean.
///
/// **What it does not catch, stated where the gap is widest.** The
/// two body-scoped clauses are absence-of-token over one body, so a
/// one-line wrapper defeats them: a
/// `fn collect_samples(path) -> Vec<Vec2>` calling `sample_path`,
/// called from `distance_to_path`, leaves this test green and puts
/// the vector back. That evasion has been demonstrated rather than
/// assumed. Those clauses are a guard against the shape drifting
/// back, not a proof that no allocation remains — and they are weaker
/// than their `grapheme_chad` sibling, because they forbid a *call*
/// rather than a construction, and a call is the easiest thing to
/// rename.
///
/// The file-scoped clause is the exception and the reason to read the
/// three differently: a wrapper cannot help there, because the
/// wrapper would have to live in the same file to be called from it,
/// and the token it spells is the one being forbidden. It still says
/// nothing about `Vec` values reaching this file from elsewhere —
/// `sample_path_into`'s `out` parameter is exactly that, by design.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_the_sampling_hot_paths_carry_no_collected_vector() {
    use crate::util::rust_source::{braced_block_after, production_code};

    const BEZIER: &str = "lib/baumhard/src/mindmap/connection/bezier.rs";
    const CONNECTION: &str = "lib/baumhard/src/mindmap/connection/mod.rs";

    let bezier = production_code(BEZIER);
    let connection = production_code(CONNECTION);
    assert!(
        bezier.len() > 2_000 && connection.len() > 2_000,
        "both files must have read as real production code; got {} and {} bytes",
        bezier.len(),
        connection.len()
    );

    let body = |code: &str, header: &str| -> String {
        let block = braced_block_after(code, header)
            .unwrap_or_else(|| panic!("`{header}` must still be declared with a body"));
        assert!(block.len() > 80, "`{header}` resolved to an empty-looking body");
        block.to_string()
    };

    // Nothing in this file allocates a vector at all: the arc-length
    // table is an array, the length path keeps a running sum, and the
    // sampler appends into a buffer its caller owns.
    //
    // This clause used to be scoped to `build_arc_length_table`'s body
    // and paired with a `bezier.contains("Vec::")` guard proving the
    // scan could see one somewhere — the collecting sampler owned it.
    // That sampler is gone (#36 item 1b), so the claim is now the
    // whole file, which is strictly stronger: the old form could not
    // fail on a *different* body allocating, and this one does. The
    // pairing it replaces is not lost, only moved: the token asserted
    // present below is one this file must contain for any reason at
    // all, so a `production_code` that returned nothing useful still
    // fails rather than reading as clean.
    assert!(
        bezier.contains("ArcLengthTable"),
        "the scan must have read real declarations out of {BEZIER}"
    );
    assert!(
        !bezier.contains("Vec::") && !bezier.contains("vec!"),
        "no body in {BEZIER} may allocate a vector: {bezier}"
    );

    // The length path does not build the table. The name still
    // occurs in the file, in the sample planner that does build one.
    assert!(
        bezier.matches("build_arc_length_table").count() >= 2,
        "the scan must be able to see the table builder named elsewhere"
    );
    let length = body(&bezier, "fn cubic_bezier_length(");
    assert!(
        !length.contains("build_arc_length_table"),
        "the length path must not build the cumulative table: {length}"
    );

    // The hit-test path does not collect its samples. `sample_path`
    // is still declared in this same file — the connection pass calls
    // it from another crate — so the absence below is again a
    // property of the body rather than of the scan.
    assert!(
        connection.contains("sample_path"),
        "the scan must be able to see `sample_path` named elsewhere"
    );
    let distance = body(&connection, "pub fn distance_to_path(");
    assert!(
        !distance.contains("sample_path"),
        "the hit-test distance must not collect a sample vector: {distance}"
    );
}
