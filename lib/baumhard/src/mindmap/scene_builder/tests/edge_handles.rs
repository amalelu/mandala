//! Session 6C edge-handle emission: unselected baseline, straight-edge midpoint, curved-edge control points, cubic two-CP, absolute canvas positioning.

use super::fixtures::*;
use super::super::*;
use crate::mindmap::loader;
use crate::mindmap::scene_cache::SceneConnectionCache;
use std::collections::HashMap;
use glam::Vec2;

#[test]
fn test_no_edge_handles_when_nothing_selected() {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    let scene = build_scene(&map, 1.0);
    assert!(scene.edge_handles.is_empty(),
        "no selection → no handles emitted");
}

#[test]
fn test_edge_handles_straight_edge_emits_midpoint() {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    // Find a straight edge
    let edge = map.edges.iter()
        .find(|e| e.visible && e.control_points.is_empty())
        .expect("testament map should have a straight edge");
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(
        &map,
        &HashMap::new(),
        Some((&edge.from_id, &edge.to_id, &edge.edge_type)),
        None,
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(
        scene.edge_handles.len(),
        3,
        "straight edge: AnchorFrom + AnchorTo + Midpoint = 3 handles"
    );
    let kinds: Vec<&EdgeHandleKind> = scene.edge_handles
        .iter()
        .map(|h| &h.kind)
        .collect();
    assert!(kinds.iter().any(|k| matches!(k, EdgeHandleKind::AnchorFrom)));
    assert!(kinds.iter().any(|k| matches!(k, EdgeHandleKind::AnchorTo)));
    assert!(kinds.iter().any(|k| matches!(k, EdgeHandleKind::Midpoint)));
}

#[test]
fn test_edge_handles_curved_edge_emits_control_points_not_midpoint() {
    let mut map = loader::load_from_file(&test_map_path()).unwrap();
    // Find a visible edge and curve it (quadratic)
    let edge_idx = map.edges.iter()
        .position(|e| e.visible)
        .unwrap();
    map.edges[edge_idx].control_points.push(
        crate::mindmap::model::ControlPoint { x: 20.0, y: 30.0 },
    );
    let edge = map.edges[edge_idx].clone();
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(
        &map,
        &HashMap::new(),
        Some((&edge.from_id, &edge.to_id, &edge.edge_type)),
        None,
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    // 2 anchors + 1 control point = 3 handles, no midpoint
    assert_eq!(scene.edge_handles.len(), 3);
    assert!(scene.edge_handles.iter().any(|h| matches!(h.kind, EdgeHandleKind::ControlPoint(0))));
    assert!(scene.edge_handles.iter().all(|h| !matches!(h.kind, EdgeHandleKind::Midpoint)));
}

#[test]
fn test_edge_handles_cubic_edge_emits_both_control_points() {
    let mut map = loader::load_from_file(&test_map_path()).unwrap();
    let edge_idx = map.edges.iter()
        .position(|e| e.visible)
        .unwrap();
    map.edges[edge_idx].control_points.push(
        crate::mindmap::model::ControlPoint { x: 10.0, y: 10.0 },
    );
    map.edges[edge_idx].control_points.push(
        crate::mindmap::model::ControlPoint { x: 40.0, y: 40.0 },
    );
    let edge = map.edges[edge_idx].clone();
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(
        &map,
        &HashMap::new(),
        Some((&edge.from_id, &edge.to_id, &edge.edge_type)),
        None,
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    // 2 anchors + 2 control points = 4 handles
    assert_eq!(scene.edge_handles.len(), 4);
    assert!(scene.edge_handles.iter().any(|h| matches!(h.kind, EdgeHandleKind::ControlPoint(0))));
    assert!(scene.edge_handles.iter().any(|h| matches!(h.kind, EdgeHandleKind::ControlPoint(1))));
}

#[test]
fn test_edge_handle_control_point_position_is_absolute_canvas() {
    let mut map = loader::load_from_file(&test_map_path()).unwrap();
    let edge_idx = map.edges.iter()
        .position(|e| e.visible)
        .unwrap();
    let cp_x = 55.0;
    let cp_y = 77.0;
    map.edges[edge_idx].control_points.push(
        crate::mindmap::model::ControlPoint { x: cp_x, y: cp_y },
    );
    let edge = map.edges[edge_idx].clone();
    let from_node = map.nodes.get(&edge.from_id).unwrap();
    let from_center_x = from_node.position.x as f32 + from_node.size.width as f32 * 0.5;
    let from_center_y = from_node.position.y as f32 + from_node.size.height as f32 * 0.5;

    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(
        &map,
        &HashMap::new(),
        Some((&edge.from_id, &edge.to_id, &edge.edge_type)),
        None,
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    let cp_handle = scene.edge_handles.iter()
        .find(|h| matches!(h.kind, EdgeHandleKind::ControlPoint(0)))
        .unwrap();
    assert!((cp_handle.position.0 - (from_center_x + cp_x as f32)).abs() < 0.01);
    assert!((cp_handle.position.1 - (from_center_y + cp_y as f32)).abs() < 0.01);
}

// ====================================================================
// Session 6D — ConnectionLabelElement emission
// ====================================================================
