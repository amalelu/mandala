// SPDX-License-Identifier: MPL-2.0

//! Border-buffer creators and glyph-advance measurement. Every
//! helper returns a [`MindMapTextBuffer`] with
//! [`ZoomVisibility::unbounded`]; scene-builder routes overwrite it
//! to gate on zoom, overlay routes leave it at default.
//!
//! Per CODE_CONVENTIONS §1, styled-region → cosmic-text spans go
//! through `baumhard::font::attrs` — never inlined here. Hex-colour
//! parsing into `cosmic_text::Color` goes through
//! `baumhard::font::hex::hex_to_cosmic_color` (§B5: cosmic-text
//! usage stays inside `font/`).

use baumhard::font::metrics::monospace_advance;
use baumhard::font::{buffer, Attrs, FontSystem, SHAPING_ADVANCED};
use baumhard::gfx_structs::zoom_visibility::ZoomVisibility;

use super::MindMapTextBuffer;

/// Widest shaped advance across `glyphs` at `font_size`, via
/// cosmic-text. Falls back to `monospace_advance(font_size)` if
/// every glyph shapes to zero (tofu + missing fallback).
pub fn measure_max_glyph_advance(font_system: &mut FontSystem, glyphs: &[&str], font_size: f32) -> f32 {
    let mut buf = buffer::create_square(font_system, font_size);
    let attrs = Attrs::new();
    let mut max_w: f32 = 0.0;
    for g in glyphs {
        buf.set_text(font_system, g, &attrs, SHAPING_ADVANCED, None);
        buf.shape_until_scroll(font_system, false);
        for run in buf.layout_runs() {
            for glyph in run.glyphs.iter() {
                if glyph.w > max_w {
                    max_w = glyph.w;
                }
            }
        }
    }
    if max_w <= 0.0 {
        monospace_advance(font_size)
    } else {
        max_w
    }
}

pub(super) fn create_border_buffer(
    font_system: &mut FontSystem,
    text: &str,
    attrs: &Attrs,
    font_size: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    create_border_buffer_lh(font_system, text, attrs, font_size, font_size, pos, bounds)
}

/// Like [`create_border_buffer`] but sets an explicit line-height on
/// the buffer metrics. Needed for multi-line console side columns,
/// where the vertical stack of `│` glyphs has to advance at the
/// content's `row_height` (font_size + 2px breathing room) — not the
/// default `font_size`, which would drift the side column short by
/// 2px per row.
pub(super) fn create_border_buffer_lh(
    font_system: &mut FontSystem,
    text: &str,
    attrs: &Attrs,
    font_size: f32,
    line_height: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    let mut buf = buffer::create(font_system, font_size, line_height);
    buf.set_size(font_system, Some(bounds.0), Some(bounds.1));
    buf.set_rich_text(
        font_system,
        vec![(text, attrs.clone())],
        &Attrs::new(),
        SHAPING_ADVANCED,
        None,
    );
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer {
        buffer: buf,
        pos,
        bounds,
        zoom_visibility: ZoomVisibility::unbounded(),
    }
}
