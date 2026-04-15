//! Pure-function layout positioning coverage: hue ring circle,
//! sat/val bar monotonicity, center override translation, backdrop
//! fit, preview centering, preview clearance from adjacent cells,
//! per-cell ink-offset application, and hue-ring chord-distance.

use super::fixtures::sample_geometry;
use crate::application::color_picker::{
    compute_color_picker_layout, CROSSHAIR_CENTER_CELL, HUE_SLOT_COUNT, PickerHit,
    SAT_CELL_COUNT, VAL_CELL_COUNT,
};
use crate::application::widgets::color_picker_widget::load_spec;

/// The per-glyph arm and preview ink offsets carried on geometry
/// are subtracted from the corresponding cell / preview-anchor
/// positions at layout time. The previous revision tested a
/// uniform per-arm offset; we now test that each cell consumes
/// its own array entry — varying the offset per index and
/// checking each cell shifted by exactly its own (dx, dy) scaled
/// to layout pixels.
#[test]
fn layout_subtracts_ink_offsets_for_arms_and_preview() {
    let baseline = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
    let mut g = sample_geometry();
    // Distinct per-cell offsets so a "single offset for the whole
    // arm" regression would visibly fail. Index `i` (arm-local)
    // carries (0.01*(i+1), -0.02*(i+1)) on the top arm, etc.
    for i in 0..CROSSHAIR_CENTER_CELL {
        let f = (i + 1) as f32;
        g.arm_top_ink_offsets[i] = (0.01 * f, -0.02 * f);
        g.arm_bottom_ink_offsets[i] = (-0.01 * f, 0.02 * f);
        g.arm_left_ink_offsets[i] = (0.015 * f, 0.005 * f);
        g.arm_right_ink_offsets[i] = (-0.015 * f, -0.005 * f);
    }
    g.preview_ink_offset = (0.25, -0.3);
    let shifted = compute_color_picker_layout(&g, 1280.0, 720.0);

    let cell_fs = baseline.cell_font_size;
    let preview_size = baseline.preview_size;

    // Val bar: each arm cell shifts by its own (dx, dy)*cell_fs.
    // Centre cell at CROSSHAIR_CENTER_CELL is untouched.
    for i in 0..VAL_CELL_COUNT {
        let (expect_dx, expect_dy) = if i == CROSSHAIR_CENTER_CELL {
            (0.0, 0.0)
        } else if i < CROSSHAIR_CENTER_CELL {
            let (rx, ry) = g.arm_top_ink_offsets[i];
            (-rx * cell_fs, -ry * cell_fs)
        } else {
            let (rx, ry) = g.arm_bottom_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1];
            (-rx * cell_fs, -ry * cell_fs)
        };
        let dx = shifted.val_cell_positions[i].0 - baseline.val_cell_positions[i].0;
        let dy = shifted.val_cell_positions[i].1 - baseline.val_cell_positions[i].1;
        assert!((dx - expect_dx).abs() < 0.001, "val[{i}].dx {dx} vs {expect_dx}");
        assert!((dy - expect_dy).abs() < 0.001, "val[{i}].dy {dy} vs {expect_dy}");
    }

    // Sat bar: same per-cell pattern using the left/right arrays.
    for i in 0..SAT_CELL_COUNT {
        let (expect_dx, expect_dy) = if i == CROSSHAIR_CENTER_CELL {
            (0.0, 0.0)
        } else if i < CROSSHAIR_CENTER_CELL {
            let (rx, ry) = g.arm_left_ink_offsets[i];
            (-rx * cell_fs, -ry * cell_fs)
        } else {
            let (rx, ry) = g.arm_right_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1];
            (-rx * cell_fs, -ry * cell_fs)
        };
        let dx = shifted.sat_cell_positions[i].0 - baseline.sat_cell_positions[i].0;
        let dy = shifted.sat_cell_positions[i].1 - baseline.sat_cell_positions[i].1;
        assert!((dx - expect_dx).abs() < 0.001, "sat[{i}].dx {dx} vs {expect_dx}");
        assert!((dy - expect_dy).abs() < 0.001, "sat[{i}].dy {dy} vs {expect_dy}");
    }

    // Preview glyph anchor shifts by (-0.25*preview_size, +0.3*preview_size).
    let dx = shifted.preview_pos.0 - baseline.preview_pos.0;
    let dy = shifted.preview_pos.1 - baseline.preview_pos.1;
    assert!((dx - (-0.25 * preview_size)).abs() < 0.001);
    assert!((dy - (0.3 * preview_size)).abs() < 0.001);
}

#[test]
fn layout_emits_24_hue_slots_on_circle() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    assert_eq!(layout.hue_slot_positions.len(), 24);
    // Ring radius is derived from the actual ring font size, not
    // the base font_size, since HUE_RING_FONT_SCALE > 1 makes
    // the ring glyphs larger than the base.
    let r_target = layout.outer_radius - layout.ring_font_size * 0.5;
    for (i, (px, py)) in layout.hue_slot_positions.iter().enumerate() {
        let dx = px - layout.center.0;
        let dy = py - layout.center.1;
        let r = (dx * dx + dy * dy).sqrt();
        assert!(
            (r - r_target).abs() < 0.5,
            "slot {i} radius {r} differs from {r_target}",
        );
    }
}

#[test]
fn layout_first_hue_slot_is_at_top() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let (px, py) = layout.hue_slot_positions[0];
    // Slot 0 sits at 12 o'clock — same x as center, smaller y.
    assert!((px - layout.center.0).abs() < 0.5);
    assert!(py < layout.center.1);
}

#[test]
fn layout_sat_bar_monotonic_x_constant_y() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    assert_eq!(layout.sat_cell_positions.len(), SAT_CELL_COUNT);
    for w in layout.sat_cell_positions.windows(2) {
        assert!(w[1].0 > w[0].0, "sat cells must increase in x");
        assert!((w[0].1 - w[1].1).abs() < 0.1, "sat cells share y");
    }
}

#[test]
fn layout_val_bar_monotonic_y_constant_x() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    assert_eq!(layout.val_cell_positions.len(), VAL_CELL_COUNT);
    for w in layout.val_cell_positions.windows(2) {
        assert!(w[1].1 > w[0].1, "val cells must increase in y");
        assert!((w[0].0 - w[1].0).abs() < 0.1, "val cells share x");
    }
}

/// With `center_override` set, every position in the layout
/// (hue slots, bar cells, chips, preview, backdrop) must
/// translate by the offset between the override and the default
/// window center.
#[test]
fn center_override_translates_all_positions() {
    let screen_w = 1280.0;
    let screen_h = 720.0;
    let mut g = sample_geometry();
    let baseline = compute_color_picker_layout(&g, screen_w, screen_h);
    let offset = (200.0_f32, -80.0_f32);
    g.center_override = Some((
        screen_w * 0.5 + offset.0,
        screen_h * 0.5 + offset.1,
    ));
    let shifted = compute_color_picker_layout(&g, screen_w, screen_h);
    assert!((shifted.center.0 - baseline.center.0 - offset.0).abs() < 1e-3);
    assert!((shifted.center.1 - baseline.center.1 - offset.1).abs() < 1e-3);
    assert!((shifted.hue_slot_positions[0].0 - baseline.hue_slot_positions[0].0 - offset.0).abs() < 1e-3);
    assert!((shifted.hue_slot_positions[0].1 - baseline.hue_slot_positions[0].1 - offset.1).abs() < 1e-3);
    assert!((shifted.sat_cell_positions[0].0 - baseline.sat_cell_positions[0].0 - offset.0).abs() < 1e-3);
    let (bl0, bt0, _, _) = baseline.backdrop;
    let (bl1, bt1, _, _) = shifted.backdrop;
    assert!((bl1 - bl0 - offset.0).abs() < 1e-3);
    assert!((bt1 - bt0 - offset.1).abs() < 1e-3);
}

/// Hover-hit diffing: a layout computed with no hover should
/// not differ structurally from one computed with a Hue hover
/// — the builder applies the scale bump, but the pure layout
/// positions don't shift.
#[test]
fn hovered_hit_does_not_alter_layout_positions() {
    let mut g = sample_geometry();
    let baseline = compute_color_picker_layout(&g, 1280.0, 720.0);
    g.hovered_hit = Some(PickerHit::Hue(0));
    let hovered = compute_color_picker_layout(&g, 1280.0, 720.0);
    assert_eq!(
        baseline.hue_slot_positions[0], hovered.hue_slot_positions[0],
        "hovered_hit must not alter hue slot positions"
    );
    assert_eq!(baseline.backdrop, hovered.backdrop);
}

/// Layout must fit inside the window even on small windows.
#[test]
fn layout_backdrop_fits_inside_small_window() {
    let g = sample_geometry();
    for &(w, h) in &[(320.0_f32, 240.0_f32), (400.0, 300.0), (200.0, 200.0)] {
        let layout = compute_color_picker_layout(&g, w, h);
        let (left, top, bw, bh) = layout.backdrop;
        assert!(left >= 0.0, "backdrop left underflows on {w}x{h}");
        assert!(top >= 0.0, "backdrop top underflows on {w}x{h}");
        assert!(left + bw <= w + 0.5,
            "backdrop right overflows on {w}x{h}: left={left} bw={bw} w={w}");
        assert!(top + bh <= h + 0.5,
            "backdrop bottom overflows on {w}x{h}: top={top} bh={bh} h={h}");
    }
}

/// Preview glyph must center on the geometric wheel center given
/// the layout-emitted `preview_size`.
#[test]
fn layout_preview_centered_on_wheel_center() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let (px, py) = layout.preview_pos;
    let cx = px + layout.preview_size * 0.5;
    let cy = py + layout.preview_size * 0.5;
    assert!((cx - layout.center.0).abs() < 1.0,
        "preview x center {cx} differs from wheel center {}", layout.center.0);
    assert!((cy - layout.center.1).abs() < 1.0,
        "preview y center {cy} differs from wheel center {}", layout.center.1);
}

/// Regression guard for the "࿕ overlaps the first arm letter" bug.
#[test]
fn layout_keeps_preview_clear_of_adjacent_arm_cells() {
    let padding_scale = load_spec().geometry.bar_to_preview_padding_scale;
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let preview_radius = layout.preview_size * 0.5;
    let min_clearance = preview_radius + layout.font_size * padding_scale;

    let center = layout.center;
    let neighbours = [
        ("val[CENTER - 1]", layout.val_cell_positions[CROSSHAIR_CENTER_CELL - 1]),
        ("val[CENTER + 1]", layout.val_cell_positions[CROSSHAIR_CENTER_CELL + 1]),
        ("sat[CENTER - 1]", layout.sat_cell_positions[CROSSHAIR_CENTER_CELL - 1]),
        ("sat[CENTER + 1]", layout.sat_cell_positions[CROSSHAIR_CENTER_CELL + 1]),
    ];
    let slack = 0.5;
    for (label, (px, py)) in neighbours {
        let dx = px - center.0;
        let dy = py - center.1;
        let dist = (dx * dx + dy * dy).sqrt();
        assert!(
            dist + slack >= min_clearance,
            "{label} at ({px:.1}, {py:.1}) is {dist:.1} px from centre — below preview clearance {min_clearance:.1}",
        );
    }
}

/// Hue ring slots must not overlap at the new 1.5× font scale.
#[test]
fn hue_ring_slots_do_not_overlap_at_new_font_scale() {
    let g = sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    for i in 0..HUE_SLOT_COUNT {
        let j = (i + 1) % HUE_SLOT_COUNT;
        let (px, py) = layout.hue_slot_positions[i];
        let (qx, qy) = layout.hue_slot_positions[j];
        let dx = qx - px;
        let dy = qy - py;
        let dist = (dx * dx + dy * dy).sqrt();
        assert!(
            dist >= g.max_ring_advance * 0.9,
            "adjacent hue slots {i} and {j} only {dist} apart, \
            expected >= {}",
            g.max_ring_advance * 0.9,
        );
    }
}
