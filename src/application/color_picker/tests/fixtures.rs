//! Shared test fixtures: the plausible-stub `ColorPickerOverlayGeometry`
//! the pure-function layout/hit tests feed into `compute_color_picker_layout`.
//! Duplicating the fixture across every submodule would be cheaper
//! in lines but harder to keep in sync — one source of truth here.

use crate::application::color_picker::{
    ColorPickerOverlayGeometry, CROSSHAIR_CENTER_CELL,
};

pub(super) fn sample_geometry() -> ColorPickerOverlayGeometry {
    // Plausible stub advances measured at a notional 16 pt
    // baseline. cell ratio = 1.0 (worst-case sacred-script-ish),
    // ring ratio = 0.7 (typical at ring_scale = 1.7). The
    // pure-function layout only cares that the numbers are
    // non-zero and self-consistent. Ink offsets default to zero
    // so the layout tests see the classic em-box centering
    // unless a test explicitly overrides them.
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

pub(super) fn sample_geometry_with_hex() -> ColorPickerOverlayGeometry {
    let mut g = sample_geometry();
    g.hex_visible = true;
    g
}
