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

/// Zoom-visibility cascade: when a label's
/// `min_zoom_to_render` / `max_zoom_to_render` are both `None`,
/// the emitted element inherits the owning edge's window verbatim.
/// Pins the "no override → full inherit" branch of the
/// replace-not-intersect rule.
#[test]
fn test_label_zoom_visibility_inherits_edge_window_when_absent() {
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;

    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false),
    ];
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.label = Some("lbl".to_string());
    edge.min_zoom_to_render = Some(0.5);
    edge.max_zoom_to_render = Some(2.0);
    // No label_config → inherit.
    let map = synthetic_map(nodes, vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(
        scene.connection_label_elements[0].zoom_visibility,
        ZoomVisibility { min: Some(0.5), max: Some(2.0) },
    );
}

/// Zoom-visibility cascade: when **any** of the label's window
/// bounds is `Some`, the label's pair **replaces** the edge's
/// pair wholesale (not intersects). The label below sets only
/// `min_zoom_to_render`; the resolved window drops the edge's
/// max — a label setting a one-sided window means exactly that,
/// not "narrow the edge window further". Pins the load-bearing
/// distinction between replace and intersect semantics.
#[test]
fn test_label_zoom_visibility_replace_not_intersect() {
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;

    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false),
    ];
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.label = Some("lbl".to_string());
    edge.min_zoom_to_render = Some(0.5);
    edge.max_zoom_to_render = Some(2.0);
    edge.label_config = Some(EdgeLabelConfig {
        min_zoom_to_render: Some(1.0),
        max_zoom_to_render: None,
        ..EdgeLabelConfig::default()
    });
    let map = synthetic_map(nodes, vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(
        scene.connection_label_elements[0].zoom_visibility,
        ZoomVisibility { min: Some(1.0), max: None },
        "label override must replace the edge window, not intersect it"
    );
}

/// Default path: an edge with no zoom window and a label with
/// no zoom window emits a label with unbounded visibility.
/// Locks in the zero-cost default so existing maps pay nothing.
#[test]
fn test_label_zoom_visibility_defaults_to_unbounded() {
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;

    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false),
    ];
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.label = Some("lbl".to_string());
    let map = synthetic_map(nodes, vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(
        scene.connection_label_elements[0].zoom_visibility,
        ZoomVisibility::unbounded(),
    );
}

/// A `label_config` that is `Some(cfg)` but whose zoom bounds
/// are both `None` must still inherit the edge's window —
/// presence of the config alone does not trigger the replace
/// branch. Pins the `cascade_replace((None, None)) → inherit`
/// semantics at the subtly-different Some/Some(default) vs.
/// None-config split; a regression that conflated "Some config
/// present" with "window authored" would render labels with a
/// default `EdgeLabelConfig` (position_t / perpendicular_offset
/// / color set but no zoom bounds) as unbounded even when the
/// owning edge wanted them gated.
#[test]
fn test_label_zoom_visibility_inherits_when_config_has_no_zoom_bounds() {
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;

    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false),
    ];
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.label = Some("lbl".to_string());
    edge.min_zoom_to_render = Some(0.5);
    edge.max_zoom_to_render = Some(2.0);
    // `Some(cfg)` but `cfg.min_zoom_to_render` and
    // `cfg.max_zoom_to_render` are both None — the label has
    // a config for other reasons (e.g., position override) but
    // no zoom-window opinion.
    edge.label_config = Some(EdgeLabelConfig {
        position_t: Some(0.75),
        min_zoom_to_render: None,
        max_zoom_to_render: None,
        ..EdgeLabelConfig::default()
    });
    let map = synthetic_map(nodes, vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(
        scene.connection_label_elements[0].zoom_visibility,
        ZoomVisibility { min: Some(0.5), max: Some(2.0) },
        "Some(config) with no zoom bounds must inherit the edge window, \
         not flip to unbounded"
    );
}

/// An edge with `visible: false` short-circuits before any
/// zoom-visibility resolution runs — no label element is
/// emitted regardless of whether the zoom window is set. Pins
/// the invariant the reviewer flagged: no double-negation
/// bug possible (the `edge.visible` check lives before the
/// label-emission path).
#[test]
fn test_invisible_edge_emits_no_label_even_with_zoom_window() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false),
    ];
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.label = Some("lbl".to_string());
    edge.visible = false;
    edge.min_zoom_to_render = Some(0.5);
    edge.max_zoom_to_render = Some(2.0);
    let map = synthetic_map(nodes, vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_label_elements.len(), 0);
}

/// Label AABB width is driven by grapheme count, not Unicode-scalar
/// count. A family-ZWJ emoji (`👨‍👩‍👧` — 7 scalars, 1 grapheme)
/// between two ASCII letters must produce the same bounds as three
/// plain graphemes; using `.chars().count()` would size it at 9
/// slots instead of 3. Pins §B3 against a revert to `.chars().count()`
/// on the label-sizing path.
#[test]
fn test_label_bounds_use_grapheme_count_not_scalar_count() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 1000.0, 0.0, 40.0, 40.0, false),
    ];
    let plain = {
        let mut e = synthetic_edge("a", "b", "auto", "auto");
        e.label = Some("XYZ".to_string());
        e
    };
    let zwj = {
        let mut e = synthetic_edge("a", "b", "auto", "auto");
        // "A👨\u{200D}👩\u{200D}👧B" — 9 Unicode scalars across 3
        // extended grapheme clusters (A, family-ZWJ, B).
        e.label = Some("A\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}B".to_string());
        e
    };
    let plain_scene = build_scene(&synthetic_map(nodes.clone(), vec![plain]), 1.0);
    let zwj_scene = build_scene(&synthetic_map(nodes, vec![zwj]), 1.0);
    assert_eq!(plain_scene.connection_label_elements.len(), 1);
    assert_eq!(zwj_scene.connection_label_elements.len(), 1);
    let plain_w = plain_scene.connection_label_elements[0].bounds.0;
    let zwj_w = zwj_scene.connection_label_elements[0].bounds.0;
    assert!(
        (plain_w - zwj_w).abs() < f32::EPSILON,
        "grapheme-sized bounds expected; got plain={plain_w} zwj={zwj_w}",
    );
}
