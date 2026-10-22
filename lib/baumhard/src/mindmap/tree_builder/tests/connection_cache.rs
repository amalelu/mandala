// SPDX-License-Identifier: MPL-2.0

//! `SceneConnectionCache` integration: population, hit identity, endpoint invalidation, drag stability, clip rerun, eviction, empty-after-new, fold edge, selection stability, plus a real-map smoke test.

use super::super::*;
use super::fixtures::*;
use crate::mindmap::loader;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap};
use crate::mindmap::scene_cache::{EdgeKey, SampleParams, SceneConnectionCache};
use glam::Vec2;
use std::collections::HashMap;

/// A canvas position no fixture in this file can sample: every path
/// here runs between nodes laid out in `y ∈ [0, 340]`, so a point 999
/// units above the canvas is unmistakably planted rather than
/// computed. It also sits outside every fixture node's clip AABB, so
/// it survives the clip filter and reaches the emitted element where
/// an assertion can see it.
const SENTINEL_POINT: Vec2 = Vec2::new(200.0, -999.0);

/// Below this `y`, geometry is planted rather than sampled.
///
/// The band has to be clear on both sides, and the two sides are
/// bounded by different things.
///
/// *Sampled side*: every real sample in these fixtures lands in
/// `y ∈ [0, 340]`, so this floor sits 500 units below the lowest of
/// them.
///
/// *Planted side*: the translate path reuses an entry **and shifts
/// it**, so a planted point moves by the drag delta, and the margin
/// between [`SENTINEL_POINT`] and this floor has to cover that. The
/// largest `y` offset any test in this file applies **to an edge
/// endpoint** is 25 — a measured maximum over the offsets currently
/// in the file, `grep 'offsets.insert'`, not a bound on what a future
/// test may add. Two larger numbers in the file are not endpoint `y`
/// offsets: the 500 and 520 go to node `c`, a blocker that is never
/// an endpoint of the edge under test, and the 280 is in `x`, which
/// this band does not read.
const SENTINEL_Y_FLOOR: f32 = -500.0;

// The plant and the predicate that recognises it are one decision;
// this is what keeps moving either of them from silently making
// `drew_sentinel` answer `false` for every input.
const _: () = assert!(SENTINEL_POINT.y < SENTINEL_Y_FLOOR);

/// The [`SampleParams`] `build_connection_elements` snapshots for
/// `map`'s edge `edge_index` at `camera_zoom`.
///
/// Used only to *plant* an entry the reuse doors will accept — never
/// to compute a value an assertion then compares against, which would
/// be the code under test grading its own homework. A test that wants
/// a door to *refuse* names the mismatched field itself.
fn live_params(map: &MindMap, edge_index: usize, camera_zoom: f32) -> SampleParams {
    let default_config = GlyphConnectionConfig::default();
    let config = map.edges[edge_index]
        .glyph_connection
        .as_ref()
        .or(map.canvas.default_connection.as_ref())
        .unwrap_or(&default_config);
    SampleParams::snapshot(
        config,
        camera_zoom,
        crate::mindmap::connection::per_path_sample_budget(map.edges.len()),
    )
}

/// A cache entry holding one sample the sampler could not have
/// produced, under params the reuse doors accept for `map`'s edge
/// `edge_index`.
///
/// This is the probe every "did the builder read the cache?" test in
/// this file uses, and it is *geometry* rather than styling on
/// purpose. Styling no longer travels through the cache at all — body
/// glyph, cap glyphs, font family, font size and body color are all
/// read from the live model on all three paths — so a styling sentinel
/// would be served identically whether the cache was consulted or not,
/// and would prove nothing. Since the removal of `color` there is no
/// styling field left to plant one in even if it did.
fn plant_sentinel(
    cache: &mut SceneConnectionCache,
    map: &MindMap,
    edge_index: usize,
    camera_zoom: f32,
    at: Vec2,
) {
    plant_geometry(cache, map, edge_index, camera_zoom, &[at], Vec2::ZERO, Vec2::ZERO);
}

/// [`plant_sentinel`] with the geometry and base endpoints spelled
/// out, for the tests that need the translate path's delta check to
/// see sane reference points.
///
/// Goes through `SceneConnectionCache::refill` because that is the
/// cache's one writer; a fixture that reached past it would be
/// exercising a shape production cannot produce.
fn plant_geometry(
    cache: &mut SceneConnectionCache,
    map: &MindMap,
    edge_index: usize,
    camera_zoom: f32,
    points: &[Vec2],
    base_from: Vec2,
    base_to: Vec2,
) {
    let params = live_params(map, edge_index, camera_zoom);
    cache.refill(
        &EdgeKey::from_edge(&map.edges[edge_index]),
        params,
        base_from,
        base_to,
        |out| out.extend_from_slice(points),
    );
}

/// Whether `elem` is drawing geometry derived from the planted
/// sentinel — i.e. whether the pass reused the cache entry rather
/// than resampling.
///
/// The test is the y band rather than the exact point, because the
/// translate path reuses an entry *and shifts it*. An exact-point
/// predicate reported "did not reuse" for a translated sentinel,
/// which is the wrong answer to the question every caller is asking;
/// the negative control for
/// `test_translate_path_falls_through_on_a_sampling_config_change` is
/// what surfaced that. See [`SENTINEL_Y_FLOOR`] for why the band is
/// wide enough on both sides.
fn drew_sentinel(elem: &ConnectionElement) -> bool {
    elem.glyph_positions.iter().any(|&(_, y)| y < SENTINEL_Y_FLOOR)
}

#[test]
fn test_cache_populated_on_first_build() {
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let scene = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    assert_eq!(scene.connection_elements.len(), 1);
    assert_eq!(cache.len(), 1);
    let key = EdgeKey::new("a", "b", "cross_link");
    assert!(cache.inspect(&key).is_some());
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
    let _first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    // Overwrite the cached entry with one whose single sample is
    // somewhere the sampler would never place one, so we can see
    // whether build #2 read it: if the cache is used, that point is
    // what the second build draws.
    plant_sentinel(&mut cache, &map, 0, 1.0, SENTINEL_POINT);

    let second = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(second.connection_elements.len(), 1);
    let conn = &second.connection_elements[0];
    assert!(
        drew_sentinel(conn),
        "cache-hit path should have used the stored entry, drew {:?}",
        conn.glyph_positions
    );
    // Single cached pre-clip point should have survived the clip
    // filter (it's outside both nodes).
    assert_eq!(conn.glyph_positions.len(), 1);
    // The color is the *model's*, not anything the cache holds —
    // `synthetic_edge` writes `#fff` and nothing overrides it here.
    // The cache carries no color to serve instead.
    assert_eq!(conn.color, "#fff");
}

#[test]
fn test_cache_invalidated_on_endpoint_offset() {
    // If endpoint `a` moves, the a↔b edge must be re-sampled — we
    // should observe fresh geometry on the element, not the sentinel
    // point we stashed in the cache.
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    let key = EdgeKey::new("a", "b", "cross_link");
    plant_sentinel(&mut cache, &map, 0, 1.0, SENTINEL_POINT);

    // Only `a` moves, so the deltas differ and the translate path
    // cannot take this edge either — the slow path is the only route
    // left, which is the point.
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (10.0, 0.0));
    let second = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    let conn = &second.connection_elements[0];
    assert!(
        !drew_sentinel(conn),
        "endpoint-moved edge should have been re-sampled, still drawing the sentinel"
    );
    assert!(
        conn.glyph_positions.len() > 1,
        "a resample of this edge yields many samples, not the planted one"
    );
    // The cache should contain the freshly-resampled entry now.
    let refreshed = cache.inspect(&key).unwrap();
    assert!(
        !refreshed
            .pre_clip_positions
            .iter()
            .any(|p| crate::util::geometry::almost_equal_vec2(*p, SENTINEL_POINT)),
        "the resample must have overwritten the planted entry"
    );
    assert!(!refreshed.pre_clip_positions.is_empty());
}

#[test]
fn test_cache_preserves_unrelated_edge_under_drag() {
    // Two edges: a↔b (long) and c↔d (short). Drag node `a`. The c↔d
    // edge should NOT be re-sampled; its cache entry should remain as
    // our sentinel.
    let map = synthetic_map(
        vec![
            sized_node("a", 0.0, 0.0, 40.0, 40.0, false),
            sized_node("b", 400.0, 0.0, 40.0, 40.0, false),
            sized_node("c", 0.0, 300.0, 40.0, 40.0, false),
            sized_node("d", 400.0, 300.0, 40.0, 40.0, false),
        ],
        vec![
            synthetic_edge("a", "b", "right", "left"),
            synthetic_edge("c", "d", "right", "left"),
        ],
    );
    let mut cache = SceneConnectionCache::new();
    let first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    let ab_key = EdgeKey::new("a", "b", "cross_link");
    let cd_key = EdgeKey::new("c", "d", "cross_link");
    let leftmost = |roles: &ProjectedRoles, key: &EdgeKey| -> f32 {
        roles
            .connection_elements
            .iter()
            .find(|e| &e.edge_key == key)
            .expect("element should exist")
            .glyph_positions
            .iter()
            .map(|&(x, _)| x)
            .fold(f32::INFINITY, f32::min)
    };
    let ab_left_before = leftmost(&first, &ab_key);

    plant_sentinel(&mut cache, &map, 1, 1.0, SENTINEL_POINT);

    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (5.0, 0.0));
    let second = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    // Find the c↔d connection element and verify it came from the
    // cache unchanged.
    let cd_elem = second
        .connection_elements
        .iter()
        .find(|e| e.edge_key == cd_key)
        .expect("c↔d element should exist");
    assert!(
        drew_sentinel(cd_elem),
        "unrelated edge should have been served from cache, not re-sampled; drew {:?}",
        cd_elem.glyph_positions
    );

    // The a↔b edge should have been re-sampled, and the +5 x offset on
    // `a` is what says so: its source anchor sits on a's right edge, so
    // the leftmost surviving sample moves with it. Asserting the shift
    // rather than merely "not the sentinel" gives the clause an input
    // that can fail — nothing ever planted a sentinel on this edge.
    let ab_left_after = leftmost(&second, &ab_key);
    assert!(
        crate::util::geometry::almost_equal(ab_left_after - ab_left_before, 5.0),
        "a↔b should have been re-sampled from a's moved right edge: {} -> {}",
        ab_left_before,
        ab_left_after
    );
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
            sized_node("a", 0.0, 0.0, 40.0, 40.0, false),
            sized_node("b", 400.0, 0.0, 40.0, 40.0, false),
            // Blocker far above the connection — no clip effect yet.
            sized_node("c", 180.0, -500.0, 60.0, 40.0, false),
        ],
        vec![synthetic_edge("a", "b", "right", "left")],
    );

    let mut cache = SceneConnectionCache::new();
    let first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    let first_count = first.connection_elements[0].glyph_positions.len();

    // Now move `c` into the middle of the connection — use a drag
    // offset. `a↔b` is NOT in the dirty set (endpoints didn't move),
    // so it hits the cache path, but the clip filter must still
    // notice `c`'s new position.
    let mut offsets = HashMap::new();
    offsets.insert("c".to_string(), (0.0, 500.0));
    let second = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    let second_count = second.connection_elements[0].glyph_positions.len();
    assert!(
        second_count < first_count,
        "moving c through the edge should reduce post-clip glyph count: {} → {}",
        first_count,
        second_count
    );

    // Now move `c` back out of the way via a model edit + full rebuild.
    map.nodes.get_mut("c").unwrap().position.y = -500.0;
    let third = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(third.connection_elements[0].glyph_positions.len(), first_count);
}

#[test]
fn test_cache_evicts_deleted_edges() {
    let mut map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    let key = EdgeKey::new("a", "b", "cross_link");
    assert!(cache.inspect(&key).is_some());

    // Remove the edge from the model and rebuild.
    map.edges.clear();
    let second = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert!(second.connection_elements.is_empty());
    assert!(
        cache.inspect(&key).is_none(),
        "deleted edge should be evicted from cache"
    );
}

#[test]
fn test_connection_element_edge_key_always_populated() {
    // Sanity: every ConnectionElement emitted by the cache-aware
    // builder carries a valid EdgeKey matching the source MindEdge.
    // The renderer's keyed buffer map is keyed off this; a missing
    // or wrong edge_key would silently break the incremental path.
    let map = synthetic_map(
        vec![
            sized_node("a", 0.0, 0.0, 40.0, 40.0, false),
            sized_node("b", 400.0, 0.0, 40.0, 40.0, false),
            sized_node("c", 0.0, 200.0, 40.0, 40.0, false),
        ],
        vec![
            synthetic_edge("a", "b", "right", "left"),
            synthetic_edge("b", "c", "right", "left"),
        ],
    );
    let mut cache = SceneConnectionCache::new();
    let scene = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(scene.connection_elements.len(), 2);
    let ab = EdgeKey::new("a", "b", "cross_link");
    let bc = EdgeKey::new("b", "c", "cross_link");
    let keys: Vec<&EdgeKey> = scene.connection_elements.iter().map(|e| &e.edge_key).collect();
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
    let first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    let second = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    assert_eq!(first.connection_elements.len(), second.connection_elements.len(),);
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
    let mut a = sized_node("a", 0.0, 0.0, 40.0, 40.0, false);
    let mut b_child = sized_node("b", 400.0, 0.0, 40.0, 40.0, false);
    b_child.parent_id = Some("a".to_string());
    a.folded = true; // hides b
    let edge = synthetic_edge("a", "b", "right", "left");
    let map = synthetic_map(vec![a, b_child], vec![edge]);

    let mut cache = SceneConnectionCache::new();
    let scene = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert!(
        scene.connection_elements.is_empty(),
        "folded edge should be skipped"
    );
    assert!(cache.is_empty(), "folded edge should not appear in cache");
}

#[test]
fn test_cache_selection_change_does_not_invalidate() {
    // Build with no selection → cache populated with the resolved
    // color. Build again with the edge selected → cache entry should
    // not be rewritten; the element's color should still reflect the
    // selection override.
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    let key = EdgeKey::new("a", "b", "cross_link");

    // Inject sentinel geometry into the cache so we can detect
    // whether the cache path was taken on the second build.
    plant_sentinel(&mut cache, &map, 0, 1.0, SENTINEL_POINT);

    let second = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext {
            edge: Some(("a", "b", "cross_link")),
            ..Default::default()
        },
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    let conn = &second.connection_elements[0];
    assert!(
        drew_sentinel(conn),
        "selection change should not have dropped the cache; drew {:?}",
        conn.glyph_positions
    );
    assert_eq!(
        conn.color,
        crate::mindmap::SELECTION_HIGHLIGHT_HEX,
        "selected element should pick up the highlight color"
    );
    // And the entry itself is untouched — a selection change must not
    // provoke a resample, which is what would have overwritten the
    // planted geometry.
    assert_eq!(
        cache.inspect(&key).unwrap().pre_clip_positions,
        vec![SENTINEL_POINT],
        "the entry must survive the selection change unrewritten"
    );
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
    let _ = project_with_cache(
        &map,
        &drain_offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    // Overwrite the cache entry with a sentinel so we can observe
    // whether the next build read through the cache (sentinel) or
    // re-sampled (non-sentinel).
    plant_sentinel(&mut cache, &map, 0, 1.0, SENTINEL_POINT);

    // Simulate release: `apply_move_multiple` commits the full
    // `total_delta = drain_offset + pending_delta` to the model,
    // advancing node `a` beyond where the drain sampled.
    map.nodes.get_mut("a").unwrap().position.x = 35.0;

    // Release's `rebuild_all` path: empty offsets. Endpoint `a` is
    // not in offsets, so the fast path fires — and returns the
    // sentinel, exactly as it returned the stale `pre_clip_positions`
    // in production before the fix.
    let without_clear = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert!(
        drew_sentinel(&without_clear.connection_elements[0]),
        "cache fast-path serves cached samples when neither endpoint appears in offsets, \
         even if the model endpoint has moved since the entry was written; drew {:?}",
        without_clear.connection_elements[0].glyph_positions
    );

    // The fix: the release-side caller must clear the cache so the
    // rebuild resamples from the committed model.
    cache.clear();
    let after_clear = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert!(
        !drew_sentinel(&after_clear.connection_elements[0]),
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
    // Sentinel geometry tells us whether the builder re-sampled
    // (sentinel gone → slow path fired) or translated (sentinel
    // survives, shifted by the shared delta → translate path fired).
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    // Overwrite the cache with a sentinel whose `base_from` / `base_to`
    // match the first-build endpoint positions so the translate path's
    // delta check gets clean numbers to compare.
    let key = EdgeKey::new("a", "b", "cross_link");
    let real = cache.inspect(&key).unwrap().clone();
    let sample_count = real.pre_clip_positions.len();
    // Overwrite `pre_clip_positions` with a distinctive uniform
    // value so we can prove the translate path fired: if the slow
    // path had resampled, positions would spread along the edge
    // from (0,0)-ish to (400,0)-ish, not cluster at (215, 207).
    // `sample_params` is carried over from the real entry so the
    // reuse door accepts it — the door's refusal is exercised by
    // `test_translate_path_falls_through_on_a_sampling_config_change`.
    // Position choice: (200, 200) + (15, 7) = (215, 207) is between
    // the nodes on X and well below them on Y — clears both AABBs.
    let uniform = vec![Vec2::new(200.0, 200.0); sample_count];
    cache.refill(&key, real.sample_params, real.base_from, real.base_to, |out| {
        out.extend_from_slice(&uniform)
    });

    // Subtree drag: both endpoints move by the same (dx, dy).
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (15.0, 7.0));
    offsets.insert("b".to_string(), (15.0, 7.0));
    let second = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
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
            x,
            y
        );
    }

    // The cache's base positions must advance to the current endpoints
    // so the NEXT drain's translate check sees the new reference.
    let after = cache.inspect(&key).unwrap();
    let from_node = map.nodes.get("a").unwrap();
    let to_node = map.nodes.get("b").unwrap();
    let expected_from = Vec2::new(
        from_node.position.x as f32 + 15.0,
        from_node.position.y as f32 + 7.0,
    );
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
    let _first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    let key = EdgeKey::new("a", "b", "cross_link");
    let real = cache.inspect(&key).unwrap().clone();
    let planted = vec![SENTINEL_POINT; real.pre_clip_positions.len()];
    cache.refill(&key, real.sample_params, real.base_from, real.base_to, |out| {
        out.extend_from_slice(&planted)
    });

    // Different deltas on each endpoint — a rotating / stretching edge,
    // not a translation.
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (10.0, 0.0));
    offsets.insert("b".to_string(), (0.0, 10.0));
    let second = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    assert!(
        !drew_sentinel(&second.connection_elements[0]),
        "mismatched endpoint deltas must fall through to the slow path"
    );
}

#[test]
fn test_translate_path_falls_through_on_a_sampling_config_change() {
    // Edge-case guard for a mid-drag config mutation (a console
    // `edge font size` edit while a drag is in flight). The cached
    // samples are spaced for the pre-edit size, so translating them
    // on the next frame would draw the new glyphs at the old stride.
    // The translate path must find no reusable entry and fall through
    // to the slow path, which resamples at the new stride.
    //
    // The mismatch is planted on the *entry* rather than on the model
    // so the drag offsets stay a clean shared delta: if the deltas
    // were what differed, this test would be a duplicate of
    // `test_translate_path_falls_through_on_mismatched_deltas` and
    // would pass with the params check deleted.
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    let key = EdgeKey::new("a", "b", "cross_link");
    let real = cache.inspect(&key).unwrap().clone();
    let mut stale_params = real.sample_params;
    stale_params.font_size_pt *= 2.0;
    let planted = vec![SENTINEL_POINT; real.pre_clip_positions.len()];
    cache.refill(&key, stale_params, real.base_from, real.base_to, |out| {
        out.extend_from_slice(&planted)
    });

    // Subtree drag with matching deltas — would hit the translate
    // path if the params check weren't in place.
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (5.0, 0.0));
    offsets.insert("b".to_string(), (5.0, 0.0));
    let second = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    assert!(
        !drew_sentinel(&second.connection_elements[0]),
        "a mid-drag sampling-config change must force the slow path; drew {:?}",
        second.connection_elements[0].glyph_positions
    );
    // And the cache entry must now reflect the fresh resample, at the
    // params this frame actually asked for.
    let refreshed = cache.inspect(&key).unwrap();
    assert!(
        refreshed.pre_clip_positions.iter().all(|p| *p != SENTINEL_POINT),
        "slow path must overwrite the planted sentinel positions"
    );
    assert!(
        crate::util::geometry::almost_equal(
            refreshed.sample_params.font_size_pt,
            real.sample_params.font_size_pt
        ),
        "the re-cached entry must carry this frame's params, not the planted ones"
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
            sized_node("a", 0.0, 0.0, 40.0, 40.0, false),
            sized_node("b", 400.0, 0.0, 40.0, 40.0, false),
            sized_node("c", 180.0, -500.0, 80.0, 40.0, false),
        ],
        vec![synthetic_edge("a", "b", "right", "left")],
    );

    let mut cache = SceneConnectionCache::new();
    let first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
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
    let second = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
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
    let scene = project(&map, 1.0);
    assert!(!scene.node_aabbs.is_empty());
    assert!(!scene.connection_elements.is_empty());
    // At least one connection should have a non-empty glyph list.
    let any_with_glyphs = scene
        .connection_elements
        .iter()
        .any(|c| !c.glyph_positions.is_empty());
    assert!(
        any_with_glyphs,
        "at least one connection should have un-clipped glyphs"
    );
}

/// A resample refills the entry's own buffer rather than appending to
/// it.
///
/// The slow path no longer allocates a sample vector: it samples into
/// the one this edge filled last frame, which arrives emptied but with
/// its capacity. If the emptying were ever dropped, every resampling
/// frame would render the edge with last frame's points still in
/// front of this frame's — the glyphs would trail behind the drag and
/// the count would grow without bound.
///
/// The expectation comes from a **cold cache**, which reaches the same
/// slow path with an empty buffer. That is the whole difference
/// between the two runs, so a disagreement can only be the reuse.
#[test]
fn test_a_resample_refills_the_reused_buffer_rather_than_appending() {
    let map = two_node_edge_map();
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (10.0, 0.0));

    // Warm: build once with no offsets so the edge is cached, then
    // move one endpoint so it must resample through the buffer the
    // cache hands back.
    let mut warm = SceneConnectionCache::new();
    let _fill = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut warm,
        1.0,
    );
    assert_eq!(warm.len(), 1, "precondition: the first build filled the cache");
    let reused = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut warm,
        1.0,
    );

    // Cold: the same resample with no cached buffer to reuse.
    let mut cold = SceneConnectionCache::new();
    let fresh = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cold,
        1.0,
    );

    assert!(
        !fresh.connection_elements[0].glyph_positions.is_empty(),
        "precondition: the resampled edge draws something"
    );
    assert_eq!(
        reused.connection_elements[0].glyph_positions, fresh.connection_elements[0].glyph_positions,
        "a resample through a reused buffer must produce what a cold one does"
    );
    assert_eq!(
        warm.inspect(&EdgeKey::new("a", "b", "cross_link"))
            .expect("warm entry")
            .pre_clip_positions,
        cold.inspect(&EdgeKey::new("a", "b", "cross_link"))
            .expect("cold entry")
            .pre_clip_positions,
        "and the re-cached geometry must match too"
    );
}

/// Every route an edge can take out of the connection pass marks it
/// seen, so the eviction at the end of that same pass keeps it.
///
/// The pass's liveness bookkeeping is a generation stamp rather than
/// a set of keys built per frame, and the failure mode of a stamp is
/// a route that forgets to apply it: that edge is written or reused,
/// drawn, and then evicted at the bottom of the very build that drew
/// it — so the next frame resamples it, forever, and the cache holds
/// nothing.
///
/// Input that makes it fail: dropping the one `last_seen = generation`
/// line from `SceneConnectionCache::refill` (route 1), from
/// `reusable` (route 2) or from `reusable_mut` (route 3). Each is one
/// line and each reddens this test, though never only this test.
/// Measured with `cargo test -p baumhard --lib mindmap::`, one
/// mutation at a time:
///
/// | line dropped | also reddens | count when measured |
/// |---|---|---|
/// | `refill`'s | 15 others, incl. `test_cache_populated_on_first_build` | 16 failed |
/// | `reusable`'s | `evict_unseen_drops_what_this_pass_did_not_touch` +2 | 4 failed |
/// | `reusable_mut`'s | `test_translate_path_reuses_cache_on_shared_delta_subtree_drag` | 2 failed |
///
/// The named tests are the durable half — a rename breaks
/// `test_no_doc_cites_a_test_that_does_not_exist` and this comment
/// with it. The counts are a snapshot and will drift as tests are
/// added; they are here to show the mutations were run, not to be
/// re-asserted.
///
/// The earlier version of this paragraph named `insert`, a function
/// deleted two commits before it was written, and claimed each
/// mutation fails "exactly one assertion below". A stated control
/// that does not reproduce is worse than none, because the next
/// reader trusts it; these three were run.
#[test]
fn test_every_route_out_of_the_connection_pass_keeps_its_edge_cached() {
    let map = two_node_edge_map();
    let key = EdgeKey::new("a", "b", "cross_link");
    let mut cache = SceneConnectionCache::new();

    // Route 1 — slow path: nothing cached, so this samples and writes.
    let _fresh = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert!(
        cache.inspect(&key).is_some(),
        "slow path: the edge it just sampled must survive the same pass's eviction"
    );

    // Route 2 — fast path: cache warm, no offsets.
    let _hit = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert!(
        cache.inspect(&key).is_some(),
        "cache-hit fast path: reusing an entry must count as touching it"
    );

    // Route 3 — translate path: both endpoints move by one delta.
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (0.0, 12.0));
    offsets.insert("b".to_string(), (0.0, 12.0));
    let _translated = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert!(
        cache.inspect(&key).is_some(),
        "translate path: shifting an entry in place must count as touching it"
    );
}

/// The element vector is reserved for the edge list rather than grown
/// from empty.
///
/// The observable is spare capacity on a map where most edges emit
/// nothing: portal-mode edges render in the portal pass, so eleven of
/// these twelve produce no `ConnectionElement` at all. A reserved
/// vector ends the pass with room for twelve; one grown from empty
/// ends it at `Vec`'s first non-zero capacity, observed to be four
/// for an element this size when the negative control was run. The
/// assertion is `>= 12` rather than `== 12` so it does not rest on
/// that number.
#[test]
fn test_the_element_vector_is_reserved_for_the_edge_list() {
    use crate::mindmap::test_helpers::synthetic_portal_edge;

    let mut nodes = vec![
        sized_node("a", 0.0, 0.0, 40.0, 40.0, false),
        sized_node("b", 400.0, 0.0, 40.0, 40.0, false),
    ];
    let mut edges = vec![synthetic_edge("a", "b", "right", "left")];
    for i in 0..11 {
        let id = format!("p{i}");
        nodes.push(sized_node(&id, 0.0, 600.0 + i as f64 * 60.0, 40.0, 40.0, false));
        edges.push(synthetic_portal_edge("a", &id, "#ffffff"));
    }
    let edge_count = edges.len();
    let map = synthetic_map(nodes, edges);

    let scene = project(&map, 1.0);
    assert_eq!(
        scene.connection_elements.len(),
        1,
        "precondition: only the one line edge emits, or capacity says nothing"
    );
    assert!(
        scene.connection_elements.capacity() >= edge_count,
        "the pass must reserve for the edge list: capacity {} for {} edges",
        scene.connection_elements.capacity(),
        edge_count
    );
}

/// A resample refills the buffer *this edge's own cache entry* was
/// holding, rather than allocating a new one.
///
/// The observable is spare capacity. Fill at a geometry that samples
/// to N points, then resample at one that wants fewer: the buffer
/// already there keeps the capacity it had, while a freshly allocated
/// one is reserved to the smaller count exactly. So the warm run's
/// entry ends up with slack and the cold run's does not, and the two
/// numbers are what tell the paths apart.
///
/// `SceneConnectionCache::refill` creates a missing entry with an
/// empty `Vec`, so preserved capacity is only reachable through the
/// already-cached branch — which makes this assertion also the
/// statement that the pass **reaches** that branch, and not only that
/// the branch would reuse a buffer if anything called it.
///
/// This test exists because the correctness test above does **not**
/// distinguish them — it was green with the buffer reuse replaced by
/// `Vec::new()`, which is the right answer for correctness and the
/// wrong one for the item. Without this, "the slow path reuses the
/// buffer" would be a claim only the diff could support.
#[test]
fn test_a_resample_reuses_the_cached_edges_own_buffer() {
    let map = two_node_edge_map();
    // Move `a` most of the way to `b`, so the edge is much shorter and
    // samples to far fewer points.
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (280.0, 0.0));
    let key = EdgeKey::new("a", "b", "cross_link");

    let mut warm = SceneConnectionCache::new();
    let _fill = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut warm,
        1.0,
    );
    let filled = warm.inspect(&key).expect("first build caches the edge");
    let filled_len = filled.pre_clip_positions.len();
    let filled_capacity = filled.pre_clip_positions.capacity();
    assert!(
        filled_len > 10,
        "precondition: the long edge samples to many points"
    );

    let _reused = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut warm,
        1.0,
    );
    let mut cold = SceneConnectionCache::new();
    let _fresh = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cold,
        1.0,
    );

    let warm_entry = warm.inspect(&key).expect("resample re-caches the edge");
    let cold_entry = cold.inspect(&key).expect("cold build caches the edge");
    assert!(
        warm_entry.pre_clip_positions.len() < filled_len,
        "precondition: the shortened edge must want fewer points ({} -> {}), or capacity \
         cannot tell a reused buffer from a fresh one",
        filled_len,
        warm_entry.pre_clip_positions.len()
    );
    assert_eq!(
        warm_entry.pre_clip_positions.len(),
        cold_entry.pre_clip_positions.len(),
        "precondition: both runs resample the same geometry"
    );

    assert_eq!(
        warm_entry.pre_clip_positions.capacity(),
        filled_capacity,
        "the resample must have refilled the entry's own buffer, keeping its capacity"
    );
    assert!(
        cold_entry.pre_clip_positions.capacity() < filled_capacity,
        "and a build with no buffer to reuse reserves only what it needs: {} vs {}",
        cold_entry.pre_clip_positions.capacity(),
        filled_capacity
    );
}

// --- #36 item 7: a cache entry must never outlive the config it was
// --- sampled and styled under -----------------------------------------

/// [`two_node_edge_map`] whose single edge carries an explicit
/// [`GlyphConnectionConfig`], so a test can edit exactly one field of
/// it between two builds and have that edit be the only difference
/// the second build sees.
fn map_with_connection_config(config: GlyphConnectionConfig) -> crate::mindmap::model::MindMap {
    let mut map = two_node_edge_map();
    map.edges[0].glyph_connection = Some(config);
    map
}

/// The starting config for the stale-config tests: every field the
/// sampler or the emitter reads is written out, so the edit under
/// test reads as a one-field diff rather than as a fall-through to
/// a different cascade tier.
fn baseline_connection_config() -> GlyphConnectionConfig {
    GlyphConnectionConfig {
        body: "\u{00B7}".into(),
        cap_start: Some("\u{25BA}".into()),
        cap_end: Some("\u{25C4}".into()),
        font: None,
        font_size_pt: 12.0,
        color: None,
        spacing: 0.0,
        ..GlyphConnectionConfig::default()
    }
}

/// Mutably reach the edge's own `GlyphConnectionConfig`. Every
/// caller built the map through [`map_with_connection_config`], so
/// the `Option` is populated by construction.
fn edge_config_mut(map: &mut crate::mindmap::model::MindMap) -> &mut GlyphConnectionConfig {
    map.edges[0]
        .glyph_connection
        .as_mut()
        .expect("fixture invariant: map_with_connection_config installs the config")
}

/// Two builds, one config edit between them, no flush and no
/// offsets — so the second build is a cache-hit build. Returns
/// `(first, second)` element pairs.
fn build_twice_across_config_edit(
    edit: impl FnOnce(&mut GlyphConnectionConfig),
) -> (super::super::ConnectionElement, super::super::ConnectionElement) {
    let mut map = map_with_connection_config(baseline_connection_config());
    let mut cache = SceneConnectionCache::new();
    let first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(
        cache.len(),
        1,
        "precondition: the first build must populate the cache, or the second build is not \
         exercising the cache-hit path this test is about"
    );
    edit(edge_config_mut(&mut map));
    let second = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(first.connection_elements.len(), 1);
    assert_eq!(second.connection_elements.len(), 1);
    let mut first = first;
    let mut second = second;
    (
        first.connection_elements.remove(0),
        second.connection_elements.remove(0),
    )
}

/// The body glyph the frame renders has to be the one the *model*
/// carries now, not the one the cache happened to be filled with.
///
/// Input that makes it fail: any `glyph_connection.body` edit that
/// reaches a rebuild without a `SceneConnectionCache::clear()` in
/// between — a console `edge glyph` edit, a `CustomMutation`, an
/// undo. The cache-hit fast path served `cached.body_glyph`, so the
/// canvas kept drawing the previous glyph until something unrelated
/// flushed the cache.
#[test]
fn test_cache_fast_path_tracks_a_body_glyph_edit_without_a_flush() {
    let (first, second) = build_twice_across_config_edit(|c| c.body = "X".into());
    assert_eq!(
        first.body_glyph, "\u{00B7}",
        "precondition: the first build renders the baseline glyph"
    );
    assert_eq!(
        second.body_glyph, "X",
        "a body-glyph edit with no cache flush must reach the canvas on the next rebuild"
    );
}

/// Same for the cap glyphs, which sit on the same config and reach
/// the element through the same cache entry.
#[test]
fn test_cache_fast_path_tracks_a_cap_glyph_edit_without_a_flush() {
    let (first, second) = build_twice_across_config_edit(|c| {
        c.cap_start = Some("S".into());
        c.cap_end = Some("E".into());
    });
    assert_eq!(
        first.cap_start.as_ref().map(|(g, _)| g.as_str()),
        Some("\u{25BA}"),
        "precondition: the first build renders the baseline start cap"
    );
    assert_eq!(second.cap_start.as_ref().map(|(g, _)| g.as_str()), Some("S"));
    assert_eq!(second.cap_end.as_ref().map(|(g, _)| g.as_str()), Some("E"));
}

/// The body color the frame renders is the one the *model* carries
/// now, like the four styling fields beside it.
///
/// This was the fifth field and the last one left: the cache held it,
/// both reuse doors handed it back, and neither compared it — the
/// exact shape the other four were removed for. A direct
/// `edge.color` edit reaching a rebuild without a
/// `SceneConnectionCache::clear()` kept the previous color on the
/// canvas.
///
/// Note this edits `edge.color` rather than a theme variable. The
/// caller-owed list named only theme variables, so the direct edit was
/// the case nothing covered in either direction — not the code, not
/// the documentation.
#[test]
fn test_cache_fast_path_tracks_an_edge_color_edit_without_a_flush() {
    let mut map = map_with_connection_config(baseline_connection_config());
    map.edges[0].color = "#112233".into();
    let mut cache = SceneConnectionCache::new();
    let first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(
        first.connection_elements[0].color, "#112233",
        "precondition: the first build renders the authored color"
    );
    assert_eq!(cache.len(), 1, "precondition: the first build filled the cache");

    // The only edit. No flush, no offsets, no zoom change — the second
    // build takes the cache-hit fast path.
    map.edges[0].color = "#445566".into();
    let second = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert!(
        drew_sentinel(&second.connection_elements[0]) || cache.len() == 1,
        "precondition: the edge is still cached"
    );
    assert_eq!(
        second.connection_elements[0].color, "#445566",
        "an edge-color edit with no cache flush must reach the canvas on the next rebuild"
    );
}

/// The same for the translate path, which served the cached color on
/// its own line fifty below the fast path's.
#[test]
fn test_translate_path_tracks_an_edge_color_edit_without_a_flush() {
    let mut map = map_with_connection_config(baseline_connection_config());
    map.edges[0].color = "#112233".into();
    let mut cache = SceneConnectionCache::new();
    let _first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    map.edges[0].color = "#445566".into();
    // A shared delta on both endpoints is the translate path's shape.
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (0.0, 9.0));
    offsets.insert("b".to_string(), (0.0, 9.0));
    let second = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(
        second.connection_elements[0].color, "#445566",
        "the translate path must read the live color too"
    );
}

/// A font-size edit changes two things at once and both have to
/// land: the size the element reports, and the arc-length step the
/// samples were taken at. Asserting only the first would pass on a
/// fix that copied the live size across but kept stale geometry.
///
/// Input that makes it fail: `font_size_pt` 12 -> 48 with no flush.
/// The cache-hit fast path reported `cached.font_size_pt` (12) and
/// reused samples spaced for a 12pt glyph, so 48pt glyphs drew four
/// deep on top of each other.
#[test]
fn test_cache_fast_path_resamples_when_the_font_size_changes() {
    let (first, second) = build_twice_across_config_edit(|c| c.font_size_pt = 48.0);
    assert!(
        crate::util::geometry::almost_equal(first.font_size_pt, 12.0),
        "precondition: the first build sampled at 12pt, got {}",
        first.font_size_pt
    );
    assert!(
        crate::util::geometry::almost_equal(second.font_size_pt, 48.0),
        "the element must report the live effective font size, got {}",
        second.font_size_pt
    );
    assert!(
        second.glyph_positions.len() < first.glyph_positions.len(),
        "4x the glyph size is 4x the arc-length step, so the resampled path must carry \
         strictly fewer body glyphs: {} -> {}",
        first.glyph_positions.len(),
        second.glyph_positions.len()
    );
}

/// `spacing` is the other addend of the arc-length step, and
/// neither reuse path compared it. Same shape as the font-size case
/// but with no presentational field to hide behind: the *only*
/// observable is the sample count.
#[test]
fn test_cache_fast_path_resamples_when_glyph_spacing_changes() {
    let (first, second) = build_twice_across_config_edit(|c| c.spacing = 60.0);
    assert!(
        second.glyph_positions.len() < first.glyph_positions.len(),
        "60 canvas units of extra spacing per glyph must thin the sampled path: {} -> {}",
        first.glyph_positions.len(),
        second.glyph_positions.len()
    );
}

/// The two caps belong to the two *ends* of the path, and which end
/// is which must not depend on how the element reached the frame.
///
/// The fixture puts `a` at x=0 and `b` at x=400 and anchors the edge
/// right-to-left, so the start cap sits at a's right edge and the end
/// cap at b's left edge — hundreds of canvas units apart, in that
/// order. A transposition of the two survives any assertion that only
/// counts caps or only checks one of them, which is why this asserts
/// the ordering on all three routes an element can take out of this
/// pass: fresh sample, cache hit, and rigid translate.
#[test]
fn test_caps_land_on_the_two_ends_of_the_path_on_every_cache_path() {
    fn assert_cap_order(elem: &super::super::ConnectionElement, route: &str) {
        let (_, (sx, _)) = elem
            .cap_start
            .as_ref()
            .unwrap_or_else(|| panic!("{route}: start cap should have survived clipping"));
        let (_, (ex, _)) = elem
            .cap_end
            .as_ref()
            .unwrap_or_else(|| panic!("{route}: end cap should have survived clipping"));
        assert!(
            *sx < *ex,
            "{route}: the start cap must sit left of the end cap on a left-to-right edge, \
             got start x={sx}, end x={ex}"
        );
        assert!(
            *sx < 200.0,
            "{route}: the start cap belongs at source node `a`'s right edge (x~40), got {sx}"
        );
        assert!(
            *ex > 200.0,
            "{route}: the end cap belongs at target node `b`'s left edge (x~400), got {ex}"
        );
    }

    let map = map_with_connection_config(baseline_connection_config());
    let mut cache = SceneConnectionCache::new();

    // Route 1 — slow path: nothing cached yet, so this samples fresh.
    let fresh = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(cache.len(), 1, "precondition: route 1 must have filled the cache");
    assert_cap_order(&fresh.connection_elements[0], "slow path");

    // Route 2 — fast path: same map, no offsets, cache warm.
    let hit = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_cap_order(&hit.connection_elements[0], "cache-hit fast path");

    // Route 3 — translate path: both endpoints move by one shared
    // delta, which is the subtree-drag shape.
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (0.0, 25.0));
    offsets.insert("b".to_string(), (0.0, 25.0));
    let translated = project_with_cache(
        &map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_cap_order(&translated.connection_elements[0], "translate path");
    // The delta actually applied, or route 3 proved nothing about a
    // translate: the caps must have moved down with their endpoints.
    let before_y = fresh.connection_elements[0]
        .cap_start
        .as_ref()
        .map(|(_, (_, y))| *y)
        .expect("route 1 start cap");
    let after_y = translated.connection_elements[0]
        .cap_start
        .as_ref()
        .map(|(_, (_, y))| *y)
        .expect("route 3 start cap");
    assert!(
        crate::util::geometry::almost_equal(after_y - before_y, 25.0),
        "precondition: the shared +25 y offset must have reached the caps, got {}",
        after_y - before_y
    );
}
