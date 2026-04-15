//! Centre preview ࿕ glyph: the wheel's focal point and commit
//! button. Tints to the current preview color (or the
//! commit-hovered highlight). Rendered ~3× the base font size so it
//! reads as the focal interactive surface.

use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::area::OutlineStyle;

use super::super::areas::{PickerAreas, PickerSection};
use super::super::make_area::make_area;
use crate::application::color_picker::{
    center_preview_glyph, picker_channel, ColorPickerLayout, ColorPickerOverlayGeometry, PickerHit,
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
    let hover_scale = spec.geometry.hover_scale;
    let preview_size = layout.preview_size;
    let preview_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, geometry.val);
    let preview_color = rgb_to_cosmic_color(preview_rgb);

    let commit_hovered = matches!(geometry.hovered_hit, Some(PickerHit::Commit));
    let commit_color = if commit_hovered {
        highlight_hovered_cell_color(preview_rgb)
    } else {
        preview_color
    };
    let preview_scale_f = if commit_hovered { hover_scale } else { 1.0 };
    let scaled_preview = preview_size * preview_scale_f;
    let center_font = Some(AppFont::NotoSerifTibetanRegular);
    let preview_glyph_center = (
        layout.preview_pos.0 + preview_size * 0.5,
        layout.preview_pos.1 + preview_size * 0.5,
    );
    // 1.5× the preview width gives the hover-grow some slack
    // without overlapping the surrounding crosshair cells.
    let preview_box_w = scaled_preview * 1.5;
    let preview_box_h = scaled_preview * 1.5;
    areas.push(
        PickerSection::Preview,
        0,
        picker_channel("preview", 0),
        make_area(
            center_preview_glyph(),
            commit_color,
            scaled_preview,
            scaled_preview,
            (
                preview_glyph_center.0 - preview_box_w * 0.5,
                preview_glyph_center.1 - preview_box_h * 0.5,
            ),
            (preview_box_w, preview_box_h),
            true,
            center_font,
            outline,
        ),
    );
}
