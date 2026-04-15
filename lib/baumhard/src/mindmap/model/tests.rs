//! Mindmap-model tests: ancestry, portals, connection config
//! resolution, label position round-trips. Kept in a sibling file so
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

    // "Lord God" (348068464) has children — descendants should include them all
    let children = map.children_of("348068464");
    assert!(!children.is_empty(), "Lord God should have children");

    let descendants = map.all_descendants("348068464");
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
    let cfg = GlyphConnectionConfig::default();
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

// Session 6D Phase 1: label_position_t + resolved_for helper.

fn synthetic_edge_with_label(label: Option<&str>, pos: Option<f32>) -> MindEdge {
    MindEdge {
        from_id: "a".to_string(),
        to_id: "b".to_string(),
        edge_type: "cross_link".to_string(),
        color: "#fff".to_string(),
        width: 1,
        line_style: 0,
        visible: true,
        label: label.map(|s| s.to_string()),
        label_position_t: pos,
        anchor_from: 0,
        anchor_to: 0,
        control_points: Vec::new(),
        glyph_connection: None,
    }
}

#[test]
fn label_position_t_round_trips_through_json() {
    // Explicit value is preserved.
    let edge = synthetic_edge_with_label(Some("hello"), Some(0.25));
    let json = serde_json::to_string(&edge).unwrap();
    assert!(json.contains("label_position_t"), "json should include the field: {json}");
    let back: MindEdge = serde_json::from_str(&json).unwrap();
    assert_eq!(back.label.as_deref(), Some("hello"));
    assert_eq!(back.label_position_t, Some(0.25));
}

#[test]
fn label_position_t_missing_defaults_to_none() {
    // Older maps without the field must still deserialize.
    let json = r##"{
        "from_id":"a","to_id":"b","type":"cross_link",
        "color":"#fff","width":1,"line_style":0,"visible":true,
        "label":null,"anchor_from":0,"anchor_to":0,"control_points":[]
    }"##;
    let edge: MindEdge = serde_json::from_str(json).unwrap();
    assert_eq!(edge.label_position_t, None);
    // And round-trips back without the field (skip_serializing_if).
    let back_json = serde_json::to_string(&edge).unwrap();
    assert!(
        !back_json.contains("label_position_t"),
        "None should not serialize: {back_json}"
    );
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
// Session 6E — portal data model tests
// ============================================================

fn synthetic_empty_map() -> MindMap {
    MindMap {
        version: "1".to_string(),
        name: "test".to_string(),
        canvas: Canvas {
            background_color: "#000".to_string(),
            default_border: None,
            default_connection: None,
            theme_variables: std::collections::HashMap::new(),
            theme_variants: std::collections::HashMap::new(),
        },
        nodes: std::collections::HashMap::new(),
        edges: Vec::new(),
        custom_mutations: Vec::new(),
        portals: Vec::new(),
    }
}

#[test]
fn column_letter_label_sequence() {
    assert_eq!(column_letter_label(1), "A");
    assert_eq!(column_letter_label(2), "B");
    assert_eq!(column_letter_label(26), "Z");
    assert_eq!(column_letter_label(27), "AA");
    assert_eq!(column_letter_label(28), "AB");
    assert_eq!(column_letter_label(52), "AZ");
    assert_eq!(column_letter_label(53), "BA");
    assert_eq!(column_letter_label(702), "ZZ");
    assert_eq!(column_letter_label(703), "AAA");
}

#[test]
fn portal_pair_round_trips_through_json() {
    let portal = PortalPair {
        endpoint_a: "node-1".to_string(),
        endpoint_b: "node-2".to_string(),
        label: "A".to_string(),
        glyph: "\u{25C8}".to_string(),
        color: "var(--accent)".to_string(),
        font_size_pt: 18.0,
        font: Some("LiberationSans".to_string()),
    };
    let json = serde_json::to_string(&portal).unwrap();
    assert!(json.contains("node-1"));
    assert!(json.contains("\"label\":\"A\""));
    let back: PortalPair = serde_json::from_str(&json).unwrap();
    assert_eq!(back.endpoint_a, "node-1");
    assert_eq!(back.endpoint_b, "node-2");
    assert_eq!(back.label, "A");
    assert_eq!(back.color, "var(--accent)");
    assert_eq!(back.font_size_pt, 18.0);
    assert_eq!(back.font.as_deref(), Some("LiberationSans"));
}

#[test]
fn portal_pair_font_size_defaults_when_missing() {
    // A portal authored without `font_size_pt` must deserialize with the
    // default 16.0 so older saved maps keep working when this field is
    // added post-hoc.
    let json = r##"{
        "endpoint_a":"a","endpoint_b":"b",
        "label":"A","glyph":"\u25C8","color":"#aa88cc"
    }"##;
    let portal: PortalPair = serde_json::from_str(json).unwrap();
    assert_eq!(portal.font_size_pt, 16.0);
    assert_eq!(portal.font, None);
}

#[test]
fn portals_missing_deserializes_empty() {
    // Maps authored before Session 6E omit the `portals` field
    // entirely. `#[serde(default)]` must give them an empty vec so
    // they keep loading cleanly.
    let map = loader::load_from_file(&test_map_path()).unwrap();
    assert!(map.portals.is_empty(), "pre-6E maps should have no portals");
}

#[test]
fn portals_empty_vec_skipped_in_serialize() {
    // A fresh map with no portals must not write the field so the
    // on-disk JSON shape for existing maps is byte-stable.
    let map = synthetic_empty_map();
    let json = serde_json::to_string(&map).unwrap();
    assert!(
        !json.contains("\"portals\""),
        "empty portals should not appear in JSON: {json}"
    );
}

#[test]
fn next_portal_label_picks_lowest_unused() {
    let mut map = synthetic_empty_map();
    assert_eq!(map.next_portal_label(), "A");

    map.portals.push(PortalPair {
        endpoint_a: "x".to_string(), endpoint_b: "y".to_string(),
        label: "A".to_string(), glyph: "\u{25C8}".to_string(),
        color: "#aa88cc".to_string(), font_size_pt: 16.0, font: None,
    });
    assert_eq!(map.next_portal_label(), "B");

    // Fill in "B" — next should be "C".
    map.portals.push(PortalPair {
        endpoint_a: "x".to_string(), endpoint_b: "y".to_string(),
        label: "B".to_string(), glyph: "\u{25C6}".to_string(),
        color: "#aa88cc".to_string(), font_size_pt: 16.0, font: None,
    });
    assert_eq!(map.next_portal_label(), "C");

    // Skip "C", use "D" — the gap at "C" should be reused first.
    map.portals.last_mut().unwrap().label = "D".to_string();
    assert_eq!(map.next_portal_label(), "B");
}

#[test]
fn next_portal_label_wraps_to_double_letter() {
    let mut map = synthetic_empty_map();
    // Fill A..Z.
    for n in 1u64..=26 {
        map.portals.push(PortalPair {
            endpoint_a: "x".to_string(), endpoint_b: "y".to_string(),
            label: column_letter_label(n),
            glyph: "\u{25C8}".to_string(),
            color: "#aa88cc".to_string(),
            font_size_pt: 16.0,
            font: None,
        });
    }
    assert_eq!(map.next_portal_label(), "AA");
}

#[test]
fn portal_glyph_presets_are_nonempty_and_unique() {
    assert!(!PORTAL_GLYPH_PRESETS.is_empty());
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for g in PORTAL_GLYPH_PRESETS {
        assert!(seen.insert(*g), "glyph preset {g} duplicated");
    }
}
