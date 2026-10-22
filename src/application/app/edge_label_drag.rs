// SPDX-License-Identifier: MPL-2.0

//! Edge-label drag math: project a canvas-space cursor onto the
//! selected edge's path and write `(position_t,
//! perpendicular_offset)` into the edge's `label_config`. Per-frame;
//! commit lands a single `UndoAction::EditEdge` at release.

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
    let Some(idx) = doc.mindmap.edges.iter().position(|e| edge_ref.matches(e)) else {
        return false;
    };

    // Re-project the cursor onto the live path. Endpoints may
    // have moved (e.g. another drag in the same frame), so we
    // rebuild the path fresh every frame.
    let path = {
        let edge = &doc.mindmap.edges[idx];
        // `debug!`, not `warn!`, per CODE_CONVENTIONS §9: returning
        // `false` skips the move but does not end the drag, so the
        // throttled interaction calls straight back in on the next
        // cursor move. At `warn!` a single delete-mid-drag would
        // repeat one line per mouse-move until mouse-up.
        let Some(from_node) = doc.mindmap.nodes.get(&edge.from_id) else {
            log::debug!(
                "edge label drag: from-endpoint {} disappeared mid-drag",
                edge.from_id
            );
            return false;
        };
        let Some(to_node) = doc.mindmap.nodes.get(&edge.to_id) else {
            log::debug!("edge label drag: to-endpoint {} disappeared mid-drag", edge.to_id);
            return false;
        };
        let from_pos = from_node.pos_vec2();
        let from_size = from_node.size_vec2();
        let to_pos = to_node.pos_vec2();
        let to_size = to_node.size_vec2();
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
    let cfg = edge.label_config.get_or_insert_with(EdgeLabelConfig::default);
    use baumhard::util::geometry::pretty_inequal;
    let existing_t = cfg.position_t.unwrap_or(0.5);
    let existing_perp = cfg.perpendicular_offset.unwrap_or(0.0);
    let changed = pretty_inequal(existing_t, t) || pretty_inequal(existing_perp, perp);
    if !changed {
        return false;
    }
    cfg.position_t = Some(t);
    cfg.perpendicular_offset = Some(perp);
    doc.dirty = true;
    true
}
