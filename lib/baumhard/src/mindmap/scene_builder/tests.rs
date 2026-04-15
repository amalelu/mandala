//! Scene-builder tests — `point_inside_any_node` boundary
//! cases, synthetic maps, and full `build_scene` / cache
//! integration coverage.

use super::*;
use super::builder::point_inside_any_node;
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
fn test_point_inside_any_node_strictly_inside() {
    let aabbs = vec![
        (Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)),
    ];
    assert!(point_inside_any_node(Vec2::new(50.0, 25.0), &aabbs));
}

#[test]
fn test_point_inside_any_node_on_boundary_is_not_inside() {
    // A point exactly on the right edge should NOT be considered
    // inside — this is where connection anchor points live.
    let aabbs = vec![
        (Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)),
    ];
    assert!(!point_inside_any_node(Vec2::new(100.0, 25.0), &aabbs));
    assert!(!point_inside_any_node(Vec2::new(0.0, 25.0), &aabbs));
    assert!(!point_inside_any_node(Vec2::new(50.0, 0.0), &aabbs));
    assert!(!point_inside_any_node(Vec2::new(50.0, 50.0), &aabbs));
}

#[test]
fn test_point_inside_any_node_outside_returns_false() {
    let aabbs = vec![
        (Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)),
    ];
    assert!(!point_inside_any_node(Vec2::new(200.0, 25.0), &aabbs));
    assert!(!point_inside_any_node(Vec2::new(-10.0, 25.0), &aabbs));
}

#[test]
fn test_point_inside_any_node_checks_all_aabbs() {
    let aabbs = vec![
        (Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0)),
        (Vec2::new(100.0, 100.0), Vec2::new(50.0, 50.0)),
    ];
    // Inside the second box
    assert!(point_inside_any_node(Vec2::new(125.0, 125.0), &aabbs));
}

// Shared helpers for the synthetic-map scene tests below.
use crate::mindmap::model::{
    Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, Position, Size,
};

fn synthetic_node(id: &str, x: f64, y: f64, w: f64, h: f64, show_frame: bool) -> MindNode {
    MindNode {
        id: id.to_string(),
        parent_id: None,
        index: 0,
        position: Position { x, y },
        size: Size { width: w, height: h },
        text: id.to_string(),
        text_runs: vec![],
        style: NodeStyle {
            background_color: "#000".into(),
            frame_color: "#fff".into(),
            text_color: "#fff".into(),
            shape_type: 0,
            corner_radius_percent: 0.0,
            frame_thickness: 1.0,
            show_frame,
            show_shadow: false,
            border: None,
        },
        layout: NodeLayout { layout_type: 0, direction: 0, spacing: 0.0 },
        folded: false,
        notes: String::new(),
        color_schema: None,
        trigger_bindings: vec![],
        inline_mutations: vec![],
    }
}

fn synthetic_edge(from: &str, to: &str, anchor_from: i32, anchor_to: i32) -> MindEdge {
    MindEdge {
        from_id: from.to_string(),
        to_id: to.to_string(),
        edge_type: "cross_link".to_string(),
        color: "#fff".to_string(),
        width: 1,
        line_style: 0,
        visible: true,
        label: None,
        label_position_t: None,
        anchor_from,
        anchor_to,
        control_points: vec![],
        glyph_connection: None,
    }
}

fn synthetic_map(nodes_vec: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap {
    use std::collections::HashMap;
    let mut nodes = HashMap::new();
    for n in nodes_vec {
        nodes.insert(n.id.clone(), n);
    }
    MindMap {
        version: "1.0".into(),
        name: "test".into(),
        canvas: Canvas {
            background_color: "#000".into(),
            default_border: None,
            default_connection: None,
            theme_variables: HashMap::new(),
            theme_variants: HashMap::new(),
        },
        nodes,
        edges,
        custom_mutations: vec![],
        portals: vec![],
    }
}

fn themed_node(id: &str, bg: &str, frame: &str, text: &str) -> MindNode {
    let mut n = synthetic_node(id, 0.0, 0.0, 40.0, 40.0, true);
    n.style.background_color = bg.to_string();
    n.style.frame_color = frame.to_string();
    n.style.text_color = text.to_string();
    n
}

#[test]
fn test_scene_background_resolves_theme_variable() {
    use std::collections::HashMap;
    let mut map = synthetic_map(
        vec![synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false)],
        vec![],
    );
    map.canvas.background_color = "var(--bg)".into();
    let mut vars = HashMap::new();
    vars.insert("--bg".into(), "#123456".into());
    map.canvas.theme_variables = vars;

    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.background_color, "#123456");
}

#[test]
fn test_scene_frame_color_resolves_theme_variable() {
    use std::collections::HashMap;
    let mut map = synthetic_map(
        vec![themed_node("a", "#000", "var(--frame)", "#fff")],
        vec![],
    );
    let mut vars = HashMap::new();
    vars.insert("--frame".into(), "#abcdef".into());
    map.canvas.theme_variables = vars;

    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.border_elements.len(), 1);
    // `BorderStyle::default_with_color` stores the color string as-is
    // on the style; check the resolved hex ends up there.
    let border = &scene.border_elements[0];
    assert_eq!(border.border_style.color, "#abcdef");
}

#[test]
fn test_scene_connection_color_resolves_theme_variable() {
    use std::collections::HashMap;
    let mut a = synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false);
    let mut b = synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false);
    a.text = "".into(); // skip text element
    b.text = "".into();
    let mut edge = synthetic_edge("a", "b", 2, 4);
    edge.color = "var(--edge)".into();
    let mut map = synthetic_map(vec![a, b], vec![edge]);
    let mut vars = HashMap::new();
    vars.insert("--edge".into(), "#fedcba".into());
    map.canvas.theme_variables = vars;

    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_elements.len(), 1);
    assert_eq!(scene.connection_elements[0].color, "#fedcba");
}

#[test]
fn test_scene_missing_variable_passes_through_raw() {
    let mut map = synthetic_map(
        vec![synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false)],
        vec![],
    );
    map.canvas.background_color = "var(--missing)".into();
    let scene = build_scene(&map, 1.0);
    // Unknown var is passed through verbatim — downstream consumers
    // decide how to handle it (hex_to_rgba_safe falls back to the
    // fallback color).
    assert_eq!(scene.background_color, "var(--missing)");
}

#[test]
fn test_scene_clips_connection_glyphs_inside_node() {
    // A on the left, B on the right, blocker C directly on the path
    // between them. The A→B connection should skip body glyphs that
    // fall inside C. All three nodes are unframed so only the raw
    // AABB clipping is exercised here.
    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            synthetic_node("c", 180.0, 0.0, 60.0, 40.0, false),
        ],
        vec![synthetic_edge("a", "b", 2, 4)], // right edge of A → left edge of B
    );

    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_elements.len(), 1);
    let conn = &scene.connection_elements[0];

    // No body glyph position should fall strictly inside C's AABB.
    for &(x, y) in &conn.glyph_positions {
        let inside_c = x > 180.5 && x < 239.5 && y > 0.5 && y < 39.5;
        assert!(!inside_c,
            "glyph at ({}, {}) should have been clipped by blocker C",
            x, y);
    }
    assert!(!conn.glyph_positions.is_empty(),
        "some glyphs should remain outside the blocker");
}

#[test]
fn test_scene_clips_connection_glyphs_in_frame_area() {
    // Same A→B→blocker layout but this time C has a visible frame.
    // The border at default 14pt font extends ~8.4 px horizontally and
    // ~14 px vertically past C's AABB, so body glyphs in the expanded
    // region should also be clipped.
    let border_font = 14.0_f32;
    let border_char_w = border_font * 0.6;

    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            synthetic_node("c", 180.0, 0.0, 60.0, 40.0, true),
        ],
        vec![synthetic_edge("a", "b", 2, 4)],
    );

    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_elements.len(), 1);
    let conn = &scene.connection_elements[0];

    // The clip AABB for framed C is expanded by (border_char_w,
    // border_font) on every side. No body glyph should fall inside
    // the expanded region.
    let min_x = 180.0 - border_char_w + 0.5;
    let max_x = 240.0 + border_char_w - 0.5;
    let min_y = 0.0 - border_font + 0.5;
    let max_y = 40.0 + border_font - 0.5;
    for &(x, y) in &conn.glyph_positions {
        let inside_expanded_c =
            x > min_x && x < max_x && y > min_y && y < max_y;
        assert!(!inside_expanded_c,
            "glyph at ({}, {}) should have been clipped by framed C's expanded AABB",
            x, y);
    }
    // Body glyphs should still render in the space between A, C's
    // expanded clip box, and B.
    assert!(!conn.glyph_positions.is_empty(),
        "connection between A and B should still have visible body glyphs outside C's frame");
}

#[test]
fn test_scene_caps_survive_for_unframed_endpoints() {
    // A→B connection with a cap_start glyph configured. Because A and
    // B are unframed, the anchor point sits exactly on A's edge and
    // the cap should render there.
    use crate::mindmap::model::GlyphConnectionConfig;
    let mut edge = synthetic_edge("a", "b", 2, 4);
    edge.glyph_connection = Some(GlyphConnectionConfig {
        body: "·".into(),
        cap_start: Some("►".into()),
        cap_end: Some("◄".into()),
        font: None,
        font_size_pt: 12.0,
        color: None,
        spacing: 0.0,
        ..GlyphConnectionConfig::default()
    });
    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
        ],
        vec![edge],
    );
    let scene = build_scene(&map, 1.0);
    let conn = &scene.connection_elements[0];
    assert!(conn.cap_start.is_some(),
        "cap_start should survive for unframed source");
    assert!(conn.cap_end.is_some(),
        "cap_end should survive for unframed target");
}

#[test]
fn test_scene_caps_clipped_for_framed_endpoints() {
    // A→B connection where the target B has a visible frame. The
    // cap_end sits on B's node edge, which is STRICTLY inside B's
    // frame-expanded clip AABB, so it should be dropped — otherwise
    // the cap would render in the visible border area.
    use crate::mindmap::model::GlyphConnectionConfig;
    let mut edge = synthetic_edge("a", "b", 2, 4);
    edge.glyph_connection = Some(GlyphConnectionConfig {
        body: "·".into(),
        cap_start: Some("►".into()),
        cap_end: Some("◄".into()),
        font: None,
        font_size_pt: 12.0,
        color: None,
        spacing: 0.0,
        ..GlyphConnectionConfig::default()
    });
    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, true), // framed!
        ],
        vec![edge],
    );
    let scene = build_scene(&map, 1.0);
    let conn = &scene.connection_elements[0];
    // Source is unframed — cap_start still shows at A's right edge.
    assert!(conn.cap_start.is_some(),
        "cap_start should survive for unframed source");
    // Target is framed — cap_end falls inside the expanded clip AABB.
    assert!(conn.cap_end.is_none(),
        "cap_end should be clipped when target has a visible frame");
}

// --- Phase B cache tests --------------------------------------------

fn two_node_edge_map() -> MindMap {
    synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
        ],
        vec![synthetic_edge("a", "b", 2, 4)],
    )
}

#[test]
fn test_cache_populated_on_first_build() {
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);

    assert_eq!(scene.connection_elements.len(), 1);
    assert_eq!(cache.len(), 1);
    let key = EdgeKey::new("a", "b", "cross_link");
    assert!(cache.get(&key).is_some());
    assert_eq!(cache.edges_touching("a"), std::slice::from_ref(&key));
    assert_eq!(cache.edges_touching("b"), std::slice::from_ref(&key));
}

#[test]
fn test_cache_hit_preserves_sample_identity() {
    // Two builds with empty offsets — the second one should serve
    // from cache. We verify the cache by mutating the cached entry in
    // place between builds and observing that the mutation flows into
    // the second build's output. If the second build had re-sampled,
    // it would have overwritten our mutation with fresh geometry.
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);

    // Mutate the cached entry so we can see whether build #2 read it.
    let key = EdgeKey::new("a", "b", "cross_link");
    // Replace with a sentinel entry with no positions and a unique
    // body glyph. If the cache is used, the second build's
    // ConnectionElement body_glyph will match.
    cache.insert(
        key.clone(),
        CachedConnection {
            pre_clip_positions: vec![Vec2::new(200.0, 20.0)],
            cap_start: None,
            cap_end: None,
            body_glyph: "SENTINEL".into(),
            font: None,
            font_size_pt: 12.0,
            color: "#ff00ff".into(),
        },
    );

    let second = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);
    assert_eq!(second.connection_elements.len(), 1);
    let conn = &second.connection_elements[0];
    assert_eq!(conn.body_glyph, "SENTINEL",
        "cache-hit path should have used the stored entry");
    assert_eq!(conn.color, "#ff00ff");
    // Single cached pre-clip point should have survived the clip
    // filter (it's outside both nodes).
    assert_eq!(conn.glyph_positions.len(), 1);
}

#[test]
fn test_cache_invalidated_on_endpoint_offset() {
    // If endpoint `a` moves, the a↔b edge must be re-sampled — we
    // should observe fresh `body_glyph` on the element, not the
    // sentinel we stashed in the cache.
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);

    let key = EdgeKey::new("a", "b", "cross_link");
    cache.insert(
        key.clone(),
        CachedConnection {
            pre_clip_positions: vec![],
            cap_start: None,
            cap_end: None,
            body_glyph: "SENTINEL".into(),
            font: None,
            font_size_pt: 12.0,
            color: "#ff00ff".into(),
        },
    );

    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (10.0, 0.0));
    let second = build_scene_with_cache(&map, &offsets, None, None, None, None, None, &mut cache, 1.0);
    let conn = &second.connection_elements[0];
    assert_ne!(conn.body_glyph, "SENTINEL",
        "endpoint-moved edge should have been re-sampled");
    // The cache should contain the freshly-resampled entry now.
    let refreshed = cache.get(&key).unwrap();
    assert_ne!(refreshed.body_glyph, "SENTINEL");
    assert!(!refreshed.pre_clip_positions.is_empty());
}

#[test]
fn test_cache_preserves_unrelated_edge_under_drag() {
    // Two edges: a↔b (long) and c↔d (short). Drag node `a`. The c↔d
    // edge should NOT be re-sampled; its cache entry should remain as
    // our sentinel.
    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            synthetic_node("c", 0.0, 300.0, 40.0, 40.0, false),
            synthetic_node("d", 400.0, 300.0, 40.0, 40.0, false),
        ],
        vec![
            synthetic_edge("a", "b", 2, 4),
            synthetic_edge("c", "d", 2, 4),
        ],
    );
    let mut cache = SceneConnectionCache::new();
    let _first = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);

    let cd_key = EdgeKey::new("c", "d", "cross_link");
    cache.insert(
        cd_key.clone(),
        CachedConnection {
            pre_clip_positions: vec![Vec2::new(200.0, 320.0)],
            cap_start: None,
            cap_end: None,
            body_glyph: "STABLE_SENTINEL".into(),
            font: None,
            font_size_pt: 12.0,
            color: "#00ff00".into(),
        },
    );

    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (5.0, 0.0));
    let second = build_scene_with_cache(&map, &offsets, None, None, None, None, None, &mut cache, 1.0);

    // Find the c↔d connection element and verify it came from the
    // cache unchanged.
    let cd_elem = second
        .connection_elements
        .iter()
        .find(|e| e.edge_key == cd_key)
        .expect("c↔d element should exist");
    assert_eq!(cd_elem.body_glyph, "STABLE_SENTINEL",
        "unrelated edge should have been served from cache, not re-sampled");

    // The a↔b edge should have been re-sampled.
    let ab_key = EdgeKey::new("a", "b", "cross_link");
    let ab_elem = second
        .connection_elements
        .iter()
        .find(|e| e.edge_key == ab_key)
        .expect("a↔b element should exist");
    assert_ne!(ab_elem.body_glyph, "SENTINEL");
}

#[test]
fn test_cache_clip_reruns_against_fresh_aabbs() {
    // Governing-invariant correctness: even when an edge is served
    // from cache, the clip filter must run against the current
    // frame's `node_aabbs`. Here, a stable a↔b edge has a blocker
    // node `c` in the middle. Moving `c` through the edge should
    // change which glyphs survive clipping, even though a↔b itself
    // is served from cache.
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            // Blocker far above the connection — no clip effect yet.
            synthetic_node("c", 180.0, -500.0, 60.0, 40.0, false),
        ],
        vec![synthetic_edge("a", "b", 2, 4)],
    );

    let mut cache = SceneConnectionCache::new();
    let first = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);
    let first_count = first.connection_elements[0].glyph_positions.len();

    // Now move `c` into the middle of the connection — use a drag
    // offset. `a↔b` is NOT in the dirty set (endpoints didn't move),
    // so it hits the cache path, but the clip filter must still
    // notice `c`'s new position.
    let mut offsets = HashMap::new();
    offsets.insert("c".to_string(), (0.0, 500.0));
    let second = build_scene_with_cache(&map, &offsets, None, None, None, None, None, &mut cache, 1.0);
    let second_count = second.connection_elements[0].glyph_positions.len();
    assert!(second_count < first_count,
        "moving c through the edge should reduce post-clip glyph count: {} → {}",
        first_count, second_count);

    // Now move `c` back out of the way via a model edit + full rebuild.
    map.nodes.get_mut("c").unwrap().position.y = -500.0;
    let third = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);
    assert_eq!(third.connection_elements[0].glyph_positions.len(), first_count);
}

#[test]
fn test_cache_evicts_deleted_edges() {
    let mut map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);
    let key = EdgeKey::new("a", "b", "cross_link");
    assert!(cache.get(&key).is_some());

    // Remove the edge from the model and rebuild.
    map.edges.clear();
    let second = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);
    assert!(second.connection_elements.is_empty());
    assert!(cache.get(&key).is_none(),
        "deleted edge should be evicted from cache");
}

#[test]
fn test_connection_element_edge_key_always_populated() {
    // Sanity: every ConnectionElement emitted by the cache-aware
    // builder carries a valid EdgeKey matching the source MindEdge.
    // The renderer's keyed buffer map is keyed off this; a missing
    // or wrong edge_key would silently break the incremental path.
    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            synthetic_node("c", 0.0, 200.0, 40.0, 40.0, false),
        ],
        vec![
            synthetic_edge("a", "b", 2, 4),
            synthetic_edge("b", "c", 2, 4),
        ],
    );
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);
    assert_eq!(scene.connection_elements.len(), 2);
    let ab = EdgeKey::new("a", "b", "cross_link");
    let bc = EdgeKey::new("b", "c", "cross_link");
    let keys: Vec<&EdgeKey> =
        scene.connection_elements.iter().map(|e| &e.edge_key).collect();
    assert!(keys.contains(&&ab));
    assert!(keys.contains(&&bc));
}

#[test]
fn test_second_cache_hit_produces_identical_output() {
    // Regression guard: build twice with no changes; the two scenes
    // must have byte-equivalent connection_element glyph_positions
    // (same count, same coordinates, same body glyph). This
    // verifies the cache-hit read path returns the same element as
    // a fresh build would.
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let first = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);
    let second = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);

    assert_eq!(
        first.connection_elements.len(),
        second.connection_elements.len(),
    );
    let a = &first.connection_elements[0];
    let b = &second.connection_elements[0];
    assert_eq!(a.edge_key, b.edge_key);
    assert_eq!(a.glyph_positions, b.glyph_positions);
    assert_eq!(a.body_glyph, b.body_glyph);
    assert_eq!(a.color, b.color);
    assert_eq!(a.font_size_pt, b.font_size_pt);
}

#[test]
fn test_cache_is_empty_after_new() {
    let cache = SceneConnectionCache::new();
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
}

#[test]
fn test_fold_hidden_edge_does_not_populate_cache() {
    // When an endpoint is hidden by fold state, the edge is skipped
    // entirely — it should not appear in the output OR the cache.
    let mut a = synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false);
    let mut b_child = synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false);
    b_child.parent_id = Some("a".to_string());
    a.folded = true; // hides b
    let edge = synthetic_edge("a", "b", 2, 4);
    let map = synthetic_map(vec![a, b_child], vec![edge]);

    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);
    assert!(scene.connection_elements.is_empty(),
        "folded edge should be skipped");
    assert!(cache.is_empty(),
        "folded edge should not appear in cache");
}

#[test]
fn test_cache_selection_change_does_not_invalidate() {
    // Build with no selection → cache populated with the resolved
    // color. Build again with the edge selected → cache entry should
    // not be rewritten; the element's color should still reflect the
    // selection override.
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = build_scene_with_cache(&map, &HashMap::new(), None, None, None, None, None, &mut cache, 1.0);
    let key = EdgeKey::new("a", "b", "cross_link");
    let stored_color = cache.get(&key).unwrap().color.clone();

    // Inject a sentinel body_glyph into the cache so we can detect
    // whether the cache path was taken on the second build.
    cache.insert(
        key.clone(),
        CachedConnection {
            pre_clip_positions: vec![Vec2::new(200.0, 20.0)],
            cap_start: None,
            cap_end: None,
            body_glyph: "SENTINEL".into(),
            font: None,
            font_size_pt: 12.0,
            color: stored_color.clone(),
        },
    );

    let second = build_scene_with_cache(
        &map,
        &HashMap::new(),
        Some(("a", "b", "cross_link")),
        None,
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    let conn = &second.connection_elements[0];
    assert_eq!(conn.body_glyph, "SENTINEL",
        "selection change should not have dropped the cache");
    assert_eq!(conn.color, SELECTED_EDGE_COLOR,
        "selected element should pick up the highlight color");
    // And the cache's stored color should be unchanged (still the
    // pre-selection value).
    assert_eq!(cache.get(&key).unwrap().color, stored_color);
}

#[test]
fn test_scene_build_still_works_on_real_map() {
    // Smoke test: loading the testament map and building a scene
    // should not crash, and connections should still render (the
    // clipping filter should not wipe out every glyph).
    let map = loader::load_from_file(&test_map_path()).unwrap();
    let scene = build_scene(&map, 1.0);
    assert!(!scene.text_elements.is_empty());
    assert!(!scene.connection_elements.is_empty());
    // At least one connection should have a non-empty glyph list.
    let any_with_glyphs = scene.connection_elements.iter()
        .any(|c| !c.glyph_positions.is_empty());
    assert!(any_with_glyphs,
        "at least one connection should have un-clipped glyphs");
}

// ---------------------------------------------------------------------
// Session 6C: edge handle emission
// ---------------------------------------------------------------------

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

#[test]
fn test_label_element_emitted_for_edge_with_label() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false),
    ];
    let mut edge = synthetic_edge("a", "b", 0, 0);
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
    let edge = synthetic_edge("a", "b", 0, 0);
    let map = synthetic_map(nodes.clone(), vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_label_elements.len(), 0);

    // label = Some("") → no element (empty-string special case).
    let mut edge = synthetic_edge("a", "b", 0, 0);
    edge.label = Some(String::new());
    let map = synthetic_map(nodes, vec![edge]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.connection_label_elements.len(), 0);
}

#[test]
fn test_label_position_follows_label_position_t() {
    // Horizontal edge from (0,0)+40x40 to (1000,0)+40x40 — center line.
    // At t=0, label should sit near the from-anchor; at t=1, near the
    // to-anchor; midpoints differ substantially.
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
        synthetic_node("b", 1000.0, 0.0, 40.0, 40.0, false),
    ];
    let make = |t: f32| {
        let mut e = synthetic_edge("a", "b", 0, 0);
        e.label = Some("x".to_string());
        e.label_position_t = Some(t);
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
    let mut edge = synthetic_edge("a", "b", 0, 0);
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
    let mut edge = synthetic_edge("a", "b", 0, 0);
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

// ====================================================================
// Session 6E — Portal marker emission
// ====================================================================

use crate::mindmap::model::PortalPair;

fn synthetic_portal(label: &str, a: &str, b: &str, color: &str) -> PortalPair {
    PortalPair {
        endpoint_a: a.to_string(),
        endpoint_b: b.to_string(),
        label: label.to_string(),
        glyph: "\u{25C8}".to_string(),
        color: color.to_string(),
        font_size_pt: 16.0,
        font: None,
    }
}

#[test]
fn portal_emits_two_elements_per_pair() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 500.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.portals.push(synthetic_portal("A", "a", "b", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.portal_elements.len(), 2);
    let ids: Vec<&str> = scene.portal_elements.iter()
        .map(|e| e.endpoint_node_id.as_str())
        .collect();
    assert!(ids.contains(&"a"));
    assert!(ids.contains(&"b"));
    // Both markers share the same portal_ref identity.
    assert_eq!(scene.portal_elements[0].portal_ref, scene.portal_elements[1].portal_ref);
}

#[test]
fn portal_skipped_when_endpoint_missing_from_map() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.portals.push(synthetic_portal("A", "a", "ghost", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert!(scene.portal_elements.is_empty(),
        "missing endpoint should silently drop the pair");
}

#[test]
fn portal_skipped_when_either_endpoint_hidden_by_fold() {
    // A parent holding a folded child — the child is hidden by
    // fold from its ancestor. A portal pointing into a folded
    // subtree has no visible anchor, so the pair must be skipped.
    let mut root = synthetic_node("root", 0.0, 0.0, 60.0, 40.0, false);
    root.folded = true;
    let mut child = synthetic_node("child", 200.0, 0.0, 60.0, 40.0, false);
    child.parent_id = Some("root".to_string());
    let other = synthetic_node("other", 500.0, 0.0, 60.0, 40.0, false);
    let mut map = synthetic_map(vec![root, child, other], vec![]);
    map.portals.push(synthetic_portal("A", "child", "other", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    assert!(scene.portal_elements.is_empty(),
        "portal should be dropped when one endpoint is hidden by fold");
}

#[test]
fn portal_color_resolves_through_theme_variable() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.canvas.theme_variables.insert(
        "--accent".to_string(), "#ff00aa".to_string(),
    );
    map.portals.push(synthetic_portal("A", "a", "b", "var(--accent)"));
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.portal_elements[0].color, "#ff00aa",
        "var(--accent) must resolve through theme_variables");
    assert_eq!(scene.portal_elements[1].color, "#ff00aa");
}

#[test]
fn selected_portal_rendered_with_highlight_color() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 200.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.portals.push(synthetic_portal("A", "a", "b", "#aa88cc"));
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(
        &map,
        &HashMap::new(),
        None,
        Some(("A", "a", "b")),
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
    map.portals.push(synthetic_portal("A", "a", "b", "#aa88cc"));
    let scene = build_scene(&map, 1.0);
    // Find the marker keyed to endpoint "a".
    let marker_a = scene.portal_elements.iter()
        .find(|e| e.endpoint_node_id == "a")
        .expect("marker for endpoint a");
    // Node "a" sits at (100, 200) with size (80, 40). The marker
    // should float above the node's top edge (y < 200) and be
    // horizontally clustered on the right half of the node.
    assert!(marker_a.position.1 < 200.0,
        "marker y {} should be above node top 200", marker_a.position.1);
    assert!(marker_a.position.0 > 100.0 + 80.0 * 0.5,
        "marker x {} should be on the right half of the node", marker_a.position.0);
}

#[test]
fn portal_marker_follows_drag_offsets() {
    let nodes = vec![
        synthetic_node("a", 0.0, 0.0, 60.0, 40.0, false),
        synthetic_node("b", 500.0, 0.0, 60.0, 40.0, false),
    ];
    let mut map = synthetic_map(nodes, vec![]);
    map.portals.push(synthetic_portal("A", "a", "b", "#aa88cc"));

    // Build a baseline scene with no offsets, then an offset scene
    // and assert the marker moved by exactly the offset amount.
    let baseline = build_scene(&map, 1.0);
    let baseline_a = baseline.portal_elements.iter()
        .find(|e| e.endpoint_node_id == "a")
        .expect("marker for endpoint a in baseline");

    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (100.0f32, 50.0f32));
    let dragged = build_scene_with_offsets(&map, &offsets, 1.0);
    let dragged_a = dragged.portal_elements.iter()
        .find(|e| e.endpoint_node_id == "a")
        .expect("marker for endpoint a in dragged scene");

    let dx = dragged_a.position.0 - baseline_a.position.0;
    let dy = dragged_a.position.1 - baseline_a.position.1;
    assert!((dx - 100.0).abs() < 0.01, "marker x should shift by +100, got {dx}");
    assert!((dy - 50.0).abs() < 0.01, "marker y should shift by +50, got {dy}");
}
