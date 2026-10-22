// SPDX-License-Identifier: MPL-2.0

//! Canvas hit-test resolution over the portal and connection-label
//! trees (§T1 — hit-test math is a fundamental).
//!
//! Two things are pinned here.
//!
//! **Overlap resolution is decided by geometry.** Portal icons can
//! overlap (two portal edges leaving the same node border point at
//! different marker sizes) and edge labels can overlap (crossing
//! edges). The winner is the smallest-area element — the topmost
//! one visually — and it must not move when the *names* of the
//! nodes involved change, nor between one run of the app and the
//! next.
//!
//! The predecessor resolved overlaps by `FxHashMap` iteration order
//! over `(EdgeKey, endpoint)` keys, so the winner was a function of
//! key hashes rather than of geometry. Over the 400-fixture sweep in
//! [`test_portal_overlap_always_resolves_to_the_smaller_icon`] it
//! picked the larger, occluded icon in roughly 220–250 of the 400 —
//! the exact count differs every run, because the renderer's setters
//! took a `std::collections::HashMap` and re-collected it into the
//! `FxHashMap`, so hashbrown's insertion order (and with it the
//! resolution of group collisions) rode on `std`'s per-process
//! `RandomState` seed. Five separate processes measured 231, 224,
//! 229, 218 and 229.
//!
//! **Migrating routing onto the tree changed no answers.** The
//! differential harness sweeps thousands of synthetic click points
//! across a whole map — a coarse lattice over the canvas plus a
//! fine lattice around each element plus every element's exact
//! corners — and compares the tree/BVH answer against an
//! independent oracle: a linear scan over the same rectangles the
//! deleted flat maps were built from. Every point covered by
//! exactly one rectangle must agree exactly; points covered by
//! several are where the old path was arbitrary, and the harness
//! asserts the new path picks the smallest-area candidate there.
//! Each sweep asserts its own coverage so it cannot silently
//! degenerate into probing empty canvas.
//!
//! The sweep found a real defect on the way in: the BVH's
//! shape-refinement step re-derived an area's local frame from
//! `position` and `render_bounds` instead of from the `min` / `max`
//! its own AABB test had just used, and f32's non-associative
//! addition made the two disagree on the far edge. See
//! `gfx_structs::tests::bvh_descent_tests::test_bvh_far_edge_hit_survives_float_rounding`.

use std::collections::HashMap;

use glam::Vec2;

use super::super::*;
use super::fixtures::*;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::model::{MindMap, PortalEndpointState};
use crate::mindmap::scene_cache::EdgeKey;
use crate::util::geometry::aabb_center;

/// `PORTAL_OUTSET_FRAC` — how far a marker sits off the border, as
/// a fraction of its font size. Mirrored from `portal_style` so the
/// overlap fixtures can cancel the offset difference between two
/// differently-sized markers and put them on top of each other.
const OUTSET_FRAC: f32 = 0.35;
/// Marker box side as a fraction of the font size, from
/// `layout_portal_label`.
const ICON_BOX_FRAC: f32 = 1.4;

/// Perpendicular slide that moves a marker of `font_size_pt` onto
/// the center of a marker of `other_font_size_pt` sharing the same
/// `border_t`. Both markers are centered at
/// `anchor + normal * (box/2 + outset + perp)`, so equalizing that
/// scalar equalizes the centers.
fn perp_to_align(font_size_pt: f32, other_font_size_pt: f32) -> f32 {
    let reach = |f: f32| f * ICON_BOX_FRAC * 0.5 + f * OUTSET_FRAC;
    reach(other_font_size_pt) - reach(font_size_pt)
}

/// A map with two portal edges leaving `hub` from the same border
/// point at different marker sizes, so the smaller marker sits
/// entirely inside the larger one. `hub_id` / `far_big` / `far_small`
/// are parameterized so a caller can sweep node names while holding
/// the geometry fixed.
fn overlapping_icons_map(hub_id: &str, far_big: &str, far_small: &str) -> MindMap {
    const BIG_PT: f32 = 50.0;
    const SMALL_PT: f32 = 16.0;
    let hub_side = |size_pt: f32, aligned_to: f32| PortalEndpointState {
        border_t: Some(0.5),
        perpendicular_offset: Some(perp_to_align(size_pt, aligned_to)),
        ..Default::default()
    };

    let mut map = synthetic_map(
        vec![
            synthetic_node(hub_id, None, 0.0, 0.0),
            synthetic_node(far_big, None, 900.0, 0.0),
            synthetic_node(far_small, None, 900.0, 400.0),
        ],
        vec![],
    );
    for (partner, size_pt) in [(far_big, BIG_PT), (far_small, SMALL_PT)] {
        let mut edge = synthetic_portal_edge(hub_id, partner, "#ff0000");
        if let Some(cfg) = edge.glyph_connection.as_mut() {
            cfg.font_size_pt = size_pt;
        }
        // The big marker keeps its natural offset; the small one
        // slides out to share the big one's center.
        edge.portal_from = Some(hub_side(size_pt, BIG_PT));
        map.edges.push(edge);
    }
    map
}

/// The `hub`-side icon area of the portal pair at `pair_idx`.
fn hub_icon(tree: &Tree<GfxElement, GfxMutator>, pair_idx: usize) -> GlyphArea {
    let pair = tree
        .root
        .children(&tree.arena)
        .nth(pair_idx)
        .expect("portal pair");
    let hub_endpoint = pair.children(&tree.arena).next().expect("from-endpoint void");
    let icon = hub_endpoint.children(&tree.arena).next().expect("icon leaf");
    glyph_area_of(tree, icon).clone()
}

#[test]
fn test_portal_overlap_resolves_to_the_topmost_smaller_icon() {
    // Two markers stacked on one border point. The click lands in
    // the overlap; the answer must be the visually topmost element,
    // which for stacked markers is the smaller one — anything else
    // routes the click to a portal the user cannot see there.
    let map = overlapping_icons_map("hub", "big_partner", "small_partner");
    let mut portal = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0);

    let big = hub_icon(&portal.tree, 0);
    let small = hub_icon(&portal.tree, 1);
    let (big_pos, big_extent) = area_rect(&big);
    let (small_pos, small_extent) = area_rect(&small);
    assert!(
        small_extent.x < big_extent.x && small_extent.y < big_extent.y,
        "fixture must stack a smaller marker on a larger one; got {small_extent:?} vs {big_extent:?}"
    );
    // The fixture is only meaningful if the two really overlap.
    assert!(
        small_pos.x >= big_pos.x
            && small_pos.y >= big_pos.y
            && small_pos.x + small_extent.x <= big_pos.x + big_extent.x
            && small_pos.y + small_extent.y <= big_pos.y + big_extent.y,
        "fixture must nest the small marker inside the large one; \
         small=[{small_pos:?} +{small_extent:?}] big=[{big_pos:?} +{big_extent:?}]"
    );

    let hit = portal_hit_at(&mut portal, area_center(&small)).expect("overlap point must hit");
    assert_eq!(
        hit,
        PortalHit {
            edge_key: EdgeKey::new("hub", "small_partner", "cross_link"),
            endpoint_node_id: "hub".to_string(),
            part: PortalPart::Icon,
        },
        "the overlap must resolve to the smaller (topmost) marker"
    );

    // A point inside the large marker but outside the small one
    // still resolves to the large marker — the rule is
    // smallest-*containing*, not "always the smallest present".
    let big_only = Vec2::new(big_pos.x + 1.0, big_pos.y + big_extent.y * 0.5);
    assert!(
        big_only.x < small_pos.x,
        "probe must fall outside the nested marker"
    );
    assert_eq!(
        portal_hit_at(&mut portal, big_only).map(|h| h.edge_key),
        Some(EdgeKey::new("hub", "big_partner", "cross_link"))
    );
}

#[test]
fn test_portal_overlap_always_resolves_to_the_smaller_icon() {
    // The same stacked geometry, 400 times, varying only the node
    // ids. Geometry never moves, so a geometry-driven hit test
    // answers identically every time. The `FxHashMap` scan this
    // replaces answered from the key hashes instead and picked the
    // larger, occluded marker in 244 of these 400 cases.
    let mut wrong = 0usize;
    let mut checked = 0usize;
    for i in 0..20 {
        for j in 0..20 {
            let (hub, big, small) = (format!("h{i}"), format!("n{i}"), format!("m{j}"));
            let map = overlapping_icons_map(&hub, &big, &small);
            let mut portal = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0);
            let probe = area_center(&hub_icon(&portal.tree, 1));
            let hit = portal_hit_at(&mut portal, probe).expect("overlap point must hit");
            checked += 1;
            if hit.edge_key != EdgeKey::new(&hub, &small, "cross_link") {
                wrong += 1;
            }
        }
    }
    assert_eq!(
        wrong, 0,
        "{wrong} of {checked} identical-geometry fixtures resolved to the wrong marker — \
         the winner is not supposed to depend on node ids"
    );
}

// ── differential harness ──────────────────────────────────────────
//
// The oracle is a linear scan over exactly the rectangles the
// deleted flat maps were populated from — a leaf's `position` +
// `render_bounds` *is* the rectangle those maps stored — with the
// documented tie-break (smallest area, earlier sibling on an exact
// tie) made explicit. Points covered by one rectangle are where the
// old and new paths must agree exactly; points covered by several
// are where the old path was arbitrary.

/// One candidate rectangle in the oracle, with the identity the hit
/// path is expected to produce for it.
struct OracleRect<T> {
    min: Vec2,
    max: Vec2,
    identity: T,
}

/// Tally of what a sweep exercised. Asserted non-degenerate by every
/// caller — a harness that only ever probes empty space proves
/// nothing.
#[derive(Default, Debug)]
struct SweepCoverage {
    misses: usize,
    single: usize,
    overlapping: usize,
}

impl SweepCoverage {
    /// Bucket one probe by how many oracle rectangles contained it.
    fn record(&mut self, candidates: usize) {
        match candidates {
            0 => self.misses += 1,
            1 => self.single += 1,
            _ => self.overlapping += 1,
        }
    }
}

/// Resolve `point` against the oracle: the smallest-area rectangle
/// containing it, ties broken toward the earlier entry. Returns the
/// winner and how many rectangles contained the point.
fn oracle_at<T>(rects: &[OracleRect<T>], point: Vec2) -> (Option<&T>, usize) {
    let mut best: Option<(&T, f32)> = None;
    let mut count = 0usize;
    for r in rects {
        if point.x < r.min.x || point.x > r.max.x || point.y < r.min.y || point.y > r.max.y {
            continue;
        }
        count += 1;
        let area = (r.max.x - r.min.x) * (r.max.y - r.min.y);
        match best {
            Some((_, best_area)) if best_area <= area => {}
            _ => best = Some((&r.identity, area)),
        }
    }
    (best.map(|(id, _)| id), count)
}

/// Fill `out` with a `steps × steps` lattice spanning
/// `[lo, hi]` inclusive.
fn push_lattice(out: &mut Vec<Vec2>, lo: Vec2, hi: Vec2, steps: usize) {
    let denom = (steps - 1).max(1) as f32;
    for iy in 0..steps {
        for ix in 0..steps {
            let t = Vec2::new(ix as f32 / denom, iy as f32 / denom);
            out.push(lo + (hi - lo) * t);
        }
    }
}

/// Probe points covering `rects`, in three layers:
///
/// 1. A coarse lattice over the whole union AABB inflated by a
///    margin, so empty canvas gets swept too.
/// 2. A fine lattice per rectangle over that rectangle inflated by
///    30%, so every rectangle is densely covered *and* its
///    immediate surroundings are probed for false positives. A
///    single global lattice cannot do this: portal markers are tens
///    of pixels across on a canvas hundreds of pixels wide, so a
///    grid coarse enough to be cheap lands almost entirely on empty
///    space.
/// 3. Each rectangle's center and four exact corners, so
///    closed-interval boundary semantics are exercised on purpose
///    rather than by luck.
fn sweep_points<T>(rects: &[OracleRect<T>], global_steps: usize, local_steps: usize) -> Vec<Vec2> {
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for r in rects {
        min = min.min(r.min);
        max = max.max(r.max);
    }
    assert!(min.x.is_finite(), "sweep needs at least one rectangle");
    let margin = ((max - min) * 0.05).max(Vec2::splat(1.0));

    let mut points =
        Vec::with_capacity(global_steps * global_steps + rects.len() * (local_steps * local_steps + 5));
    push_lattice(&mut points, min - margin, max + margin, global_steps);
    for r in rects {
        let pad = (r.max - r.min) * 0.3;
        push_lattice(&mut points, r.min - pad, r.max + pad, local_steps);
        points.push((r.min + r.max) * 0.5);
        points.push(r.min);
        points.push(Vec2::new(r.max.x, r.min.y));
        points.push(Vec2::new(r.min.x, r.max.y));
        points.push(r.max);
    }
    points
}

/// Every clickable portal leaf as an oracle rectangle, in tree
/// order. Zero-extent leaves (reserved text slots on text-less
/// endpoints) are excluded — matching both the BVH, which requires
/// strictly positive bounds, and the deleted `text_hitboxes` map,
/// which suppressed the same entries.
fn portal_oracle(portal: &PortalTree) -> Vec<OracleRect<PortalHit>> {
    let tree = &portal.tree;
    let mut out = Vec::new();
    for pair in tree.root.children(&tree.arena) {
        for endpoint in pair.children(&tree.arena) {
            for leaf in endpoint.children(&tree.arena) {
                let (pos, extent) = area_rect(glyph_area_of(tree, leaf));
                if extent.x <= 0.0 || extent.y <= 0.0 {
                    continue;
                }
                let identity = portal
                    .hit_index
                    .resolve(tree, leaf)
                    .expect("every portal leaf must resolve to an identity");
                out.push(OracleRect {
                    min: pos,
                    max: pos + extent,
                    identity,
                });
            }
        }
    }
    out
}

/// Every connection-label leaf as an oracle rectangle, in tree
/// order.
fn label_oracle(labels: &ConnectionLabelTree) -> Vec<OracleRect<EdgeKey>> {
    let tree = &labels.tree;
    let mut out = Vec::new();
    for leaf in tree.root.children(&tree.arena) {
        let (pos, extent) = area_rect(glyph_area_of(tree, leaf));
        if extent.x <= 0.0 || extent.y <= 0.0 {
            continue;
        }
        let identity = labels
            .hit_index
            .resolve(tree, leaf)
            .expect("every label leaf must resolve to an edge");
        out.push(OracleRect {
            min: pos,
            max: pos + extent,
            identity,
        });
    }
    out
}

/// A portal map with enough variety to be worth sweeping: bare
/// icons, icons with text on one or both sides, a marker dragged
/// to a custom border position, and one nested-overlap pair.
fn portal_sweep_map() -> MindMap {
    let mut map = overlapping_icons_map("hub", "big_partner", "small_partner");
    map.nodes
        .insert("t1".to_string(), synthetic_node("t1", None, 0.0, 500.0));
    map.nodes
        .insert("t2".to_string(), synthetic_node("t2", None, 300.0, 500.0));
    map.nodes
        .insert("t3".to_string(), synthetic_node("t3", None, 300.0, 700.0));

    // Text on both endpoints.
    let mut both = synthetic_portal_edge("t1", "t2", "#00ff00");
    both.portal_from = Some(PortalEndpointState {
        text: Some("alpha".to_string()),
        ..Default::default()
    });
    both.portal_to = Some(PortalEndpointState {
        text: Some("omega".to_string()),
        ..Default::default()
    });
    map.edges.push(both);

    // Text on one endpoint only, and a dragged border position on
    // the other so the marker leaves the default corner.
    let mut one = synthetic_portal_edge("t1", "t3", "#0000ff");
    one.portal_from = Some(PortalEndpointState {
        text: Some("solo".to_string()),
        border_t: Some(2.25),
        perpendicular_offset: Some(6.0),
        ..Default::default()
    });
    map.edges.push(one);

    map
}

#[test]
fn test_portal_tree_hit_matches_a_linear_scan_over_the_same_rectangles() {
    let map = portal_sweep_map();
    let mut portal = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0);
    let oracle = portal_oracle(&portal);
    assert!(
        oracle.len() >= 10,
        "sweep map should be non-trivial; got {}",
        oracle.len()
    );

    let mut coverage = SweepCoverage::default();
    for point in sweep_points(&oracle, 40, 24) {
        let (expected, candidates) = oracle_at(&oracle, point);
        let actual = portal_hit_at(&mut portal, point);
        assert_eq!(
            actual.as_ref(),
            expected,
            "portal hit disagreed with the linear scan at {point:?} ({candidates} candidates)"
        );
        coverage.record(candidates);
    }

    assert!(
        coverage.misses > 100,
        "sweep must exercise empty space: {coverage:?}"
    );
    assert!(
        coverage.single > 1000,
        "sweep must densely exercise unambiguous hits — these are the points where \
         the deleted flat maps and the tree path have to agree exactly: {coverage:?}"
    );
    assert!(
        coverage.overlapping > 100,
        "sweep must exercise overlaps — these are the points the deleted flat \
         maps resolved arbitrarily: {coverage:?}"
    );
}

#[test]
fn test_connection_label_tree_hit_matches_a_linear_scan_over_the_same_rectangles() {
    // The canonical fixture — every labeled edge in `testament`,
    // laid out by the real loader — swept against the oracle.
    let map = crate::mindmap::loader::load_from_file(&test_map_path()).expect("testament map loads");
    let hidden = map.fold_hidden_set();
    let offsets = HashMap::new();
    let mut paths = super::super::EdgePathCache::new(&map, &offsets);
    let elements = build_label_elements(&map, None, None, None, &mut paths, 1.0, &hidden);
    assert!(
        elements.len() >= 10,
        "fixture should carry labels; got {}",
        elements.len()
    );

    let mut labels = build_connection_label_tree(&elements);
    // The index names exactly the labels the tree carries — a
    // shorter index would silently answer `None` on the tail
    // channels, which the sweep below would then blame on the BVH.
    assert!(!labels.hit_index.is_empty());
    assert_eq!(labels.hit_index.len(), elements.len());
    let oracle = label_oracle(&labels);
    assert_eq!(oracle.len(), elements.len());

    let mut coverage = SweepCoverage::default();
    for point in sweep_points(&oracle, 40, 20) {
        let (expected, candidates) = oracle_at(&oracle, point);
        let node_id = labels.tree.descendant_at(point);
        let actual = node_id.and_then(|n| labels.hit_index.resolve(&labels.tree, n));
        assert_eq!(
            actual.as_ref(),
            expected,
            "label hit disagreed with the linear scan at {point:?} ({candidates} candidates)"
        );
        coverage.record(candidates);
    }
    assert!(
        coverage.misses > 100,
        "sweep must exercise empty space: {coverage:?}"
    );
    assert!(
        coverage.single > 1000,
        "sweep must densely exercise unambiguous hits: {coverage:?}"
    );
}

/// Two line-mode edges crossing at right angles, each carrying a
/// label pinned to the crossing point, with different label
/// lengths so their AABBs overlap at different areas. The
/// `testament` fixture happens to have no overlapping labels, and
/// crossing-edge label overlap is precisely the case the deleted
/// `connection_label_hitboxes` scan resolved by hash order.
fn crossing_labels_map() -> MindMap {
    use crate::mindmap::model::EdgeLabelConfig;

    let mut map = synthetic_map(
        vec![
            synthetic_node("w", None, -300.0, 0.0),
            synthetic_node("e", None, 300.0, 0.0),
            synthetic_node("n", None, 0.0, -300.0),
            synthetic_node("s", None, 0.0, 300.0),
        ],
        vec![],
    );
    for (a, b, text) in [("w", "e", "a much longer label"), ("n", "s", "short")] {
        let mut e = synthetic_edge(a, b, "auto", "auto");
        e.label = Some(text.to_string());
        e.label_config = Some(EdgeLabelConfig {
            position_t: Some(0.5),
            ..Default::default()
        });
        map.edges.push(e);
    }
    map
}

#[test]
fn test_crossing_edge_labels_resolve_to_the_smaller_label() {
    let map = crossing_labels_map();
    let hidden = map.fold_hidden_set();
    let offsets = HashMap::new();
    let mut paths = super::super::EdgePathCache::new(&map, &offsets);
    let elements = build_label_elements(&map, None, None, None, &mut paths, 1.0, &hidden);
    assert_eq!(elements.len(), 2);

    let mut labels = build_connection_label_tree(&elements);
    let oracle = label_oracle(&labels);
    let mut coverage = SweepCoverage::default();
    for point in sweep_points(&oracle, 40, 30) {
        let (expected, candidates) = oracle_at(&oracle, point);
        let node_id = labels.tree.descendant_at(point);
        let actual = node_id.and_then(|n| labels.hit_index.resolve(&labels.tree, n));
        assert_eq!(
            actual.as_ref(),
            expected,
            "label hit disagreed with the linear scan at {point:?} ({candidates} candidates)"
        );
        coverage.record(candidates);
    }
    assert!(
        coverage.overlapping > 100,
        "fixture must actually overlap the two labels: {coverage:?}"
    );

    // Both labels are centered on the crossing point, so the
    // shorter one nests inside the longer one and wins there.
    let short = &elements[1];
    let center = aabb_center(
        Vec2::new(short.position.0, short.position.1),
        Vec2::new(short.bounds.0, short.bounds.1),
    );
    let node_id = labels
        .tree
        .descendant_at(center)
        .expect("crossing point must hit");
    assert_eq!(
        labels.hit_index.resolve(&labels.tree, node_id),
        Some(EdgeKey::new("n", "s", "cross_link")),
        "the smaller of two overlapping labels owns the overlap"
    );
}
