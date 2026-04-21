//! Edge-label drag math: given a cursor position in canvas
//! space, project it onto the selected edge's connection path
//! and write the resulting `(position_t, perpendicular_offset)`
//! into the edge's `label_config`. Used by the per-frame drain
//! loop and once more at release to commit the final position.
//!
//! Mirrors the shape of [`super::portal_label_drag`]: one free
//! function, no per-frame allocation, direct-field writes so
//! the undo stack isn't flooded with per-frame `EditEdge`
//! entries. The event-loop commit (mouse-up in
//! `event_mouse_click.rs`) pushes one `UndoAction::EditEdge`
//! with the pre-drag snapshot.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;

use baumhard::mindmap::connection;
use baumhard::mindmap::model::EdgeLabelConfig;

use crate::application::document::{EdgeRef, MindMapDocument};

/// Apply one frame of edge-label drag. Projects `cursor_canvas`
/// onto the edge's path via
/// [`baumhard::mindmap::connection::closest_point_on_path`] and
/// writes the resulting `(position_t, perpendicular_offset)`
/// directly into the edge's `label_config` — forking a fresh
/// `EdgeLabelConfig` if the edge didn't carry one already.
/// Returns `true` if the frame produced a visible change;
/// `false` when nothing moved beyond float epsilon, the edge
/// disappeared between click and drag, or an endpoint node
/// vanished. Interactive-path safe — never panics (§9).
pub(in crate::application::app) fn apply_edge_label_drag(
    doc: &mut MindMapDocument,
    edge_ref: &EdgeRef,
    cursor_canvas: Vec2,
) -> bool {
    let Some(idx) = doc
        .mindmap
        .edges
        .iter()
        .position(|e| edge_ref.matches(e))
    else {
        return false;
    };

    // Re-project the cursor onto the live path. Endpoints may
    // have moved (e.g. another drag in the same frame), so we
    // rebuild the path fresh every frame.
    let path = {
        let edge = &doc.mindmap.edges[idx];
        let Some(from_node) = doc.mindmap.nodes.get(&edge.from_id) else {
            log::warn!(
                "apply_edge_label_drag: from-endpoint {} disappeared mid-drag",
                edge.from_id
            );
            return false;
        };
        let Some(to_node) = doc.mindmap.nodes.get(&edge.to_id) else {
            log::warn!(
                "apply_edge_label_drag: to-endpoint {} disappeared mid-drag",
                edge.to_id
            );
            return false;
        };
        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size =
            Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);
        connection::build_connection_path(
            from_pos,
            from_size,
            &edge.anchor_from,
            to_pos,
            to_size,
            &edge.anchor_to,
            &edge.control_points,
        )
    };
    let (t, perp) = connection::closest_point_on_path(&path, cursor_canvas);
    let t = t.clamp(0.0, 1.0);

    // Direct field write — bypassing the setters that push an
    // `EditEdge` per call. The drain frame would flood the undo
    // stack; we snapshot once at drag start and push a single
    // `EditEdge` at release (same discipline as the portal-
    // label and edge-handle drags).
    let edge = &mut doc.mindmap.edges[idx];
    let cfg = edge
        .label_config
        .get_or_insert_with(EdgeLabelConfig::default);
    let existing_t = cfg.position_t.unwrap_or(0.5);
    let existing_perp = cfg.perpendicular_offset.unwrap_or(0.0);
    let changed =
        (existing_t - t).abs() >= f32::EPSILON || (existing_perp - perp).abs() >= f32::EPSILON;
    if !changed {
        return false;
    }
    cfg.position_t = Some(t);
    cfg.perpendicular_offset = Some(perp);
    doc.dirty = true;
    true
}
