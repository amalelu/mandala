// SPDX-License-Identifier: MPL-2.0

//! Portal tree builder tests — marker emission, fold filtering, selection highlight, ascending channels, mutator round-trip, structural signature. The signatures' own coverage lives in [`super::canvas_signature`]. Edges with `display_mode = "portal"` drive the portal pass.

use super::super::*;
use super::fixtures::*;
use crate::mindmap::scene_cache::EdgeKey;

#[test]
fn portal_tree_emits_two_markers_per_edge() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
        ],
        vec![],
    );
    map.edges.push(synthetic_portal_edge("a", "b", "#ff0000"));

    let result = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0);
    let pairs: Vec<NodeId> = result.tree.root.children(&result.tree.arena).collect();
    assert_eq!(pairs.len(), 1);

    // New shape: pair → endpoint void → [icon, text]. Two endpoints
    // per edge, each with two GlyphArea children (icon + text).
    let endpoint_voids: Vec<NodeId> = pairs[0].children(&result.tree.arena).collect();
    assert_eq!(endpoint_voids.len(), 2);
    for ev in &endpoint_voids {
        let leaves: Vec<NodeId> = ev.children(&result.tree.arena).collect();
        assert_eq!(leaves.len(), 2, "icon + text under each endpoint void");
    }
    // The hit index names the one visible pair; its two
    // endpoints are the clickable units. Both endpoints have
    // empty text in this fixture, so their text slots lay out at
    // zero extent and only the icons are hittable.
    assert_eq!(result.hit_index.len(), 1);
    assert_eq!(hittable_parts(&result), vec![PortalPart::Icon, PortalPart::Icon]);
}

#[test]
fn portal_tree_skips_edge_with_folded_endpoint() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("parent", None, 0.0, 0.0),
            synthetic_node("child", Some("parent"), 0.0, 100.0),
            synthetic_node("other", None, 200.0, 0.0),
        ],
        vec![],
    );
    map.nodes.get_mut("parent").unwrap().folded = true;
    // Portal endpoints: hidden child + visible other. Should be
    // skipped wholesale because is_hidden_by_fold(child) is true.
    map.edges.push(synthetic_portal_edge("child", "other", "#00ff00"));
    let result = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0);
    assert_eq!(result.tree.root.children(&result.tree.arena).count(), 0);
    assert!(result.hit_index.is_empty());
}

#[test]
fn portal_tree_skips_line_mode_edges() {
    // A `cross_link` edge without portal display_mode must render
    // through the connection pipeline, not the portal pass. The
    // portal tree should ignore it entirely.
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
        ],
        vec![],
    );
    let mut line_edge = synthetic_portal_edge("a", "b", "#ff0000");
    line_edge.display_mode = None;
    map.edges.push(line_edge);

    let result = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0);
    assert_eq!(result.tree.root.children(&result.tree.arena).count(), 0);
    assert!(result.hit_index.is_empty());
}

#[test]
fn portal_tree_selection_overrides_color() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
        ],
        vec![],
    );
    map.edges.push(synthetic_portal_edge("a", "b", "#ff0000"));

    let selected = Some(("a", "b", "cross_link"));
    let result = build_portal_tree(&map, &HashMap::new(), selected, None, None, None, 1.0);

    // Walk pair → endpoint void → [icon, text]. Only the icon
    // GlyphArea carries a color region (the text area is empty
    // when no text is set). Assert the icon color on each endpoint
    // got the cyan override, not the red edge color.
    let pair = result.tree.root.children(&result.tree.arena).next().unwrap();
    for endpoint_void in pair.children(&result.tree.arena) {
        let icon_leaf = endpoint_void.children(&result.tree.arena).next().unwrap();
        let area = result
            .tree
            .arena
            .get(icon_leaf)
            .unwrap()
            .get()
            .glyph_area()
            .unwrap();
        let region = area.regions.all_regions()[0];
        let c = region.color.unwrap();
        // #00E5FF: r=0, g≈229/255, b≈1.0
        assert!(c[0] < 0.05);
        assert!((c[1] - 229.0 / 255.0).abs() < 0.02);
        assert!((c[2] - 1.0).abs() < 0.02);
    }
}

/// `portal_pair_data` is the single source of truth for both
/// [`build_portal_tree`] and [`build_portal_mutator_tree`]; the
/// mutator path needs the resulting `pair_channel` set to be
/// strictly ascending (Baumhard's `align_child_walks` pairs
/// mutator children against target children by ascending
/// channel and breaks alignment if the order is violated).
#[test]
fn portal_pair_channels_are_strictly_ascending() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
            synthetic_node("c", None, 400.0, 0.0),
        ],
        vec![],
    );
    map.edges.push(synthetic_portal_edge("a", "b", "#ff0000"));
    map.edges.push(synthetic_portal_edge("b", "c", "#00ff00"));

    let pairs = portal_pair_data(
        &map,
        &HashMap::new(),
        None,
        None,
        None,
        None,
        1.0,
        &map.fold_hidden_set(),
    );
    assert_eq!(pairs.len(), 2);
    let channels: Vec<usize> = pairs.iter().map(|p| p.pair_channel).collect();
    let mut prev = 0;
    for c in &channels {
        assert!(
            *c > prev,
            "pair channels must be strictly ascending: {channels:?}"
        );
        prev = *c;
    }
}

/// Round-trip: building a tree at state A and then applying the
/// mutator computed from state B to a tree built from state A must
/// produce a tree whose per-channel GlyphAreas match what
/// `build_portal_tree(B)` would produce directly. Pins the
/// canonical §B2 "mutation, not rebuild" promise — the in-place
/// path's observable output is identical to a full rebuild's.
#[test]
fn portal_mutator_round_trip_matches_full_rebuild() {
    use crate::core::primitives::Applicable;
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
        ],
        vec![],
    );
    let mut edge = synthetic_portal_edge("a", "b", "#ff0000");
    // Author a non-default zoom window on the edge so the
    // round-trip catches a regression in the portal mutator
    // delta's `GlyphAreaField::ZoomVisibility` write (§B2):
    // without the delta, `tree_a`'s icon + text areas would
    // retain the unbounded default while a fresh build picks
    // up `{0.75, 3.0}`, and the per-field assertion below
    // would trip on `zoom_visibility`.
    edge.min_zoom_to_render = Some(0.75);
    edge.max_zoom_to_render = Some(3.0);
    map.edges.push(edge);

    // State A: no offsets, no selection.
    let mut tree_a = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0).tree;

    // State B: drag offset on `b`, plus selection.
    let mut offsets = HashMap::new();
    offsets.insert("b".to_string(), (10.0, -5.0));
    let selected = Some(("a", "b", "cross_link"));

    let mutator = build_portal_mutator_tree(&map, &offsets, selected, None, None, None, 1.0);
    mutator.mutator.apply_to(&mut tree_a);

    let expected = build_portal_tree(&map, &offsets, selected, None, None, None, 1.0).tree;

    // Walk both: per pair, per slot, GlyphArea fields (text,
    // position, bounds, scale, line_height, regions, outline,
    // zoom_visibility) must match.
    // Walk three levels: pair → endpoint voids → [icon, text].
    let actual_pairs: Vec<NodeId> = tree_a.root.children(&tree_a.arena).collect();
    let expected_pairs: Vec<NodeId> = expected.root.children(&expected.arena).collect();
    assert_eq!(actual_pairs.len(), expected_pairs.len());
    for (a_pair, e_pair) in actual_pairs.iter().zip(expected_pairs.iter()) {
        let a_endpoints: Vec<NodeId> = a_pair.children(&tree_a.arena).collect();
        let e_endpoints: Vec<NodeId> = e_pair.children(&expected.arena).collect();
        assert_eq!(a_endpoints.len(), e_endpoints.len());
        for (a_ep, e_ep) in a_endpoints.iter().zip(e_endpoints.iter()) {
            let a_leaves: Vec<NodeId> = a_ep.children(&tree_a.arena).collect();
            let e_leaves: Vec<NodeId> = e_ep.children(&expected.arena).collect();
            assert_eq!(a_leaves.len(), e_leaves.len());
            for (a_leaf, e_leaf) in a_leaves.iter().zip(e_leaves.iter()) {
                let a_area = tree_a.arena.get(*a_leaf).unwrap().get().glyph_area().unwrap();
                let e_area = expected.arena.get(*e_leaf).unwrap().get().glyph_area().unwrap();
                assert_eq!(a_area.text, e_area.text);
                assert_eq!(a_area.position, e_area.position);
                assert_eq!(a_area.render_bounds, e_area.render_bounds);
                assert_eq!(a_area.scale, e_area.scale);
                assert_eq!(a_area.line_height, e_area.line_height);
                assert_eq!(a_area.regions, e_area.regions);
                assert_eq!(a_area.outline, e_area.outline);
                assert_eq!(a_area.zoom_visibility, e_area.zoom_visibility);
            }
        }
    }
}

/// Folding a node drops its outgoing portal-mode edges from the
/// pair sequence `portal_structure_signature` hashes, so the
/// dispatcher in `update_portal_tree` takes the full-rebuild path
/// instead of the in-place mutator path (the mutator assumes a
/// fixed slot count).
///
/// The visible pair keys are asserted beside the signature on
/// purpose: a `u64` that moved says nothing about *which* pair
/// left.
#[test]
fn test_portal_structure_signature_drops_folded_pairs() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
            synthetic_node("parent", None, 400.0, 0.0),
            synthetic_node("child", Some("parent"), 0.0, 100.0),
        ],
        vec![],
    );
    map.edges.push(synthetic_portal_edge("a", "b", "#ff0000"));
    map.edges.push(synthetic_portal_edge("b", "child", "#00ff00"));

    let pairs_before = portal_pair_data(
        &map,
        &HashMap::new(),
        None,
        None,
        None,
        None,
        1.0,
        &map.fold_hidden_set(),
    );
    assert_eq!(
        visible_pair_keys(&pairs_before),
        vec![
            EdgeKey::new("a", "b", "cross_link"),
            EdgeKey::new("b", "child", "cross_link"),
        ]
    );
    let before = portal_structure_signature(&pairs_before);

    map.nodes.get_mut("parent").unwrap().folded = true;
    let pairs_after = portal_pair_data(
        &map,
        &HashMap::new(),
        None,
        None,
        None,
        None,
        1.0,
        &map.fold_hidden_set(),
    );
    assert_eq!(
        visible_pair_keys(&pairs_after),
        vec![EdgeKey::new("a", "b", "cross_link")]
    );
    let after = portal_structure_signature(&pairs_after);
    assert_ne!(before, after);
}

/// The `EdgeKey`s of the visible portal pairs, in tree-insertion
/// order — what [`portal_structure_signature`] folds into its
/// hash, spelled out so a failing assertion names an edge instead
/// of a `u64`.
fn visible_pair_keys(pairs: &[PortalPairData]) -> Vec<EdgeKey> {
    pairs.iter().map(|p| p.identity.clone()).collect()
}

/// A portal glyph containing a ZWJ (zero-width joiner) sequence —
/// e.g. the family emoji "👨‍👩‍👧" which is three codepoints joined
/// into one grapheme cluster — must size its `ColorFontRegions`
/// span to the grapheme-cluster count (1), not the codepoint count
/// (5). Guards against a revert to `.chars().count()` on the
/// region-building path; `.chars().count()` would produce 5 here
/// and the region would extend past the rendered glyph, bleeding
/// the marker color into empty space.
#[test]
fn portal_marker_region_sized_by_grapheme_cluster_count_not_codepoints() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
        ],
        vec![],
    );
    let mut edge = synthetic_portal_edge("a", "b", "#ff0000");
    // Override the glyph body with a ZWJ sequence emoji.
    if let Some(cfg) = edge.glyph_connection.as_mut() {
        cfg.body = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}".into(); // 👨‍👩‍👧
    }
    map.edges.push(edge);

    let result = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0);
    let pair = result.tree.root.children(&result.tree.arena).next().unwrap();
    // Descend pair → endpoint void → icon leaf.
    let endpoint_void = pair.children(&result.tree.arena).next().unwrap();
    let icon_leaf = endpoint_void.children(&result.tree.arena).next().unwrap();
    let area = glyph_area_of(&result.tree, icon_leaf);
    let regions = area.regions.all_regions();
    assert_eq!(regions.len(), 1, "portal marker should emit one region");
    // 5 codepoints joined by ZWJ render as a single grapheme cluster.
    assert_eq!(
        regions[0].range.end - regions[0].range.start,
        1,
        "region must cover 1 grapheme cluster, not 5 codepoints"
    );
}

#[test]
fn portal_tree_text_area_carries_text_color_and_size_overrides() {
    // Integration check for the portal text-styling wiring —
    // a per-endpoint `text_color` + `text_font_size_pt` must
    // reach the emitted text `GlyphArea`, not just the
    // resolver. Guards against a regression where
    // `resolve_portal_endpoint_text_style` stays correct while
    // the tree builder accidentally reuses the icon's style.
    use crate::mindmap::model::PortalEndpointState;
    use crate::util::color::hex_to_rgba_safe;

    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 400.0, 0.0),
        ],
        vec![],
    );
    let mut edge = synthetic_portal_edge("a", "b", "#aa88cc");
    edge.portal_from = Some(PortalEndpointState {
        text: Some("hi".to_string()),
        text_color: Some("#11bb33".to_string()),
        text_font_size_pt: Some(10.0),
        text_min_font_size_pt: Some(4.0),
        text_max_font_size_pt: Some(24.0),
        ..Default::default()
    });
    map.edges.push(edge);

    let result = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0);
    let pair = result.tree.root.children(&result.tree.arena).next().unwrap();
    // Locate the endpoint void for `a` (the `from_id` side).
    // New shape: pair → endpoint void → [icon, text]; endpoint
    // channel 1 is the from-side per `portal_pair_data`.
    let endpoint_void = pair.children(&result.tree.arena).next().unwrap();
    // The text leaf is the second child (TEXT_SLOT = 2).
    let children: Vec<_> = endpoint_void.children(&result.tree.arena).collect();
    assert_eq!(children.len(), 2);
    let text_area = glyph_area_of(&result.tree, children[1]);

    // Text content should be the endpoint's `text` field.
    assert_eq!(text_area.text, "hi");
    // Color regions should carry the override (not icon color).
    let regions = text_area.regions.all_regions();
    assert_eq!(regions.len(), 1);
    let expected = hex_to_rgba_safe("#11bb33", [0.0; 4]);
    let actual = regions[0].color.expect("text region should be colored");
    for i in 0..4 {
        assert!(
            (actual[i] - expected[i]).abs() < 1.0e-4,
            "text color channel {i} mismatch: got {:?} expected {:?}",
            actual,
            expected
        );
    }
    // Font size must reflect the text override at zoom 1.0 (10 pt
    // sits inside [4, 24] → canvas size 10, not the icon's size).
    assert!(
        (text_area.scale.0 - 10.0).abs() < 1.0e-4,
        "text font size should be 10 pt, got {}",
        text_area.scale.0
    );
}

#[test]
fn test_portal_tree_resolves_icon_and_text_clicks_to_their_own_sub_parts() {
    // An endpoint with visible text exposes two separately
    // clickable leaves: a click on the icon rectangle resolves to
    // `PortalPart::Icon`, a click on the text rectangle to
    // `PortalPart::Text`, and both name the same endpoint. That
    // split is what lets the app route icon clicks to
    // `SelectionState::PortalLabel` and text clicks to
    // `SelectionState::PortalText`.
    use crate::mindmap::model::PortalEndpointState;

    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 400.0, 0.0),
        ],
        vec![],
    );
    let mut edge = synthetic_portal_edge("a", "b", "#aa88cc");
    edge.portal_from = Some(PortalEndpointState {
        text: Some("hi".to_string()),
        ..Default::default()
    });
    map.edges.push(edge);

    let mut result = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0);
    let key = EdgeKey::new("a", "b", "cross_link");
    let (icon_a, text_a) = endpoint_leaf_areas(&result.tree, 0);
    let (icon_b, text_b) = endpoint_leaf_areas(&result.tree, 1);

    // Icon and text occupy distinct rectangles.
    assert_ne!(area_rect(&icon_a), area_rect(&text_a));
    // Only the from-endpoint has text; the to-endpoint's reserved
    // text slot is zero-extent and therefore unclickable.
    assert!(area_rect(&text_a).1.x > 0.0);
    assert_eq!(area_rect(&text_b).1, glam::Vec2::ZERO);

    assert_eq!(
        portal_hit_at(&mut result, area_center(&icon_a)),
        Some(PortalHit {
            edge_key: key.clone(),
            endpoint_node_id: "a".to_string(),
            part: PortalPart::Icon,
        })
    );
    assert_eq!(
        portal_hit_at(&mut result, area_center(&text_a)),
        Some(PortalHit {
            edge_key: key.clone(),
            endpoint_node_id: "a".to_string(),
            part: PortalPart::Text,
        })
    );
    assert_eq!(
        portal_hit_at(&mut result, area_center(&icon_b)),
        Some(PortalHit {
            edge_key: key,
            endpoint_node_id: "b".to_string(),
            part: PortalPart::Icon,
        })
    );
}

#[test]
fn test_portal_tree_reserved_text_slot_is_unclickable_when_text_absent() {
    // A text-less endpoint still emits its text leaf — the
    // channel layout has to stay stable for the §B2 in-place
    // mutator path — but that leaf must not answer clicks, or a
    // phantom hot zone beside the icon would steal them from
    // whatever is underneath. Zero-extent geometry is what
    // enforces it: `Tree::descendant_at` requires strictly
    // positive bounds.
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 400.0, 0.0),
        ],
        vec![],
    );
    // Edge has NO text on either endpoint.
    map.edges.push(synthetic_portal_edge("a", "b", "#aa88cc"));

    let mut result = build_portal_tree(&map, &HashMap::new(), None, None, None, None, 1.0);
    let (icon, text) = endpoint_leaf_areas(&result.tree, 0);
    let (text_pos, text_bounds) = area_rect(&text);
    assert_eq!(
        text_bounds,
        glam::Vec2::ZERO,
        "empty text must lay out at zero extent"
    );

    // The icon itself still routes.
    assert_eq!(
        portal_hit_at(&mut result, area_center(&icon)).map(|h| h.part),
        Some(PortalPart::Icon)
    );
    // The point where the phantom text box *would* have sat —
    // one grapheme wide at the text font size, centered on the
    // degenerate rect — resolves to nothing at all.
    let phantom_probe = text_pos + glam::Vec2::new(text.scale.0 * 0.6 * 0.5, text.scale.0 * 1.3 * 0.5);
    assert_eq!(
        portal_hit_at(&mut result, phantom_probe),
        None,
        "a click beside a text-less icon must fall through, not select empty portal text"
    );
}
