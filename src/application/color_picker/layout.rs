//! `ColorPickerLayout` — pure-function output of the picker layout
//! pass, holding every screen-space anchor the renderer and the
//! hit-test need. The `compute_color_picker_layout` fn that
//! produces one lives in `compute.rs`; the fn is split out so this
//! file stays scannable as the picker's observable geometry
//! contract.

use super::glyph_tables::{HUE_SLOT_COUNT, SAT_CELL_COUNT, VAL_CELL_COUNT};

/// Pure-function output of the color-picker layout pass. All positions
/// are in screen-space pixels.
///
/// `PartialEq` exists so callers can detect "did anything that the
/// layout phase needs to refresh actually move?" without diffing the
/// upstream geometry / viewport tuple by hand. `Eq` is *not* implemented
/// because `f32` rules it out.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorPickerLayout {
    pub center: (f32, f32),
    pub outer_radius: f32,
    pub font_size: f32,
    pub char_width: f32,
    /// Actual per-cell advance used for both bars (derived from
    /// `geometry.max_cell_advance`). Exposed so the hit-test can use
    /// the same tolerance the renderer uses.
    pub cell_advance: f32,
    /// Cell font size (crosshair glyphs) actually used —
    /// `font_size * cell_font_scale`. Exposed so the renderer and
    /// the hit test use the same value.
    pub cell_font_size: f32,
    /// Ring font size actually used (`font_size * HUE_RING_FONT_SCALE`).
    /// Exposed so the renderer and hit test stay in sync.
    pub ring_font_size: f32,
    /// 24 hue ring positions, ordered clockwise from 12-o'clock.
    pub hue_slot_positions: [(f32, f32); HUE_SLOT_COUNT],
    /// 17 sat-bar cell centers, left → right. Cell 8 is the wheel
    /// center — NOT rendered (center glyph shows through), but still
    /// used by hit-testing so a click at the exact center resolves
    /// to it.
    pub sat_cell_positions: [(f32, f32); SAT_CELL_COUNT],
    /// 17 val-bar cell centers, top → bottom (top = brightest). Cell
    /// 8 is the wheel center — same skip rule as sat.
    pub val_cell_positions: [(f32, f32); VAL_CELL_COUNT],
    /// Center preview glyph anchor (the ࿕). Top-left corner of the
    /// glyph box, computed so the glyph visually centers on the wheel
    /// center given `preview_size`.
    pub preview_pos: (f32, f32),
    /// Font size for the central preview glyph. A multiple of the
    /// base `font_size` per `spec.geometry.preview_size_scale`.
    pub preview_size: f32,
    /// `(left, top, width, height)` of the opaque backdrop rect that
    /// the renderer draws under the overlay text pass.
    pub backdrop: (f32, f32, f32, f32),
    /// Title text anchor (top of frame).
    pub title_pos: (f32, f32),
    /// Hint footer text anchor.
    pub hint_pos: (f32, f32),
    /// `Some((x, y))` top-left anchor for the hex readout when it
    /// should render this frame, `None` otherwise. Derived from
    /// `geometry.hex_visible`. When `Some`, the readout is anchored
    /// below the wheel, horizontally centered on `center.0`.
    pub hex_pos: Option<(f32, f32)>,
}
