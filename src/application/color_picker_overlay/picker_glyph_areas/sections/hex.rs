//! Hex readout: the `#rrggbb` text shown when the cursor is inside
//! the picker's backdrop. Always emitted at a stable channel so the
//! mutator path doesn't have to handle a flickering element — when
//! the readout is hidden, the cell carries empty text instead of
//! being unregistered.

use baumhard::gfx_structs::area::OutlineStyle;

use super::super::areas::{PickerAreas, PickerSection};
use super::super::make_area::make_area;
use crate::application::color_picker::{picker_channel, ColorPickerLayout, ColorPickerOverlayGeometry};
use crate::application::color_picker_overlay::color::rgb_to_cosmic_color;
use baumhard::util::color::{hsv_to_hex, hsv_to_rgb};

pub(in crate::application::color_picker_overlay::picker_glyph_areas) fn build(
    areas: &mut PickerAreas,
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
    outline: Option<OutlineStyle>,
) {
    let preview_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, geometry.val);
    let preview_color = rgb_to_cosmic_color(preview_rgb);
    let font_size = layout.font_size;

    let (hex_text, hex_pos, hex_bounds) = match layout.hex_pos {
        Some(anchor) => (
            hsv_to_hex(geometry.hue_deg, geometry.sat, geometry.val),
            anchor,
            (font_size * 8.0, font_size * 1.5),
        ),
        None => (String::new(), (0.0, 0.0), (0.0, 0.0)),
    };
    areas.push(
        PickerSection::Hex,
        0,
        picker_channel("hex", 0),
        make_area(
            &hex_text,
            preview_color,
            font_size,
            font_size,
            hex_pos,
            hex_bounds,
            false,
            None,
            outline,
        ),
    );
}
