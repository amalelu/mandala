// SPDX-License-Identifier: MPL-2.0

//! Selection-rectangle overlay buffers — the dashed box-drawing
//! frame the rubber-band select draws in canvas space. Every other
//! canvas visual (nodes, borders, connections, labels, portals,
//! handles) reaches the GPU through the Baumhard tree pipeline
//! (`tree_walker.rs` → `canvas_scene_elements`); this overlay is the
//! one transient rectangle that has no model behind it, so it
//! shapes its own buffers directly.

use baumhard::font::fonts;
use baumhard::font::metrics::monospace_advance;
use baumhard::font::{Attrs, Color, Metrics};
use glam::Vec2;

use super::borders::create_border_buffer;
use super::Renderer;

/// Alpha the rubber band's cyan is drawn at. Reduced from opaque so
/// the content the band sweeps over stays readable underneath it —
/// the one thing about this color that is genuinely the selection
/// rectangle's own, which is why it is named here and the RGB is
/// not.
const SELECTION_RECT_ALPHA: u8 = 200;

/// The rubber band's color: the shared selection cyan at
/// [`SELECTION_RECT_ALPHA`]. Derived rather than written out, which
/// is how this used to read `Color::rgba(0, 230, 255, 200)` against
/// a `#00E5FF` whose green channel is 229.
pub(crate) const SELECTION_RECT_COLOR: Color = baumhard::font::color::cosmic_color_from_color(
    baumhard::mindmap::SELECTION_HIGHLIGHT.with_alpha(SELECTION_RECT_ALPHA),
);

impl Renderer {
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

        let attrs = Attrs::new()
            .color(SELECTION_RECT_COLOR)
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
