//! `make_area` — the single `GlyphArea` constructor every layout-phase
//! per-section builder routes through. Centralizes the conversion
//! from `(text, color, font_size, line_height, position, bounds)` into
//! a fully-populated `GlyphArea` with align / outline / single-span
//! color region set.

use baumhard::core::primitives::ColorFontRegions;
use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::area::{GlyphArea, OutlineStyle};
use glam::Vec2;

/// Build a single picker `GlyphArea`. Every layout-phase section
/// builder calls this so the per-cell layout fields (alignment,
/// outline, color region) are produced consistently.
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
    area.regions = ColorFontRegions::single_span(
        baumhard::util::grapheme_chad::count_grapheme_clusters(text),
        Some(rgba),
        font,
    );
    area
}
