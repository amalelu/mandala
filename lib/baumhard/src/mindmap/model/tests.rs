// SPDX-License-Identifier: MPL-2.0

//! Mindmap-model tests: ancestry, connection config resolution, label
//! position + display_mode round-trips, and the edge color cascade.
//! Kept in a sibling file so
//! the `mod.rs` itself reads purely as the public surface.

use super::*;
use crate::mindmap::loader;
use crate::mindmap::test_helpers::{
    blank_canvas, synthetic_edge, synthetic_map, synthetic_node_full, synthetic_portal_edge,
    testament_map_path as test_map_path,
};
use crate::util::geometry::is_positive_finite;

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
        assert!(
            descendants.contains(&child.id),
            "Child {} missing from descendants",
            child.id
        );
    }
    // Descendants should be >= children (includes grandchildren etc.)
    assert!(descendants.len() >= children.len());
}

#[test]
fn test_all_descendants_leaf_node() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();

    // Find a leaf node (no children)
    let leaf = map
        .nodes
        .values()
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
    assert!(is_positive_finite(z0), "expected finite > 0, got {z0}");
    let zn = cfg.effective_font_size_pt(-1.0);
    assert!(is_positive_finite(zn), "expected finite > 0, got {zn}");
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
    let mut e = crate::mindmap::test_helpers::synthetic_edge("a", "b", "auto", "auto");
    e.label = label.map(|s| s.to_string());
    e.label_config = config;
    e
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
        min_zoom_to_render: None,
        max_zoom_to_render: None,
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
fn label_config_perpendicular_offset_only_round_trips() {
    // Asymmetric case: only the perpendicular offset is set.
    // Protects against a future regression that accidentally
    // drops `skip_serializing_if` on that field.
    let edge = synthetic_edge_with_label(
        Some("side"),
        Some(EdgeLabelConfig {
            perpendicular_offset: Some(-8.5),
            ..Default::default()
        }),
    );
    let json = serde_json::to_string(&edge).unwrap();
    assert!(json.contains("perpendicular_offset"));
    assert!(!json.contains("position_t"));
    assert!(!json.contains("font_size_pt"));
    let back: MindEdge = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.label_config.as_ref().and_then(|c| c.perpendicular_offset),
        Some(-8.5)
    );
    assert_eq!(back.label_config.as_ref().and_then(|c| c.position_t), None);
}

#[test]
fn effective_font_size_pt_partial_clamp_inheritance() {
    // Own `min` only: resolver should pick up the label's min and
    // fall back to the body's max. Inverts for "own max only".
    use crate::mindmap::model::GlyphConnectionConfig;
    let canvas = blank_canvas();
    let mut edge = synthetic_edge_with_label(Some("x"), None);
    edge.glyph_connection = Some(GlyphConnectionConfig {
        font_size_pt: 20.0,
        min_font_size_pt: 8.0,
        max_font_size_pt: 64.0,
        ..GlyphConnectionConfig::default()
    });
    // Own `font_size_pt = 40`, own `min = 30`, no own max → body
    // max 64 applies. At zoom 1, target 40 ∈ [30, 64] → 40.
    let cfg_min_only = EdgeLabelConfig {
        font_size_pt: Some(40.0),
        min_font_size_pt: Some(30.0),
        ..Default::default()
    };
    let got = EdgeLabelConfig::effective_font_size_pt(Some(&cfg_min_only), &edge, &canvas, 1.0);
    assert!((got - 40.0).abs() < 1.0e-4);
    // At zoom 0.5, target 20 → pinned at own min 30 → canvas
    // size = 30 / 0.5 = 60.
    let got_zoomed = EdgeLabelConfig::effective_font_size_pt(Some(&cfg_min_only), &edge, &canvas, 0.5);
    assert!((got_zoomed - 60.0).abs() < 1.0e-4);

    // Own `max = 24` only, size inherits body × factor (22).
    // At zoom 1.0, target 22 clamps against body min 8 / own max
    // 24 → 22 (unchanged). At zoom 2.0, target 44 → pinned at
    // own max 24 → canvas size = 24 / 2 = 12.
    let cfg_max_only = EdgeLabelConfig {
        max_font_size_pt: Some(24.0),
        ..Default::default()
    };
    let got_max_1 = EdgeLabelConfig::effective_font_size_pt(Some(&cfg_max_only), &edge, &canvas, 1.0);
    assert!(
        (got_max_1 - 22.0).abs() < 1.0e-4,
        "expected 22 (body × 1.1), got {got_max_1}"
    );
    let got_max_2 = EdgeLabelConfig::effective_font_size_pt(Some(&cfg_max_only), &edge, &canvas, 2.0);
    assert!(
        (got_max_2 - 12.0).abs() < 1.0e-4,
        "expected 12 (own max pinned), got {got_max_2}"
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
    assert_eq!(back.label_config.as_ref().and_then(|c| c.position_t), Some(0.75));
}

#[test]
fn effective_font_size_pt_inherits_body_when_label_override_absent() {
    use crate::mindmap::model::{GlyphConnectionConfig, DEFAULT_LABEL_SIZE_FACTOR};
    let canvas = blank_canvas();
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
    use crate::mindmap::model::GlyphConnectionConfig;
    let canvas = blank_canvas();
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
        perpendicular_offset: Some(12.5),
        text: Some("→ jumps".to_string()),
        text_color: Some("#99ccff".to_string()),
        text_font_size_pt: Some(14.0),
        text_min_font_size_pt: Some(10.0),
        text_max_font_size_pt: Some(48.0),
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    };
    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("text_color"));
    assert!(json.contains("text_font_size_pt"));
    assert!(json.contains("perpendicular_offset"));
    let back: PortalEndpointState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, state);
    // Defaults stay absent.
    let empty = PortalEndpointState::default();
    let empty_json = serde_json::to_string(&empty).unwrap();
    assert!(!empty_json.contains("text_color"));
    assert!(!empty_json.contains("text_font_size_pt"));
    assert!(!empty_json.contains("perpendicular_offset"));
}

#[test]
fn resolved_for_returns_borrowed_from_edge_when_present() {
    let mut edge = synthetic_edge_with_label(None, None);
    let custom = GlyphConnectionConfig {
        body: "◆".to_string(),
        ..GlyphConnectionConfig::default()
    };
    edge.glyph_connection = Some(custom);
    let canvas = blank_canvas();
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
        default_section_frame_border: None,
        default_focused_section_frame_border: None,
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
    let canvas = blank_canvas();
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

/// `format/schema.md` §"Portal-mode edges" prints a portal edge as
/// literal JSON, and `maptool convert` emits exactly this shape for
/// every folded legacy portal. A documented example that does not
/// deserialize is worse than none — a reader hand-authoring an edge
/// from it produces a file the loader refuses.
///
/// The example is **read out of the spec**, not restated here: a
/// hard-copied literal would pin this test against itself and let
/// the doc drift away from the code undetected (the same reasoning
/// as `gfx_structs::tests::area_tests::documented_rotate_example`).
#[test]
fn test_documented_portal_edge_example_deserializes() {
    let doc = crate::util::doc_fixtures::format_doc_path("schema.md");
    let published =
        crate::util::doc_fixtures::documented_json_block(&doc, "### Portal-mode edges", 0);

    let edge: MindEdge = serde_json::from_str(&published).unwrap_or_else(|e| {
        panic!("format/schema.md's portal-edge example must deserialize: {e}\n{published}")
    });
    assert!(is_portal_edge(&edge));
    assert_eq!(edge.from_id, "0.3");
    assert_eq!(edge.to_id, "1.7.2");
    assert_eq!(edge.glyph_connection.as_ref().unwrap().body, "◈");

    // `label` has no `skip_serializing_if`, so every serializer
    // writes it. The spec must show it, or the two portal-edge
    // examples in `format/` disagree about the shape on disk.
    assert!(
        published.contains("\"label\""),
        "the published portal edge must carry `label`, which is always serialized:\n{published}"
    );
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

// ──────────────────────────────────────────────────────────
// Zoom-visibility serde on mindmap-model types.
// `GlyphArea.zoom_visibility` (Baumhard-level) is covered by
// `gfx_structs::tests::zoom_visibility_tests`. These tests
// cover the model-level surface: the flat `min_zoom_to_render`
// / `max_zoom_to_render` pair that maps serialize to, one pair
// per model type that can carry an authored window.
// ──────────────────────────────────────────────────────────

/// `(min_zoom_to_render, max_zoom_to_render)` skip-default and
/// round-trip contract on every model struct that carries the
/// pair: `MindEdge`, `EdgeLabelConfig`, `PortalEndpointState`.
/// Each shape is verified against the same three states (default,
/// authored both-sides, authored one-side) so a future field
/// rename or `skip_serializing_if` regression on any one of them
/// fires the same test.
#[test]
fn zoom_window_skip_default_and_round_trip_on_every_struct() {
    fn check_pair(
        label: &str,
        default_json: &str,
        authored_min: f32,
        mut set_pair: impl FnMut(Option<f32>, Option<f32>) -> (String, Option<f32>, Option<f32>),
    ) {
        // Default: neither key emitted.
        assert!(
            !default_json.contains("min_zoom_to_render"),
            "{label}: default emitted min: {default_json}"
        );
        assert!(
            !default_json.contains("max_zoom_to_render"),
            "{label}: default emitted max: {default_json}"
        );

        // Both-sides authored: both keys present and round-trip.
        let (json, back_min, back_max) = set_pair(Some(authored_min), Some(authored_min * 4.0));
        assert!(json.contains(&format!("\"min_zoom_to_render\":{authored_min}")));
        assert!(json.contains(&format!("\"max_zoom_to_render\":{}", authored_min * 4.0)));
        assert_eq!(back_min, Some(authored_min));
        assert_eq!(back_max, Some(authored_min * 4.0));

        // One-sided: only min present, max absent both in JSON and after round-trip.
        let (json, back_min, back_max) = set_pair(Some(authored_min), None);
        assert!(json.contains(&format!("\"min_zoom_to_render\":{authored_min}")));
        assert!(
            !json.contains("max_zoom_to_render"),
            "{label}: one-sided max leaked: {json}"
        );
        assert_eq!(back_min, Some(authored_min));
        assert!(back_max.is_none());
    }

    let edge_default = serde_json::to_string(&synthetic_edge_with_label(None, None)).unwrap();
    check_pair("MindEdge", &edge_default, 0.5, |min, max| {
        let mut edge = synthetic_edge_with_label(None, None);
        edge.min_zoom_to_render = min;
        edge.max_zoom_to_render = max;
        let json = serde_json::to_string(&edge).unwrap();
        let back: MindEdge = serde_json::from_str(&json).unwrap();
        (json, back.min_zoom_to_render, back.max_zoom_to_render)
    });

    let label_default = serde_json::to_string(&EdgeLabelConfig::default()).unwrap();
    check_pair("EdgeLabelConfig", &label_default, 1.5, |min, max| {
        let cfg = EdgeLabelConfig {
            min_zoom_to_render: min,
            max_zoom_to_render: max,
            ..EdgeLabelConfig::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: EdgeLabelConfig = serde_json::from_str(&json).unwrap();
        (json, back.min_zoom_to_render, back.max_zoom_to_render)
    });

    let portal_default = serde_json::to_string(&PortalEndpointState::default()).unwrap();
    check_pair("PortalEndpointState", &portal_default, 0.75, |min, max| {
        let state = PortalEndpointState {
            min_zoom_to_render: min,
            max_zoom_to_render: max,
            ..PortalEndpointState::default()
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: PortalEndpointState = serde_json::from_str(&json).unwrap();
        (json, back.min_zoom_to_render, back.max_zoom_to_render)
    });
}

/// `MindNode` round-trip — node-level pair follows the same
/// pattern; the border inherits the resolved window by
/// construction (see `tree_builder::node_clip`), so there is
/// no separate border serde surface.
#[test]
fn mindnode_zoom_window_round_trips() {
    // Deserialize a minimal node with both zoom fields set and
    // check the pair survives. Constructed as raw JSON rather
    // than a struct literal so this test also pins the on-disk
    // key names (authors grep for `min_zoom_to_render` in the
    // format docs; breaking either name breaks that contract).
    let raw = r##"{
        "id":"0","parent_id":null,
        "position":{"x":0,"y":0},
        "size":{"width":100,"height":100},
        "sections":[{"text":""}],
        "style":{
            "background_color":"#000","frame_color":"#000","text_color":"#fff",
            "shape":"rectangle","corner_radius_percent":0,"frame_thickness":0,
            "show_frame":false,"show_shadow":false
        },
        "layout":{"type":"map","direction":"auto","spacing":0},
        "folded":false,"notes":"","color_schema":null,
        "min_zoom_to_render":0.25,
        "max_zoom_to_render":4.0
    }"##;
    let node: MindNode = serde_json::from_str(raw).expect("parses");
    assert_eq!(node.min_zoom_to_render, Some(0.25));
    assert_eq!(node.max_zoom_to_render, Some(4.0));

    // Reserialize and confirm the pair is preserved.
    let back_json = serde_json::to_string(&node).unwrap();
    assert!(back_json.contains("\"min_zoom_to_render\":0.25"));
    assert!(back_json.contains("\"max_zoom_to_render\":4.0"));
}

/// `node_locations()` yields every node in the map exactly once,
/// stamping each with its `node.id` as the location string. The
/// stamp format is part of the public contract — `maptool verify`
/// emits violations against this string and a future loader-time
/// validator will share the same canonical stamp.
#[test]
fn node_locations_yields_every_node_with_id_stamp() {
    use crate::mindmap::test_helpers::{synthetic_map, synthetic_node_full};
    let map = synthetic_map(
        vec![
            synthetic_node_full("0", None, 0.0, 0.0, 100.0, 50.0, false),
            synthetic_node_full("0.0", Some("0"), 10.0, 10.0, 80.0, 40.0, false),
            synthetic_node_full("0.1", Some("0"), 90.0, 10.0, 80.0, 40.0, false),
        ],
        Vec::new(),
    );

    let locations: Vec<(String, String)> = map.node_locations().map(|(loc, n)| (loc, n.id.clone())).collect();
    assert_eq!(locations.len(), 3);
    // Location stamp must equal the node's id for every yield.
    for (loc, id) in &locations {
        assert_eq!(loc, id);
    }
    // All three node ids appear; HashMap iteration order is
    // unspecified, so check via set membership.
    let ids: std::collections::HashSet<&str> = locations.iter().map(|(_, id)| id.as_str()).collect();
    assert!(ids.contains("0"));
    assert!(ids.contains("0.0"));
    assert!(ids.contains("0.1"));
}

/// `edge_locations()` yields every edge in vector order, stamping
/// each with `"edge[<idx>]"`. Bracket-format is the contract every
/// per-checker previously open-coded; locking it here prevents a
/// silent drift to e.g. `"edges[0]"` from flowing through to
/// downstream tools.
#[test]
fn edge_locations_uses_bracket_index_stamp() {
    use crate::mindmap::test_helpers::{synthetic_edge, synthetic_map, synthetic_node_full};
    let map = synthetic_map(
        vec![
            synthetic_node_full("a", None, 0.0, 0.0, 50.0, 25.0, false),
            synthetic_node_full("b", None, 100.0, 0.0, 50.0, 25.0, false),
            synthetic_node_full("c", None, 200.0, 0.0, 50.0, 25.0, false),
        ],
        vec![
            synthetic_edge("a", "b", "auto", "auto"),
            synthetic_edge("b", "c", "auto", "auto"),
            synthetic_edge("a", "c", "auto", "auto"),
        ],
    );

    let stamps: Vec<String> = map.edge_locations().map(|(loc, _e)| loc).collect();
    assert_eq!(stamps, vec!["edge[0]", "edge[1]", "edge[2]"]);
}

/// `MindSection` defaults round-trip cleanly on a freshly-built
/// node. The default shape — `offset=(0,0)`, `size=None`,
/// `channel=None`, no runs — must serialize to a minimal JSON
/// form (only `text` is serialized) so the on-disk file stays
/// tight for the migration-default case where every node ships
/// exactly one default section. Pins the `skip_serializing_if`
/// contracts on `text_runs`, `offset`, `size`, and `channel`.
#[test]
fn mindsection_defaults_serialize_minimally() {
    let section = MindSection::new_default("hi".into(), Vec::new());
    let json = serde_json::to_string(&section).expect("serializes");
    // Only the `text` field should be present.
    assert!(json.contains("\"text\":\"hi\""), "json: {json}");
    assert!(!json.contains("text_runs"), "empty runs must not serialize");
    assert!(!json.contains("offset"), "default offset must not serialize");
    assert!(!json.contains("\"size\""), "None size must not serialize");
    assert!(!json.contains("channel"), "default channel must not serialize");
    assert!(
        !json.contains("trigger_bindings"),
        "empty trigger_bindings must not serialize"
    );

    // Round-trip: parse the minimal JSON back; defaults must hold.
    let back: MindSection = serde_json::from_str("{\"text\":\"hi\"}").unwrap();
    assert_eq!(back.text, "hi");
    assert!(back.text_runs.is_empty());
    assert_eq!(back.offset.x, 0.0);
    assert_eq!(back.offset.y, 0.0);
    assert!(back.size.is_none());
    assert_eq!(back.channel, None);
    assert!(back.trigger_bindings.is_empty());
}

/// `MindSection::effective_size` returns the explicit pin when
/// set and falls back to `node_size` for fill-parent
/// (`size = None`). Single source of truth shared by the
/// document-side setters and `maptool verify`'s containment
/// rule — drift between them would re-open the C3 hole the
/// helper was extracted to close.
#[test]
fn mindsection_effective_size_falls_back_to_node_size_when_none() {
    let node_size = Size {
        width: 200.0,
        height: 100.0,
    };
    let none_section = MindSection::new_default("a".into(), Vec::new());
    assert_eq!(
        none_section.effective_size(node_size),
        node_size,
        "fill-parent inherits node.size"
    );

    let mut pinned = MindSection::new_default("a".into(), Vec::new());
    pinned.size = Some(Size {
        width: 50.0,
        height: 30.0,
    });
    assert_eq!(
        pinned.effective_size(node_size),
        Size {
            width: 50.0,
            height: 30.0
        },
        "Some-sized honors the explicit pin"
    );
}

/// `MindSection.text` carries `#[serde(default)]` so a hand-
/// edited or partially-converted JSON file with `{"sections":
/// [{}]}` parses cleanly with `text == ""` instead of failing
/// the typed loader with "missing field `text`".
#[test]
fn mindsection_missing_text_field_defaults_to_empty() {
    let s: MindSection = serde_json::from_str("{}").unwrap();
    assert_eq!(s.text, "");
    assert!(s.text_runs.is_empty());
    assert_eq!(s.channel, None);
    assert!(s.trigger_bindings.is_empty());
}

/// `MindSection.channel: Option<usize>` round-trip (Tier-E).
/// Two cases that pre-`Option` were indistinguishable:
///
/// - `None` (the default; falls through to the section's index
///   at tree-build time) skip-serializes — empty on disk.
/// - `Some(0)` (explicit author override) **must** round-trip:
///   the JSON carries `"channel": 0`, and parsing it back
///   yields `Some(0)`, not `None`. Pre-`Option` the bare `usize`
///   collapsed both cases to `0` and the tree builder silently
///   substituted the section index for idx > 0.
#[test]
fn mindsection_channel_option_round_trip() {
    // None ⇒ skip-serialize.
    let none_section = MindSection::new_default("a".into(), Vec::new());
    let none_json = serde_json::to_string(&none_section).unwrap();
    assert!(
        !none_json.contains("channel"),
        "default channel must skip-serialize"
    );
    let parsed_none: MindSection = serde_json::from_str("{\"text\":\"a\"}").unwrap();
    assert_eq!(parsed_none.channel, None, "absent field parses as None");

    // Some(0) ⇒ serialize + round-trip preserves `Some(0)`.
    let mut explicit_zero = MindSection::new_default("a".into(), Vec::new());
    explicit_zero.channel = Some(0);
    let zero_json = serde_json::to_string(&explicit_zero).unwrap();
    assert!(
        zero_json.contains("\"channel\":0"),
        "explicit Some(0) must serialize: {zero_json}"
    );
    let parsed_zero: MindSection = serde_json::from_str(&zero_json).unwrap();
    assert_eq!(
        parsed_zero.channel,
        Some(0),
        "Some(0) must round-trip — pre-Option this collapsed to 0/None"
    );

    // Some(n) ⇒ standard round-trip.
    let mut explicit_n = MindSection::new_default("a".into(), Vec::new());
    explicit_n.channel = Some(7);
    let n_json = serde_json::to_string(&explicit_n).unwrap();
    let parsed_n: MindSection = serde_json::from_str(&n_json).unwrap();
    assert_eq!(parsed_n.channel, Some(7));
}

/// `MindSection.trigger_bindings` is a reserved-but-not-yet-
/// dispatched seam for per-section triggers. Authoring tools
/// stamp it; the runtime ignores it today (whole-node bindings
/// still live on `MindNode.trigger_bindings`). The field must
/// round-trip through serde, and an empty bindings vec must not
/// serialize (skip-empty contract for forward compatibility).
#[test]
fn mindsection_trigger_bindings_round_trip() {
    use crate::mindmap::custom_mutation::{Trigger, TriggerBinding};
    let section = MindSection {
        text: "hi".into(),
        text_runs: Vec::new(),
        offset: Position::default(),
        size: None,
        channel: None,
        trigger_bindings: vec![TriggerBinding {
            trigger: Trigger::OnClick,
            mutation_id: "m_focus".into(),
            contexts: Vec::new(),
        }],
        frame_border: None,
    };
    let json = serde_json::to_string(&section).expect("serializes");
    assert!(json.contains("trigger_bindings"));
    assert!(json.contains("m_focus"));
    let back: MindSection = serde_json::from_str(&json).expect("round-trips");
    assert_eq!(back.trigger_bindings.len(), 1);
    assert_eq!(back.trigger_bindings[0].mutation_id, "m_focus");
}

/// `MindSection.frame_border` is `Option<GlyphBorderConfig>` —
/// `None` (the default) must skip serialization entirely so
/// existing maps round-trip byte-identical, and `Some(cfg)` must
/// preserve every field on the round trip.
#[test]
fn test_mindsection_frame_border_round_trip() {
    use crate::mindmap::model::{CustomBorderGlyphs, GlyphBorderConfig};
    // Default-constructed section: no `frame_border` key in JSON.
    let plain = MindSection::new_default("hi".into(), Vec::new());
    let json = serde_json::to_string(&plain).expect("serializes");
    assert!(
        !json.contains("frame_border"),
        "None must skip serialization: {}",
        json
    );

    // Section with a per-section override survives the round-trip
    // with every field intact, including the per-side `glyphs`
    // payload.
    let mut authored = MindSection::new_default("hi".into(), Vec::new());
    authored.frame_border = Some(GlyphBorderConfig {
        preset: "custom".into(),
        font: Some("Liberation Mono".into()),
        font_size_pt: 11.0,
        color: Some("#ff8800".into()),
        glyphs: Some(CustomBorderGlyphs {
            top: "+=##=+".into(),
            bottom: "─".into(),
            left: "│".into(),
            right: "│".into(),
            top_left: "◆".into(),
            top_right: "◇".into(),
            bottom_left: "◇".into(),
            bottom_right: "◆".into(),
        }),
        padding: 4.0,
        color_palette: Some("rainbow".into()),
        color_palette_field: Some("frame".into()),
    });
    let json = serde_json::to_string(&authored).expect("serializes");
    assert!(json.contains("frame_border"));
    assert!(json.contains("+=##=+"));
    let back: MindSection = serde_json::from_str(&json).expect("round-trips");
    let back_cfg = back.frame_border.expect("frame_border survives");
    assert_eq!(back_cfg.preset, "custom");
    assert_eq!(back_cfg.color.as_deref(), Some("#ff8800"));
    assert_eq!(back_cfg.color_palette.as_deref(), Some("rainbow"));
    let back_glyphs = back_cfg.glyphs.expect("glyphs survive");
    assert_eq!(back_glyphs.top, "+=##=+");
    assert_eq!(back_glyphs.top_left, "◆");
    assert_eq!(back_glyphs.bottom_right, "◆");
}

/// `Canvas.default_section_frame_border` and
/// `default_focused_section_frame_border` round-trip the same way
/// — `None` skips serialization, `Some(cfg)` survives the
/// full serialize → deserialize cycle.
#[test]
fn test_canvas_section_frame_defaults_round_trip() {
    use crate::mindmap::model::{Canvas, GlyphBorderConfig};
    use std::collections::HashMap;

    // Empty canvas: neither field appears in JSON.
    let plain = Canvas {
        background_color: "#000".into(),
        default_border: None,
        default_connection: None,
        default_section_frame_border: None,
        default_focused_section_frame_border: None,
        theme_variables: HashMap::new(),
        theme_variants: HashMap::new(),
    };
    let json = serde_json::to_string(&plain).expect("serializes");
    assert!(
        !json.contains("default_section_frame_border"),
        "None skips: {}",
        json
    );
    assert!(!json.contains("default_focused_section_frame_border"));

    // Authored canvas: both fields land + round-trip.
    let mut authored = plain.clone();
    authored.default_section_frame_border = Some(GlyphBorderConfig {
        preset: "double".into(),
        font: None,
        font_size_pt: 10.0,
        color: Some("#aaaaaa".into()),
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });
    authored.default_focused_section_frame_border = Some(GlyphBorderConfig {
        preset: "heavy".into(),
        font: None,
        font_size_pt: 12.0,
        color: Some("#ffffff".into()),
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });
    let json = serde_json::to_string(&authored).expect("serializes");
    let back: Canvas = serde_json::from_str(&json).expect("round-trips");
    let unfocused = back.default_section_frame_border.expect("unfocused survives");
    let focused = back
        .default_focused_section_frame_border
        .expect("focused survives");
    assert_eq!(unfocused.preset, "double");
    assert_eq!(unfocused.color.as_deref(), Some("#aaaaaa"));
    assert_eq!(focused.preset, "heavy");
    assert_eq!(focused.color.as_deref(), Some("#ffffff"));
}

/// `MindNode.display_text` joins every section's text with `'\n'`
/// — the legacy bridge for export / clipboard / copy paths that
/// want one rendered string per node. Single-section nodes
/// round-trip identically with the pre-section behavior.
#[test]
fn mindnode_display_text_joins_sections() {
    use crate::mindmap::test_helpers::synthetic_node_full;
    let mut node = synthetic_node_full("n", None, 0.0, 0.0, 80.0, 40.0, false);
    node.sections = vec![
        MindSection::new_default("alpha".into(), Vec::new()),
        MindSection::new_default("beta".into(), Vec::new()),
        MindSection::new_default("gamma".into(), Vec::new()),
    ];
    assert_eq!(node.display_text(), "alpha\nbeta\ngamma");
}

/// Empty maps yield empty iterators — the no-op base case the
/// checker call sites rely on (no location-stamp leakage when
/// the map carries zero nodes / zero edges).
#[test]
fn node_and_edge_locations_empty_on_blank_map() {
    let map = MindMap::new_blank("blank");
    assert_eq!(map.node_locations().count(), 0);
    assert_eq!(map.edge_locations().count(), 0);
}

// Defense in depth (P0-05): `loader::load_from_str` rejects a
// `parent_id` cycle before a `MindMap` value ever exists, but these
// three walkers can in principle be reached with a cycle built by
// other means (e.g. a future in-memory mutation bug), so each one
// caps its own walk instead of trusting the loader alone. These
// tests build the cyclic map directly via `synthetic_map` —
// bypassing the loader entirely — to exercise that cap.

#[test]
fn is_hidden_by_fold_does_not_hang_on_parent_cycle() {
    let a = synthetic_node_full("a", Some("b"), 0.0, 0.0, 10.0, 10.0, false);
    let b = synthetic_node_full("b", Some("a"), 0.0, 0.0, 10.0, 10.0, false);
    let map = synthetic_map(vec![a, b], vec![]);
    let node = map.nodes.get("a").unwrap();
    // Must return rather than loop forever; the guarded default is "not hidden".
    assert!(!map.is_hidden_by_fold(node));
}

#[test]
fn all_descendants_does_not_overflow_on_parent_cycle() {
    let a = synthetic_node_full("a", Some("b"), 0.0, 0.0, 10.0, 10.0, false);
    let b = synthetic_node_full("b", Some("a"), 0.0, 0.0, 10.0, 10.0, false);
    let map = synthetic_map(vec![a, b], vec![]);
    // Must return a bounded result rather than recurse forever.
    let descendants = map.all_descendants("a");
    assert!(descendants.len() <= map.nodes.len());
}

#[test]
fn is_ancestor_or_self_does_not_hang_on_parent_cycle() {
    let a = synthetic_node_full("a", Some("b"), 0.0, 0.0, 10.0, 10.0, false);
    let b = synthetic_node_full("b", Some("a"), 0.0, 0.0, 10.0, 10.0, false);
    let c = synthetic_node_full("c", None, 0.0, 0.0, 10.0, 10.0, false);
    let map = synthetic_map(vec![a, b, c], vec![]);
    // "c" is not part of the cycle and not an ancestor of "a"; the
    // walk up "a"'s cyclic parent chain must terminate and say so.
    assert!(!map.is_ancestor_or_self("c", "a"));
}

/// `ChildIndex` exposes the same root/child order as `root_nodes` /
/// `children_of` but in O(N) total instead of O(N²).
#[test]
fn child_index_matches_root_nodes_and_children_of_order() {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    let index = map.child_index();

    let expected_roots = map.root_nodes();
    assert_eq!(index.roots().len(), expected_roots.len());
    for (a, b) in index.roots().iter().zip(expected_roots.iter()) {
        assert_eq!(a.id, b.id);
    }

    for node in map.nodes.values() {
        let expected = map.children_of(&node.id);
        let actual = index.children_of(&node.id);
        assert_eq!(
            actual.len(),
            expected.len(),
            "children_of mismatch for {}",
            node.id
        );
        for (a, b) in actual.iter().zip(expected.iter()) {
            assert_eq!(a.id, b.id);
        }
    }
}

/// `ChildIndex::all_descendant_ids` matches `MindMap::all_descendants`
/// without rebuilding the index on every recursive step.
#[test]
fn child_index_all_descendant_ids_matches_all_descendants() {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    let index = map.child_index();
    for node in map.nodes.values() {
        let expected = map.all_descendants(&node.id);
        let mut actual = index.all_descendant_ids(&node.id, map.nodes.len());
        actual.sort();
        let mut expected_sorted = expected;
        expected_sorted.sort();
        assert_eq!(actual, expected_sorted, "descendants mismatch for {}", node.id);
    }
}

/// `fold_hidden_set` captures every node under a folded ancestor and
/// nothing else.
#[test]
fn fold_hidden_set_hides_descendants_of_folded_nodes() {
    let mut root = synthetic_node_full("0", None, 0.0, 0.0, 10.0, 10.0, false);
    root.folded = true;
    let child = synthetic_node_full("0.0", Some("0"), 0.0, 0.0, 10.0, 10.0, false);
    let grand = synthetic_node_full("0.0.0", Some("0.0"), 0.0, 0.0, 10.0, 10.0, false);
    let other_root = synthetic_node_full("1", None, 0.0, 0.0, 10.0, 10.0, false);
    let map = synthetic_map(vec![root, child, grand, other_root], vec![]);

    let hidden: std::collections::HashSet<&str> = map.fold_hidden_set();
    assert!(hidden.contains("0.0"), "child of folded root should be hidden");
    assert!(
        hidden.contains("0.0.0"),
        "grandchild of folded root should be hidden"
    );
    assert!(!hidden.contains("0"), "folded root itself is visible");
    assert!(!hidden.contains("1"), "unrelated root is visible");
}

/// `fold_hidden_set` tolerates a parent cycle by treating the cycle
/// nodes as visible rather than hanging.
#[test]
fn fold_hidden_set_does_not_hang_on_parent_cycle() {
    let a = synthetic_node_full("a", Some("b"), 0.0, 0.0, 10.0, 10.0, false);
    let b = synthetic_node_full("b", Some("a"), 0.0, 0.0, 10.0, 10.0, false);
    let map = synthetic_map(vec![a, b], vec![]);
    let hidden = map.fold_hidden_set();
    assert!(hidden.is_empty());
}

/// Nodes whose parent_id is missing from the map are treated as
/// root-like by the loader; folding such an orphan must still hide
/// its descendants.
#[test]
fn fold_hidden_set_traverses_orphan_component() {
    let mut orphan = synthetic_node_full("0", Some("missing"), 0.0, 0.0, 10.0, 10.0, false);
    orphan.folded = true;
    let child = synthetic_node_full("0.0", Some("0"), 0.0, 0.0, 10.0, 10.0, false);
    let map = synthetic_map(vec![orphan, child], vec![]);
    let hidden = map.fold_hidden_set();
    assert!(hidden.contains("0.0"), "child of folded orphan should be hidden");
    assert!(!hidden.contains("0"), "folded orphan itself is visible");
}

// ── Edge color cascade (`MindEdge::*_color`) ──────────────────
//
// The three channels' precedence used to be re-typed at six call
// sites across two crates. These pin the cascade the helpers now
// own; the scene builder and the document's clipboard resolvers
// read through them.

/// With no overrides anywhere, every channel bottoms out at
/// `edge.color` — the field the model always carries, so a
/// resolver can never hand back an empty string.
#[test]
fn test_edge_colors_bottom_out_at_edge_color() {
    let edge = synthetic_edge("a", "b", "auto", "auto");
    let canvas = blank_canvas();
    assert_eq!(edge.body_color(&canvas), "#fff");
    assert_eq!(edge.label_color(&canvas), "#fff");
    assert_eq!(edge.portal_endpoint_color(&canvas, None), "#fff");
    assert_eq!(edge.portal_endpoint_text_color(&canvas, None), "#fff");
}

/// The canvas default connection supplies the body color for an
/// edge that has not forked a `glyph_connection` of its own, and
/// every downstream channel inherits it.
#[test]
fn test_edge_body_color_falls_back_to_canvas_default_connection() {
    let edge = synthetic_edge("a", "b", "auto", "auto");
    let mut canvas = blank_canvas();
    canvas.default_connection = Some(GlyphConnectionConfig {
        color: Some("#canvas".into()),
        ..GlyphConnectionConfig::default()
    });
    assert_eq!(edge.body_color(&canvas), "#canvas");
    assert_eq!(edge.label_color(&canvas), "#canvas");
}

/// **Struct-level, not field-level.** Once an edge forks a
/// `glyph_connection`, the canvas default drops out of the
/// cascade entirely — even for `color`, which the fork left at
/// `None`. Resolving field-by-field would silently re-attach a
/// forked edge to the canvas default; this is the regression
/// guard for that.
#[test]
fn test_edge_body_color_ignores_canvas_default_once_forked() {
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.glyph_connection = Some(GlyphConnectionConfig::default());
    let mut canvas = blank_canvas();
    canvas.default_connection = Some(GlyphConnectionConfig {
        color: Some("#canvas".into()),
        ..GlyphConnectionConfig::default()
    });
    assert_eq!(
        edge.body_color(&canvas),
        "#fff",
        "a forked edge with color=None uses edge.color, not the canvas default"
    );
}

/// A per-edge `glyph_connection.color` wins over both the canvas
/// default and `edge.color`.
#[test]
fn test_edge_body_color_prefers_glyph_connection_override() {
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.glyph_connection = Some(GlyphConnectionConfig {
        color: Some("#body".into()),
        ..GlyphConnectionConfig::default()
    });
    let mut canvas = blank_canvas();
    canvas.default_connection = Some(GlyphConnectionConfig {
        color: Some("#canvas".into()),
        ..GlyphConnectionConfig::default()
    });
    assert_eq!(edge.body_color(&canvas), "#body");
}

/// The label channel detaches from the body when it authors its
/// own color, and follows it otherwise.
#[test]
fn test_edge_label_color_override_detaches_from_body() {
    let mut edge = synthetic_edge("a", "b", "auto", "auto");
    edge.glyph_connection = Some(GlyphConnectionConfig {
        color: Some("#body".into()),
        ..GlyphConnectionConfig::default()
    });
    let canvas = blank_canvas();
    assert_eq!(edge.label_color(&canvas), "#body");
    edge.label_config = Some(EdgeLabelConfig {
        color: Some("#label".into()),
        ..EdgeLabelConfig::default()
    });
    assert_eq!(edge.label_color(&canvas), "#label");
    assert_eq!(edge.body_color(&canvas), "#body", "body is unaffected");
}

/// A portal endpoint's icon color overrides the body cascade,
/// and its text color overrides the icon — the two portal
/// channels are independent so a colored badge can carry a
/// differently-colored annotation.
#[test]
fn test_portal_endpoint_color_channels_are_independent() {
    let mut edge = synthetic_portal_edge("a", "b", "#edge");
    let canvas = blank_canvas();
    edge.portal_from = Some(PortalEndpointState {
        color: Some("#icon".into()),
        ..PortalEndpointState::default()
    });
    let from = portal_endpoint_state(&edge, "a");
    assert_eq!(edge.portal_endpoint_color(&canvas, from), "#icon");
    assert_eq!(
        edge.portal_endpoint_text_color(&canvas, from),
        "#icon",
        "text with no override follows the icon"
    );

    edge.portal_from = Some(PortalEndpointState {
        color: Some("#icon".into()),
        text_color: Some("#text".into()),
        ..PortalEndpointState::default()
    });
    let from = portal_endpoint_state(&edge, "a");
    assert_eq!(edge.portal_endpoint_color(&canvas, from), "#icon");
    assert_eq!(edge.portal_endpoint_text_color(&canvas, from), "#text");
}

/// An endpoint with no state at all inherits the body cascade on
/// both channels, and one endpoint's override never leaks to the
/// other side.
#[test]
fn test_portal_endpoint_color_does_not_leak_across_endpoints() {
    let mut edge = synthetic_portal_edge("a", "b", "#edge");
    let canvas = blank_canvas();
    edge.portal_from = Some(PortalEndpointState {
        color: Some("#icon".into()),
        ..PortalEndpointState::default()
    });
    let to = portal_endpoint_state(&edge, "b");
    assert!(to.is_none());
    assert_eq!(edge.portal_endpoint_color(&canvas, to), "#edge");
    assert_eq!(edge.portal_endpoint_text_color(&canvas, to), "#edge");
}
