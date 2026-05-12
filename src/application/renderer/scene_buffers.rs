// SPDX-License-Identifier: MPL-2.0

//! Flat-scene buffer builders — `rebuild_*_buffers*` for borders,
//! connections, edge handles, connection labels, plus the selection
//! overlay. Per CODE_CONVENTIONS §1, styled-region → cosmic-text
//! spans go through `baumhard::font::attrs`.

use baumhard::font::fonts;
use baumhard::font::{buffer, Attrs, Color, Metrics, SHAPING_ADVANCED};
use baumhard::mindmap::scene_builder::BorderElement;
use glam::Vec2;

use super::borders::create_border_buffer;
use super::{MindMapTextBuffer, Renderer};
use baumhard::font::attrs::{rich_text_spans_from_regions, RegionFamilies};
use baumhard::font::hex::hex_to_cosmic_color;
use baumhard::font::metrics::monospace_advance;
use baumhard::mindmap::border::build_border_regions;
use baumhard::util::color::hex_to_rgba_safe;

impl Renderer {
    /// Full border rebuild — wipes the cache and shapes every element
    /// from scratch through baumhard's styled-region → cosmic-text bridge
    /// (the same `(text, ColorFontRegions) → Vec<(&str, Attrs)>` path the
    /// tree walker uses; see CODE_CONVENTIONS §1).
    pub fn rebuild_border_buffers(&mut self, border_elements: &[BorderElement]) {
        // Eviction-by-clear: every input element is reshaped fresh,
        // so wiping the cache here is sufficient. A future keyed /
        // incremental fast path that reuses cached buffers must
        // remove this `clear()` AND reintroduce a `seen`-set
        // `retain(|k, _| seen.contains(k))` at the end of the loop —
        // the two halves are complementary.
        self.border_buffers.clear();
        let mut font_system = fonts::acquire_font_system_write("rebuild_border_buffers");

        for elem in border_elements {
            let font_size = elem.border_style.font_size_pt;
            let specs = baumhard::mindmap::border::border_run_specs(
                &elem.border_style,
                elem.node_position,
                elem.node_size,
            );

            let fallback_rgba = hex_to_rgba_safe(&elem.border_style.color, [1.0, 1.0, 1.0, 1.0]);

            let zv = elem.zoom_visibility;
            let cycle = elem.palette_cycle.as_slice();

            // Per-side dance: build `ColorFontRegions` via
            // `build_border_regions` (the same helper the tree
            // builder uses, so both pipelines paint identical
            // colours per cluster), resolve family pins through
            // `RegionFamilies::resolve`, and bridge to spans via
            // `rich_text_spans_from_regions`. When `cycle` is
            // empty `build_border_regions` emits a single uniform
            // region, so the no-palette and palette-cycling paths
            // collapse to one shape here.
            let mut shape_spec = |spec: &baumhard::mindmap::border::BorderRunSpec| -> MindMapTextBuffer {
                let regions =
                    build_border_regions(spec.cluster_count, cycle, fallback_rgba, spec.palette_offset);
                let families = RegionFamilies::resolve(&regions, &mut font_system);
                // Plan revision 4: per-spec line_height. Vertical
                // fill rails set this to the measured ink-height
                // of the fill glyph so consecutive cluster rows
                // touch (no inter-row gap from cosmic-text's
                // default line_height = font_size). Horizontal
                // rails + corners default to line_height_pt =
                // font_size_pt (the existing behaviour).
                let line_h = spec.line_height_pt.max(1.0);
                let spans = rich_text_spans_from_regions(&spec.text, &families, font_size, line_h, None);
                let mut buf = buffer::create(&mut font_system, font_size, line_h);
                buf.set_size(&mut font_system, Some(spec.bounds.0), Some(spec.bounds.1));
                buf.set_rich_text(&mut font_system, spans, &Attrs::new(), SHAPING_ADVANCED, None);
                buf.shape_until_scroll(&mut font_system, false);
                MindMapTextBuffer {
                    buffer: buf,
                    pos: spec.position,
                    bounds: spec.bounds,
                    zoom_visibility: zv,
                }
            };

            let entry: Vec<MindMapTextBuffer> = specs.iter().map(&mut shape_spec).collect();
            self.border_buffers.insert(elem.node_id.clone(), entry);
        }
    }

    /// Rebuild the edge grab-handle overlay buffers. Called after every
    /// scene build — the handles are bounded (≤ 5 per selected edge)
    /// and always rebuilt from scratch, so no keyed cache is used.
    pub fn rebuild_edge_handle_buffers(
        &mut self,
        handles: &[baumhard::mindmap::scene_builder::EdgeHandleElement],
    ) {
        self.edge_handle_buffers.clear();
        if handles.is_empty() {
            return;
        }
        let mut font_system = fonts::acquire_font_system_write("rebuild_edge_handle_buffers");
        for handle in handles {
            let cosmic_color = hex_to_cosmic_color(&handle.color).unwrap_or(Color::rgba(0, 229, 255, 255));
            let attrs = Attrs::new()
                .color(cosmic_color)
                .metrics(Metrics::new(handle.font_size_pt, handle.font_size_pt));

            let half_w = handle.font_size_pt * 0.3;
            let half_h = handle.font_size_pt * 0.5;
            let pos = (handle.position.0 - half_w, handle.position.1 - half_h);
            let bounds = (handle.font_size_pt, handle.font_size_pt);

            self.edge_handle_buffers.push(create_border_buffer(
                &mut font_system,
                &handle.glyph,
                &attrs,
                handle.font_size_pt,
                pos,
                bounds,
            ));
        }
    }

    /// Rebuild the per-edge label buffers from a freshly computed
    /// scene. Labels are ≤ 1 per edge and rebuilt every scene build
    /// — cheap enough that no incremental-reuse cache is warranted.
    pub fn rebuild_connection_label_buffers(
        &mut self,
        label_elements: &[baumhard::mindmap::scene_builder::ConnectionLabelElement],
    ) {
        self.connection_label_buffers.clear();
        self.connection_label_hitboxes.clear();
        if label_elements.is_empty() {
            return;
        }
        let mut font_system = fonts::acquire_font_system_write("rebuild_connection_label_buffers");

        for elem in label_elements {
            let cosmic_color = hex_to_cosmic_color(&elem.color).unwrap_or(Color::rgba(235, 235, 235, 255));
            let attrs = Attrs::new()
                .color(cosmic_color)
                .metrics(Metrics::new(elem.font_size_pt, elem.font_size_pt));

            let mut buffer = create_border_buffer(
                &mut font_system,
                &elem.text,
                &attrs,
                elem.font_size_pt,
                elem.position,
                elem.bounds,
            );
            buffer.zoom_visibility = elem.zoom_visibility;
            self.connection_label_buffers
                .insert(elem.edge_key.clone(), buffer);

            let min = Vec2::new(elem.position.0, elem.position.1);
            let max = Vec2::new(elem.position.0 + elem.bounds.0, elem.position.1 + elem.bounds.1);
            self.connection_label_hitboxes
                .insert(elem.edge_key.clone(), (min, max));
        }
    }

    /// Build overlay buffers for a selection rectangle using dashed box-drawing glyphs.
    /// Coordinates are in canvas space.
    ///
    /// Per-tick fast path: when `(char_count, row_count)` round to
    /// the same cells as the previous call, the four shaped buffers
    /// in `overlay_buffers` are reused — only their positions are
    /// updated. The drag hot path commonly drifts under 1 char per
    /// tick, so this skips 4 `cosmic_text::Buffer::set_rich_text`
    /// shapings per cursor-move event.
    pub fn rebuild_selection_rect_overlay(&mut self, min: Vec2, max: Vec2) {
        let font_size: f32 = 14.0;
        let approx_char_width = monospace_advance(font_size);

        let w = max.x - min.x;
        let h = max.y - min.y;
        let h_width = w + approx_char_width * 2.0;
        let v_width = approx_char_width * 2.0;
        let char_count = (w / approx_char_width).max(1.0) as usize;
        let row_count = (h / font_size).max(1.0) as usize;

        let positions = [
            (min.x - approx_char_width, min.y - font_size), // top
            (min.x - approx_char_width, max.y),             // bottom
            (min.x - approx_char_width, min.y),             // left
            (max.x, min.y),                                 // right
        ];
        let bounds = [
            (h_width, font_size * 1.5),
            (h_width, font_size * 1.5),
            (v_width, h),
            (v_width, h),
        ];

        if self.selection_rect_shape_cache == Some((char_count, row_count)) && self.overlay_buffers.len() == 4
        {
            for (i, tb) in self.overlay_buffers.iter_mut().enumerate() {
                tb.pos = positions[i];
                tb.bounds = bounds[i];
            }
            return;
        }

        self.overlay_buffers.clear();
        let mut font_system = fonts::acquire_font_system_write("rebuild_selection_rect_overlay");

        let rect_color = Color::rgba(0, 230, 255, 200);
        let attrs = Attrs::new()
            .color(rect_color)
            .metrics(Metrics::new(font_size, font_size));

        let top_text = format!("\u{256D}{}\u{256E}", "\u{2504}".repeat(char_count));
        let bottom_text = format!("\u{2570}{}\u{256F}", "\u{2504}".repeat(char_count));
        let side_text: String = std::iter::repeat_n("\u{2506}\n", row_count).collect();

        for (text, pos, bound) in [
            (top_text.as_str(), positions[0], bounds[0]),
            (bottom_text.as_str(), positions[1], bounds[1]),
            (side_text.as_str(), positions[2], bounds[2]),
            (side_text.as_str(), positions[3], bounds[3]),
        ] {
            self.overlay_buffers.push(create_border_buffer(
                &mut font_system,
                text,
                &attrs,
                font_size,
                pos,
                bound,
            ));
        }
        self.selection_rect_shape_cache = Some((char_count, row_count));
    }

    /// Clear all overlay buffers (e.g., after selection rect is finished).
    pub fn clear_overlay_buffers(&mut self) {
        self.overlay_buffers.clear();
        self.selection_rect_shape_cache = None;
    }
}
