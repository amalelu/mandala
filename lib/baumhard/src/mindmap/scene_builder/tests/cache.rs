//! `SceneConnectionCache` integration: population, hit identity, endpoint invalidation, drag stability, clip rerun, eviction, empty-after-new, fold edge, selection stability, plus a real-map smoke test.

use super::fixtures::*;
use super::super::*;
use crate::mindmap::loader;
use crate::mindmap::model::GlyphConnectionConfig;
use crate::mindmap::scene_cache::{CachedConnection, SceneConnectionCache};
use std::collections::HashMap;
use glam::Vec2;

#[test]
fn test_cache_populated_on_first_build() {
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);

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
    let _first = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);

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
            base_from: Vec2::ZERO,
            base_to: Vec2::ZERO,
        },
    );

    let second = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);
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
    let _first = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);

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
            base_from: Vec2::ZERO,
            base_to: Vec2::ZERO,
        },
    );

    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (10.0, 0.0));
    let second = build_scene_with_cache(&map, &offsets, SceneSelectionContext::default(), None, None, &mut cache, 1.0);
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
            synthetic_edge("a", "b", "right", "left"),
            synthetic_edge("c", "d", "right", "left"),
        ],
    );
    let mut cache = SceneConnectionCache::new();
    let _first = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);

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
            base_from: Vec2::ZERO,
            base_to: Vec2::ZERO,
        },
    );

    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (5.0, 0.0));
    let second = build_scene_with_cache(&map, &offsets, SceneSelectionContext::default(), None, None, &mut cache, 1.0);

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
        vec![synthetic_edge("a", "b", "right", "left")],
    );

    let mut cache = SceneConnectionCache::new();
    let first = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);
    let first_count = first.connection_elements[0].glyph_positions.len();

    // Now move `c` into the middle of the connection — use a drag
    // offset. `a↔b` is NOT in the dirty set (endpoints didn't move),
    // so it hits the cache path, but the clip filter must still
    // notice `c`'s new position.
    let mut offsets = HashMap::new();
    offsets.insert("c".to_string(), (0.0, 500.0));
    let second = build_scene_with_cache(&map, &offsets, SceneSelectionContext::default(), None, None, &mut cache, 1.0);
    let second_count = second.connection_elements[0].glyph_positions.len();
    assert!(second_count < first_count,
        "moving c through the edge should reduce post-clip glyph count: {} → {}",
        first_count, second_count);

    // Now move `c` back out of the way via a model edit + full rebuild.
    map.nodes.get_mut("c").unwrap().position.y = -500.0;
    let third = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);
    assert_eq!(third.connection_elements[0].glyph_positions.len(), first_count);
}

#[test]
fn test_cache_evicts_deleted_edges() {
    let mut map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);
    let key = EdgeKey::new("a", "b", "cross_link");
    assert!(cache.get(&key).is_some());

    // Remove the edge from the model and rebuild.
    map.edges.clear();
    let second = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);
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
            synthetic_edge("a", "b", "right", "left"),
            synthetic_edge("b", "c", "right", "left"),
        ],
    );
    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);
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
    let first = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);
    let second = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);

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
    let edge = synthetic_edge("a", "b", "right", "left");
    let map = synthetic_map(vec![a, b_child], vec![edge]);

    let mut cache = SceneConnectionCache::new();
    let scene = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);
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
    let _first = build_scene_with_cache(&map, &HashMap::new(), SceneSelectionContext::default(), None, None, &mut cache, 1.0);
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
            base_from: Vec2::ZERO,
            base_to: Vec2::ZERO,
        },
    );

    let second = build_scene_with_cache(
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
fn test_cache_fast_path_serves_stale_when_model_moved_without_offsets() {
    // Regression for "edges stuck at pre-drag position after rapid
    // node drag" (b41a638). The `MovingNode` throttle can skip the
    // final drain or two under fast cursor motion, stranding
    // `pending_delta` outside the cache. On release the tree is
    // flushed and `apply_move_multiple` advances the model by
    // `total_delta` — exceeding the cached `offsets = total_delta -
    // pending_delta` that the last successful drain wrote. The
    // follow-up `rebuild_scene_only` runs with empty offsets, so
    // every edge hits the fast path here and returns the stale
    // samples.
    //
    // This test pins the baumhard-side invariant: a cached entry
    // whose endpoint has moved in the model (and not in the offsets
    // map) is stale, and the cache-aware builder will serve it. The
    // fix is for the release-side caller to invalidate the cache
    // before the rebuild. If that clear is ever removed this test
    // documents the invariant the caller must uphold.
    let mut map = two_node_edge_map();

    // Simulate a drag drain: offsets carry the current total_delta,
    // populating the cache with samples at `model + offset`.
    let mut cache = SceneConnectionCache::new();
    let mut drain_offsets = HashMap::new();
    drain_offsets.insert("a".to_string(), (30.0_f32, 0.0));
    let _ = build_scene_with_cache(
        &map,
        &drain_offsets,
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );

    // Overwrite the cache entry with a sentinel so we can observe
    // whether the next build read through the cache (sentinel) or
    // re-sampled (non-sentinel).
    let key = EdgeKey::new("a", "b", "cross_link");
    cache.insert(
        key.clone(),
        CachedConnection {
            pre_clip_positions: vec![Vec2::new(123.0, 456.0)],
            cap_start: None,
            cap_end: None,
            body_glyph: "STALE_SENTINEL".into(),
            font: None,
            font_size_pt: 12.0,
            color: "#ff00ff".into(),
            base_from: Vec2::ZERO,
            base_to: Vec2::ZERO,
        },
    );

    // Simulate release: `apply_move_multiple` commits the full
    // `total_delta = drain_offset + pending_delta` to the model,
    // advancing node `a` beyond where the drain sampled.
    map.nodes.get_mut("a").unwrap().position.x = 35.0;

    // Release's `rebuild_all` path: empty offsets. Endpoint `a` is
    // not in offsets, so the fast path fires — and returns the
    // sentinel, exactly as it returned the stale `pre_clip_positions`
    // in production before the fix.
    let without_clear = build_scene_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(
        without_clear.connection_elements[0].body_glyph,
        "STALE_SENTINEL",
        "cache fast-path serves cached samples when neither endpoint appears in offsets, \
         even if the model endpoint has moved since the entry was written"
    );

    // The fix: the release-side caller must clear the cache so the
    // rebuild resamples from the committed model.
    cache.clear();
    let after_clear = build_scene_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_ne!(
        after_clear.connection_elements[0].body_glyph,
        "STALE_SENTINEL",
        "after scene_cache.clear() the rebuild must resample from the committed model"
    );
    assert!(
        !after_clear.connection_elements[0].glyph_positions.is_empty(),
        "freshly-sampled edge should emit glyphs"
    );
}

#[test]
fn test_translate_path_reuses_cache_on_shared_delta_subtree_drag() {
    // Performance regression for the "zoom-in drag feels laggy"
    // symptom. A subtree drag pushes every moved node into `offsets`
    // with the same delta, so every edge internal to the subtree has
    // both endpoints moved by the same amount — a pure translation of
    // last-sampled geometry. The translate path must skip the Bezier
    // sampler and just shift the cached samples.
    //
    // Sentinel body_glyph tells us whether the builder re-sampled
    // (sentinel gone → slow path fired) or translated (sentinel
    // survives → translate path fired).
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = build_scene_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );

    // Overwrite the cache with a sentinel whose `base_from` / `base_to`
    // match the first-build endpoint positions so the translate path's
    // delta check gets clean numbers to compare.
    let key = EdgeKey::new("a", "b", "cross_link");
    let real = cache.get(&key).unwrap().clone();
    let sample_count = real.pre_clip_positions.len();
    // Overwrite `pre_clip_positions` with a distinctive uniform
    // value so we can prove the translate path fired: if the slow
    // path had resampled, positions would spread along the edge
    // from (0,0)-ish to (400,0)-ish, not cluster at (215, 207).
    // `body_glyph` / `font` must match the live config so the new
    // glyph-config guard doesn't force a fall-through here — the
    // guard is exercised by `test_translate_path_falls_through_on_glyph_config_change`.
    // Position choice: (200, 200) + (15, 7) = (215, 207) is between
    // the nodes on X and well below them on Y — clears both AABBs.
    let live_config = GlyphConnectionConfig::default();
    cache.insert(
        key.clone(),
        CachedConnection {
            pre_clip_positions: vec![Vec2::new(200.0, 200.0); sample_count],
            cap_start: None,
            cap_end: None,
            body_glyph: live_config.body.clone(),
            font: live_config.font.clone(),
            font_size_pt: real.font_size_pt,
            color: "#abcdef".into(),
            base_from: real.base_from,
            base_to: real.base_to,
        },
    );

    // Subtree drag: both endpoints move by the same (dx, dy).
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (15.0, 7.0));
    offsets.insert("b".to_string(), (15.0, 7.0));
    let second = build_scene_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );

    let elem = &second.connection_elements[0];
    // Every sample is the cached (200, 200) shifted by (15, 7) = (215, 207).
    // If the slow path had fired instead it would have resampled the
    // real edge geometry from (~40, 20) to (~400, 20) — these
    // position assertions would all fail.
    assert_eq!(elem.glyph_positions.len(), sample_count);
    for (x, y) in &elem.glyph_positions {
        assert!(
            (x - 215.0).abs() < 1e-4 && (y - 207.0).abs() < 1e-4,
            "translated sample should be (215, 207), got ({}, {})",
            x, y
        );
    }

    // The cache's base positions must advance to the current endpoints
    // so the NEXT drain's translate check sees the new reference.
    let after = cache.get(&key).unwrap();
    let from_node = map.nodes.get("a").unwrap();
    let to_node = map.nodes.get("b").unwrap();
    let expected_from = Vec2::new(from_node.position.x as f32 + 15.0, from_node.position.y as f32 + 7.0);
    let expected_to = Vec2::new(to_node.position.x as f32 + 15.0, to_node.position.y as f32 + 7.0);
    assert!((after.base_from - expected_from).length_squared() < 1e-6);
    assert!((after.base_to - expected_to).length_squared() < 1e-6);
}

#[test]
fn test_translate_path_falls_through_on_mismatched_deltas() {
    // Boundary-edge case: only one endpoint (or endpoints with
    // different deltas) means the edge's shape — not just position —
    // changed. The slow path must fire to resample the new geometry;
    // translating by either delta would misplace samples.
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = build_scene_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );

    let key = EdgeKey::new("a", "b", "cross_link");
    let real = cache.get(&key).unwrap().clone();
    cache.insert(
        key.clone(),
        CachedConnection {
            pre_clip_positions: vec![Vec2::new(999.0, 999.0); real.pre_clip_positions.len()],
            cap_start: None,
            cap_end: None,
            body_glyph: "NO_TRANSLATE_SENTINEL".into(),
            font: None,
            font_size_pt: real.font_size_pt,
            color: "#deadbe".into(),
            base_from: real.base_from,
            base_to: real.base_to,
        },
    );

    // Different deltas on each endpoint — a rotating / stretching edge,
    // not a translation.
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (10.0, 0.0));
    offsets.insert("b".to_string(), (0.0, 10.0));
    let second = build_scene_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );

    assert_ne!(
        second.connection_elements[0].body_glyph,
        "NO_TRANSLATE_SENTINEL",
        "mismatched endpoint deltas must fall through to the slow path"
    );
}

#[test]
fn test_translate_path_falls_through_on_glyph_config_change() {
    // Edge-case guard for a mid-drag glyph-config mutation (a
    // console edit that flips `glyph_connection.body` while a drag
    // is in flight). The cached entry is frozen at the pre-edit
    // glyph, so serving it on the next translate frame would emit
    // a stale glyph. The translate path must notice `cached.body_glyph
    // != config.body` and fall through to the slow path, which
    // resamples and caches with the new glyph.
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = build_scene_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );

    let key = EdgeKey::new("a", "b", "cross_link");
    let real = cache.get(&key).unwrap().clone();
    // Overwrite with a body_glyph that DIFFERS from what the live
    // config (defaulted `·`) would resolve to. The delta + font
    // guards would otherwise pass — only the glyph mismatch should
    // cause the fall-through.
    cache.insert(
        key.clone(),
        CachedConnection {
            pre_clip_positions: vec![Vec2::new(999.0, 999.0); real.pre_clip_positions.len()],
            cap_start: None,
            cap_end: None,
            body_glyph: "X".into(),
            font: None,
            font_size_pt: real.font_size_pt,
            color: real.color.clone(),
            base_from: real.base_from,
            base_to: real.base_to,
        },
    );

    // Subtree drag with matching deltas — would hit translate path
    // if the glyph-guard weren't in place.
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (5.0, 0.0));
    offsets.insert("b".to_string(), (5.0, 0.0));
    let second = build_scene_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );

    // Emitted body_glyph must be the live config's, not the cached
    // "X" — proves the slow path resampled instead of translating.
    assert_ne!(
        second.connection_elements[0].body_glyph, "X",
        "mid-drag body_glyph change must force the slow path so the emitted glyph tracks the live config"
    );
    // And the cache entry must now reflect the fresh resample.
    let refreshed = cache.get(&key).unwrap();
    assert_ne!(
        refreshed.body_glyph, "X",
        "slow path resample must overwrite the stale body_glyph"
    );
    assert!(
        refreshed.pre_clip_positions.iter().all(|p| *p != Vec2::new(999.0, 999.0)),
        "slow path must overwrite the placeholder sentinel positions"
    );
}

#[test]
fn test_translate_path_still_applies_clip_filter() {
    // Governing invariant: every path in the builder runs the
    // `node_aabbs` clip filter against the current frame's geometry,
    // including the translate path. An unrelated blocker node whose
    // AABB covers some translated samples must still clip them out.
    //
    // Setup: three-node map where the edge a↔b passes clear of `c`
    // initially (c is well south of the connection). Populate the
    // cache. Then subtree-drag (a, b) together so the translate path
    // fires — but move `c` into the middle of the translated edge at
    // the same time. The clip filter must notice `c`'s new AABB and
    // drop samples inside it, even though the edge itself came from
    // the cache.
    let map = synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            synthetic_node("c", 180.0, -500.0, 80.0, 40.0, false),
        ],
        vec![synthetic_edge("a", "b", "right", "left")],
    );

    let mut cache = SceneConnectionCache::new();
    let first = build_scene_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );
    let baseline_count = first.connection_elements[0].glyph_positions.len();

    // Subtree drag: move a and b together; AT THE SAME TIME move c
    // into the translated edge's path. The offsets map carries all
    // three nodes; a's and b's deltas match (translate path fires),
    // but c's delta is different (and c isn't an endpoint anyway).
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (0.0, 20.0));
    offsets.insert("b".to_string(), (0.0, 20.0));
    offsets.insert("c".to_string(), (0.0, 520.0));
    let second = build_scene_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        &mut cache,
        1.0,
    );
    let after_count = second.connection_elements[0].glyph_positions.len();
    assert!(
        after_count < baseline_count,
        "blocker `c` moved into the translated edge path should clip samples: {} -> {}",
        baseline_count,
        after_count,
    );
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
