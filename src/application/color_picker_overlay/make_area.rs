//! `make_area` — the per-element GlyphArea constructor used by
//! [`super::picker_glyph_areas`].
//!
//! Originally a local `fn` nested inside `picker_glyph_areas`; lifted
//! to a sibling file so each module stays small. Behavior is
//! unchanged — the helper captures nothing from the enclosing scope,
//! so extracting it is purely structural.
//!
//! `centered = true` shapes the text with `Align::Center` so
//! cross-script glyphs (Devanagari / Hebrew / Tibetan in the hue
//! ring, mixed sat/val cells) sit on the same visual radius.
//!
//! `font` pins a specific `AppFont` for this area's region span
//! when cosmic-text's default fallback won't pick a covering face —
//! the SMP-range Egyptian hieroglyphs in particular.

use baumhard::core::primitives::ColorFontRegions;
use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::area::{GlyphArea, OutlineStyle};
use glam::Vec2;

#[allow(clippy::too_many_arguments)]
pub(super) fn make_area(
    text: &str,
    color: cosmic_text::Color,
    font_size: f32,
    line_height: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
    centered: bool,
    font: Option<AppFont>,
    outline: Option<OutlineStyle>,
) -> GlyphArea {
    let mut area = GlyphArea::new_with_str(
        text,
        font_size,
        line_height,
        Vec2::new(pos.0, pos.1),
        Vec2::new(bounds.0, bounds.1),
    );
    area.align_center = centered;
    area.outline = outline;
    let rgba = [
        color.r() as f32 / 255.0,
        color.g() as f32 / 255.0,
        color.b() as f32 / 255.0,
        color.a() as f32 / 255.0,
    ];
    area.regions = ColorFontRegions::single_span(text.chars().count(), Some(rgba), font);
    area
}
