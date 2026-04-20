//! Connection tree builder tests — per-edge voids, cap filters, identity-sequence drift, mutator round-trips (incl. connection labels).

use super::super::*;

#[test]
fn connection_tree_emits_one_void_per_edge_with_glyph_children() {
    use crate::mindmap::scene_builder::ConnectionElement;
    use crate::mindmap::scene_cache::EdgeKey;

    let elem = ConnectionElement {
        edge_key: EdgeKey::new("a", "b", "child"),
        glyph_positions: vec![(10.0, 0.0), (20.0, 0.0), (30.0, 0.0)],
        body_glyph: "·".into(),
        cap_start: Some(("◀".into(), (0.0, 0.0))),
        cap_end: Some(("▶".into(), (40.0, 0.0))),
        font: None,
        font_size_pt: 12.0,
        color: "#ff0000".into(),
    };
    let tree = build_connection_tree(&[elem]);
    let edge_parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
    assert_eq!(edge_parents.len(), 1);
    let glyphs: Vec<NodeId> = edge_parents[0].children(&tree.arena).collect();
    // 1 cap-start + 3 body + 1 cap-end = 5
    assert_eq!(glyphs.len(), 5);
    for id in &glyphs {
        assert!(tree.arena.get(*id).unwrap().get().glyph_area().is_some());
    }
}

#[test]
fn connection_tree_skips_caps_when_absent() {
    use crate::mindmap::scene_builder::ConnectionElement;
    use crate::mindmap::scene_cache::EdgeKey;

    let elem = ConnectionElement {
        edge_key: EdgeKey::new("a", "b", "child"),
        glyph_positions: vec![(0.0, 0.0)],
        body_glyph: "·".into(),
        cap_start: None,
        cap_end: None,
        font: None,
        font_size_pt: 12.0,
        color: "#ffffff".into(),
    };
    let tree = build_connection_tree(&[elem]);
    let edge_parent = tree.root.children(&tree.arena).next().unwrap();
    assert_eq!(edge_parent.children(&tree.arena).count(), 1);
}
#[test]
fn connection_identity_sequence_changes_with_structural_shifts() {
    use crate::mindmap::scene_builder::ConnectionElement;
    use crate::mindmap::scene_cache::EdgeKey;

    let mk = |body_count: usize,
              cap_start: Option<(String, (f32, f32))>,
              cap_end: Option<(String, (f32, f32))>,
              color: &str| ConnectionElement {
        edge_key: EdgeKey::new("a", "b", "child"),
        glyph_positions: (0..body_count).map(|i| (i as f32 * 10.0, 0.0)).collect(),
        body_glyph: "·".into(),
        cap_start,
        cap_end,
        font: None,
        font_size_pt: 12.0,
        color: color.into(),
    };

    let cap_start = Some(("◀".to_string(), (0.0, 0.0)));
    let cap_end = Some(("▶".to_string(), (30.0, 0.0)));
    let base = mk(2, cap_start.clone(), cap_end.clone(), "#ff0000");
    let id_base = connection_identity_sequence(std::slice::from_ref(&base));

    // Body count change (drag-shrinks-path): structural shift.
    let shorter = mk(1, cap_start.clone(), cap_end.clone(), "#ff0000");
    assert_ne!(
        id_base,
        connection_identity_sequence(std::slice::from_ref(&shorter))
    );

    // Cap removal: structural shift.
    let no_cap = mk(2, None, cap_end.clone(), "#ff0000");
    assert_ne!(
        id_base,
        connection_identity_sequence(std::slice::from_ref(&no_cap))
    );

    // Color change at fixed structure: identity preserved (the
    // mutator path is sound for color-only updates like
    // selection toggle and color preview).
    let recolored = mk(2, cap_start, cap_end, "#00E5FF");
    assert_eq!(
        id_base,
        connection_identity_sequence(std::slice::from_ref(&recolored))
    );
}

/// Round-trip: `build_connection_tree(A)` + the mutator from B
/// reads identical to a fresh `build_connection_tree(B)` when A
/// and B share an identity sequence (typical for selection /
/// color preview / theme switches that do not move endpoints).
#[test]
fn connection_mutator_round_trip_matches_full_rebuild() {
    use crate::core::primitives::Applicable;
    use crate::mindmap::scene_builder::ConnectionElement;
    use crate::mindmap::scene_cache::EdgeKey;

    let mk = |color: &str| ConnectionElement {
        edge_key: EdgeKey::new("a", "b", "child"),
        glyph_positions: vec![(10.0, 0.0), (20.0, 0.0)],
        body_glyph: "·".into(),
        cap_start: Some(("◀".into(), (0.0, 0.0))),
        cap_end: Some(("▶".into(), (30.0, 0.0))),
        font: None,
        font_size_pt: 12.0,
        color: color.into(),
    };
    let elem_a = mk("#ff0000");
    let elem_b = mk("#00E5FF");

    let mut tree_a = build_connection_tree(std::slice::from_ref(&elem_a));
    let mutator = build_connection_mutator_tree(std::slice::from_ref(&elem_b));
    mutator.apply_to(&mut tree_a);

    let expected = build_connection_tree(std::slice::from_ref(&elem_b));

    let actual_edges: Vec<NodeId> = tree_a.root.children(&tree_a.arena).collect();
    let expected_edges: Vec<NodeId> = expected.root.children(&expected.arena).collect();
    assert_eq!(actual_edges.len(), expected_edges.len());
    for (a_e, e_e) in actual_edges.iter().zip(expected_edges.iter()) {
        let a_glyphs: Vec<NodeId> = a_e.children(&tree_a.arena).collect();
        let e_glyphs: Vec<NodeId> = e_e.children(&expected.arena).collect();
        assert_eq!(a_glyphs.len(), e_glyphs.len());
        // Full-field parity — every mutator-written field
        // must match what a fresh build produces. Missing one
        // would let silent drift accumulate on that field
        // across mutator updates.
        for (a, e) in a_glyphs.iter().zip(e_glyphs.iter()) {
            let a_area = tree_a.arena.get(*a).unwrap().get().glyph_area().unwrap();
            let e_area = expected.arena.get(*e).unwrap().get().glyph_area().unwrap();
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

/// Connection-label round-trip with a label-text edit (the
/// hot path for inline label editing): identity is the per-edge
/// `EdgeKey` sequence, so changing the text alone keeps the
/// identity stable and the in-place mutator path picks up the new
/// glyphs without touching the arena.
#[test]
fn connection_label_mutator_round_trip_handles_text_edit() {
    use crate::core::primitives::Applicable;
    use crate::mindmap::scene_builder::ConnectionLabelElement;
    use crate::mindmap::scene_cache::EdgeKey;

    let mk = |text: &str| ConnectionLabelElement {
        edge_key: EdgeKey::new("a", "b", "child"),
        text: text.into(),
        position: (10.0, 10.0),
        bounds: (40.0, 16.0),
        color: "#ffffff".into(),
        font: None,
        font_size_pt: 12.0,
    };
    let elem_a = mk("old");
    let elem_b = mk("new label");
    assert_eq!(
        connection_label_identity_sequence(std::slice::from_ref(&elem_a)),
        connection_label_identity_sequence(std::slice::from_ref(&elem_b))
    );

    let mut tree_a = build_connection_label_tree(std::slice::from_ref(&elem_a)).tree;
    let mutator = build_connection_label_mutator_tree(std::slice::from_ref(&elem_b));
    mutator.mutator.apply_to(&mut tree_a);

    let expected = build_connection_label_tree(std::slice::from_ref(&elem_b)).tree;
    let actual_leaves: Vec<NodeId> = tree_a.root.children(&tree_a.arena).collect();
    let expected_leaves: Vec<NodeId> = expected.root.children(&expected.arena).collect();
    assert_eq!(actual_leaves.len(), expected_leaves.len());
    // Full-field parity — see `connection_mutator_round_trip...`
    // for the rationale.
    for (a, e) in actual_leaves.iter().zip(expected_leaves.iter()) {
        let a_area = tree_a.arena.get(*a).unwrap().get().glyph_area().unwrap();
        let e_area = expected.arena.get(*e).unwrap().get().glyph_area().unwrap();
        assert_eq!(a_area.text, "new label");
        assert_eq!(a_area.text, e_area.text);
        assert_eq!(a_area.position, e_area.position);
        assert_eq!(a_area.render_bounds, e_area.render_bounds);
        assert_eq!(a_area.scale, e_area.scale);
        assert_eq!(a_area.line_height, e_area.line_height);
        assert_eq!(a_area.regions, e_area.regions);
        assert_eq!(a_area.outline, e_area.outline);
    }
}

/// Connection body / cap glyphs whose text is a ZWJ sequence (a
/// single grapheme cluster spanning multiple codepoints) must size
/// the `ColorFontRegions` span to the cluster count, not the
/// codepoint count. Guards the grapheme-aware connection.rs
/// builder against a revert to `.chars().count()`.
#[test]
fn connection_region_sized_by_grapheme_cluster_count_not_codepoints() {
    use crate::mindmap::scene_builder::ConnectionElement;
    use crate::mindmap::scene_cache::EdgeKey;

    let elem = ConnectionElement {
        edge_key: EdgeKey::new("a", "b", "child"),
        glyph_positions: vec![(10.0, 0.0)],
        // 👨‍👩‍👧 — 5 codepoints, 1 grapheme cluster.
        body_glyph: "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}".into(),
        cap_start: None,
        cap_end: None,
        font: None,
        font_size_pt: 12.0,
        color: "#ffffff".into(),
    };
    let tree = build_connection_tree(&[elem]);
    let edge_parent = tree.root.children(&tree.arena).next().unwrap();
    let glyph = edge_parent.children(&tree.arena).next().unwrap();
    let area = tree.arena.get(glyph).unwrap().get().glyph_area().unwrap();
    let regions = area.regions.all_regions();
    assert_eq!(regions.len(), 1, "connection body emits one region");
    assert_eq!(
        regions[0].range.end - regions[0].range.start,
        1,
        "region must cover 1 grapheme cluster, not 5 codepoints"
    );
}

/// Connection labels are user-editable text on every edge that
/// carries a `label`. A ZWJ emoji in the label must size the
/// region to the grapheme-cluster count (1). Mirrors the body /
/// cap test above for the connection-label surface.
#[test]
fn connection_label_region_sized_by_grapheme_cluster_count_not_codepoints() {
    use crate::mindmap::scene_builder::ConnectionLabelElement;
    use crate::mindmap::scene_cache::EdgeKey;

    let elem = ConnectionLabelElement {
        edge_key: EdgeKey::new("a", "b", "cross_link"),
        text: "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}".into(), // 👨‍👩‍👧
        position: (0.0, 0.0),
        bounds: (120.0, 20.0),
        color: "#ffffff".into(),
        font: None,
        font_size_pt: 12.0,
    };
    let tree = build_connection_label_tree(&[elem]);
    let label_node = tree.tree.root.children(&tree.tree.arena).next().unwrap();
    let area = tree.tree.arena.get(label_node).unwrap().get().glyph_area().unwrap();
    let regions = area.regions.all_regions();
    assert_eq!(regions.len(), 1, "connection label emits one region");
    assert_eq!(
        regions[0].range.end - regions[0].range.start,
        1,
        "region must cover 1 grapheme cluster, not 5 codepoints"
    );
}
