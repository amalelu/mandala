// SPDX-License-Identifier: MPL-2.0

//! Hot-path walker turning a Baumhard `Tree<GfxElement, GfxMutator>`
//! into shaped text buffers for glyphon. Lives renderer-side because
//! `Buffer::new → set_size → set_rich_text → shape_until_scroll` is
//! renderer territory. Per CODE_CONVENTIONS §1, styled-region →
//! cosmic-text spans go through `baumhard::font::attrs`.

use glam::Vec2;

use baumhard::font::attrs::{rich_text_spans_into, RegionFamilies};
use baumhard::font::{buffer, Align, Attrs, Color, FontSystem, SHAPING_ADVANCED};
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;

use super::{MindMapTextBuffer, NodeBackgroundRect};

/// Extract the background-fill rect for a single `GlyphArea`, or
/// `None` if the area carries no `background_color`. Single source
/// for both the full-walker text path
/// ([`shape_one_element_into_buffers`]) and the drag-fast-path
/// rebuild ([`super::Renderer::rebuild_node_backgrounds_from_tree`])
/// — they used to inline this padding-and-rect math twice and the
/// audit flagged the divergence as a bug magnet.
///
/// `offset` is added to the rect's position; pass `Vec2::ZERO` for
/// canvas-space callers.
pub(super) fn extract_background_rect(
    element: &GfxElement,
    area: &baumhard::gfx_structs::area::GlyphArea,
    offset: Vec2,
) -> Option<NodeBackgroundRect> {
    let color = area.background_color?;
    // Inflate the fill rect outward by `background_padding` —
    // per-edge values so framed nodes whose eight border runs sit at
    // different visible-stroke offsets get an asymmetric fill that
    // matches each side. The `is_zero` fast-path skips the four-add
    // arithmetic for unframed nodes (the common case);
    // `EdgePadding::ZERO` means the fill coincides with the text
    // rect, the historical behavior.
    let pad = area.background_padding;
    let pos = Vec2::new(area.position.x.0, area.position.y.0);
    let size = Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0);
    let (rect_pos, rect_size) = if pad.is_zero() {
        (pos, size)
    } else {
        (
            Vec2::new(pos.x - pad.left(), pos.y - pad.top()),
            Vec2::new(
                size.x + pad.left() + pad.right(),
                size.y + pad.top() + pad.bottom(),
            ),
        )
    };
    Some(NodeBackgroundRect {
        position: rect_pos + offset,
        size: rect_size,
        color,
        shape_id: area.shape.shader_id(),
        zoom_visibility: area.zoom_visibility,
        unique_id: element.unique_id(),
    })
}

/// Shared tree → cosmic-text buffer walker.
///
/// Iterates every `GlyphArea` descendant of `tree`, shapes a
/// `cosmic_text::Buffer` for each one, and hands the result to
/// `yield_buffer` together with the element's `unique_id` (raw
/// `usize`, not stringified — keying is the caller's choice).
/// Background fills (if any) are forwarded to `yield_background`
/// before the buffer is built so rects attached to text-empty
/// areas still land.
///
/// `offset` is added to every `position` — callers pass
/// `Vec2::ZERO` whenever the tree's areas are already in the
/// destination coordinate space (e.g. the mindmap, whose nodes
/// hold canvas-space positions); pass the registered tree offset
/// for scene trees that lay out in their own local frame.
///
/// # Costs
///
/// O(descendants). One `cosmic_text::Buffer` allocated per
/// non-empty-text area; background rect yields are trivial. No
/// per-area `String` allocation — the `unique_id` flows as a raw
/// integer all the way to the buffer map's key. Holds the provided
/// `font_system` write guard for the duration of the walk — keep
/// the call site's own guard scope tight.
pub(super) fn walk_tree_into_buffers(
    tree: &Tree<GfxElement, GfxMutator>,
    offset: Vec2,
    font_system: &mut FontSystem,
    mut yield_buffer: impl FnMut(usize, MindMapTextBuffer),
    mut yield_background: impl FnMut(NodeBackgroundRect),
) {
    for descendant_id in tree.root().descendants(&tree.arena) {
        let node = match tree.arena.get(descendant_id) {
            Some(n) => n,
            None => continue,
        };
        shape_one_element_into_buffers(
            node.get(),
            offset,
            font_system,
            &mut yield_buffer,
            &mut yield_background,
        );
    }
}

/// Shape a single `GfxElement` into a (background, buffer-set)
/// pair via the same per-element work the full walker does.
/// Extracted so the keyed-reshape API
/// (`Renderer::reshape_buffer_for`) can re-shape one element
/// without walking the whole tree on every keystroke. `Void` /
/// `GlyphModel` elements (no text-bearing payload) yield
/// nothing — same fast-skip the full walker uses.
pub(super) fn shape_one_element_into_buffers(
    element: &GfxElement,
    offset: Vec2,
    font_system: &mut FontSystem,
    yield_buffer: &mut dyn FnMut(usize, MindMapTextBuffer),
    yield_background: &mut dyn FnMut(NodeBackgroundRect),
) {
    let area = match element.glyph_area() {
        Some(a) => a,
        None => return, // Void and GlyphModel nodes carry no text.
    };

    if let Some(rect) = extract_background_rect(element, area, offset) {
        yield_background(rect);
    }

    if area.text.is_empty() {
        return;
    }

    let scale = area.scale.0;
    let line_height = area.line_height.0;
    let bound_x = area.render_bounds.x.0;
    let bound_y = area.render_bounds.y.0;

    // Pre-resolve every region's family-name string and byte range
    // once; reuse the result across the main glyph + every halo
    // stamp so neither the `font_system.db().face(...)` lookups nor
    // the grapheme walk that turns cluster ranges into byte ranges
    // re-runs per stamp. Lives in baumhard so the styled-region →
    // cosmic-text bridge has a single owner.
    let families = RegionFamilies::resolve(&area.text, &area.regions, font_system);
    // Declared after `families` so it drops first: the spans borrow
    // out of it.
    let mut spans: Vec<(&str, Attrs)> = Vec::new();

    let alignment = if area.align_center {
        Some(Align::Center)
    } else {
        None
    };

    // Helper to shape one buffer at an offset and yield it. The
    // wrap mode stays at cosmic-text's default `Wrap::WordOrGlyph`
    // — `Word` mode silently dropped supplementary-plane glyphs
    // (e.g. picker Egyptian hieroglyphs) whose shaped advance
    // exceeded the cell box.
    let mut shape_and_yield = |spans: &[(&str, Attrs)], x_off: f32, y_off: f32, fs: &mut FontSystem| {
        let mut buffer = buffer::create(fs, scale, line_height);
        buffer.set_size(Some(bound_x), Some(bound_y));
        buffer.set_rich_text(spans.iter().cloned(), &Attrs::new(), SHAPING_ADVANCED, alignment);
        buffer.shape_until_scroll(fs, false);
        let text_buffer = MindMapTextBuffer {
            buffer,
            pos: (
                area.position.x.0 + x_off + offset.x,
                area.position.y.0 + y_off + offset.y,
            ),
            emission_offset: (x_off, y_off),
            bounds: (bound_x, bound_y),
            zoom_visibility: area.zoom_visibility,
        };
        yield_buffer(element.unique_id(), text_buffer);
    };

    // Halos first — DFS yield order means later buffers render on
    // top, so emitting halos before the main glyph puts them
    // visually behind. The stamp geometry is canonical in
    // baumhard (`OutlineStyle::offsets`); the per-region attrs
    // construction is canonical in
    // `baumhard::font::attrs::rich_text_spans_into`. Every stamp
    // shapes the *same* span list — only the buffer position
    // differs — so the list is built once outside the loop rather
    // than once per offset.
    if let Some(outline) = area.outline {
        if outline.px > 0.0 {
            let halo_color = Color::rgba(
                outline.color[0],
                outline.color[1],
                outline.color[2],
                outline.color[3],
            );
            rich_text_spans_into(&families, scale, line_height, Some(halo_color), &mut spans);
            for (dx, dy) in outline.offsets() {
                shape_and_yield(&spans, dx, dy, font_system);
            }
        }
    }

    // Main glyph. Always emitted last so it sits on top of any
    // halos. Refills the same span buffer, which by now already
    // holds the capacity the halo pass grew it to.
    rich_text_spans_into(&families, scale, line_height, None, &mut spans);
    shape_and_yield(&spans, 0.0, 0.0, font_system);
}
