//! Tree-to-cosmic-text walker. The hot-path function that turns a
//! Baumhard `Tree<GfxElement, GfxMutator>` into shaped text buffers
//! the renderer pipes to glyphon. Owned by the renderer module
//! (rather than baumhard) because the input shape requires
//! cosmic-text awareness — `attrs_list_from_regions` lives in
//! baumhard but the `Buffer::set_rich_text` call sites stay here.

use cosmic_text::{Attrs, Family, FontSystem};
use glam::Vec2;

use baumhard::font::fonts;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use baumhard::util::grapheme_chad;

use super::{MindMapTextBuffer, NodeBackgroundRect};

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
/// integer and only the mindmap closure stringifies it for its
/// `FxHashMap` key. Holds the provided `font_system` write guard
/// for the duration of the walk — keep the call site's own guard
/// scope tight.
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
        let element = node.get();
        let area = match element.glyph_area() {
            Some(a) => a,
            None => continue, // Void and GlyphModel nodes carry no text.
        };

        if let Some(color) = area.background_color {
            yield_background(NodeBackgroundRect {
                position: Vec2::new(area.position.x.0, area.position.y.0) + offset,
                size: Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0),
                color,
                shape_id: area.shape.shader_id(),
            });
        }

        if area.text.is_empty() {
            continue;
        }

        let scale = area.scale.0;
        let line_height = area.line_height.0;
        let bound_x = area.render_bounds.x.0;
        let bound_y = area.render_bounds.y.0;

        // Pre-compute font family names per region. The walker had a
        // long-standing bug where `region.font` was stored on the
        // GlyphArea but never threaded into the cosmic-text `Attrs`,
        // so SMP-range glyphs that needed an explicit face (Egyptian
        // hieroglyphs in the color-picker bottom arm in particular)
        // silently rendered as tofu — cosmic-text's default fallback
        // doesn't pick the Noto Sans Egyptian Hieroglyphs face.
        //
        // The family lookup borrows `font_system.db()` immutably,
        // while `set_rich_text` below needs `&mut font_system`. We
        // collect the family strings into an owned `Vec<Option<String>>`
        // here so the immutable borrow ends before the mutable one
        // begins, and the owned strings outlive each spans Vec that
        // borrows them via `Family::Name`. The same names are reused
        // across the main buffer and every halo copy.
        let family_names: Vec<Option<String>> = if area.regions.num_regions() == 0 {
            vec![None]
        } else {
            area.regions
                .all_regions()
                .iter()
                .map(|region| {
                    region.font.and_then(|f| {
                        fonts::COMPILED_FONT_ID_MAP.get(&f).and_then(|ids| {
                            font_system
                                .db()
                                .face(ids[0])
                                .map(|face| face.families[0].0.clone())
                        })
                    })
                })
                .collect()
        };

        let text = &area.text;
        let alignment = if area.align_center {
            Some(cosmic_text::Align::Center)
        } else {
            None
        };

        // Build a `Vec<(&str, Attrs)>` for shaping, with an optional
        // color override that recolors *every* span to the given
        // color (used by the halo loop below). `None` keeps each
        // region's own color. Per-region font pinning is preserved
        // either way, so a halo behind an Egyptian hieroglyph still
        // shapes through the Noto Egyptian Hieroglyphs face.
        let build_spans = |color_override: Option<cosmic_text::Color>| -> Vec<(&str, Attrs)> {
            if area.regions.num_regions() == 0 {
                let mut attrs = Attrs::new();
                if let Some(c) = color_override {
                    attrs = attrs.color(c);
                }
                attrs = attrs.metrics(cosmic_text::Metrics::new(scale, line_height));
                vec![(text.as_str(), attrs)]
            } else {
                area.regions
                    .all_regions()
                    .iter()
                    .enumerate()
                    .filter_map(|(i, region)| {
                        let start =
                            grapheme_chad::find_byte_index_of_char(text, region.range.start)
                                .unwrap_or(text.len());
                        let end = grapheme_chad::find_byte_index_of_char(text, region.range.end)
                            .unwrap_or(text.len());
                        if start >= end {
                            return None;
                        }
                        let slice = &text[start..end];
                        let mut attrs = Attrs::new();
                        let color = color_override.or_else(|| {
                            region.color.map(|rgba| {
                                let u8c = baumhard::util::color::convert_f32_to_u8(&rgba);
                                cosmic_text::Color::rgba(u8c[0], u8c[1], u8c[2], u8c[3])
                            })
                        });
                        if let Some(c) = color {
                            attrs = attrs.color(c);
                        }
                        // Pin the per-region font when the GlyphArea
                        // specified one. The family name is owned by
                        // `family_names`; `Family::Name` borrows it
                        // for the lifetime of `attrs`. Iterators have
                        // identical length by construction (both run
                        // over `area.regions.all_regions()`), so
                        // direct indexing is safe.
                        if let Some(family) = family_names[i].as_deref() {
                            attrs = attrs.family(Family::Name(family));
                        }
                        attrs = attrs.metrics(cosmic_text::Metrics::new(scale, line_height));
                        Some((slice, attrs))
                    })
                    .collect()
            }
        };

        // Helper to shape one buffer at an offset and yield it. The
        // wrap mode stays at cosmic-text's default `Wrap::WordOrGlyph`
        // — `Word` mode silently dropped supplementary-plane glyphs
        // (e.g. picker Egyptian hieroglyphs) whose shaped advance
        // exceeded the cell box.
        let mut shape_and_yield =
            |spans: Vec<(&str, Attrs)>, x_off: f32, y_off: f32, fs: &mut FontSystem| {
                let mut buffer = cosmic_text::Buffer::new(
                    fs,
                    cosmic_text::Metrics::new(scale, line_height),
                );
                buffer.set_size(fs, Some(bound_x), Some(bound_y));
                buffer.set_rich_text(
                    fs,
                    spans,
                    &Attrs::new(),
                    cosmic_text::Shaping::Advanced,
                    alignment,
                );
                buffer.shape_until_scroll(fs, false);
                let text_buffer = MindMapTextBuffer {
                    buffer,
                    pos: (
                        area.position.x.0 + x_off + offset.x,
                        area.position.y.0 + y_off + offset.y,
                    ),
                    bounds: (bound_x, bound_y),
                };
                yield_buffer(element.unique_id(), text_buffer);
            };

        // Halos first — DFS yield order means later buffers render on
        // top, so emitting halos before the main glyph puts them
        // visually behind. The stamp geometry is canonical in
        // baumhard (`OutlineStyle::offsets`) — we just recolor every
        // span to `outline.color` and shape one buffer per offset.
        if let Some(outline) = area.outline {
            if outline.px > 0.0 {
                let halo_color = cosmic_text::Color::rgba(
                    outline.color[0],
                    outline.color[1],
                    outline.color[2],
                    outline.color[3],
                );
                for (dx, dy) in outline.offsets() {
                    let halo_spans = build_spans(Some(halo_color));
                    shape_and_yield(halo_spans, dx, dy, font_system);
                }
            }
        }

        // Main glyph. Always emitted last so it sits on top of any
        // halos.
        let main_spans = build_spans(None);
        shape_and_yield(main_spans, 0.0, 0.0, font_system);
    }
}
