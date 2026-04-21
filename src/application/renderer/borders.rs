//! Border-buffer creators + glyph-advance measurement + hex-color
//! parser. The renderer's flat-pass shapes (border rows, columns,
//! single-row spans) all flow through these helpers so cosmic-text
//! `Buffer::new` happens in one place per shape rather than being
//! inlined per call site.
//!
//! Every helper returns a [`MindMapTextBuffer`] with
//! [`ZoomVisibility::unbounded`] — the buffer always renders by
//! default. Callers that route scene-builder output through these
//! helpers (mindmap borders, line-mode connection glyphs, edge
//! labels) overwrite `zoom_visibility` on the returned buffer to
//! gate presence on camera zoom; overlay callers (edge handles,
//! selection rects, console, palette) leave it at the default so
//! they always render regardless of zoom.

use cosmic_text::{Attrs, FontSystem};

use baumhard::gfx_structs::zoom_visibility::ZoomVisibility;

use super::MindMapTextBuffer;

/// Measure the widest shaped advance across a set of glyph strings
/// at the given font size, via cosmic-text. Used by the color picker
/// to pick a cell-spacing unit that accommodates the actual shaped
/// width of sacred-script glyphs — Devanagari clusters, Tibetan
/// stacks, and especially Egyptian hieroglyphs shape meaningfully
/// wider than the Latin `font_size * 0.6` baseline.
///
/// Returns the max `glyph.w` (advance in pixels) seen across every
/// glyph string passed in. Falls back to `font_size * 0.6` if every
/// glyph somehow shapes to zero width (e.g., tofu + missing fallback).
pub fn measure_max_glyph_advance(
    font_system: &mut cosmic_text::FontSystem,
    glyphs: &[&str],
    font_size: f32,
) -> f32 {
    let mut buffer = cosmic_text::Buffer::new(
        font_system,
        cosmic_text::Metrics::new(font_size, font_size),
    );
    let attrs = Attrs::new();
    let mut max_w: f32 = 0.0;
    for g in glyphs {
        buffer.set_text(
            font_system,
            g,
            &attrs,
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                if glyph.w > max_w {
                    max_w = glyph.w;
                }
            }
        }
    }
    if max_w <= 0.0 {
        font_size * 0.6
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
    let mut buf = cosmic_text::Buffer::new(
        font_system,
        cosmic_text::Metrics::new(font_size, line_height),
    );
    buf.set_size(font_system, Some(bounds.0), Some(bounds.1));
    buf.set_rich_text(
        font_system,
        vec![(text, attrs.clone())],
        &Attrs::new(),
        cosmic_text::Shaping::Advanced,
        None,
    );
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer { buffer: buf, pos, bounds, zoom_visibility: ZoomVisibility::unbounded() }
}

/// Multi-span variant of [`create_border_buffer`] — hands cosmic-text
/// a sequence of `(text, attrs)` pairs in one buffer so adjacent
/// spans with different colors (e.g. accent-colored prompt glyph +
/// text-colored input) lay out as one line without the caller having
/// to position them separately.
pub(super) fn create_border_buffer_spans(
    font_system: &mut FontSystem,
    spans: &[(&str, Attrs)],
    font_size: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    let mut buf = cosmic_text::Buffer::new(
        font_system,
        cosmic_text::Metrics::new(font_size, font_size),
    );
    buf.set_size(font_system, Some(bounds.0), Some(bounds.1));
    let span_refs: Vec<(&str, Attrs)> =
        spans.iter().map(|(t, a)| (*t, a.clone())).collect();
    buf.set_rich_text(
        font_system,
        span_refs,
        &Attrs::new(),
        cosmic_text::Shaping::Advanced,
        None,
    );
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer { buffer: buf, pos, bounds, zoom_visibility: ZoomVisibility::unbounded() }
}

/// Like `create_border_buffer` but center-aligns the text within its
/// box via `cosmic_text::Align::Center`. Used for the color picker's
/// crosshair-arm glyphs and hue-ring glyphs: with sacred-script
/// glyphs varying significantly in shaped width (~5 px for Hebrew,
/// ~20 px for Egyptian hieroglyphs at base `font_size`), flush-left
/// positioning would produce a visibly drifting cross and a ring
/// thrown out of round. Center alignment pins each glyph's visual
/// center to the middle of its box, independent of advance width.
pub(super) fn create_centered_cell_buffer(
    font_system: &mut FontSystem,
    text: &str,
    attrs: &Attrs,
    font_size: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    let mut buf = cosmic_text::Buffer::new(
        font_system,
        cosmic_text::Metrics::new(font_size, font_size),
    );
    buf.set_size(font_system, Some(bounds.0), Some(bounds.1));
    buf.set_rich_text(
        font_system,
        vec![(text, attrs.clone())],
        &Attrs::new(),
        cosmic_text::Shaping::Advanced,
        Some(cosmic_text::Align::Center),
    );
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer { buffer: buf, pos, bounds, zoom_visibility: ZoomVisibility::unbounded() }
}

pub(super) fn parse_hex_color(hex: &str) -> Option<cosmic_text::Color> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    Some(cosmic_text::Color::rgba(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
        255,
    ))
}

