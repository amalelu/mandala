// SPDX-License-Identifier: MPL-2.0

//! Node resize-handle emission. Emits 8 handles (corners + edge
//! midpoints) on the selected node when its size is finite +
//! positive; zero handles otherwise. Nodes have no parent AABB
//! containment guard.

use std::collections::{HashMap, HashSet};

use glam::Vec2;

use crate::mindmap::model::MindMap;

use super::section_resize_handle::{
    resize_handle_positions, ResizeHandleSide, SECTION_RESIZE_HANDLE_FONT_SIZE_PT,
    SECTION_RESIZE_HANDLE_GLYPH,
};
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;

/// One resize-handle glyph emitted on top of a selected node.
/// Sibling of [`super::SectionResizeHandleElement`]; differs only in
/// the carried identity (`node_id` here, `node_id + section_idx` on
/// the section variant). The renderer treats `node_resize_handles`
/// as its own buffer family — small (≤ 8), only for the currently-
/// selected node.
pub struct NodeResizeHandleElement {
    /// Owning MindNode id.
    pub node_id: String,
    /// Which of the 8 handles this element represents.
    pub side: ResizeHandleSide,
    /// Canvas-space center of the handle.
    pub position: (f32, f32),
    /// Glyph string (single char).
    pub glyph: String,
    /// Color as `#RRGGBB` hex.
    pub color: String,
    /// Font size in points.
    pub font_size_pt: f32,
}

impl crate::mindmap::tree_builder::HandleVisual for NodeResizeHandleElement {
    fn position(&self) -> (f32, f32) {
        self.position
    }
    fn glyph(&self) -> &str {
        &self.glyph
    }
    fn color(&self) -> &str {
        &self.color
    }
    fn font_size_pt(&self) -> f32 {
        self.font_size_pt
    }
    fn channel(&self) -> usize {
        self.side.channel()
    }
}

/// Build the 8-handle set for a single selected node. Returns an
/// empty vector when `node_size` has any non-finite or non-positive
/// component — those nodes can't host a meaningful resize gesture
/// (the verifier flags the underlying state already).
pub fn build_node_resize_handles(
    node_id: &str,
    node_pos: Vec2,
    node_size: Vec2,
) -> Vec<NodeResizeHandleElement> {
    let Some(positions) = resize_handle_positions(node_pos, node_size) else {
        return Vec::new();
    };
    positions
        .into_iter()
        .map(|(side, position)| NodeResizeHandleElement {
            node_id: node_id.to_string(),
            side,
            position,
            glyph: SECTION_RESIZE_HANDLE_GLYPH.to_string(),
            color: SELECTION_HIGHLIGHT_HEX.to_string(),
            font_size_pt: SECTION_RESIZE_HANDLE_FONT_SIZE_PT,
        })
        .collect()
}

/// Resolve `selected_node` into the node's canvas-space
/// `(position, size)` and dispatch to
/// [`build_node_resize_handles`]. Returns `Vec::new()` when no
/// node is selected, the named node is missing or hidden by fold,
/// or the node's size has any non-finite / non-positive
/// component.
///
/// The selection gate lives here rather than at the call site so
/// every rebuild path (drag drain, console verb, mode change)
/// applies the same "only the selected node grows handles" rule.
///
/// # Costs
///
/// O(1) — one hash lookup plus, at most, the eight-element
/// allocation [`build_node_resize_handles`] returns.
pub fn build_selected_node_handles(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_node: Option<&str>,
    hidden_set: &HashSet<&str>,
) -> Vec<NodeResizeHandleElement> {
    let Some(node_id) = selected_node else {
        return Vec::new();
    };
    let Some(node) = map.nodes.get(node_id) else {
        return Vec::new();
    };
    if hidden_set.contains(node.id.as_str()) {
        return Vec::new();
    }
    let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
    let node_pos = node.pos_vec2();
    let canvas_pos = Vec2::new(node_pos.x + ox, node_pos.y + oy);
    build_node_resize_handles(node_id, canvas_pos, node.size_vec2())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Some`-sized nodes get exactly 8 handles, one per side, at
    /// the AABB's corners and edge midpoints.
    #[test]
    fn build_emits_eight_handles_at_corners_and_edge_mids() {
        let handles = build_node_resize_handles("0", Vec2::new(10.0, 20.0), Vec2::new(100.0, 40.0));
        assert_eq!(handles.len(), 8);
        let by_side: std::collections::HashMap<ResizeHandleSide, (f32, f32)> =
            handles.iter().map(|h| (h.side, h.position)).collect();
        assert_eq!(by_side[&ResizeHandleSide::NW], (10.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::NE], (110.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::SW], (10.0, 60.0));
        assert_eq!(by_side[&ResizeHandleSide::SE], (110.0, 60.0));
        assert_eq!(by_side[&ResizeHandleSide::N], (60.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::S], (60.0, 60.0));
        assert_eq!(by_side[&ResizeHandleSide::W], (10.0, 40.0));
        assert_eq!(by_side[&ResizeHandleSide::E], (110.0, 40.0));
    }

    /// Non-finite size → no handles.
    #[test]
    fn build_returns_empty_for_non_finite_size() {
        let handles = build_node_resize_handles("0", Vec2::ZERO, Vec2::new(f32::NAN, 10.0));
        assert!(handles.is_empty());
    }

    /// Non-positive size → no handles.
    #[test]
    fn build_returns_empty_for_non_positive_size() {
        let handles = build_node_resize_handles("0", Vec2::ZERO, Vec2::new(0.0, 10.0));
        assert!(handles.is_empty());
        let handles = build_node_resize_handles("0", Vec2::ZERO, Vec2::new(10.0, -5.0));
        assert!(handles.is_empty());
    }
}
