//! Edge-handle emission for the currently-selected edge. One
//! function, called at most once per scene build (single-edge
//! selection). Mirrors `tree_builder/edge_handle.rs` — same role,
//! different crate surface (here we emit `EdgeHandleElement`s that
//! the renderer later projects into the `EdgeHandles` canvas
//! tree).

use glam::Vec2;

use crate::mindmap::connection;
use crate::mindmap::scene_cache::EdgeKey;

use super::{
    EdgeHandleElement, EdgeHandleKind, EDGE_HANDLE_FONT_SIZE_PT, EDGE_HANDLE_GLYPH,
    SELECTED_EDGE_COLOR,
};

/// Build the grab-handle set for a single selected edge, given the
/// current (offset-applied) positions and sizes of its endpoint
/// nodes. Called once per scene build (for the selected edge only),
/// so the cost is trivial and needs no cache.
///
/// Always emits AnchorFrom + AnchorTo. On top of that:
/// - an edge with 0 control points gets a `Midpoint` handle
///   (dragging it curves the straight line);
/// - an edge with ≥ 1 control points gets `ControlPoint(i)` handles
///   at each stored offset-from-center.
pub fn build_edge_handles(
    edge: &crate::mindmap::model::MindEdge,
    edge_key: &EdgeKey,
    from_pos: Vec2,
    from_size: Vec2,
    to_pos: Vec2,
    to_size: Vec2,
) -> Vec<EdgeHandleElement> {
    let path = connection::build_connection_path(
        from_pos,
        from_size,
        edge.anchor_from,
        to_pos,
        to_size,
        edge.anchor_to,
        &edge.control_points,
    );
    let (start, end) = match &path {
        connection::ConnectionPath::Straight { start, end } => (*start, *end),
        connection::ConnectionPath::CubicBezier { start, end, .. } => (*start, *end),
    };

    let from_center = Vec2::new(from_pos.x + from_size.x * 0.5, from_pos.y + from_size.y * 0.5);
    let to_center = Vec2::new(to_pos.x + to_size.x * 0.5, to_pos.y + to_size.y * 0.5);

    let make = |kind: EdgeHandleKind, position: Vec2| EdgeHandleElement {
        edge_key: edge_key.clone(),
        kind,
        position: (position.x, position.y),
        glyph: EDGE_HANDLE_GLYPH.to_string(),
        color: SELECTED_EDGE_COLOR.to_string(),
        font_size_pt: EDGE_HANDLE_FONT_SIZE_PT,
    };

    let mut handles = Vec::with_capacity(5);
    handles.push(make(EdgeHandleKind::AnchorFrom, start));
    handles.push(make(EdgeHandleKind::AnchorTo, end));

    match edge.control_points.len() {
        0 => {
            // Straight edge: offer a midpoint handle that starts a
            // "curve this line" gesture on drag.
            let mid = start.lerp(end, 0.5);
            handles.push(make(EdgeHandleKind::Midpoint, mid));
        }
        1 => {
            // Quadratic Bezier (stored as 1 CP offset from from_center).
            let cp0 = from_center
                + Vec2::new(
                    edge.control_points[0].x as f32,
                    edge.control_points[0].y as f32,
                );
            handles.push(make(EdgeHandleKind::ControlPoint(0), cp0));
        }
        _ => {
            // Cubic Bezier (stored as 2 CPs: cp[0] from from_center,
            // cp[1] from to_center).
            let cp0 = from_center
                + Vec2::new(
                    edge.control_points[0].x as f32,
                    edge.control_points[0].y as f32,
                );
            let cp1 = to_center
                + Vec2::new(
                    edge.control_points[1].x as f32,
                    edge.control_points[1].y as f32,
                );
            handles.push(make(EdgeHandleKind::ControlPoint(0), cp0));
            handles.push(make(EdgeHandleKind::ControlPoint(1), cp1));
        }
    }

    handles
}
