//! `hit_test_picker` coverage: Outside, Commit on center, DragAnchor
//! on backdrop-inside-but-not-on-glyph, Hue slot, SatCell. Together
//! with the hit enum variants covered by `crate::application::color_picker::hit`,
//! these tests pin the public contract of picker hit-testing.

use super::fixtures::sample_geometry;
use crate::application::color_picker::{
    compute_color_picker_layout, hit_test_picker, PickerHit, SAT_CELL_COUNT,
};

#[test]
fn hit_test_outside_backdrop_returns_outside() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    assert_eq!(hit_test_picker(&layout, -10.0, -10.0), PickerHit::Outside);
    assert_eq!(hit_test_picker(&layout, 5000.0, 5000.0), PickerHit::Outside);
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

/// A click just inside the backdrop but outside every interactive
/// glyph must resolve to `DragAnchor` — the
/// anywhere-you-can-grab-the-wheel zone.
#[test]
fn hit_test_drag_anchor_when_inside_backdrop_not_on_glyph() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let (bl, bt, _bw, _bh) = layout.backdrop;
    let x = bl + 4.0;
    let y = bt + 4.0;
    assert_eq!(hit_test_picker(&layout, x, y), PickerHit::DragAnchor);
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
