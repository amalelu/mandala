//! Shared test fixtures for the color picker overlay tests. Kept
//! `pub(super)` so each sibling test module can construct geometry +
//! pull pre-computed area lists without re-deriving the constants.

use baumhard::gfx_structs::area::GlyphArea;

use crate::application::color_picker::{
    compute_color_picker_layout, ColorPickerOverlayGeometry, CROSSHAIR_CENTER_CELL,
};
use crate::application::color_picker_overlay::picker_glyph_areas::picker_glyph_areas;

/// Plausible stub picker geometry for tests. The pure-function layout
/// only cares that the advances are non-zero and self-consistent;
/// ink offsets default to zero so the layout uses classic em-box
/// centering unless a test overrides them.
pub(super) fn picker_sample_geometry() -> ColorPickerOverlayGeometry {
    ColorPickerOverlayGeometry {
        target_label: "edge",
        hue_deg: 0.0,
        sat: 1.0,
        val: 1.0,
        preview_hex: "#ff0000".to_string(),
        hex_visible: false,
        max_cell_advance: 16.0,
        max_ring_advance: 24.0,
        measurement_font_size: 16.0,
        size_scale: 1.0,
        center_override: None,
        hovered_hit: None,
        arm_top_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
        arm_bottom_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
        arm_left_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
        arm_right_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
        preview_ink_offset: (0.0, 0.0),
    }
}

/// Compute the picker's channel-ordered `(channel, GlyphArea)` list
/// at the canonical 1280×720 viewport — what every test that reasons
/// about emitted areas wants.
pub(super) fn picker_glyph_areas_for(
    geometry: &ColorPickerOverlayGeometry,
) -> Vec<(usize, GlyphArea)> {
    let layout = compute_color_picker_layout(geometry, 1280.0, 720.0);
    picker_glyph_areas(geometry, &layout)
}
