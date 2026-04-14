//! Hint footer: small mode-aware affordance text rendered below the
//! wheel (e.g. `Esc cancel · ࿕ commit · drag to move · RMB resize`).
//! Re-tinted to the live preview color every frame.

use baumhard::gfx_structs::area::OutlineStyle;

use super::super::areas::{PickerAreas, PickerSection};
use super::super::make_area::make_area;
use crate::application::color_picker::{picker_channel, ColorPickerLayout, ColorPickerOverlayGeometry};
use crate::application::color_picker_overlay::color::rgb_to_cosmic_color;
use crate::application::widgets::color_picker_widget::load_spec;
use baumhard::util::color::hsv_to_rgb;

pub(in crate::application::color_picker_overlay::picker_glyph_areas) fn build(
    areas: &mut PickerAreas,
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
    outline: Option<OutlineStyle>,
) {
    let spec = load_spec();
    let preview_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, geometry.val);
    let preview_color = rgb_to_cosmic_color(preview_rgb);
    let font_size = layout.font_size;

    let is_standalone = geometry.target_label.is_empty();
    let hint_text = if is_standalone {
        spec.hint_text_standalone.as_str()
    } else {
        spec.hint_text_contextual.as_str()
    };
    areas.push(
        PickerSection::Hint,
        0,
        picker_channel("hint", 0),
        make_area(
            hint_text,
            preview_color,
            font_size * 0.85,
            font_size * 0.85,
            layout.hint_pos,
            (font_size * 30.0, font_size * 1.5),
            false,
            None,
            outline,
        ),
    );
}
