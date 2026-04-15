//! Border tree builder tests — void-per-framed, frame filters, drag offset, theme resolution, stable channels, mutator round-trip, identity sequence.

use super::fixtures::*;
use super::super::*;
use crate::mindmap::border::{BORDER_APPROX_CHAR_WIDTH_FRAC, BORDER_CORNER_OVERLAP_FRAC};

#[test]
fn border_tree_has_one_void_parent_per_framed_node() {
    let map = synthetic_map(
        vec![
            synthetic_node("a", None, 0, 0.0, 0.0),
            synthetic_node("b", None, 1, 200.0, 0.0),
        ],
        vec![],
    );
    let tree = build_border_tree(&map, &HashMap::new());
    // Two framed nodes → two per-node Void parents under root.
    let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
    assert_eq!(parents.len(), 2);
    for parent in parents {
        let element = tree.arena.get(parent).unwrap().get();
        assert!(element.glyph_area().is_none(), "per-node parent is Void");
        // Every parent has exactly 4 GlyphArea run children.
        let runs: Vec<NodeId> = parent.children(&tree.arena).collect();
        assert_eq!(runs.len(), 4);
        for run_id in runs {
            let run = tree.arena.get(run_id).unwrap().get();
            assert!(run.glyph_area().is_some(), "run is a GlyphArea");
        }
    }
}

#[test]
fn border_tree_skips_nodes_with_show_frame_false() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0, 0.0, 0.0),
            synthetic_node("b", None, 1, 200.0, 0.0),
        ],
        vec![],
    );
    map.nodes.get_mut("a").unwrap().style.show_frame = false;
    let tree = build_border_tree(&map, &HashMap::new());
    // Only `b` is framed → one per-node parent.
    let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
    assert_eq!(parents.len(), 1);
}

#[test]
fn border_tree_skips_folded_nodes() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("parent", None, 0, 0.0, 0.0),
            synthetic_node("child", Some("parent"), 0, 0.0, 100.0),
        ],
        vec![],
    );
    map.nodes.get_mut("parent").unwrap().folded = true;
    let tree = build_border_tree(&map, &HashMap::new());
    // Parent itself still frames; child is hidden.
    let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
    assert_eq!(parents.len(), 1);
}

#[test]
fn border_tree_applies_drag_offset() {
    let map = synthetic_map(vec![synthetic_node("a", None, 0, 0.0, 0.0)], vec![]);
    let mut offsets: HashMap<String, (f32, f32)> = HashMap::new();
    offsets.insert("a".into(), (50.0, 25.0));
    let tree = build_border_tree(&map, &offsets);
    // Drag offset must show up on the *top* run's position.x
    // (which is `pos_x - approx_char_width`).
    let parent = tree.root.children(&tree.arena).next().unwrap();
    let top_run = parent.children(&tree.arena).next().unwrap();
    let area = tree
        .arena
        .get(top_run)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    // pos_x + offset = 0 + 50 = 50, then shifted by
    // -approx_char_width (0.6 * font_size).
    let font_size = 14.0_f32;
    let approx_char_width = font_size * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let expected_x = 50.0 - approx_char_width;
    assert!(
        (area.position.x.0 - expected_x).abs() < 0.001,
        "top-run x ({}) should match drag-applied layout ({})",
        area.position.x.0,
        expected_x
    );
    // y follows pos_y + offset - font_size + corner_overlap.
    let corner_overlap = font_size * BORDER_CORNER_OVERLAP_FRAC;
    let expected_y = 25.0 - font_size + corner_overlap;
    assert!((area.position.y.0 - expected_y).abs() < 0.001);
}

#[test]
fn border_tree_resolves_frame_color_through_theme_vars() {
    let mut map = synthetic_map(vec![synthetic_node("a", None, 0, 0.0, 0.0)], vec![]);
    // Theme variable keys include the leading `--`, matching
    // the CSS-ish `var(--name)` syntax used in mindmap JSON.
    map.canvas
        .theme_variables
        .insert("--my-frame".into(), "#ff0000".into());
    map.nodes.get_mut("a").unwrap().style.frame_color = "var(--my-frame)".into();
    let tree = build_border_tree(&map, &HashMap::new());
    let parent = tree.root.children(&tree.arena).next().unwrap();
    let top_run = parent.children(&tree.arena).next().unwrap();
    let area = tree
        .arena
        .get(top_run)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    let region = area.regions.all_regions()[0];
    let c = region.color.unwrap();
    // #ff0000 → red channel 1.0, green/blue 0.0.
    assert!((c[0] - 1.0).abs() < 0.01);
    assert!(c[1] < 0.01);
    assert!(c[2] < 0.01);
}

#[test]
fn border_tree_run_channels_are_stable_1_to_4() {
    // Top=1, Bottom=2, Left=3, Right=4. Stability matters
    // because mutator trees target runs by channel.
    use crate::gfx_structs::tree::BranchChannel;
    let map = synthetic_map(vec![synthetic_node("a", None, 0, 0.0, 0.0)], vec![]);
    let tree = build_border_tree(&map, &HashMap::new());
    let parent = tree.root.children(&tree.arena).next().unwrap();
    let runs: Vec<_> = parent.children(&tree.arena).collect();
    let channels: Vec<usize> = runs
        .iter()
        .map(|id| tree.arena.get(*id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![1, 2, 3, 4]);
}

/// Per-node Void parents use the 1-based sorted index as
/// their channel, not a monotonic counter. Stability across
/// rebuilds is the prerequisite for the in-place mutator
/// path: `align_child_walks` matches mutator children to
/// target children by ascending channel, so two consecutive
/// `border_node_data` calls with the same identity must emit
/// the same channel set.
#[test]
fn border_parent_channels_are_sorted_index_based() {
    use crate::gfx_structs::tree::BranchChannel;
    // Three framed nodes; lexicographic order is a, b, c.
    let map = synthetic_map(
        vec![
            synthetic_node("c", None, 0, 0.0, 0.0),
            synthetic_node("a", None, 1, 100.0, 0.0),
            synthetic_node("b", None, 2, 200.0, 0.0),
        ],
        vec![],
    );
    let tree = build_border_tree(&map, &HashMap::new());
    let parents: Vec<_> = tree.root.children(&tree.arena).collect();
    let channels: Vec<usize> = parents
        .iter()
        .map(|id| tree.arena.get(*id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![1, 2, 3]);
}

/// Round-trip: build a border tree at state A, apply the
/// mutator computed from state B, and the resulting tree's
/// per-channel GlyphAreas must match what
/// `build_border_tree(B)` produced directly. Picks the
/// picker-hover hot path as the canonical case: same nodes,
/// same frame flag, but a drag offset and a color change.
#[test]
fn border_mutator_round_trip_matches_full_rebuild() {
    use crate::core::primitives::Applicable;

    let map = synthetic_map(
        vec![
            synthetic_node("a", None, 0, 0.0, 0.0),
            synthetic_node("b", None, 1, 200.0, 0.0),
        ],
        vec![],
    );

    // State A: no offsets.
    let mut tree_a = build_border_tree(&map, &HashMap::new());

    // State B: same identity, offset applied to node "a".
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (12.5, -6.0));

    let nodes_b = border_node_data(&map, &offsets);
    let mutator = build_border_mutator_tree_from_nodes(&nodes_b);
    mutator.apply_to(&mut tree_a);

    let expected = build_border_tree(&map, &offsets);

    let actual_parents: Vec<NodeId> =
        tree_a.root.children(&tree_a.arena).collect();
    let expected_parents: Vec<NodeId> =
        expected.root.children(&expected.arena).collect();
    assert_eq!(actual_parents.len(), expected_parents.len());
    // Full-field parity — text / position / bounds / scale /
    // line_height / regions / outline — so any silent drift
    // on a mutator-written field surfaces here.
    for (a_p, e_p) in actual_parents.iter().zip(expected_parents.iter()) {
        let a_runs: Vec<NodeId> = a_p.children(&tree_a.arena).collect();
        let e_runs: Vec<NodeId> = e_p.children(&expected.arena).collect();
        assert_eq!(a_runs.len(), e_runs.len());
        for (a, e) in a_runs.iter().zip(e_runs.iter()) {
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

/// Toggling `show_frame = false` on a node shifts the
/// identity sequence so the dispatcher in
/// `update_border_tree_with_offsets` falls back to a full
/// rebuild. Without this, applying a mutator against a tree
/// whose parent set has changed would silently misalign.
#[test]
fn border_identity_sequence_changes_on_show_frame_toggle() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0, 0.0, 0.0),
            synthetic_node("b", None, 1, 200.0, 0.0),
        ],
        vec![],
    );
    let before =
        border_identity_sequence(&border_node_data(&map, &HashMap::new()));
    assert_eq!(before, vec!["a".to_string(), "b".to_string()]);

    map.nodes.get_mut("b").unwrap().style.show_frame = false;
    let after =
        border_identity_sequence(&border_node_data(&map, &HashMap::new()));
    assert_eq!(after, vec!["a".to_string()]);
    assert_ne!(before, after);
}

// -----------------------------------------------------------------
// Portal tree builder
// -----------------------------------------------------------------
