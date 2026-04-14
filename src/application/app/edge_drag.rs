//! Edge handle drag math: applies a full drag delta to the
//! document's edge model in place. Used by the per-frame mouse-move
//! handler in the event loop and once more at release to commit the
//! final position.

use glam::Vec2;

use crate::application::document::{EdgeRef, MindMapDocument};

/// Apply a full edge-handle drag to the document model in place —
/// writes the new control point / anchor into
/// `doc.mindmap.edges[idx]` based on the current cursor delta.
/// Called every frame during the drag and once more at release to
/// commit the final position. Mutates `handle` in place when a
/// `Midpoint` handle crosses over into a fresh control point so
/// subsequent frames take the `ControlPoint(0)` path.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn apply_edge_handle_drag(
    doc: &mut MindMapDocument,
    edge_ref: &EdgeRef,
    handle: baumhard::mindmap::scene_builder::EdgeHandleKind,
    start_handle_pos: Vec2,
    total_delta: Vec2,
) -> baumhard::mindmap::scene_builder::EdgeHandleKind {
    use baumhard::mindmap::model::ControlPoint;
    use baumhard::mindmap::scene_builder::EdgeHandleKind;

    let idx = match doc.edge_index(edge_ref) {
        Some(i) => i,
        None => return handle,
    };
    let (from_center, to_center) = {
        let edge = &doc.mindmap.edges[idx];
        let from_node = match doc.mindmap.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => return handle,
        };
        let to_node = match doc.mindmap.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => return handle,
        };
        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);
        (
            Vec2::new(from_pos.x + from_size.x * 0.5, from_pos.y + from_size.y * 0.5),
            Vec2::new(to_pos.x + to_size.x * 0.5, to_pos.y + to_size.y * 0.5),
        )
    };
    let new_handle_canvas = start_handle_pos + total_delta;

    let edge = &mut doc.mindmap.edges[idx];
    match handle {
        EdgeHandleKind::ControlPoint(i) => {
            let center = if i == 0 { from_center } else { to_center };
            let offset = new_handle_canvas - center;
            while edge.control_points.len() <= i {
                edge.control_points.push(ControlPoint { x: 0.0, y: 0.0 });
            }
            edge.control_points[i] = ControlPoint {
                x: offset.x as f64,
                y: offset.y as f64,
            };
            EdgeHandleKind::ControlPoint(i)
        }
        EdgeHandleKind::Midpoint => {
            // First drained frame of a "curve this line" gesture:
            // insert a single control point (quadratic Bezier,
            // offset from source node center) at the new cursor
            // position. Subsequent frames promote to ControlPoint(0).
            let offset = new_handle_canvas - from_center;
            edge.control_points.clear();
            edge.control_points.push(ControlPoint {
                x: offset.x as f64,
                y: offset.y as f64,
            });
            EdgeHandleKind::ControlPoint(0)
        }
        EdgeHandleKind::AnchorFrom => {
            // Pick the side of from_node whose midpoint is closest to
            // the new cursor position. Value in 1..=4 (top/right/
            // bottom/left) — never 0 (auto) during manual drag.
            let from_node = doc.mindmap.nodes.get(&edge.from_id).unwrap();
            let node_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
            let node_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
            edge.anchor_from = nearest_anchor_side(new_handle_canvas, node_pos, node_size);
            EdgeHandleKind::AnchorFrom
        }
        EdgeHandleKind::AnchorTo => {
            let to_node = doc.mindmap.nodes.get(&edge.to_id).unwrap();
            let node_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
            let node_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);
            edge.anchor_to = nearest_anchor_side(new_handle_canvas, node_pos, node_size);
            EdgeHandleKind::AnchorTo
        }
    }
}

/// Given a canvas-space position and a node's AABB, return the
/// anchor code (1=top, 2=right, 3=bottom, 4=left) of the edge
/// midpoint closest to the position. Used by the anchor-handle
/// drag to snap the anchor to whichever side the cursor is nearest.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn nearest_anchor_side(point: Vec2, node_pos: Vec2, node_size: Vec2) -> i32 {
    let half_w = node_size.x * 0.5;
    let half_h = node_size.y * 0.5;
    let top = Vec2::new(node_pos.x + half_w, node_pos.y);
    let right = Vec2::new(node_pos.x + node_size.x, node_pos.y + half_h);
    let bottom = Vec2::new(node_pos.x + half_w, node_pos.y + node_size.y);
    let left = Vec2::new(node_pos.x, node_pos.y + half_h);
    let candidates = [(1, top), (2, right), (3, bottom), (4, left)];
    candidates
        .iter()
        .min_by(|a, b| {
            let da = a.1.distance_squared(point);
            let db = b.1.distance_squared(point);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(code, _)| *code)
        .unwrap_or(0)
}
