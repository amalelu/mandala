//! Hue ring: 24 sacred-script glyphs around the wheel's outer ring,
//! each fixed to its own hue (15° apart). Hover bumps a single cell's
//! scale + brightness; everything else stays stable across frames.

use baumhard::gfx_structs::area::OutlineStyle;

use super::super::areas::{PickerAreas, PickerSection};
use super::super::make_area::make_area;
use crate::application::color_picker::{
    hue_ring_glyphs, hue_slot_to_degrees, picker_channel, ColorPickerLayout,
    ColorPickerOverlayGeometry, PickerHit,
};
use crate::application::color_picker_overlay::color::{
    highlight_hovered_cell_color, rgb_to_cosmic_color,
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
    let hover_scale: f32 = spec.geometry.hover_scale;
    let ring_font_size = layout.ring_font_size;
    let ring_box_w = ring_font_size * spec.geometry.ring_box_scale;

    for (i, &ring_glyph) in hue_ring_glyphs().iter().enumerate() {
        let hue = hue_slot_to_degrees(i);
        let rgb = hsv_to_rgb(hue, 1.0, 1.0);
        let is_hovered = matches!(geometry.hovered_hit, Some(PickerHit::Hue(h)) if h == i);
        let color = if is_hovered {
            highlight_hovered_cell_color(rgb)
        } else {
            rgb_to_cosmic_color(rgb)
        };
        let scale = if is_hovered { hover_scale } else { 1.0 };
        let pos = layout.hue_slot_positions[i];
        let fs = ring_font_size * scale;
        let bw = ring_box_w * scale;
        areas.push(
            PickerSection::HueRing,
            i,
            picker_channel("hue_ring", i),
            make_area(
                ring_glyph,
                color,
                fs,
                fs,
                (pos.0 - bw * 0.5, pos.1 - fs * 0.5),
                (bw, fs * 1.5),
                true,
                None,
                outline,
            ),
        );
    }
}
