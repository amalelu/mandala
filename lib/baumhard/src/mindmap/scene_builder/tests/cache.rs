//! `SceneConnectionCache` integration: population, hit identity, endpoint invalidation, drag stability, clip rerun, eviction, empty-after-new, fold edge, selection stability, plus a real-map smoke test.

use super::fixtures::*;
use super::super::*;
use crate::mindmap::loader; use crate::mindmap::scene_cache::{CachedConnection, SceneConnectionCache};
use std::collections::HashMap;
use glam::Vec2;

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
