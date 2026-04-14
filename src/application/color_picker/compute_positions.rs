//! Per-cell position computations for the picker layout — step 4
//! of the canonical formula, after `compute_sizing` has settled the
//! font_size / ring_r / step. Produces the 24 hue-ring anchors, the
//! 17-cell sat/val arrays with per-glyph ink offsets, the centre
//! preview anchor, the backdrop rect, and the title / hint / hex
//! anchors.

use std::f32::consts::{FRAC_PI_2, TAU};

use super::compute_sizing::Sizing;
use super::geometry::ColorPickerOverlayGeometry;
use super::glyph_tables::{CROSSHAIR_CENTER_CELL, HUE_SLOT_COUNT, SAT_CELL_COUNT, VAL_CELL_COUNT};
use super::layout::ColorPickerLayout;
use crate::application::widgets::color_picker_widget::GeometrySpec;

/// Expand the sizing struct + geometry into a full `ColorPickerLayout`.
/// All per-cell math is confined here so `compute_sizing` can stay
/// focused on the font-size back-solve.
pub(super) fn compute_positions(
    geometry: &ColorPickerOverlayGeometry,
    g: &GeometrySpec,
    screen_w: f32,
    screen_h: f32,
    sizing: Sizing,
) -> ColorPickerLayout {
    let Sizing {
        font_size,
        ring_font_size,
        cell_fs,
        char_width,
        outer_radius,
        ring_r,
        side,
        step,
        cell_advance: _,
    } = sizing;

    // Wheel center: honor the drag-override if the user has moved
    // the wheel, else sit at the window center.
    let center = geometry
        .center_override
        .unwrap_or((screen_w * 0.5, screen_h * 0.5));

    // ---- Hue ring (24 slots, clockwise from 12 o'clock) ----
    let mut hue_slot_positions = [(0.0_f32, 0.0_f32); HUE_SLOT_COUNT];
    for (i, slot) in hue_slot_positions.iter_mut().enumerate() {
        let angle = (i as f32 / HUE_SLOT_COUNT as f32) * TAU - FRAC_PI_2;
        *slot = (
            center.0 + angle.cos() * ring_r,
            center.1 + angle.sin() * ring_r,
        );
    }

    // ---- Crosshair sat/val bars (17 cells each) ----
    let bar_span = step * (SAT_CELL_COUNT as f32 - 1.0);

    let mut sat_cell_positions = [(0.0_f32, 0.0_f32); SAT_CELL_COUNT];
    let mut val_cell_positions = [(0.0_f32, 0.0_f32); VAL_CELL_COUNT];
    for i in 0..SAT_CELL_COUNT {
        let base_x = center.0 - bar_span * 0.5 + i as f32 * step;
        let base_y = center.1;
        let ink_ratio = if i < CROSSHAIR_CENTER_CELL {
            geometry.arm_left_ink_offsets[i]
        } else if i > CROSSHAIR_CENTER_CELL {
            geometry.arm_right_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1]
        } else {
            (0.0, 0.0)
        };
        sat_cell_positions[i] = (base_x - ink_ratio.0 * cell_fs, base_y - ink_ratio.1 * cell_fs);
    }
    for i in 0..VAL_CELL_COUNT {
        let base_x = center.0;
        let base_y = center.1 - bar_span * 0.5 + i as f32 * step;
        let ink_ratio = if i < CROSSHAIR_CENTER_CELL {
            geometry.arm_top_ink_offsets[i]
        } else if i > CROSSHAIR_CENTER_CELL {
            geometry.arm_bottom_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1]
        } else {
            (0.0, 0.0)
        };
        val_cell_positions[i] = (base_x - ink_ratio.0 * cell_fs, base_y - ink_ratio.1 * cell_fs);
    }

    // ---- Centre preview ࿕ ----
    let preview_size = font_size * g.preview_size_scale;
    let preview_ink_px = (
        geometry.preview_ink_offset.0 * preview_size,
        geometry.preview_ink_offset.1 * preview_size,
    );
    let preview_pos = (
        center.0 - preview_size * 0.5 - preview_ink_px.0,
        center.1 - preview_size * 0.5 - preview_ink_px.1,
    );

    // ---- Backdrop, title, hint ----
    // Backdrop height leaves room for title (1 font_size above the
    // wheel) + wheel diameter + hex readout row (1.5 font_size) +
    // hint footer (1.5 font_size) + bottom padding (3 font_size).
    let backdrop_width = side.min((screen_w - font_size * 2.0).max(0.0));
    let backdrop_left = center.0 - backdrop_width * 0.5;
    let backdrop_top = center.1 - side * 0.5 - font_size;
    let backdrop_height = side + font_size * 7.0;
    let backdrop = (backdrop_left, backdrop_top, backdrop_width, backdrop_height);
    let title_pos = (backdrop_left + font_size * 0.5, backdrop_top + font_size * 0.5);
    let hint_pos = (
        backdrop_left + font_size * 0.5,
        backdrop_top + backdrop_height - font_size * 1.5,
    );

    // ---- Hex readout position ----
    let hex_pos = if geometry.hex_visible {
        let hex_width = char_width * 7.0;
        let hex_y = center.1 + outer_radius + font_size * 1.5;
        Some((center.0 - hex_width * 0.5, hex_y))
    } else {
        None
    };

    ColorPickerLayout {
        center,
        outer_radius,
        font_size,
        char_width,
        cell_advance: step,
        cell_font_size: cell_fs,
        ring_font_size,
        hue_slot_positions,
        sat_cell_positions,
        val_cell_positions,
        preview_pos,
        preview_size,
        backdrop,
        title_pos,
        hint_pos,
        hex_pos,
    }
}
