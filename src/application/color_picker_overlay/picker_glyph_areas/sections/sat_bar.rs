//! Saturation bar: 16 cells (8 left + 8 right of the centre slot,
//! centre slot is the preview ࿕ glyph and rendered separately).
//! Each cell tints to the HSV at its position, with the
//! currently-selected cell glowing 60% toward white and the hovered
//! cell glowing 40% toward white + scaled up.

use baumhard::gfx_structs::area::OutlineStyle;

use super::super::areas::{PickerAreas, PickerSection};
use super::super::make_area::make_area;
use crate::application::color_picker::{
    arm_left_glyphs, arm_right_glyphs, picker_channel, sat_cell_to_value, ColorPickerLayout,
    ColorPickerOverlayGeometry, PickerHit, CROSSHAIR_CENTER_CELL, SAT_CELL_COUNT,
};
use crate::application::color_picker_overlay::color::{
    highlight_hovered_cell_color, highlight_selected_cell_color, rgb_to_cosmic_color,
};
use crate::application::widgets::color_picker_widget::ColorPickerWidgetSpec;
use baumhard::util::color::hsv_to_rgb;

pub(in crate::application::color_picker_overlay::picker_glyph_areas) fn build(
    areas: &mut PickerAreas,
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
    outline: Option<OutlineStyle>,
    spec: &ColorPickerWidgetSpec,
) {
    let hover_scale = spec.geometry.hover_scale;
    let cell_font_size = layout.cell_font_size;
    let cell_box_w =
        (layout.cell_advance * spec.geometry.cell_box_scale).max(cell_font_size * 1.5);

    let current_sat_cell = (geometry.sat * (SAT_CELL_COUNT as f32 - 1.0))
        .round()
        .clamp(0.0, (SAT_CELL_COUNT - 1) as f32) as usize;

    for i in 0..SAT_CELL_COUNT {
        if i == CROSSHAIR_CENTER_CELL {
            continue;
        }
        let cell_sat = sat_cell_to_value(i);
        let base_rgb = hsv_to_rgb(geometry.hue_deg, cell_sat, geometry.val);
        let is_hovered = matches!(geometry.hovered_hit, Some(PickerHit::SatCell(h)) if h == i);
        let color = if is_hovered {
            highlight_hovered_cell_color(base_rgb)
        } else if i == current_sat_cell {
            highlight_selected_cell_color(base_rgb)
        } else {
            rgb_to_cosmic_color(base_rgb)
        };
        let glyph = if i < CROSSHAIR_CENTER_CELL {
            arm_left_glyphs()[i]
        } else {
            arm_right_glyphs()[i - CROSSHAIR_CENTER_CELL - 1]
        };
        let scale = if is_hovered { hover_scale } else { 1.0 };
        let (cx, cy) = layout.sat_cell_positions[i];
        let fs = cell_font_size * scale;
        let bw = cell_box_w * scale;
        areas.push(
            PickerSection::SatBar,
            i,
            picker_channel("sat_bar", i),
            make_area(
                glyph,
                color,
                fs,
                fs,
                (cx - bw * 0.5, cy - fs * 0.5),
                (bw, fs * 1.5),
                true,
                None,
                outline,
            ),
        );
    }
}
