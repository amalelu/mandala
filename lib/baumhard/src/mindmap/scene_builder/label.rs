//! Connection-label emission. Runs as a post-pass over
//! `map.edges` — labels are at most one per edge and rebuilt each
//! frame at trivial cost (no cache). Two internal passes:
//!
//! 1. Emit a `ConnectionLabelElement` for every visible edge with a
//!    non-empty committed `label`. If `label_edit_override` targets
//!    this edge, substitute the live buffer (+ caret) for the
//!    committed text so inline editing shows up on the next frame.
//!
//! 2. If `label_edit_override` targets an edge whose committed
//!    label is empty / `None` (so the first pass skipped it),
//!    synthesize a label element anyway. Without this, the caret
//!    for the very first character of a fresh label wouldn't be
//!    visible.

use std::collections::HashMap;

use glam::Vec2;

use crate::mindmap::connection;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap};
use crate::mindmap::scene_cache::EdgeKey;
use crate::util::color::resolve_var;

use super::{ConnectionLabelElement, EdgeColorPreview};

/// Emit connection labels for the given map + overrides. Returns
/// the two-pass union: committed labels first, then (optionally) a
/// synthesized label for the inline-edited edge if its committed
/// label was empty.
pub fn build_label_elements(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    label_edit_override: Option<(&EdgeKey, &str)>,
    edge_color_preview: Option<EdgeColorPreview<'_>>,
    camera_zoom: f32,
) -> Vec<ConnectionLabelElement> {
    let vars = &map.canvas.theme_variables;
    let mut connection_label_elements: Vec<ConnectionLabelElement> = Vec::new();
    let mut label_override_emitted = false;

    for edge in &map.edges {
        if !edge.visible {
            continue;
        }
        let from_node = match map.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => continue,
        };
        let to_node = match map.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => continue,
        };
        if map.is_hidden_by_fold(from_node) || map.is_hidden_by_fold(to_node) {
            continue;
        }

        let edge_key = EdgeKey::from_edge(edge);
        let is_edited = label_edit_override.map_or(false, |(k, _)| *k == edge_key);

        // The label text to render: either the inline edit buffer
        // (plus caret) for the currently-edited edge, or the
        // committed label. Committed-empty non-edited edges skip
        // emission entirely.
        let rendered_label: String = if is_edited {
            let (_, buf) = label_edit_override.unwrap();
            format!("{buf}\u{258C}")
        } else {
            match edge.label.as_deref() {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            }
        };

        let (fox, foy) = offsets.get(&from_node.id).copied().unwrap_or((0.0, 0.0));
        let (tox, toy) = offsets.get(&to_node.id).copied().unwrap_or((0.0, 0.0));
        let from_pos = Vec2::new(
            from_node.position.x as f32 + fox,
            from_node.position.y as f32 + foy,
        );
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(
            to_node.position.x as f32 + tox,
            to_node.position.y as f32 + toy,
        );
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);

        let path = connection::build_connection_path(
            from_pos,
            from_size,
            edge.anchor_from,
            to_pos,
            to_size,
            edge.anchor_to,
            &edge.control_points,
        );
        let t = edge.label_position_t.unwrap_or(0.5);
        let anchor = connection::point_at_t(&path, t);

        let config = GlyphConnectionConfig::resolved_for(edge, &map.canvas);
        let base_font_size = config.effective_font_size_pt(camera_zoom);
        // Labels render slightly larger than the body glyphs so they
        // read as a distinct element on top of the connection path.
        let font_size_pt = base_font_size * 1.1;
        // Color picker preview: substitute the preview hex for this
        // edge's label color if the preview targets it. Applied
        // before `resolve_var` so `var(--accent)`-style preview values
        // still theme-resolve correctly.
        let raw_color: &str = edge_color_preview
            .and_then(|p| {
                if *p.edge_key == edge_key {
                    Some(p.color)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| config.color.as_deref().unwrap_or(edge.color.as_str()));
        let color = resolve_var(raw_color, vars).to_string();

        // Loose AABB sized from the glyph-count approximation
        // (`font_size * 0.6` per glyph — same constant the connection
        // body sampler uses). Height is one font-size plus a small
        // vertical margin.
        let char_count = rendered_label.chars().count() as f32;
        let bounds_w = (char_count * font_size_pt * 0.6).max(font_size_pt);
        let bounds_h = font_size_pt * 1.3;
        // Center the AABB on the path anchor.
        let top_left = (anchor.x - bounds_w * 0.5, anchor.y - bounds_h * 0.5);

        if is_edited {
            label_override_emitted = true;
        }

        connection_label_elements.push(ConnectionLabelElement {
            edge_key,
            text: rendered_label,
            position: top_left,
            bounds: (bounds_w, bounds_h),
            color,
            font: config.font.clone(),
            font_size_pt,
        });
    }

    // Synthesized-label pass: if `label_edit_override` targets an
    // edge whose committed label was empty / None (so the first
    // pass skipped it), emit a label element anyway so the caret
    // for the very first character is visible. Fills the gap in
    // the previous renderer-side override path, whose
    // "belt and suspenders" branch was a dead no-op for this case.
    if let Some((target_key, buffer)) = label_edit_override {
        if !label_override_emitted {
            if let Some(edge) = map
                .edges
                .iter()
                .find(|e| e.visible && EdgeKey::from_edge(e) == *target_key)
            {
                if let (Some(from_node), Some(to_node)) = (
                    map.nodes.get(&edge.from_id),
                    map.nodes.get(&edge.to_id),
                ) {
                    if !map.is_hidden_by_fold(from_node) && !map.is_hidden_by_fold(to_node) {
                        let (fox, foy) =
                            offsets.get(&from_node.id).copied().unwrap_or((0.0, 0.0));
                        let (tox, toy) = offsets.get(&to_node.id).copied().unwrap_or((0.0, 0.0));
                        let from_pos = Vec2::new(
                            from_node.position.x as f32 + fox,
                            from_node.position.y as f32 + foy,
                        );
                        let from_size = Vec2::new(
                            from_node.size.width as f32,
                            from_node.size.height as f32,
                        );
                        let to_pos = Vec2::new(
                            to_node.position.x as f32 + tox,
                            to_node.position.y as f32 + toy,
                        );
                        let to_size = Vec2::new(
                            to_node.size.width as f32,
                            to_node.size.height as f32,
                        );
                        let path = connection::build_connection_path(
                            from_pos,
                            from_size,
                            edge.anchor_from,
                            to_pos,
                            to_size,
                            edge.anchor_to,
                            &edge.control_points,
                        );
                        let t = edge.label_position_t.unwrap_or(0.5);
                        let anchor = connection::point_at_t(&path, t);
                        let config = GlyphConnectionConfig::resolved_for(edge, &map.canvas);
                        let base_font_size = config.effective_font_size_pt(camera_zoom);
                        let font_size_pt = base_font_size * 1.1;
                        // Synthesized path is for an edge being edited
                        // with an empty committed label — if the color
                        // picker is also previewing this edge,
                        // substitute the preview value.
                        let raw_color: &str = edge_color_preview
                            .and_then(|p| {
                                if p.edge_key == target_key {
                                    Some(p.color)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| {
                                config.color.as_deref().unwrap_or(edge.color.as_str())
                            });
                        let color = resolve_var(raw_color, vars).to_string();
                        let rendered = format!("{buffer}\u{258C}");
                        let char_count = rendered.chars().count() as f32;
                        let bounds_w = (char_count * font_size_pt * 0.6).max(font_size_pt);
                        let bounds_h = font_size_pt * 1.3;
                        let top_left =
                            (anchor.x - bounds_w * 0.5, anchor.y - bounds_h * 0.5);
                        connection_label_elements.push(ConnectionLabelElement {
                            edge_key: target_key.clone(),
                            text: rendered,
                            position: top_left,
                            bounds: (bounds_w, bounds_h),
                            color,
                            font: config.font.clone(),
                            font_size_pt,
                        });
                    }
                }
            }
        }
    }

    connection_label_elements
}
