//! Portal label drag math: given a cursor position in canvas
//! space, snap it to the nearest point on the owning node's
//! border and write the resulting `border_t` into the edge's
//! per-endpoint state. Used by the per-frame drain loop and
//! once more at release to commit the final position.
//!
//! Mirrors the shape of [`super::edge_drag`]: one free function,
//! no per-frame allocation, mutating through
//! [`MindMapDocument`] setters so undo / dirty bookkeeping is
//! consistent with every other edge edit. The drain path does
//! **not** push undo per frame — the event-loop commit
//! (mouse-up in `event_mouse_click.rs`) pushes one
//! `UndoAction::EditEdge` with the pre-drag snapshot, matching
//! how `DraggingEdgeHandle` handles commits.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;

use baumhard::mindmap::portal_geometry::nearest_border_t;

use crate::application::document::{EdgeRef, MindMapDocument};

/// Apply one frame of portal-label drag. Converts `cursor_canvas`
/// into a perimeter-parameter `t` on the owning node's border
/// and writes it to the per-endpoint state on the edge. Returns
/// `true` if the write changed the stored `border_t` (so the
/// caller knows whether the frame produced a visible change);
/// `false` otherwise, including the "edge disappeared mid-drag"
/// case (logged and skipped).
pub(in crate::application::app) fn apply_portal_label_drag(
    doc: &mut MindMapDocument,
    edge_ref: &EdgeRef,
    endpoint_node_id: &str,
    cursor_canvas: Vec2,
) -> bool {
    // Locate the owning node to project onto. Portal labels are
    // always attached to the node named by `endpoint_node_id`
    // (one of `edge.from_id` / `edge.to_id`), and that id reached
    // us via a click hit-test, so a missing node here means the
    // node was deleted between click and drag.
    let Some(node) = doc.mindmap.nodes.get(endpoint_node_id) else {
        log::warn!(
            "apply_portal_label_drag: endpoint node {endpoint_node_id} disappeared mid-drag"
        );
        return false;
    };
    let node_pos = Vec2::new(node.position.x as f32, node.position.y as f32);
    let node_size = Vec2::new(node.size.width as f32, node.size.height as f32);
    let t = nearest_border_t(node_pos, node_size, cursor_canvas);

    // Direct field write — bypassing `set_portal_label_border_t`
    // because that setter pushes an `EditEdge` per call. The
    // per-frame drain would flood the undo stack; we snapshot
    // once at drag start and push a single `EditEdge` at release.
    let Some(idx) = doc
        .mindmap
        .edges
        .iter()
        .position(|e| edge_ref.matches(e))
    else {
        return false;
    };
    let edge = &mut doc.mindmap.edges[idx];
    let slot = baumhard::mindmap::model::portal_endpoint_state_mut(edge, endpoint_node_id);
    let slot = match slot {
        Some(s) => s,
        None => return false,
    };
    let existing = slot.as_ref().and_then(|s| s.border_t);
    if existing.map_or(false, |prev| (prev - t).abs() < f32::EPSILON) {
        return false;
    }
    slot.get_or_insert_with(baumhard::mindmap::model::PortalEndpointState::default)
        .border_t = Some(t);
    doc.dirty = true;
    true
}
