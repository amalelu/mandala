//! Portal tree builder tests — marker emission, fold filtering, selection highlight, ascending channels, mutator round-trip, identity sequence. Edges with `display_mode = "portal"` drive the portal pass.

use super::fixtures::*;
use super::super::*;
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

    let result = build_portal_tree(&map, &HashMap::new(), None, None);
    let pairs: Vec<NodeId> = result.tree.root.children(&result.tree.arena).collect();
    assert_eq!(pairs.len(), 1);

    let markers: Vec<NodeId> = pairs[0].children(&result.tree.arena).collect();
    assert_eq!(markers.len(), 2);
    // Hitboxes: one entry per (edge, endpoint).
    assert_eq!(result.hitboxes.len(), 2);
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
    map.edges
        .push(synthetic_portal_edge("child", "other", "#00ff00"));
    let result = build_portal_tree(&map, &HashMap::new(), None, None);
    assert_eq!(result.tree.root.children(&result.tree.arena).count(), 0);
    assert!(result.hitboxes.is_empty());
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

    let result = build_portal_tree(&map, &HashMap::new(), None, None);
    assert_eq!(result.tree.root.children(&result.tree.arena).count(), 0);
    assert!(result.hitboxes.is_empty());
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
    let result = build_portal_tree(&map, &HashMap::new(), selected, None);

    // Each marker's GlyphArea should carry the cyan color, not red.
    let pair = result.tree.root.children(&result.tree.arena).next().unwrap();
    for marker in pair.children(&result.tree.arena) {
        let area = result
            .tree
            .arena
            .get(marker)
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

    let pairs = portal_pair_data(&map, &HashMap::new(), None, None);
    assert_eq!(pairs.len(), 2);
    let channels: Vec<usize> = pairs.iter().map(|p| p.pair_channel).collect();
    let mut prev = 0;
    for c in &channels {
        assert!(*c > prev, "pair channels must be strictly ascending: {channels:?}");
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
    map.edges.push(synthetic_portal_edge("a", "b", "#ff0000"));

    // State A: no offsets, no selection.
    let mut tree_a = build_portal_tree(&map, &HashMap::new(), None, None).tree;

    // State B: drag offset on `b`, plus selection.
    let mut offsets = HashMap::new();
    offsets.insert("b".to_string(), (10.0, -5.0));
    let selected = Some(("a", "b", "cross_link"));

    let mutator = build_portal_mutator_tree(&map, &offsets, selected, None);
    mutator.mutator.apply_to(&mut tree_a);

    let expected = build_portal_tree(&map, &offsets, selected, None).tree;

    // Walk both: per pair, per slot, GlyphArea fields (text,
    // position, bounds, scale, line_height, regions, outline)
    // must match.
    let actual_pairs: Vec<NodeId> = tree_a.root.children(&tree_a.arena).collect();
    let expected_pairs: Vec<NodeId> = expected.root.children(&expected.arena).collect();
    assert_eq!(actual_pairs.len(), expected_pairs.len());
    for (a_pair, e_pair) in actual_pairs.iter().zip(expected_pairs.iter()) {
        let a_markers: Vec<NodeId> = a_pair.children(&tree_a.arena).collect();
        let e_markers: Vec<NodeId> = e_pair.children(&expected.arena).collect();
        assert_eq!(a_markers.len(), e_markers.len());
        for (a_m, e_m) in a_markers.iter().zip(e_markers.iter()) {
            let a_area = tree_a.arena.get(*a_m).unwrap().get().glyph_area().unwrap();
            let e_area = expected.arena.get(*e_m).unwrap().get().glyph_area().unwrap();
            assert_eq!(a_area.text, e_area.text);
            assert_eq!(a_area.position, e_area.position);
            assert_eq!(a_area.render_bounds, e_area.render_bounds);
            assert_eq!(a_area.scale, e_area.scale);
            assert_eq!(a_area.line_height, e_area.line_height);
            assert_eq!(a_area.regions, e_area.regions);
            assert_eq!(a_area.outline, e_area.outline);
        }
    }
}

/// Folding a node drops its outgoing portal-mode edges from
/// `portal_identity_sequence` so the dispatcher in
/// `update_portal_tree` takes the full-rebuild path instead of the
/// in-place mutator path (the mutator assumes a fixed slot count).
#[test]
fn portal_identity_sequence_drops_folded_pairs() {
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
    map.edges
        .push(synthetic_portal_edge("b", "child", "#00ff00"));

    let pairs_before = portal_pair_data(&map, &HashMap::new(), None, None);
    assert_eq!(
        portal_identity_sequence(&pairs_before),
        vec![
            EdgeKey::new("a", "b", "cross_link"),
            EdgeKey::new("b", "child", "cross_link"),
        ]
    );

    map.nodes.get_mut("parent").unwrap().folded = true;
    let pairs_after = portal_pair_data(&map, &HashMap::new(), None, None);
    assert_eq!(
        portal_identity_sequence(&pairs_after),
        vec![EdgeKey::new("a", "b", "cross_link")]
    );
}


/// A portal glyph containing a ZWJ (zero-width joiner) sequence —
/// e.g. the family emoji "👨‍👩‍👧" which is three codepoints joined
/// into one grapheme cluster — must size its `ColorFontRegions`
/// span to the grapheme-cluster count (1), not the codepoint count
/// (5). Guards against a revert to `.chars().count()` on the
/// region-building path; `.chars().count()` would produce 5 here
/// and the region would extend past the rendered glyph, bleeding
/// the marker colour into empty space.
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

    let result = build_portal_tree(&map, &HashMap::new(), None, None);
    let pair = result.tree.root.children(&result.tree.arena).next().unwrap();
    let marker = pair.children(&result.tree.arena).next().unwrap();
    let area = glyph_area_of(&result.tree, marker);
    let regions = area.regions.all_regions();
    assert_eq!(regions.len(), 1, "portal marker should emit one region");
    // 5 codepoints joined by ZWJ render as a single grapheme cluster.
    assert_eq!(
        regions[0].range.end - regions[0].range.start,
        1,
        "region must cover 1 grapheme cluster, not 5 codepoints"
    );
}
