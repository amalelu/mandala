//! `hit_test_picker` coverage: Outside (far + backdrop-corner), Commit
//! on center, DragAnchor inside the wheel disk but not on any glyph,
//! Hue slot, SatCell. Together with the hit enum variants covered by
//! `crate::application::color_picker::hit`, these tests pin the
//! public contract of picker hit-testing — now with a circular outer
//! gate that matches the visual wheel.

use super::fixtures::sample_geometry;
use crate::application::color_picker::{
    compute_color_picker_layout, hit_test_picker, PickerHit, SAT_CELL_COUNT,
};

#[test]
fn hit_test_far_outside_returns_outside() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    assert_eq!(hit_test_picker(&layout, -10.0, -10.0), PickerHit::Outside);
    assert_eq!(hit_test_picker(&layout, 5000.0, 5000.0), PickerHit::Outside);
}

/// A click in the top-left corner of the (rectangular) backdrop —
/// inside the old AABB gate but outside the wheel's circle —
/// must now resolve to `Outside`. This is the whole reason for the
/// switch to a circular outer gate: corners of the backdrop chrome
/// belong to the canvas beneath, not to the picker.
#[test]
fn hit_test_backdrop_corner_is_outside_not_drag_anchor() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let (bl, bt, _bw, _bh) = layout.backdrop;
    let x = bl + 4.0;
    let y = bt + 4.0;
    assert_eq!(hit_test_picker(&layout, x, y), PickerHit::Outside);
}

/// A click at the exact wheel center — where the central ࿕ glyph
/// lives — must resolve to `Commit`.
#[test]
fn hit_test_hits_commit_on_center() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let (cx, cy) = layout.center;
    assert_eq!(hit_test_picker(&layout, cx, cy), PickerHit::Commit);
}

/// A click inside the wheel's circle but off every interactive
/// glyph must resolve to `DragAnchor` — the
/// anywhere-you-can-grab-the-wheel zone. Position ourselves at a
/// 45° diagonal offset between the center and the hue ring, which
/// lands inside the disk but safely off the hue glyphs and off the
/// sat/val bar lines.
#[test]
fn hit_test_drag_anchor_between_bars_and_ring() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    // Halfway between center and ring radius, along a 45° diagonal —
    // stays off every cardinal bar and every ring slot for any
    // reasonable sizing of the sample geometry. The assertions
    // below pin the fixture invariants this test relies on so a
    // future tweak that makes the ring font big relative to the
    // wheel (violating "off the ring") fails loudly here rather
    // than silently flipping the result of `hit_test_picker`.
    let r = layout.outer_radius * 0.5;
    let diag = r / std::f32::consts::SQRT_2;
    let x = layout.center.0 + diag;
    let y = layout.center.1 + diag;
    // `diag` must clear the cardinal-bar tolerance on both axes.
    let cell_half = (layout.cell_advance * 0.5).max(layout.font_size * 0.4);
    assert!(
        diag > cell_half,
        "test point at diag={diag} must sit off the cardinal bars \
         (cell_half={cell_half})"
    );
    // The closest hue slot sits on a circle of radius `ring_r` at
    // the nearest 15° step; the distance from our test point to
    // any ring slot is at least `ring_r − r = outer_radius / 2 −
    // ring_font_size / 2` once you collapse to the diagonal, which
    // must exceed `ring_font_size / 2` (the glyph hit radius) or
    // the test point grazes the ring.
    let ring_half = layout.ring_font_size * 0.5;
    let min_dist_to_ring = (layout.outer_radius * 0.5 - ring_half).abs();
    assert!(
        min_dist_to_ring > ring_half,
        "test point must clear the hue ring: min_dist_to_ring={min_dist_to_ring}, \
         ring_half={ring_half}"
    );
    assert_eq!(hit_test_picker(&layout, x, y), PickerHit::DragAnchor);
}

/// Just past the wheel's outer rim on a cardinal — still inside
/// the backdrop rect, but outside the circle. Must miss.
#[test]
fn hit_test_just_past_outer_radius_is_outside() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let x = layout.center.0 + layout.outer_radius + 1.0;
    let y = layout.center.1;
    assert_eq!(hit_test_picker(&layout, x, y), PickerHit::Outside);
}

#[test]
fn hit_test_hits_first_hue_slot() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let (px, py) = layout.hue_slot_positions[0];
    assert_eq!(hit_test_picker(&layout, px, py), PickerHit::Hue(0));
}

#[test]
fn hit_test_hits_off_center_sat_cell() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    // Pick a sat cell offset from the center so the val-bar branch
    // is not a candidate.
    let off = SAT_CELL_COUNT / 2 + 2;
    let (cx, cy) = layout.sat_cell_positions[off];
    assert_eq!(hit_test_picker(&layout, cx, cy), PickerHit::SatCell(off));
}
