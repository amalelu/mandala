// SPDX-License-Identifier: MPL-2.0

//! The per-frame [`EdgePathCache`]: one build per edge across the
//! three passes that want one, none at all for an edge no pass asks
//! about, and the endpoint-rect resolution the memo shares with the
//! sampler.

use super::super::{EdgePathCache, SceneSelectionContext};
use super::fixtures::*;
use crate::mindmap::connection::ConnectionPath;
use crate::mindmap::model::{ControlPoint, GlyphConnectionConfig, MindMap};
use crate::mindmap::scene_cache::SceneConnectionCache;
use crate::mindmap::test_helpers::synthetic_edge;
use glam::Vec2;
use std::collections::HashMap;

/// The two-node fixture with a label on its single edge, so the
/// label pass has something to lay out.
fn labeled_edge_map() -> MindMap {
    let mut map = two_node_edge_map();
    map.edges[0].label = Some("hello".to_string());
    map
}

/// Y of the straight chord between the fixture's two anchors — a's
/// right edge at (40, 20), b's left edge at (400, 20).
const CHORD_Y: f32 = 20.0;

/// The control-point offsets [`curved_edge_map`] installs, as offsets
/// from the respective node centers — which is how the format stores
/// them and how `build_connection_path` reads them.
const CURVE_OFFSET_Y: f64 = 180.0;

/// [`two_node_edge_map`] whose single edge carries two control
/// points, so its path is a cubic Bezier rather than a straight
/// chord.
///
/// Every other fixture in this file is straight and axis-aligned and
/// control-point-free, which means none of them can see a path that
/// dropped its control points on the way through. Node centers are
/// (20, 20) and (420, 20), so with both offsets at `+180` in y the
/// control points land at (20, 200) and (420, 200) and the curve
/// bulges to `y = 155` at its midpoint — 135 units off the chord,
/// which no rounding can explain away.
fn curved_edge_map() -> MindMap {
    let mut map = two_node_edge_map();
    map.edges[0].control_points = vec![
        ControlPoint {
            x: 0.0,
            y: CURVE_OFFSET_Y,
        },
        ControlPoint {
            x: 0.0,
            y: CURVE_OFFSET_Y,
        },
    ];
    map
}

/// [`curved_edge_map`] with a label, for the label-layout half.
fn curved_labeled_edge_map() -> MindMap {
    let mut map = curved_edge_map();
    map.edges[0].label = Some("hello".to_string());
    map
}

/// The memo hands out the edge's *curve*, control points and all.
///
/// Input that makes it fail: `EdgePathCache::path` passing `&[]` for
/// `control_points`, which is a one-token slip that turns every
/// curved edge in the document into a straight chord.
#[test]
fn test_the_shared_path_carries_the_edges_control_points() {
    let map = curved_edge_map();
    let offsets = HashMap::new();
    let mut paths = EdgePathCache::new(&map, &offsets);

    let ConnectionPath::CubicBezier {
        start,
        control1,
        control2,
        end,
    } = paths.path(0).expect("the fixture edge resolves").clone()
    else {
        panic!("an edge with two control points must project to a cubic, not a straight chord");
    };
    // Anchors are unchanged by the curve — a's right edge and b's
    // left edge — which is why the anchors alone cannot detect the
    // slip and the control points have to be read.
    assert_eq!(start, Vec2::new(40.0, CHORD_Y));
    assert_eq!(end, Vec2::new(400.0, CHORD_Y));
    // Offsets from the node centers (20, 20) and (420, 20).
    assert_eq!(control1, Vec2::new(20.0, 20.0 + CURVE_OFFSET_Y as f32));
    assert_eq!(control2, Vec2::new(420.0, 20.0 + CURVE_OFFSET_Y as f32));
}

/// The samples the connection pass draws follow the curve.
///
/// The chord is flat at `y = 20`, so a path that lost its control
/// points puts every sample there. Asserting on the *emitted element*
/// rather than on the memo is the point: this is the sampler reading
/// the shared path, which is the consumer the sharing introduced.
#[test]
fn test_the_sampler_draws_the_curve_the_shared_path_carries() {
    let map = curved_edge_map();
    let scene = project(&map, 1.0);
    assert_eq!(scene.connection_elements.len(), 1);
    let off_chord = scene.connection_elements[0]
        .glyph_positions
        .iter()
        .filter(|(_, y)| (y - CHORD_Y).abs() > 10.0)
        .count();
    assert!(
        off_chord > 5,
        "a curved edge must sample off its chord; {} of {} points left y={CHORD_Y}",
        off_chord,
        scene.connection_elements[0].glyph_positions.len()
    );
}

/// The label sits on the curve, not on the chord.
///
/// `point_at_t(path, 0.5)` is the whole of the label's dependency on
/// the shared path, and for this fixture the two answers are 135
/// canvas units apart.
#[test]
fn test_the_label_layout_follows_the_curve_the_shared_path_carries() {
    let map = curved_labeled_edge_map();
    let scene = project(&map, 1.0);
    assert_eq!(scene.connection_label_elements.len(), 1);
    let label = &scene.connection_label_elements[0];
    let center_y = label.position.1 + label.bounds.1 * 0.5;
    // B(0.5) of this curve is (220, 155): (start + 3c1 + 3c2 + end)/8.
    assert!(
        crate::util::geometry::almost_equal(center_y, 155.0),
        "the label belongs at the curve's midpoint y=155, got {center_y}"
    );
}

/// The grab-handles are **not** in this set, and that is a fact about
/// the code rather than a gap in the corpus.
///
/// `build_edge_handles` reads the shared path only for its `start`
/// and `end`, and `build_connection_path` gives a cubic the same
/// anchors it gives the straight form — so dropping the control
/// points cannot move a handle. What decides the handle *set* is
/// `edge.control_points`, which the emitter reads off the edge
/// directly and not through the memo. This test pins both halves, so
/// a future change that routes the handle set through the path has to
/// notice it is doing so.
#[test]
fn test_the_grab_handles_read_the_shared_path_only_for_its_endpoints() {
    let straight = project_with_overrides(
        &two_node_edge_map(),
        &HashMap::new(),
        SceneSelectionContext {
            edge: Some(("a", "b", "cross_link")),
            ..Default::default()
        },
        None,
        None,
        None,
        1.0,
    );
    let curved = project_with_overrides(
        &curved_edge_map(),
        &HashMap::new(),
        SceneSelectionContext {
            edge: Some(("a", "b", "cross_link")),
            ..Default::default()
        },
        None,
        None,
        None,
        1.0,
    );

    let anchors = |roles: &ProjectedRoles| -> Vec<(f32, f32)> {
        roles
            .edge_handles
            .iter()
            .filter(|h| {
                matches!(
                    h.kind,
                    super::super::EdgeHandleKind::AnchorFrom | super::super::EdgeHandleKind::AnchorTo
                )
            })
            .map(|h| h.position)
            .collect()
    };
    assert_eq!(
        anchors(&straight).len(),
        2,
        "precondition: both anchors are emitted"
    );
    assert_eq!(
        anchors(&straight),
        anchors(&curved),
        "the anchors a handle reads off the path are the same for a chord and its curve"
    );
    // And the handle *set* differs, because that half comes off the
    // edge rather than off the path.
    assert!(
        straight
            .edge_handles
            .iter()
            .any(|h| h.kind == super::super::EdgeHandleKind::Midpoint),
        "a straight edge offers the drag-to-curve midpoint"
    );
    assert!(
        curved
            .edge_handles
            .iter()
            .any(|h| h.kind == super::super::EdgeHandleKind::ControlPoint(1)),
        "a two-control-point edge offers a handle for the second one"
    );
}

/// One connection path per edge per frame, however many passes want
/// it.
///
/// The fixture edge is labeled *and* selected, and the scene cache
/// starts empty, so all three askers fire in one rebuild: the
/// sampler on its slow path, the selected edge's grab-handles, and
/// the label layout. Each used to call `build_connection_path`
/// itself.
///
/// Input that makes it fail: any of the three going back to building
/// its own — the count goes to 2 or 3, and the assertion names which.
#[test]
fn test_one_connection_path_serves_the_sampler_the_handles_and_the_label() {
    let map = labeled_edge_map();
    let mut cache = SceneConnectionCache::new();
    let roles = project_with_cache(
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

    // Preconditions: all three consumers really did run, or the
    // count below is 1 because two of them never asked.
    assert_eq!(
        roles.connection_elements.len(),
        1,
        "precondition: the sampler must have emitted this edge"
    );
    assert!(
        !roles.edge_handles.is_empty(),
        "precondition: the selected edge must have emitted grab-handles"
    );
    assert_eq!(
        roles.connection_label_elements.len(),
        1,
        "precondition: the labeled edge must have emitted a label"
    );
    assert!(
        cache.is_empty() || cache.len() == 1,
        "precondition: the sampler took its slow path and wrote the cache"
    );

    assert_eq!(
        roles.paths_built, 1,
        "sampler, handles and label share one built path per edge per frame"
    );
}

/// Nothing is built for an edge no pass needs a path for.
///
/// The failure this guards against is the obvious wrong fix for the
/// item above: resolve every edge's path up front and hand out a
/// slice. That trades three builds on a few edges for one build on
/// all of them, and on a static frame — cache warm, no label, no
/// selection — the right answer is zero.
///
/// Input that makes it fail: making the memo eager, in which case
/// the second build reports one path for an edge nothing drew from a
/// path.
#[test]
fn test_no_connection_path_is_built_for_an_edge_no_pass_asks_about() {
    // Unlabeled and never selected.
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
    assert_eq!(
        first.paths_built, 1,
        "precondition: the first build misses the scene cache and does sample"
    );
    assert_eq!(
        cache.len(),
        1,
        "precondition: the first build filled the scene cache"
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
    assert_eq!(
        second.connection_elements.len(),
        1,
        "precondition: the edge still renders, from the scene cache"
    );
    assert_eq!(
        second.paths_built, 0,
        "a cache-hit edge with no label and no selection needs no path at all"
    );
}

/// A labeled edge still gets its one path on a scene-cache hit — the
/// label pass has no cache of its own, so it is the one asker left.
///
/// This is the other half of the test above: without it, "0 paths on
/// a warm frame" would also pass on an implementation that never
/// built a path for labels at all, which would put every label at
/// the origin.
#[test]
fn test_a_labeled_edge_still_gets_its_path_on_a_scene_cache_hit() {
    let map = labeled_edge_map();
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
    assert_eq!(
        second.paths_built, 1,
        "the label pass has no cache of its own and must still resolve the path"
    );
    // And the label landed somewhere on the edge rather than at the
    // origin: the fixture runs from a's right edge (x=40) to b's left
    // edge (x=400) at y=20, so the midpoint label sits near (220, 20).
    let label = &second.connection_label_elements[0];
    let center = (
        label.position.0 + label.bounds.0 * 0.5,
        label.position.1 + label.bounds.1 * 0.5,
    );
    assert!(
        crate::util::geometry::almost_equal(center.0, 220.0)
            && crate::util::geometry::almost_equal(center.1, 20.0),
        "label should sit at the path midpoint, got {center:?}"
    );
}

/// The memo answers a repeat ask from storage.
#[test]
fn test_edge_path_cache_builds_each_edge_once() {
    let map = two_node_edge_map();
    let offsets = HashMap::new();
    let mut paths = EdgePathCache::new(&map, &offsets);
    assert_eq!(paths.built(), 0, "a fresh memo has built nothing");

    let first = paths.path(0).expect("the fixture edge resolves").clone();
    assert_eq!(paths.built(), 1);
    let second = paths.path(0).expect("the fixture edge resolves").clone();
    assert_eq!(paths.built(), 1, "the second ask must not build");

    let (ConnectionPath::Straight { start: s1, end: e1 }, ConnectionPath::Straight { start: s2, end: e2 }) =
        (first, second)
    else {
        panic!("the fixture edge has no control points, so its path is straight");
    };
    assert_eq!((s1, e1), (s2, e2), "both asks must describe the same path");
}

/// Out-of-range and dangling edges answer `None` and build nothing,
/// rather than panicking on the index or caching a path resolved
/// from a missing node.
#[test]
fn test_edge_path_cache_declines_a_dangling_or_out_of_range_edge() {
    let mut map = two_node_edge_map();
    map.edges
        .push(synthetic_edge("a", "nonexistent", "right", "left"));
    let offsets = HashMap::new();
    let mut paths = EdgePathCache::new(&map, &offsets);

    assert!(
        paths.path(1).is_none(),
        "an edge whose target is missing has no path"
    );
    assert!(paths.path(99).is_none(), "an out-of-range index has no path");
    assert_eq!(paths.built(), 0, "neither may occupy a slot");

    // The healthy sibling is unaffected.
    assert!(paths.path(0).is_some());
    assert_eq!(paths.built(), 1);
}

/// The memo resolves an edge's endpoints through the same drag
/// offsets the sampler does, so a shared path is the *moved* path
/// rather than the committed one.
///
/// Input that makes it fail: a memo that read `node.pos_vec2()`
/// directly — the path would be right on a static frame and a whole
/// drag delta wrong on every drag frame, which is the sort of thing
/// that only shows up in motion.
#[test]
fn test_edge_path_cache_applies_the_drag_offsets() {
    let map = two_node_edge_map();
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (0.0, 60.0));

    let no_offset = HashMap::new();
    let mut still = EdgePathCache::new(&map, &no_offset);
    let mut dragged = EdgePathCache::new(&map, &offsets);

    let (ConnectionPath::Straight { start: rest, .. }, ConnectionPath::Straight { start: moved, .. }) = (
        still.path(0).expect("static path").clone(),
        dragged.path(0).expect("dragged path").clone(),
    ) else {
        panic!("the fixture edge has no control points, so its path is straight");
    };
    assert!(
        crate::util::geometry::almost_equal(moved.y - rest.y, 60.0),
        "the source anchor must ride node a's +60 y offset: {} -> {}",
        rest.y,
        moved.y
    );
}

/// `offset_node_rect` is the one spelling of "this node's live
/// rectangle" — the size passes through untouched and only the
/// position moves.
#[test]
fn test_offset_node_rect_moves_the_position_and_not_the_size() {
    let node = sized_node("n", 10.0, 20.0, 30.0, 40.0, false);
    let mut offsets = HashMap::new();
    offsets.insert("n".to_string(), (3.0, -7.0));

    let (pos, size) = super::super::offset_node_rect(&node, &offsets);
    assert_eq!(pos, Vec2::new(13.0, 13.0));
    assert_eq!(size, Vec2::new(30.0, 40.0));

    let (unmoved, same_size) = super::super::offset_node_rect(&node, &HashMap::new());
    assert_eq!(unmoved, Vec2::new(10.0, 20.0), "an unlisted node does not move");
    assert_eq!(same_size, size);
}

/// A glyph-config edit does not change where the path runs, so a
/// path already in the memo is still the right one for it.
///
/// This is the boundary between the two memos this pass now carries:
/// `SceneConnectionCache` refuses geometry sampled under different
/// `SampleParams` (#36 item 7), while `EdgePathCache` correctly does
/// not care, because `build_connection_path` reads no config at all.
/// Asserting it keeps a future "be safe, key the path on the config
/// too" from landing unnoticed.
#[test]
fn test_the_connection_path_does_not_depend_on_the_glyph_config() {
    let base = two_node_edge_map();
    let mut restyled = two_node_edge_map();
    restyled.edges[0].glyph_connection = Some(GlyphConnectionConfig {
        body: "#".into(),
        cap_start: Some("<".into()),
        cap_end: Some(">".into()),
        font: Some("Some Other Family".into()),
        font_size_pt: 40.0,
        spacing: 12.0,
        ..GlyphConnectionConfig::default()
    });

    let offsets = HashMap::new();
    let mut a = EdgePathCache::new(&base, &offsets);
    let mut b = EdgePathCache::new(&restyled, &offsets);
    let (ConnectionPath::Straight { start: s1, end: e1 }, ConnectionPath::Straight { start: s2, end: e2 }) = (
        a.path(0).expect("base path").clone(),
        b.path(0).expect("restyled path").clone(),
    ) else {
        panic!("the fixture edge has no control points, so its path is straight");
    };
    assert_eq!(
        (s1, e1),
        (s2, e2),
        "the path is geometry; the glyph config is not"
    );
}
