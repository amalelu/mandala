//! Value bar: 16 cells (8 top + 8 bottom of the centre slot, centre
//! is the preview ࿕). Top arm uses Devanagari fallback fonts; bottom
//! arm pins `arm_bottom_font()` (Egyptian hieroglyphs). Selected /
//! hovered cells highlight the same way as sat_bar.

use baumhard::gfx_structs::area::OutlineStyle;

use super::super::areas::{PickerAreas, PickerSection};
use super::super::make_area::make_area;
use crate::application::color_picker::{
    arm_bottom_font, arm_bottom_glyphs, arm_top_glyphs, picker_channel, val_cell_to_value,
    ColorPickerLayout, ColorPickerOverlayGeometry, PickerHit, CROSSHAIR_CENTER_CELL,
    VAL_CELL_COUNT,
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

    let current_val_cell = ((1.0 - geometry.val) * (VAL_CELL_COUNT as f32 - 1.0))
        .round()
        .clamp(0.0, (VAL_CELL_COUNT - 1) as f32) as usize;

    for i in 0..VAL_CELL_COUNT {
        if i == CROSSHAIR_CENTER_CELL {
            continue;
        }
        let cell_val = val_cell_to_value(i);
        let base_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, cell_val);
        let is_hovered = matches!(geometry.hovered_hit, Some(PickerHit::ValCell(h)) if h == i);
        let color = if is_hovered {
            highlight_hovered_cell_color(base_rgb)
        } else if i == current_val_cell {
            highlight_selected_cell_color(base_rgb)
        } else {
            rgb_to_cosmic_color(base_rgb)
        };
        let (glyph, font) = if i < CROSSHAIR_CENTER_CELL {
            (arm_top_glyphs()[i], None)
        } else {
            (
                arm_bottom_glyphs()[i - CROSSHAIR_CENTER_CELL - 1],
                arm_bottom_font(),
            )
        };
        let scale = if is_hovered { hover_scale } else { 1.0 };
        let (cx, cy) = layout.val_cell_positions[i];
        let fs = cell_font_size * scale;
        let bw = cell_box_w * scale;
        areas.push(
            PickerSection::ValBar,
            i,
            picker_channel("val_bar", i),
            make_area(
                glyph,
                color,
                fs,
                fs,
                (cx - bw * 0.5, cy - fs * 0.5),
                (bw, fs * 1.5),
                true,
                font,
                outline,
            ),
        );
    }
}
