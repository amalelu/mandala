// SPDX-License-Identifier: MPL-2.0

//! Edge label text, position-along-curve, and perpendicular offset.

use baumhard::mindmap::model::EdgeLabelConfig;
use baumhard::util::geometry::almost_equal;

use super::super::types::EdgeRef;
use super::super::MindMapDocument;
use super::closure_helpers::ensure_label_config_inline;

impl MindMapDocument {
    /// Set the label text on an edge. Passing `None` (or `Some("")`)
    /// clears the label. Returns `true` if the value actually changed.
    pub fn set_edge_label(&mut self, edge_ref: &EdgeRef, text: Option<String>) -> bool {
        // Normalize empty string to None so hit testing and rendering
        // only need to check one absence case.
        let new_val = match text {
            Some(s) if s.is_empty() => None,
            other => other,
        };
        self.mutate_edge(edge_ref, |edge, _canvas| {
            if edge.label == new_val {
                return false;
            }
            edge.label = new_val;
            true
        })
    }

    /// Set the label's tangential position along the connection path.
    /// `t` is clamped into `[0.0, 1.0]` — values outside that range
    /// are silently pulled back. Returns `true` if the clamped value
    /// actually differs from the current. Forks a fresh
    /// `EdgeLabelConfig` on the edge if one isn't already present
    /// (mirrors `ensure_glyph_connection_inline` on the body cascade).
    pub fn set_edge_label_position(&mut self, edge_ref: &EdgeRef, t: f32) -> bool {
        let clamped = t.clamp(0.0, 1.0);
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let current = EdgeLabelConfig::effective_position_t(edge.label_config.as_ref());
            if almost_equal(current, clamped) {
                return false;
            }
            ensure_label_config_inline(edge).position_t = Some(clamped);
            true
        })
    }

    /// Set (or clear, with `offset = None`) the label's
    /// perpendicular offset — the signed distance from the path
    /// point along `normal_at_t(position_t)`. Used by the label
    /// drag and the `label perpendicular=<f32>` console key.
    /// `None` returns the label to the on-path position.
    /// Rolls back an all-default `EdgeLabelConfig` on clear so
    /// unchanged selections leave no undo droppings.
    pub fn set_edge_label_perpendicular_offset(&mut self, edge_ref: &EdgeRef, offset: Option<f32>) -> bool {
        // Reject NaN / infinity at the boundary; the label
        // config stores only finite values.
        if let Some(v) = offset {
            if !v.is_finite() {
                return false;
            }
        }
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let current = edge.label_config.as_ref().and_then(|c| c.perpendicular_offset);
            if baumhard::util::geometry::option_almost_equal(current, offset) {
                return false;
            }
            match offset {
                Some(v) => {
                    ensure_label_config_inline(edge).perpendicular_offset = Some(v);
                }
                None => {
                    if let Some(cfg) = edge.label_config.as_mut() {
                        cfg.perpendicular_offset = None;
                        if cfg == &EdgeLabelConfig::default() {
                            edge.label_config = None;
                        }
                    }
                }
            }
            true
        })
    }
}
