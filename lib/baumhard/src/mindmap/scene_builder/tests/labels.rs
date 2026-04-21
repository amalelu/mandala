//! Connection-label emission: label present, missing/empty, position_t follow, color inheritance, GlyphConnectionConfig override.

use super::fixtures::*;
use super::super::*;
use crate::mindmap::model::{EdgeLabelConfig, GlyphConnectionConfig};

#[test]
fn test_label_element_emitted_for_edge_with_label() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false),
    ];
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.label = Some("hello".to_string());
    let map = synthetic_map(nodes, vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_label_elements.len(), 1);
    assert_eq!(scene.connection_label_elements[0].text, "hello");
}

#[test]
fn test_no_label_element_for_missing_or_empty_label() {
    // label = None → no element.
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false),
    ];
    let edge = synthetic_edge("a", "b", "auto", "auto");
    let map = synthetic_map(nodes.clone(), vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_label_elements.len(), 0);

    // label = Some("") → no element (empty-string special case).
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.label = Some(String::new());
    let map = synthetic_map(nodes, vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_label_elements.len(), 0);
}

#[test]
fn test_label_position_follows_label_config_position_t() {
    // Horizontal edge from (0,0)+40x40 to (1000,0)+40x40 — center line.
    // At t=0, label should sit near the from-anchor; at t=1, near the
    // to-anchor; midpoints differ substantially.
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 1000.0, 0.0, 40.0, 40.0, false),
    ];
    let make = |t: f32| {
        let mut e = synthetic_edge("a", "b", "auto", "auto");
        e.label = Some("x".to_string());
        e.label_config = Some(EdgeLabelConfig {
            position_t: Some(t),
            ..Default::default()
        });
        e
    };
    let scene_start = build_scene(&synthetic_map(nodes.clone(), vec![make(0.0)]), 1.0);
    let scene_end = build_scene(&synthetic_map(nodes.clone(), vec![make(1.0)]), 1.0);
    let scene_mid = build_scene(&synthetic_map(nodes, vec![make(0.5)]), 1.0);

    let pos_x = |s: &RenderScene| {
        let e = &s.connection_label_elements[0];
        // Return the center x (position + half width).
        e.position.0 + e.bounds.0 * 0.5
    };
    let x_start = pos_x(&scene_start);
    let x_end = pos_x(&scene_end);
    let x_mid = pos_x(&scene_mid);
    assert!(x_start < x_mid, "t=0 should be left of t=0.5: {x_start} vs {x_mid}");
    assert!(x_mid < x_end, "t=0.5 should be left of t=1.0: {x_mid} vs {x_end}");
}

#[test]
fn test_label_color_inherits_edge_color_when_config_color_none() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false),
    ];
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.label = Some("lbl".to_string());
    edge.color = "#abcdef".to_string();
    // glyph_connection is None → falls back to edge.color.
    let map = synthetic_map(nodes, vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_label_elements[0].color, "#abcdef");
}

#[test]
fn test_label_color_follows_glyph_connection_color_override() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false),
    ];
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.label = Some("lbl".to_string());
    edge.color = "#abcdef".to_string();
    edge.glyph_connection = Some(GlyphConnectionConfig {
        color: Some("#112233".to_string()),
        ..GlyphConnectionConfig::default()
    });
    let map = synthetic_map(nodes, vec![edge]);
    let scene = build_scene(&map, 1.0);
    // The glyph_connection.color override wins over edge.color.
    assert_eq!(scene.connection_label_elements[0].color, "#112233");
}
