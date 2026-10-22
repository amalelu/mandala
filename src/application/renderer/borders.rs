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

use baumhard::font::metric_cache::glyph_advance_with;
use baumhard::font::metrics::monospace_advance;
use baumhard::font::{buffer, Attrs, FontSystem, SHAPING_ADVANCED};
use baumhard::gfx_structs::zoom_visibility::ZoomVisibility;

use super::MindMapTextBuffer;

/// Widest shaped advance across `glyphs` at `font_size`, via
/// cosmic-text. Falls back to `monospace_advance(font_size)` if
/// every glyph shapes to zero (tofu + missing fallback).
///
/// Each cluster is measured through
/// [`baumhard::font::metric_cache::glyph_advance_with`], the crate's
/// one `(face, size, cluster) → advance` cache, rather than through
/// a scratch `Buffer` per call. The console rebuild path asks for
/// the same two box-drawing clusters at the same size on every
/// keystroke; with the cache that pass shapes them once for the
/// process instead of once per rebuild. The `_with` variant is
/// mandatory here — every caller already holds the `FONT_SYSTEM`
/// write guard, and a nested acquire is a self-deadlock (§B5).
///
/// The face pin is `None`, matching the bare `Attrs::new()` this
/// used to shape with: cosmic-text's default fallback face.
///
/// One semantic correction rides along. Each entry of `glyphs` is a
/// cluster, and a cluster can lay out as more than one glyph (a
/// Devanagari base plus its matra, say). The scratch-buffer version
/// took the widest *layout glyph* inside the cluster, which
/// under-measures such a cluster by everything after its first
/// glyph; the cache returns the cluster's summed advance, which is
/// the width it actually occupies. Single-glyph clusters — the
/// console's two box-drawing characters, and every entry of the
/// picker's tables that shapes to one glyph — are unaffected,
/// because for them the sum and the max are the same number. The
/// `max` in the name is the max *across* `glyphs`, which is
/// unchanged.
pub fn measure_max_glyph_advance(font_system: &mut FontSystem, glyphs: &[&str], font_size: f32) -> f32 {
    let mut max_w: f32 = 0.0;
    for g in glyphs {
        let w = glyph_advance_with(font_system, None, font_size, g);
        if w > max_w {
            max_w = w;
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
    buf.set_size(Some(bounds.0), Some(bounds.1));
    buf.set_rich_text(vec![(text, attrs.clone())], &Attrs::new(), SHAPING_ADVANCED, None);
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer {
        buffer: buf,
        pos,
        // Border buffers are authored directly at their final
        // position rather than stamped around a `GlyphArea` anchor,
        // and none of them reaches `patch_drag_positions`.
        emission_offset: (0.0, 0.0),
        bounds,
        zoom_visibility: ZoomVisibility::unbounded(),
    }
}
