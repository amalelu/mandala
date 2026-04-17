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
        SceneSelectionContext {
            edge: Some(("a", "b", "cross_link")),
            ..Default::default()
        },
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
fn portal_marker_points_at_partner_endpoint() {
    // Post-directional-default: the marker sits on the owning
    // node's border at the point facing its partner endpoint,
    // not floated above a fixed corner. Partner "b" is to the
    // south-east of "a", so a's marker anchors on the
    // right / bottom side.
    let nodes = vec![
        synthetic_node("a", 100.0, 200.0, 80.0, 40.0, false),
        synthetic_node("b", 500.0, 500.0, 80.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    let marker_a = scene
        .portal_elements
        .iter()
        .find(|e| e.endpoint_node_id == "a")
        .expect("marker for endpoint a");
    // Center of marker AABB should sit outside the node's right
    // or bottom side (the two sides facing partner b). Check that
    // the marker lives outside the node AABB in at least one axis.
    let marker_cx = marker_a.position.0 + marker_a.bounds.0 * 0.5;
    let marker_cy = marker_a.position.1 + marker_a.bounds.1 * 0.5;
    let right_edge = 100.0 + 80.0;
    let bottom_edge = 200.0 + 40.0;
    assert!(
        marker_cx > right_edge || marker_cy > bottom_edge,
        "marker ({marker_cx}, {marker_cy}) should be right or below node AABB"
    );
}

#[test]
fn portal_marker_follows_drag_offsets() {
    // When the owning node is offset during a drag, the marker
    // moves with it. The relationship between marker shift and
    // node shift is not strict equality because the directional
    // default also uses the partner's position (which is
    // unchanged here), but the marker stays anchored to the
    // moved node's border — so its delta has the same sign and
    // similar magnitude in each axis.
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));

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
    // Partner stays to the right → marker stays on a's right
    // edge, x shifts by roughly the node's x offset. y shift is
    // somewhere between 0 and the node's y offset depending on
    // whether the anchor slid vertically to face the new partner
    // direction.
    assert!(dx > 50.0, "marker x should follow node rightward, got {dx}");
    assert!(dy >= -10.0 && dy <= 60.0, "marker y shift {dy} within node move range");
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
        SceneSelectionContext {
            edge: Some(("a", "b", "cross_link")),
            ..Default::default()
        },
        None,
        Some(preview),
        &mut cache,
        1.0,
    );
    assert_eq!(scene.portal_elements[0].color, "#112233");
    assert_eq!(scene.portal_elements[1].color, "#112233");
}
