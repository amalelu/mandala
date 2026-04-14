//! `compute_color_picker_layout` — pure-function layout pass.
//! Orchestrates the two-step derivation in `compute_sizing` (font
//! size, ring radius, cell step) and `compute_positions` (per-cell
//! anchors, backdrop, title / hint / hex positions).
//!
//! No GPU access, no font system — unit tests construct a layout
//! from nothing but a geometry struct + screen dimensions.

use super::compute_positions::compute_positions;
use super::compute_sizing::derive_sizing;
use super::geometry::ColorPickerOverlayGeometry;
use super::layout::ColorPickerLayout;
use crate::application::widgets::color_picker_widget::load_spec;

/// Pure-function layout. No GPU access, no font system — mirrors
/// `compute_palette_frame_layout` so unit tests can construct one from
/// nothing but a geometry struct + screen dimensions.
///
/// # Canonical sizing formula
///
/// The picker is a *widget*, not a modal — its size is driven from a
/// target wheel-diameter fraction of the screen's shorter side. The
/// fn back-solves font_size by inverting the geometry chain:
///
/// 1. Convert the measured glyph advances (which arrive as absolute
///    pixels from `measure_max_glyph_advance` at picker open) into
///    dimensionless ratios by dividing by `measurement_font_size`.
/// 2. Compute `wheel_side_in_fonts` — the wheel-enclosing square's
///    side measured in units of `font_size`.
/// 3. Pick `target_side = short_axis * target_frac * size_scale` and
///    derive `font_size = clamp(target_side / wheel_side_in_fonts,
///    font_min, font_max)`.
///
/// Steps 1–3 live in [`super::compute_sizing`]. Everything downstream
/// — hue ring anchors, sat/val cell positions, preview anchor,
/// backdrop, title / hint / hex — lives in
/// [`super::compute_positions`]. This fn threads the spec + viewport
/// through both.
pub fn compute_color_picker_layout(
    geometry: &ColorPickerOverlayGeometry,
    screen_w: f32,
    screen_h: f32,
) -> ColorPickerLayout {
    let spec = load_spec();
    let g = &spec.geometry;
    let sizing = derive_sizing(geometry, g, screen_w, screen_h);
    compute_positions(geometry, g, screen_w, screen_h, sizing)
}
