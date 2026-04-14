//! Title cell: one `GlyphArea` showing the picker's mode-aware
//! header text (e.g. `࿕ edge color` in contextual mode,
//! `࿕ color palette` in standalone mode). Re-tinted to the live
//! preview color every frame; text rebuilt only on the layout phase.

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
    let title_text = if is_standalone {
        spec.title_template_standalone.clone()
    } else {
        spec.title_template_contextual
            .replace("{target_label}", geometry.target_label)
    };

    areas.push(
        PickerSection::Title,
        0,
        picker_channel("title", 0),
        make_area(
            &title_text,
            preview_color,
            font_size,
            font_size,
            layout.title_pos,
            (font_size * 24.0, font_size * 1.5),
            false,
            None,
            outline,
        ),
    );
}
