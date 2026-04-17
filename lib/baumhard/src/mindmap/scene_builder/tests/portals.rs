//! Portal element emission tests. Portals are now edges with
//! `display_mode = "portal"` — these tests verify two-marker
//! emission, missing/folded endpoint filtering, theme-variable
//! resolution, selection highlight, top-right anchor, drag offset
//! follow. Line-mode edges stay in the connection pipeline and are
//! covered by the connection tests.

use super::fixtures::*;
use super::super::*;
use crate::mindmap::scene_cache::{EdgeKey, SceneConnectionCache};
use std::collections::HashMap;

#[test]
fn portal_emits_two_elements_per_edge() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 500.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.portal_elements.len(), 2);
    let ids: Vec<&str> = scene
        .portal_elements
        .iter()
        .map(|e| e.endpoint_node_id.as_str())
        .collect();
    assert!(ids.contains(&"a"));
    assert!(ids.contains(&"b"));
    // Both markers share the same edge identity.
    assert_eq!(
        scene.portal_elements[0].edge_key,
        scene.portal_elements[1].edge_key
    );
}

#[test]
fn portal_skipped_when_endpoint_missing_from_map() {
    let nodes = vec![synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false)];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges
        .push(synthetic_portal_edge("a", "ghost", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert!(
        scene.portal_elements.is_empty(),
        "missing endpoint should silently drop the portal-mode edge"
    );
}

#[test]
fn portal_skipped_when_either_endpoint_hidden_by_fold() {
    // A parent holding a folded child — the child is hidden by
    // fold from its ancestor. A portal edge pointing into a folded
    // subtree has no visible anchor, so it must be skipped.
    let mut root = synthetic_node("root", 0.0, 0.0, 60.0, 40.0, false);
    root.folded = true;
    let mut child = synthetic_node("child", 200.0, 0.0, 60.0, 40.0, false);
    child.parent_id = Some("root".to_string());
    let other = synthetic_node("other", 500.0, 0.0, 60.0, 40.0, false);
    let mut map = synthetic_map(vec![root, child, other], vec![]);
    map.edges
        .push(synthetic_portal_edge("child", "other", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert!(
        scene.portal_elements.is_empty(),
        "portal edge should be dropped when one endpoint is hidden by fold"
    );
}

#[test]
fn portal_color_resolves_through_theme_variable() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.canvas
        .theme_variables
        .insert("--accent".to_string(), "#ff00aa".to_string());
    map.edges
        .push(synthetic_portal_edge("a", "b", "var(--accent)"));
    let scene = build_scene(&map, 1.0);
    assert_eq!(
        scene.portal_elements[0].color, "#ff00aa",
        "var(--accent) must resolve through theme_variables"
    );
    assert_eq!(scene.portal_elements[1].color, "#ff00aa");
}

#[test]
fn selected_portal_edge_rendered_with_highlight_color() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(
        &map,
        &HashMap::new(),
        Some(("a", "b", "cross_link")),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    // Both emitted markers flip to the cyan highlight color.
    assert_eq!(scene.portal_elements[0].color, "#00E5FF");
    assert_eq!(scene.portal_elements[1].color, "#00E5FF");
}

#[test]
fn portal_marker_position_is_above_top_right_of_node() {
    let nodes = vec![
        synthetic_node("a", 100.0, 200.0, 80.0, 40.0, false),
        synthetic_node("b", 500.0, 500.0, 80.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    // Find the marker keyed to endpoint "a".
    let marker_a = scene
        .portal_elements
        .iter()
        .find(|e| e.endpoint_node_id == "a")
        .expect("marker for endpoint a");
    // Node "a" sits at (100, 200) with size (80, 40). The marker
    // should float above the node's top edge (y < 200) and be
    // horizontally clustered on the right half of the node.
    assert!(
        marker_a.position.1 < 200.0,
        "marker y {} should be above node top 200",
        marker_a.position.1
    );
    assert!(
        marker_a.position.0 > 100.0 + 80.0 * 0.5,
        "marker x {} should be on the right half of the node",
        marker_a.position.0
    );
}

#[test]
fn portal_marker_follows_drag_offsets() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));

    // Build a baseline scene with no offsets, then an offset scene
    // and assert the marker moved by exactly the offset amount.
    let baseline = build_scene(&map, 1.0);
    let baseline_a = baseline
        .portal_elements
        .iter()
        .find(|e| e.endpoint_node_id == "a")
        .expect("marker for endpoint a in baseline");

    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (100.0f32, 50.0f32));
    let dragged = build_scene_with_offsets(&map, &offsets, 1.0);
    let dragged_a = dragged
        .portal_elements
        .iter()
        .find(|e| e.endpoint_node_id == "a")
        .expect("marker for endpoint a in dragged scene");

    let dx = dragged_a.position.0 - baseline_a.position.0;
    let dy = dragged_a.position.1 - baseline_a.position.1;
    assert!((dx - 100.0).abs() < 0.01, "marker x should shift by +100, got {dx}");
    assert!((dy - 50.0).abs() < 0.01, "marker y should shift by +50, got {dy}");
}

#[test]
fn portal_mode_edge_skipped_by_connection_pipeline() {
    // A portal-mode edge must not produce a `ConnectionElement`
    // (no line, no body glyphs). The connection pass is the one
    // place where this would leak into rendering.
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert!(
        scene.connection_elements.is_empty(),
        "portal-mode edges must not produce connection elements"
    );
    assert_eq!(scene.portal_elements.len(), 2);
}

#[test]
fn portal_color_preview_wins_over_selection() {
    // Picker hover feedback on a selected portal edge must show the
    // preview color on both markers, not the cyan selection highlight.
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let key = EdgeKey::new("a", "b", "cross_link");
    let preview = PortalColorPreview { edge_key: &key, color: "#112233" };
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(
        &map,
        &HashMap::new(),
        Some(("a", "b", "cross_link")),
        None,
        None,
        Some(preview),
        &mut cache,
        1.0,
    );
    assert_eq!(scene.portal_elements[0].color, "#112233");
    assert_eq!(scene.portal_elements[1].color, "#112233");
}
