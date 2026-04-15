//! Crosshair-arm geometry: each arm renders exactly 8 cells, the
//! four arms emit a symmetric per-cell advance, and per-cell ink
//! correction lands each restored cell on its radial target.

use super::fixtures::sample_geometry;
use crate::application::color_picker::{
    arm_bottom_glyphs, arm_left_glyphs, arm_right_glyphs, arm_top_glyphs,
    compute_color_picker_layout, CROSSHAIR_CENTER_CELL, SAT_CELL_COUNT, VAL_CELL_COUNT,
};

/// Each crosshair arm must render exactly 8 cells. The bars have
/// SAT_CELL_COUNT / VAL_CELL_COUNT = 17 cells, cell
/// CROSSHAIR_CENTER_CELL = 8 is the shared wheel-center slot
/// (࿕ overlay), and each arm covers 8 non-center cells —
/// totaling 32 rendered crosshair glyphs.
#[test]
fn crosshair_arms_render_exactly_8_cells_each() {
    let layout = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
    // Center cell of the sat bar = wheel center.
    let (scx, scy) = layout.sat_cell_positions[CROSSHAIR_CENTER_CELL];
    assert!((scx - layout.center.0).abs() < 0.1);
    assert!((scy - layout.center.1).abs() < 0.1);
    // Center cell of the val bar = wheel center.
    let (vcx, vcy) = layout.val_cell_positions[CROSSHAIR_CENTER_CELL];
    assert!((vcx - layout.center.0).abs() < 0.1);
    assert!((vcy - layout.center.1).abs() < 0.1);
    assert_eq!(CROSSHAIR_CENTER_CELL, 8);
    assert_eq!(arm_left_glyphs().len(), 8);
    assert_eq!(SAT_CELL_COUNT - CROSSHAIR_CENTER_CELL - 1, 8);
    assert_eq!(arm_right_glyphs().len(), 8);
    assert_eq!(arm_top_glyphs().len(), 8);
    assert_eq!(arm_bottom_glyphs().len(), 8);
    assert_eq!(
        arm_top_glyphs().len()
            + arm_bottom_glyphs().len()
            + arm_left_glyphs().len()
            + arm_right_glyphs().len(),
        32,
    );
}

/// Every arm cell, after ink correction, must sit on its target
/// radial point — sat cells on `center.y`, val cells on
/// `center.x` — within 0.1 px. Regression against the previous
/// "single per-arm offset" model: a worst-case heuristic
/// mis-corrected non-worst glyphs by their delta. We re-add each
/// cell's stored ink offset and check the result hits the
/// unrotated radial point.
#[test]
fn crosshair_arms_per_cell_ink_correction_aligns_to_radial_target() {
    let mut g = sample_geometry();
    for i in 0..CROSSHAIR_CENTER_CELL {
        let f = (i + 1) as f32;
        g.arm_top_ink_offsets[i] = (0.02 * f, -0.03 * f);
        g.arm_bottom_ink_offsets[i] = (-0.02 * f, 0.03 * f);
        g.arm_left_ink_offsets[i] = (0.025 * f, 0.01 * f);
        g.arm_right_ink_offsets[i] = (-0.025 * f, -0.01 * f);
    }
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let cell_fs = layout.cell_font_size;
    let step = layout.cell_advance;
    let cx = layout.center.0;
    let cy = layout.center.1;

    // Sat cells: each (after re-adding its ink offset) must land
    // on (cx + (i - CENTER)*step, cy).
    for i in 0..SAT_CELL_COUNT {
        if i == CROSSHAIR_CENTER_CELL {
            continue;
        }
        let (rx, ry) = if i < CROSSHAIR_CENTER_CELL {
            g.arm_left_ink_offsets[i]
        } else {
            g.arm_right_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1]
        };
        let (px, py) = layout.sat_cell_positions[i];
        let restored_x = px + rx * cell_fs;
        let restored_y = py + ry * cell_fs;
        let target_x = cx + (i as f32 - CROSSHAIR_CENTER_CELL as f32) * step;
        assert!(
            (restored_x - target_x).abs() < 0.1,
            "sat[{i}] restored x {restored_x} != target {target_x}",
        );
        assert!(
            (restored_y - cy).abs() < 0.1,
            "sat[{i}] restored y {restored_y} != center y {cy}",
        );
    }

    // Val cells: each must land on (cx, cy + (i - CENTER)*step).
    for i in 0..VAL_CELL_COUNT {
        if i == CROSSHAIR_CENTER_CELL {
            continue;
        }
        let (rx, ry) = if i < CROSSHAIR_CENTER_CELL {
            g.arm_top_ink_offsets[i]
        } else {
            g.arm_bottom_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1]
        };
        let (px, py) = layout.val_cell_positions[i];
        let restored_x = px + rx * cell_fs;
        let restored_y = py + ry * cell_fs;
        let target_y = cy + (i as f32 - CROSSHAIR_CENTER_CELL as f32) * step;
        assert!(
            (restored_x - cx).abs() < 0.1,
            "val[{i}] restored x {restored_x} != center x {cx}",
        );
        assert!(
            (restored_y - target_y).abs() < 0.1,
            "val[{i}] restored y {restored_y} != target {target_y}",
        );
    }
}

/// The four crosshair arms must emit the same per-cell advance so
/// the cross reads as a symmetric cross, not a plus sign with one
/// fat arm. Checks that consecutive sat cells and consecutive val
/// cells have identical step distances.
#[test]
fn crosshair_arms_emit_symmetric_cell_advance() {
    let layout = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
    let sat_step = layout.sat_cell_positions[1].0 - layout.sat_cell_positions[0].0;
    let val_step = layout.val_cell_positions[1].1 - layout.val_cell_positions[0].1;
    assert!(
        (sat_step - val_step).abs() < 0.1,
        "sat step {sat_step} differs from val step {val_step} — \
        cross would render asymmetrically",
    );
    for i in 0..SAT_CELL_COUNT - 1 {
        let s = layout.sat_cell_positions[i + 1].0 - layout.sat_cell_positions[i].0;
        let v = layout.val_cell_positions[i + 1].1 - layout.val_cell_positions[i].1;
        assert!((s - sat_step).abs() < 0.1, "sat step {i}→{} drifted", i + 1);
        assert!((v - val_step).abs() < 0.1, "val step {i}→{} drifted", i + 1);
    }
}
