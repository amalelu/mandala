//! `make_area` — the single `GlyphArea` constructor every layout-phase
//! per-section builder routes through. Centralizes the conversion
//! from `(text, position, bounds, style)` into a fully-populated
//! `GlyphArea` with align / outline / single-span color region set.

use baumhard::core::primitives::ColorFontRegions;
use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::area::{GlyphArea, OutlineStyle};
use glam::Vec2;

/// Typographic + decoration parameters for a picker glyph area.
/// Grouped into one struct so the builder takes four arguments
/// (text, position, bounds, style) instead of nine; the former was
/// a §6 smell that had to be silenced with
/// `#[allow(clippy::too_many_arguments)]`.
pub(super) struct PickerAreaStyle {
    /// Fill color for the single color region covering the whole
    /// string.
    pub color: baumhard::font::Color,
    /// Font size in cosmic-text points.
    pub font_size: f32,
    /// Line height in cosmic-text points.
    pub line_height: f32,
    /// If `true`, text renders centered inside its bounds.
    pub centered: bool,
    /// Optional font override; `None` inherits the picker's base font.
    pub font: Option<AppFont>,
    /// Optional halo outline.
    pub outline: Option<OutlineStyle>,
}

/// Build a single picker `GlyphArea`. Every layout-phase section
/// builder calls this so the per-cell layout fields (alignment,
/// outline, color region) are produced consistently.
pub(super) fn make_area(
    text: &str,
    pos: (f32, f32),
    bounds: (f32, f32),
    style: PickerAreaStyle,
) -> GlyphArea {
    let mut area = GlyphArea::new_with_str(
        text,
        style.font_size,
        style.line_height,
        Vec2::new(pos.0, pos.1),
        Vec2::new(bounds.0, bounds.1),
    );
    area.align_center = style.centered;
    area.outline = style.outline;
    let rgba = [
        style.color.r() as f32 / 255.0,
        style.color.g() as f32 / 255.0,
        style.color.b() as f32 / 255.0,
        style.color.a() as f32 / 255.0,
    ];
    area.regions = ColorFontRegions::single_span(
        baumhard::util::grapheme_chad::count_grapheme_clusters(text),
        Some(rgba),
        style.font,
    );
    area
}
