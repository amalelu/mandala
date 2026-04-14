//! Font-size derivation for the picker layout — steps 1–3 of the
//! canonical sizing formula: recover per-script advance ratios,
//! compute `wheel_side_in_fonts`, back-solve font_size from the
//! target wheel-diameter fraction.
//!
//! Extracted from `compute.rs` so the size-derivation math can be
//! read (and later unit-tested) in isolation from the per-cell
//! position work.

use std::f32::consts::TAU;

use super::geometry::ColorPickerOverlayGeometry;
use super::glyph_tables::{CROSSHAIR_CENTER_CELL, HUE_SLOT_COUNT};
use crate::application::widgets::color_picker_widget::GeometrySpec;

/// Everything `compute_positions` needs from the sizing pass. One
/// struct so the orchestrator in `compute.rs` threads a single value,
/// not nine.
pub(super) struct Sizing {
    pub(super) font_size: f32,
    pub(super) ring_font_size: f32,
    pub(super) cell_fs: f32,
    pub(super) char_width: f32,
    pub(super) outer_radius: f32,
    pub(super) ring_r: f32,
    pub(super) side: f32,
    pub(super) step: f32,
    pub(super) cell_advance: f32,
}

/// Derive every size-level value from the viewport + geometry +
/// spec. No per-cell positions — those come after, in
/// `compute_positions`.
pub(super) fn derive_sizing(
    geometry: &ColorPickerOverlayGeometry,
    g: &GeometrySpec,
    screen_w: f32,
    screen_h: f32,
) -> Sizing {
    let ring_scale = g.hue_ring_font_scale;

    // Step 1: dimensionless advance ratios. The renderer measures
    // `max_cell_advance` at `measurement_font_size` and
    // `max_ring_advance` at `measurement_font_size * ring_scale`,
    // so the per-font ratios are extracted symmetrically. Fall
    // back to plausible Latin-ish defaults if the measurement was
    // skipped (test stubs with measurement_font_size == 0).
    let measurement_fs = geometry.measurement_font_size.max(1.0);
    let cell_factor = if geometry.measurement_font_size > 0.0 {
        geometry.max_cell_advance / measurement_fs
    } else {
        (geometry.max_cell_advance / g.font_max).max(0.6)
    };
    let ring_factor = if geometry.measurement_font_size > 0.0 {
        geometry.max_ring_advance / (measurement_fs * ring_scale)
    } else {
        (geometry.max_ring_advance / (g.font_max * ring_scale)).max(0.6)
    };

    // Preview-clearance floor on the per-font cell advance. Floors
    // cell_factor at `preview_size / 2 + padding` so the preview's
    // ink doesn't overlap cell[9] / cell[11] on the arm.
    let preview_clearance_per_font = g.preview_size_scale * 0.5 + g.bar_to_preview_padding_scale;
    let cell_factor = cell_factor.max(preview_clearance_per_font);

    // Step 2: wheel_side_in_fonts.
    let inner_per_font = CROSSHAIR_CENTER_CELL as f32 * cell_factor;
    let bar_pad_per_font = ring_scale * g.bar_to_ring_padding_scale;
    let crosshair_ring_per_font = inner_per_font + bar_pad_per_font;
    let glyph_ring_per_font = (HUE_SLOT_COUNT as f32 * ring_scale * ring_factor) / TAU;
    let ring_r_per_font = crosshair_ring_per_font.max(glyph_ring_per_font);
    let wheel_side_in_fonts = 2.0 * (ring_r_per_font + ring_scale * 0.5 + 1.0);

    // Step 3: derive font_size from desired widget size, then clamp
    // for width / height fit. See file-level doc in compute.rs for
    // the full reasoning on the clamp chain.
    let short = screen_w.min(screen_h).max(1.0);
    let target_side = short * g.target_frac * geometry.size_scale.max(0.01);
    let font_from_target = target_side / wheel_side_in_fonts.max(1.0);
    let font_clamped = font_from_target.clamp(g.font_min, g.font_max);
    let max_font_for_h = (screen_h / (wheel_side_in_fonts + 12.0)).max(1.0);
    let chip_width_in_fonts: f32 = 32.0;
    let max_font_for_w =
        (screen_w / (wheel_side_in_fonts + 2.0).max(chip_width_in_fonts)).max(1.0);
    let font_size = font_clamped.min(max_font_for_h).min(max_font_for_w).max(1.0);

    let char_width = font_size * 0.6;
    let ring_font_size = font_size * ring_scale;

    // Step 4: re-derive every dimension at the chosen font_size.
    let cell_advance = (cell_factor * font_size).max(char_width);
    let ring_advance = (ring_factor * ring_font_size).max(ring_font_size * 0.6);
    let inner_extent = CROSSHAIR_CENTER_CELL as f32 * cell_advance;
    let bar_to_ring_padding = ring_font_size * g.bar_to_ring_padding_scale;
    let min_ring_r = (HUE_SLOT_COUNT as f32 * ring_advance) / TAU;
    let desired_ring_r = (inner_extent + bar_to_ring_padding).max(min_ring_r);

    // Backdrop side derived from the now-canonical ring_r. Clamps
    // are defensive against rounding and the rare case where the
    // chip-row constraint forced a smaller font than the wheel-side
    // formula expected.
    let ring_outer = desired_ring_r + ring_font_size * 0.5;
    let side_from_ring = (ring_outer + font_size) * 2.0;
    let max_side_for_w = (screen_w - font_size * 2.0).max(0.0);
    let max_side_for_h = (screen_h - font_size * 12.0).max(0.0);
    let side = side_from_ring
        .min(max_side_for_w)
        .min(max_side_for_h)
        .max(0.0);
    let outer_radius = (side * 0.5 - font_size).max(0.0);
    let ring_r = (outer_radius - ring_font_size * 0.5).max(0.0);

    // Cell-step may shrink below `cell_advance` when the constrained
    // ring forced the inner extent smaller than
    // `CROSSHAIR_CENTER_CELL * cell_advance` — keeps small-window
    // bars from producing overlapping arm glyphs.
    let constrained_inner = ring_r - bar_to_ring_padding;
    let actual_cell_advance = if constrained_inner > 0.0 {
        (constrained_inner / CROSSHAIR_CENTER_CELL as f32).min(cell_advance)
    } else {
        0.0
    };

    Sizing {
        font_size,
        ring_font_size,
        cell_fs: font_size * g.cell_font_scale,
        char_width,
        outer_radius,
        ring_r,
        side,
        step: actual_cell_advance,
        cell_advance,
    }
}
