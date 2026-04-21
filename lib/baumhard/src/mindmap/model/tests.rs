//! Mindmap-model tests: ancestry, connection config resolution, label
//! position + display_mode round-trips. Kept in a sibling file so
//! the `mod.rs` itself reads purely as the public surface.

use super::*;
use crate::mindmap::loader;
use std::path::PathBuf;

fn test_map_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // lib/baumhard -> lib
    path.pop(); // lib -> root
    path.push("maps/testament.mindmap.json");
    path
}

#[test]
fn test_all_descendants() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();

    // "Lord God" (0) has children — descendants should include them all
    let children = map.children_of("0");
    assert!(!children.is_empty(), "Lord God should have children");

    let descendants = map.all_descendants("0");
    // Every direct child should appear in descendants
    for child in &children {
        assert!(descendants.contains(&child.id), "Child {} missing from descendants", child.id);
    }
    // Descendants should be >= children (includes grandchildren etc.)
    assert!(descendants.len() >= children.len());
}

#[test]
fn test_all_descendants_leaf_node() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();

    // Find a leaf node (no children)
    let leaf = map.nodes.values()
        .find(|n| map.children_of(&n.id).is_empty())
        .expect("Should have at least one leaf node");

    let descendants = map.all_descendants(&leaf.id);
    assert!(descendants.is_empty(), "Leaf node should have no descendants");
}

/// Find a (root_id, child_id, grandchild_id) triple in the testament map.
/// Used by the ancestor tests below.
fn find_hierarchy_triple(map: &MindMap) -> (String, String, String) {
    for root in map.root_nodes() {
        for child in map.children_of(&root.id) {
            let grands = map.children_of(&child.id);
            if let Some(grand) = grands.first() {
                return (root.id.clone(), child.id.clone(), grand.id.clone());
            }
        }
    }
    panic!("testament map should contain a root -> child -> grandchild chain");
}

#[test]
fn test_is_ancestor_or_self_reflexive() {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    let (root, child, grand) = find_hierarchy_triple(&map);
    assert!(map.is_ancestor_or_self(&root, &root));
    assert!(map.is_ancestor_or_self(&child, &child));
    assert!(map.is_ancestor_or_self(&grand, &grand));
}

#[test]
fn test_is_ancestor_or_self_direct_parent() {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    let (root, child, grand) = find_hierarchy_triple(&map);
    // root is a direct ancestor of child; child is a direct ancestor of grand
    assert!(map.is_ancestor_or_self(&root, &child));
    assert!(map.is_ancestor_or_self(&child, &grand));
}

#[test]
fn test_is_ancestor_or_self_deep_descendant() {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    let (root, _child, grand) = find_hierarchy_triple(&map);
    // root is a transitive ancestor of grand (two hops away)
    assert!(map.is_ancestor_or_self(&root, &grand));
}

#[test]
fn test_is_ancestor_or_self_reversed_is_false() {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    let (root, child, grand) = find_hierarchy_triple(&map);
    // A descendant is never the ancestor of its own parent chain.
    assert!(!map.is_ancestor_or_self(&child, &root));
    assert!(!map.is_ancestor_or_self(&grand, &child));
    assert!(!map.is_ancestor_or_self(&grand, &root));
}

#[test]
fn test_is_ancestor_or_self_sibling_is_unrelated() {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    // Find two sibling roots (they share parent_id = None but are not
    // ancestors of each other).
    let roots = map.root_nodes();
    if roots.len() >= 2 {
        let a = roots[0].id.clone();
        let b = roots[1].id.clone();
        assert!(!map.is_ancestor_or_self(&a, &b));
        assert!(!map.is_ancestor_or_self(&b, &a));
    }
    // Also check: the first root and some node whose parent chain does not
    // include it (pick an unrelated subtree if available).
    // The above two-sibling-roots case is sufficient for testament.
}

/// Tiny tolerance for floating-point comparisons in the
/// `effective_font_size_pt` tests below — the formula is just two
/// multiplies and a divide, so anything tighter than this means a
/// real bug.
const EFFECTIVE_FONT_EPSILON: f32 = 1.0e-4;

#[test]
fn effective_font_size_unity_zoom_returns_base() {
    let cfg = GlyphConnectionConfig::default(); // 12 / 8 / 24
    // At zoom = 1.0 the base 12 is inside [8, 24], so screen size
    // = 12 and canvas size = 12 / 1 = 12.
    assert!(
        (cfg.effective_font_size_pt(1.0) - 12.0).abs() < EFFECTIVE_FONT_EPSILON,
        "expected 12.0 at zoom 1.0, got {}",
        cfg.effective_font_size_pt(1.0)
    );
}

#[test]
fn effective_font_size_zoomed_out_floors_to_min() {
    let cfg = GlyphConnectionConfig::default();
    // At zoom = 0.1: base * zoom = 1.2 → clamp up to 8 → canvas
    // = 8 / 0.1 = 80.
    let got = cfg.effective_font_size_pt(0.1);
    assert!(
        (got - 80.0).abs() < EFFECTIVE_FONT_EPSILON,
        "expected 80.0 at zoom 0.1, got {got}"
    );

    // At zoom = 0.5: base * zoom = 6 → clamp up to 8 → canvas
    // = 8 / 0.5 = 16.
    let got = cfg.effective_font_size_pt(0.5);
    assert!(
        (got - 16.0).abs() < EFFECTIVE_FONT_EPSILON,
        "expected 16.0 at zoom 0.5, got {got}"
    );
}

#[test]
fn effective_font_size_zoomed_in_ceils_to_max() {
    // Configure an explicit smaller ceiling so this test exercises
    // the clamp behavior without tracking the default cap.
    let cfg = GlyphConnectionConfig {
        max_font_size_pt: 24.0,
        ..GlyphConnectionConfig::default()
    };
    // At zoom = 2.0: base * zoom = 24 (right at the cap) → canvas
    // = 24 / 2 = 12.
    let got = cfg.effective_font_size_pt(2.0);
    assert!(
        (got - 12.0).abs() < EFFECTIVE_FONT_EPSILON,
        "expected 12.0 at zoom 2.0, got {got}"
    );

    // At zoom = 5.0: base * zoom = 60 → clamp down to 24 → canvas
    // = 24 / 5 = 4.8.
    let got = cfg.effective_font_size_pt(5.0);
    assert!(
        (got - 4.8).abs() < EFFECTIVE_FONT_EPSILON,
        "expected 4.8 at zoom 5.0, got {got}"
    );
}

#[test]
fn effective_font_size_handles_zero_and_negative_zoom() {
    // Zero or negative zoom would divide by zero / produce a
    // negative font; the implementation guards with EPSILON. Just
    // assert it returns a finite, positive value rather than
    // panicking or returning NaN.
    let cfg = GlyphConnectionConfig::default();
    let z0 = cfg.effective_font_size_pt(0.0);
    assert!(z0.is_finite() && z0 > 0.0, "expected finite > 0, got {z0}");
    let zn = cfg.effective_font_size_pt(-1.0);
    assert!(zn.is_finite() && zn > 0.0, "expected finite > 0, got {zn}");
}

#[test]
fn effective_font_size_respects_custom_bounds() {
    // Tighter clamp: [10, 14] with the same base.
    let cfg = GlyphConnectionConfig {
        min_font_size_pt: 10.0,
        max_font_size_pt: 14.0,
        ..GlyphConnectionConfig::default()
    };
    // zoom = 1.0: 12 in [10, 14] → canvas 12.
    assert!((cfg.effective_font_size_pt(1.0) - 12.0).abs() < EFFECTIVE_FONT_EPSILON);
    // zoom = 0.5: 6 → up to 10 → canvas 20.
    assert!((cfg.effective_font_size_pt(0.5) - 20.0).abs() < EFFECTIVE_FONT_EPSILON);
    // zoom = 2.0: 24 → down to 14 → canvas 7.
    assert!((cfg.effective_font_size_pt(2.0) - 7.0).abs() < EFFECTIVE_FONT_EPSILON);
}

// label_config + resolved_for helper.

fn synthetic_edge_with_label(label: Option<&str>, config: Option<EdgeLabelConfig>) -> MindEdge {
    MindEdge {
        from_id: "a".to_string(),
        to_id: "b".to_string(),
        edge_type: "cross_link".to_string(),
        color: "#fff".to_string(),
        width: 1,
        line_style: "solid".to_string(),
        visible: true,
        label: label.map(|s| s.to_string()),
        label_config: config,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: Vec::new(),
        glyph_connection: None,
        display_mode: None,
        portal_from: None,
        portal_to: None,
    }
}

#[test]
fn label_config_round_trips_through_json() {
    // Explicit values are preserved across serde round-trip.
    let cfg = EdgeLabelConfig {
        position_t: Some(0.25),
        perpendicular_offset: Some(12.5),
        color: Some("#ff8800".to_string()),
        font_size_pt: Some(18.0),
        min_font_size_pt: Some(9.0),
        max_font_size_pt: Some(64.0),
    };
    let edge = synthetic_edge_with_label(Some("hello"), Some(cfg.clone()));
    let json = serde_json::to_string(&edge).unwrap();
    assert!(
        json.contains("label_config"),
        "json should include label_config: {json}"
    );
    let back: MindEdge = serde_json::from_str(&json).unwrap();
    assert_eq!(back.label.as_deref(), Some("hello"));
    assert_eq!(back.label_config.as_ref(), Some(&cfg));
}

#[test]
fn label_config_missing_defaults_to_none() {
    // Older maps without the field must still deserialize — and
    // round-trip back out without the field.
    let json = r##"{
        "from_id":"a","to_id":"b","type":"cross_link",
        "color":"#fff","width":1,"line_style":"solid","visible":true,
        "label":null,"anchor_from":"auto","anchor_to":"auto","control_points":[]
    }"##;
    let edge: MindEdge = serde_json::from_str(json).unwrap();
    assert!(edge.label_config.is_none());
    let back_json = serde_json::to_string(&edge).unwrap();
    assert!(
        !back_json.contains("label_config"),
        "None should not serialize: {back_json}"
    );
}

#[test]
fn label_config_partial_fields_round_trip() {
    // A user who only sets `position_t` keeps the rest as `None`
    // and doesn't accidentally serialize defaults for the other
    // fields (each field carries `skip_serializing_if`).
    let edge = synthetic_edge_with_label(
        Some("hi"),
        Some(EdgeLabelConfig {
            position_t: Some(0.75),
            ..Default::default()
        }),
    );
    let json = serde_json::to_string(&edge).unwrap();
    assert!(json.contains("position_t"));
    assert!(!json.contains("perpendicular_offset"));
    assert!(!json.contains("font_size_pt"));
    let back: MindEdge = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.label_config.as_ref().and_then(|c| c.position_t),
        Some(0.75)
    );
}

#[test]
fn effective_font_size_pt_inherits_body_when_label_override_absent() {
    use crate::mindmap::model::{Canvas, GlyphConnectionConfig, DEFAULT_LABEL_SIZE_FACTOR};
    let canvas = Canvas {
        background_color: "#000".into(),
        default_border: None,
        default_connection: None,
        theme_variables: std::collections::HashMap::new(),
        theme_variants: std::collections::HashMap::new(),
    };
    let mut edge = synthetic_edge_with_label(Some("x"), None);
    edge.glyph_connection = Some(GlyphConnectionConfig {
        font_size_pt: 20.0,
        min_font_size_pt: 8.0,
        max_font_size_pt: 64.0,
        ..GlyphConnectionConfig::default()
    });
    // With no label_config, the effective size inherits body × factor.
    let expected = 20.0 * DEFAULT_LABEL_SIZE_FACTOR;
    let got = EdgeLabelConfig::effective_font_size_pt(None, &edge, &canvas, 1.0);
    assert!((got - expected).abs() < 1.0e-4, "expected {expected} got {got}");
}

#[test]
fn effective_font_size_pt_label_override_wins_over_body() {
    use crate::mindmap::model::{Canvas, GlyphConnectionConfig};
    let canvas = Canvas {
        background_color: "#000".into(),
        default_border: None,
        default_connection: None,
        theme_variables: std::collections::HashMap::new(),
        theme_variants: std::collections::HashMap::new(),
    };
    let mut edge = synthetic_edge_with_label(Some("x"), None);
    edge.glyph_connection = Some(GlyphConnectionConfig {
        font_size_pt: 20.0,
        min_font_size_pt: 8.0,
        max_font_size_pt: 64.0,
        ..GlyphConnectionConfig::default()
    });
    let label_cfg = EdgeLabelConfig {
        font_size_pt: Some(32.0),
        min_font_size_pt: Some(16.0),
        max_font_size_pt: Some(64.0),
        ..Default::default()
    };
    // Target-on-screen = 32 × zoom; at zoom 1.0, inside [16,64] → 32.
    let got = EdgeLabelConfig::effective_font_size_pt(Some(&label_cfg), &edge, &canvas, 1.0);
    assert!((got - 32.0).abs() < 1.0e-4);
    // Zoom 2.0: target = 64 (pinned at max); canvas = 64 / 2 = 32.
    let got2 = EdgeLabelConfig::effective_font_size_pt(Some(&label_cfg), &edge, &canvas, 2.0);
    assert!((got2 - 32.0).abs() < 1.0e-4);
    // Zoom 0.5: target = 16 (pinned at min); canvas = 16 / 0.5 = 32.
    let got3 = EdgeLabelConfig::effective_font_size_pt(Some(&label_cfg), &edge, &canvas, 0.5);
    assert!((got3 - 32.0).abs() < 1.0e-4);
}

#[test]
fn portal_endpoint_text_fields_round_trip() {
    // Portal text overrides round-trip cleanly and stay absent
    // from serialized output when `None`.
    let state = PortalEndpointState {
        color: Some("#ff8800".to_string()),
        border_t: Some(1.5),
        text: Some("→ jumps".to_string()),
        text_color: Some("#99ccff".to_string()),
        text_font_size_pt: Some(14.0),
        text_min_font_size_pt: Some(10.0),
        text_max_font_size_pt: Some(48.0),
    };
    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("text_color"));
    assert!(json.contains("text_font_size_pt"));
    let back: PortalEndpointState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, state);
    // Defaults stay absent.
    let empty = PortalEndpointState::default();
    let empty_json = serde_json::to_string(&empty).unwrap();
    assert!(!empty_json.contains("text_color"));
    assert!(!empty_json.contains("text_font_size_pt"));
}

#[test]
fn resolved_for_returns_borrowed_from_edge_when_present() {
    let mut edge = synthetic_edge_with_label(None, None);
    let custom = GlyphConnectionConfig {
        body: "◆".to_string(),
        ..GlyphConnectionConfig::default()
    };
    edge.glyph_connection = Some(custom);
    let canvas = Canvas {
        background_color: "#000".to_string(),
        default_border: None,
        default_connection: None,
        theme_variables: std::collections::HashMap::new(),
        theme_variants: std::collections::HashMap::new(),
    };
    let resolved = GlyphConnectionConfig::resolved_for(&edge, &canvas);
    assert_eq!(resolved.body, "◆");
    // It's borrowed, not owned — clone-count unchanged.
    assert!(matches!(resolved, std::borrow::Cow::Borrowed(_)));
}

#[test]
fn resolved_for_falls_back_to_canvas_default() {
    let edge = synthetic_edge_with_label(None, None);
    let canvas_cfg = GlyphConnectionConfig {
        body: "═".to_string(),
        ..GlyphConnectionConfig::default()
    };
    let canvas = Canvas {
        background_color: "#000".to_string(),
        default_border: None,
        default_connection: Some(canvas_cfg),
        theme_variables: std::collections::HashMap::new(),
        theme_variants: std::collections::HashMap::new(),
    };
    let resolved = GlyphConnectionConfig::resolved_for(&edge, &canvas);
    assert_eq!(resolved.body, "═");
    assert!(matches!(resolved, std::borrow::Cow::Borrowed(_)));
}

#[test]
fn resolved_for_falls_back_to_hardcoded_default() {
    let edge = synthetic_edge_with_label(None, None);
    let canvas = Canvas {
        background_color: "#000".to_string(),
        default_border: None,
        default_connection: None,
        theme_variables: std::collections::HashMap::new(),
        theme_variants: std::collections::HashMap::new(),
    };
    let resolved = GlyphConnectionConfig::resolved_for(&edge, &canvas);
    assert_eq!(resolved.body, GlyphConnectionConfig::default().body);
    // Owned — the caller got a freshly-built default.
    assert!(matches!(resolved, std::borrow::Cow::Owned(_)));
}

// ============================================================
// display_mode (portals as an edge render mode)
// ============================================================

#[test]
fn display_mode_absent_defaults_to_none() {
    // Pre-refactor maps wrote no `display_mode` field. `#[serde(default)]`
    // must deserialize those edges with `None` so they keep rendering
    // as lines.
    let json = r##"{
        "from_id":"a","to_id":"b","type":"cross_link",
        "color":"#fff","width":1,"line_style":"solid","visible":true,
        "label":null,"anchor_from":"auto","anchor_to":"auto","control_points":[]
    }"##;
    let edge: MindEdge = serde_json::from_str(json).unwrap();
    assert_eq!(edge.display_mode, None);
    assert!(!is_portal_edge(&edge));
}

#[test]
fn display_mode_portal_round_trips_through_json() {
    let mut edge = synthetic_edge_with_label(None, None);
    edge.display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
    let json = serde_json::to_string(&edge).unwrap();
    assert!(json.contains("\"display_mode\":\"portal\""), "json: {json}");
    let back: MindEdge = serde_json::from_str(&json).unwrap();
    assert_eq!(back.display_mode.as_deref(), Some(DISPLAY_MODE_PORTAL));
    assert!(is_portal_edge(&back));
}

#[test]
fn display_mode_none_omitted_in_serialize() {
    let edge = synthetic_edge_with_label(None, None);
    let json = serde_json::to_string(&edge).unwrap();
    assert!(
        !json.contains("display_mode"),
        "None should be omitted per skip_serializing_if: {json}"
    );
}

#[test]
fn portal_glyph_presets_are_nonempty_and_unique() {
    assert!(!PORTAL_GLYPH_PRESETS.is_empty());
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for g in PORTAL_GLYPH_PRESETS {
        assert!(seen.insert(*g), "glyph preset {g} duplicated");
    }
}
