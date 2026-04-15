//! Tree-builder tests — node tree, borders, portals, connections,
//! connection labels, edge handles. Shared fixtures (`test_map_path`,
//! `mk_chain_map`, `mk_star_map`) stay in this file.

use super::*;
use crate::mindmap::border::{BORDER_APPROX_CHAR_WIDTH_FRAC, BORDER_CORNER_OVERLAP_FRAC};
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
fn test_build_tree_structure() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    // Testament map has 243 nodes (none folded by default)
    assert_eq!(result.node_map.len(), 243);

    // Root of tree is Void, its children are the mindmap root nodes
    let root_children: Vec<_> = result.tree.root.children(&result.tree.arena).collect();
    let mindmap_roots = map.root_nodes();
    assert_eq!(root_children.len(), mindmap_roots.len());
}

#[test]
fn test_tree_root_nodes_match_mindmap() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    let mindmap_roots = map.root_nodes();
    let tree_root_children: Vec<NodeId> =
        result.tree.root.children(&result.tree.arena).collect();

    // Each mindmap root should be in the node_map and a child of tree root
    for root in &mindmap_roots {
        let node_id = result.node_map.get(&root.id).expect("Root not in node_map");
        assert!(
            tree_root_children.contains(node_id),
            "Root {} not a child of tree root",
            root.id
        );
    }
}

#[test]
fn test_glyph_area_properties() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    // Check "Lord God" node (id: 348068464)
    let lord_god = map.nodes.get("348068464").unwrap();
    let node_id = result.node_map.get("348068464").unwrap();
    let element = result.tree.arena.get(*node_id).unwrap().get();

    let area = element.glyph_area().expect("Expected GlyphArea");
    assert_eq!(area.text, "Lord God");
    assert_eq!(area.position.x.0, lord_god.position.x as f32);
    assert_eq!(area.position.y.0, lord_god.position.y as f32);
    assert_eq!(area.render_bounds.x.0, lord_god.size.width as f32);
    assert_eq!(area.render_bounds.y.0, lord_god.size.height as f32);
    assert_eq!(area.scale.0, lord_god.text_runs[0].size_pt as f32);
}

#[test]
fn test_color_regions_from_text_runs() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    // Lord God has 1 text run with color #ffffff
    let node_id = result.node_map.get("348068464").unwrap();
    let element = result.tree.arena.get(*node_id).unwrap().get();
    let area = element.glyph_area().unwrap();

    assert_eq!(area.regions.num_regions(), 1);
    let region = area.regions.all_regions()[0];
    assert_eq!(region.range.start, 0);
    assert_eq!(region.range.end, 8);
    // White color: [1.0, 1.0, 1.0, 1.0]
    let c = region.color.unwrap();
    assert!((c[0] - 1.0).abs() < 0.01);
    assert!((c[1] - 1.0).abs() < 0.01);
    assert!((c[2] - 1.0).abs() < 0.01);
}

#[test]
fn test_parent_child_hierarchy_preserved() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    // Lord God's children in the mindmap should be children in the tree
    let lord_god_tree_id = result.node_map.get("348068464").unwrap();
    let mindmap_children = map.children_of("348068464");

    let tree_children: Vec<NodeId> = lord_god_tree_id
        .children(&result.tree.arena)
        .collect();
    assert_eq!(tree_children.len(), mindmap_children.len());

    for child in &mindmap_children {
        let child_tree_id = result.node_map.get(&child.id).expect("Child not in node_map");
        assert!(
            tree_children.contains(child_tree_id),
            "Child {} not a tree child of Lord God",
            child.id
        );
    }
}

#[test]
fn test_unique_ids_are_unique() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    let mut seen_ids = std::collections::HashSet::new();
    for node_id in result.node_map.values() {
        let element = result.tree.arena.get(*node_id).unwrap().get();
        let uid = element.unique_id();
        assert!(seen_ids.insert(uid), "Duplicate unique_id: {}", uid);
    }
}

#[test]
fn test_all_elements_are_glyph_areas() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    for node_id in result.node_map.values() {
        let element = result.tree.arena.get(*node_id).unwrap().get();
        assert!(
            element.glyph_area().is_some(),
            "Expected GlyphArea for node"
        );
    }
}

// -----------------------------------------------------------------
// Scale / performance regression guards
//
// `build_mindmap_tree` runs on every mutation sync — any regression
// from O(N) to O(N²) here would blow the drag budget on large maps
// without being caught by the existing correctness tests (which load
// the 243-node testament fixture).
// -----------------------------------------------------------------

use crate::mindmap::model::{
    Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, PortalPair, Position, Size,
};
use std::collections::HashMap;

fn synthetic_node(id: &str, parent: Option<&str>, index: i32, x: f64, y: f64) -> MindNode {
    MindNode {
        id: id.to_string(),
        parent_id: parent.map(|s| s.to_string()),
        index,
        position: Position { x, y },
        size: Size { width: 80.0, height: 40.0 },
        text: id.to_string(),
        text_runs: vec![],
        style: NodeStyle {
            background_color: "#000".into(),
            frame_color: "#fff".into(),
            text_color: "#fff".into(),
            shape_type: 0,
            corner_radius_percent: 0.0,
            frame_thickness: 1.0,
            show_frame: true,
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

fn synthetic_map(nodes_vec: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap {
    let mut nodes = HashMap::new();
    for n in nodes_vec {
        nodes.insert(n.id.clone(), n);
    }
    MindMap {
        version: "1.0".into(),
        name: "synthetic".into(),
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

/// Builds an N-node linear spine: `n0 -> n1 -> n2 -> ... -> n{N-1}`.
/// Useful for depth-stress tests and O(N²) regression guards.
fn mk_chain_map(n: usize) -> MindMap {
    assert!(n >= 1);
    let mut nodes = Vec::with_capacity(n);
    nodes.push(synthetic_node("c0", None, 0, 0.0, 0.0));
    for i in 1..n {
        let parent = format!("c{}", i - 1);
        let id = format!("c{}", i);
        nodes.push(synthetic_node(&id, Some(&parent), 0, 0.0, i as f64 * 50.0));
    }
    synthetic_map(nodes, vec![])
}

/// Builds a star: one root and `n - 1` sibling children.
fn mk_star_map(n: usize) -> MindMap {
    assert!(n >= 1);
    let mut nodes = Vec::with_capacity(n);
    nodes.push(synthetic_node("root", None, 0, 0.0, 0.0));
    for i in 1..n {
        let id = format!("s{}", i);
        nodes.push(synthetic_node(
            &id,
            Some("root"),
            (i - 1) as i32,
            (i as f64) * 100.0,
            100.0,
        ));
    }
    synthetic_map(nodes, vec![])
}

/// Build a 1000-node chain and assert the resulting `node_map` size
/// equals the input count. If a regression made the builder O(N²) it
/// would not change this assertion — but the synthetic large-map
/// scaffold becomes the natural place to plug a wall-clock bench if
/// needed later, and this test proves the builder is linearly
/// functional at scale. Also verifies correctness at size.
#[test]
fn test_build_tree_scale_1000_node_chain() {
    let map = mk_chain_map(1000);
    let result = build_mindmap_tree(&map);
    assert_eq!(result.node_map.len(), 1000);
    // The spine root is the only root, so the tree's root has one
    // child (the Void -> first chain node).
    let roots: Vec<_> = result.tree.root.children(&result.tree.arena).collect();
    assert_eq!(roots.len(), 1);
    // Every chain node is reachable via the node_map.
    for i in 0..1000 {
        let id = format!("c{}", i);
        assert!(result.node_map.contains_key(&id),
            "missing node {}", id);
    }
}

/// A 500-child star fans out from a single root. Guards the
/// wide-breadth case — a regression that used Vec::insert(0, ...)
/// or otherwise grew quadratically in the child list would still
/// produce a correct node_map, but this test's companion 1000-node
/// chain test plus this one together cover both topology extremes.
#[test]
fn test_build_tree_wide_fan_out_500() {
    let map = mk_star_map(500);
    let result = build_mindmap_tree(&map);
    assert_eq!(result.node_map.len(), 500);
    // Root is "root", all others are direct children.
    let root_tree_id = result.node_map.get("root").unwrap();
    let children: Vec<_> = root_tree_id.children(&result.tree.arena).collect();
    assert_eq!(children.len(), 499);
}

/// A 500-node deep spine must build without a stack overflow. The
/// current `build_mindmap_tree` walks iteratively — this test
/// guards against a future refactor silently introducing recursion
/// over the hierarchy.
#[test]
fn test_build_tree_deep_chain_no_stack_overflow() {
    let map = mk_chain_map(500);
    let result = build_mindmap_tree(&map);
    assert_eq!(result.node_map.len(), 500);
    // Walk from the root down the spine and confirm depth == 500.
    let mut current = *result.node_map.get("c0").unwrap();
    let mut depth = 1;
    while let Some(child) = current.children(&result.tree.arena).next() {
        current = child;
        depth += 1;
    }
    assert_eq!(depth, 500);
}

// -----------------------------------------------------------------
// Background color → GlyphArea.background_color plumbing
//
// Session 6C follow-up: node backgrounds now live on the Baumhard
// tree (as `GlyphArea.background_color`) so they can be mutated
// through the tree walker and efficiently rendered as filled
// rectangles by the renderer. These tests lock in that
// `NodeStyle.background_color` survives the tree build intact,
// honors the theme-variable indirection, and degrades safely on
// malformed input or the explicit `transparent` sentinel.
// -----------------------------------------------------------------

fn glyph_area_of<'a>(
    tree: &'a crate::gfx_structs::tree::Tree<
        crate::gfx_structs::element::GfxElement,
        crate::gfx_structs::mutator::GfxMutator,
    >,
    node_id: indextree::NodeId,
) -> &'a crate::gfx_structs::area::GlyphArea {
    tree.arena.get(node_id).unwrap().get().glyph_area().unwrap()
}

#[test]
fn test_background_color_opaque_hex_populates_field() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0, 0.0, 0.0)],
        vec![],
    );
    map.nodes.get_mut("n").unwrap().style.background_color = "#ff8800".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert_eq!(area.background_color, Some([255, 136, 0, 255]));
}

#[test]
fn test_background_color_empty_string_becomes_none() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0, 0.0, 0.0)],
        vec![],
    );
    map.nodes.get_mut("n").unwrap().style.background_color = "".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert!(area.background_color.is_none());
}

#[test]
fn test_background_color_fully_transparent_becomes_none() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0, 0.0, 0.0)],
        vec![],
    );
    // `#00000000` is the conventional "no fill" opt-out.
    map.nodes.get_mut("n").unwrap().style.background_color = "#00000000".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert!(area.background_color.is_none());
}

#[test]
fn test_background_color_resolves_theme_variable() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0, 0.0, 0.0)],
        vec![],
    );
    map.canvas
        .theme_variables
        .insert("--panel".into(), "#112233".into());
    map.nodes.get_mut("n").unwrap().style.background_color = "var(--panel)".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert_eq!(area.background_color, Some([17, 34, 51, 255]));
}

#[test]
fn test_background_color_malformed_hex_degrades_to_none() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0, 0.0, 0.0)],
        vec![],
    );
    // `hex_to_rgba_safe` degrades unknown/bad strings to the
    // fallback we passed in — `[0,0,0,0]` for background — which
    // then trips the transparent-alpha sentinel below and becomes
    // `None`. Keeps a typo from crashing the render.
    map.nodes.get_mut("n").unwrap().style.background_color = "not-a-color".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert!(area.background_color.is_none());
}

#[test]
fn test_background_color_three_digit_hex_works() {
    let mut map = synthetic_map(
        vec![synthetic_node("n", None, 0, 0.0, 0.0)],
        vec![],
    );
    // `#000` is the default in all the synthetic nodes above, and
    // it's opaque black — verify the builder treats it as a real
    // fill (not transparent) so the renderer draws the rect. A
    // future refactor that mis-parses short hex values would
    // regress this.
    map.nodes.get_mut("n").unwrap().style.background_color = "#000".into();
    let result = build_mindmap_tree(&map);
    let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
    assert_eq!(area.background_color, Some([0, 0, 0, 255]));
}

// -----------------------------------------------------------------
// Border tree builder
// -----------------------------------------------------------------

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

fn synthetic_portal(label: &str, a: &str, b: &str, color: &str) -> PortalPair {
    PortalPair {
        endpoint_a: a.into(),
        endpoint_b: b.into(),
        label: label.into(),
        glyph: "◈".into(),
        color: color.into(),
        font_size_pt: 16.0,
        font: None,
    }
}

#[test]
fn portal_tree_emits_two_markers_per_pair() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0, 0.0, 0.0),
            synthetic_node("b", None, 1, 200.0, 0.0),
        ],
        vec![],
    );
    map.portals.push(synthetic_portal("X", "a", "b", "#ff0000"));

    let result = build_portal_tree(&map, &HashMap::new(), None, None);
    let pairs: Vec<NodeId> = result.tree.root.children(&result.tree.arena).collect();
    assert_eq!(pairs.len(), 1);

    let markers: Vec<NodeId> = pairs[0].children(&result.tree.arena).collect();
    assert_eq!(markers.len(), 2);
    // Hitboxes: one entry per (pair, endpoint).
    assert_eq!(result.hitboxes.len(), 2);
}

#[test]
fn portal_tree_skips_pair_with_folded_endpoint() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("parent", None, 0, 0.0, 0.0),
            synthetic_node("child", Some("parent"), 0, 0.0, 100.0),
            synthetic_node("other", None, 1, 200.0, 0.0),
        ],
        vec![],
    );
    map.nodes.get_mut("parent").unwrap().folded = true;
    // Pair endpoints: hidden child + visible other. Should be
    // skipped wholesale because is_hidden_by_fold(child) is true.
    map.portals
        .push(synthetic_portal("Y", "child", "other", "#00ff00"));
    let result = build_portal_tree(&map, &HashMap::new(), None, None);
    assert_eq!(result.tree.root.children(&result.tree.arena).count(), 0);
    assert!(result.hitboxes.is_empty());
}

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
fn portal_tree_selection_overrides_color() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0, 0.0, 0.0),
            synthetic_node("b", None, 1, 200.0, 0.0),
        ],
        vec![],
    );
    map.portals.push(synthetic_portal("Z", "a", "b", "#ff0000"));

    let selected = Some(("Z", "a", "b"));
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
            synthetic_node("a", None, 0, 0.0, 0.0),
            synthetic_node("b", None, 1, 200.0, 0.0),
            synthetic_node("c", None, 2, 400.0, 0.0),
        ],
        vec![],
    );
    map.portals.push(synthetic_portal("X", "a", "b", "#ff0000"));
    map.portals.push(synthetic_portal("Y", "b", "c", "#00ff00"));

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
/// mutator computed from state B must produce a tree whose
/// per-channel GlyphAreas match what `build_portal_tree(B)`
/// would produce directly. Pins the canonical §B2
/// "mutation, not rebuild" promise — the in-place path's
/// observable output is identical to a full rebuild's, modulo
/// the arena identity.
#[test]
fn portal_mutator_round_trip_matches_full_rebuild() {
    use crate::core::primitives::Applicable;
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0, 0.0, 0.0),
            synthetic_node("b", None, 1, 200.0, 0.0),
        ],
        vec![],
    );
    map.portals.push(synthetic_portal("X", "a", "b", "#ff0000"));

    // State A: no offsets, no selection.
    let mut tree_a = build_portal_tree(&map, &HashMap::new(), None, None).tree;

    // State B: drag offset on `b`, plus selection.
    let mut offsets = HashMap::new();
    offsets.insert("b".to_string(), (10.0, -5.0));
    let selected = Some(("X", "a", "b"));

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

/// Connection identity sequence captures cap presence and body
/// glyph count per edge. A change in any of those is structural
/// and must drop the equality so the dispatcher in
/// `update_connection_tree` falls back to a full rebuild.
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
/// hot path for inline label editing in Phase 2.1): identity
/// is the per-edge `EdgeKey` sequence, so changing the text
/// alone keeps the identity stable and the in-place mutator
/// path runs.
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

/// `edge_handle_channel_for` keeps the AnchorFrom < AnchorTo <
/// (Midpoint | ControlPoint) ordering that
/// `align_child_walks` relies on. Channels also need to be
/// distinct between Midpoint and any ControlPoint so a switch
/// between a straight edge and a curved one shows up as a
/// structural change in the identity sequence.
#[test]
fn edge_handle_channels_preserve_ordering_and_distinctness() {
    use crate::mindmap::scene_builder::EdgeHandleKind;
    let from = edge_handle_channel_for(EdgeHandleKind::AnchorFrom);
    let to = edge_handle_channel_for(EdgeHandleKind::AnchorTo);
    let mid = edge_handle_channel_for(EdgeHandleKind::Midpoint);
    let cp0 = edge_handle_channel_for(EdgeHandleKind::ControlPoint(0));
    let cp1 = edge_handle_channel_for(EdgeHandleKind::ControlPoint(1));
    assert!(from < to, "AnchorFrom < AnchorTo");
    assert!(to < mid, "AnchorTo < Midpoint");
    assert!(to < cp0, "AnchorTo < ControlPoint(0)");
    assert!(cp0 < cp1, "ControlPoint(0) < ControlPoint(1)");
    assert_ne!(mid, cp0, "Midpoint and ControlPoint(0) must occupy different channels");
}

/// Round-trip: a tree built from handle set A, with the mutator
/// computed from handle set B applied, reads identical to a
/// fresh `build_edge_handle_tree(B)` — provided B has the same
/// identity sequence as A (same kind ordering). Pins the §B2
/// "mutation, not rebuild" promise for the drag hot path: only
/// positions move during a handle drag, so identity stays
/// stable and the mutator path is sound.
#[test]
fn edge_handle_mutator_round_trip_matches_full_rebuild() {
    use crate::core::primitives::Applicable;
    use crate::mindmap::scene_builder::{EdgeHandleElement, EdgeHandleKind};
    use crate::mindmap::scene_cache::EdgeKey;

    let mk = |kind: EdgeHandleKind, x: f32, y: f32| EdgeHandleElement {
        edge_key: EdgeKey::new("a", "b", "child"),
        kind,
        position: (x, y),
        glyph: "◆".into(),
        color: "#00E5FF".into(),
        font_size_pt: 14.0,
    };

    let set_a = vec![
        mk(EdgeHandleKind::AnchorFrom, 0.0, 0.0),
        mk(EdgeHandleKind::AnchorTo, 100.0, 0.0),
        mk(EdgeHandleKind::Midpoint, 50.0, 0.0),
    ];
    let set_b = vec![
        mk(EdgeHandleKind::AnchorFrom, 5.0, -2.0),
        mk(EdgeHandleKind::AnchorTo, 110.0, -2.0),
        mk(EdgeHandleKind::Midpoint, 57.0, -2.0),
    ];
    assert_eq!(
        edge_handle_identity_sequence(&set_a),
        edge_handle_identity_sequence(&set_b),
        "drag preserves identity sequence; only positions move"
    );

    let mut tree_a = build_edge_handle_tree(&set_a);
    let mutator = build_edge_handle_mutator_tree(&set_b);
    mutator.apply_to(&mut tree_a);

    let expected = build_edge_handle_tree(&set_b);
    let actual_leaves: Vec<NodeId> =
        tree_a.root.children(&tree_a.arena).collect();
    let expected_leaves: Vec<NodeId> =
        expected.root.children(&expected.arena).collect();
    assert_eq!(actual_leaves.len(), expected_leaves.len());
    for (a, e) in actual_leaves.iter().zip(expected_leaves.iter()) {
        let a_area = tree_a.arena.get(*a).unwrap().get().glyph_area().unwrap();
        let e_area = expected.arena.get(*e).unwrap().get().glyph_area().unwrap();
        assert_eq!(a_area.text, e_area.text);
        assert_eq!(a_area.position, e_area.position);
        assert_eq!(a_area.render_bounds, e_area.render_bounds);
        assert_eq!(a_area.regions, e_area.regions);
    }
}

/// Adding a control point (drag-midpoint-creates-cp) or
/// switching selection from a 0-CP edge to a 1-CP edge must
/// register as a structural change in the identity sequence,
/// so the dispatcher in `update_edge_handle_tree` falls back to
/// a full rebuild rather than apply a mutator against a tree
/// whose channel set has shifted.
#[test]
fn edge_handle_identity_sequence_changes_on_midpoint_to_cp() {
    use crate::mindmap::scene_builder::{EdgeHandleElement, EdgeHandleKind};
    use crate::mindmap::scene_cache::EdgeKey;

    let mk = |kind: EdgeHandleKind| EdgeHandleElement {
        edge_key: EdgeKey::new("a", "b", "child"),
        kind,
        position: (0.0, 0.0),
        glyph: "◆".into(),
        color: "#00E5FF".into(),
        font_size_pt: 14.0,
    };
    let straight = vec![
        mk(EdgeHandleKind::AnchorFrom),
        mk(EdgeHandleKind::AnchorTo),
        mk(EdgeHandleKind::Midpoint),
    ];
    let curved = vec![
        mk(EdgeHandleKind::AnchorFrom),
        mk(EdgeHandleKind::AnchorTo),
        mk(EdgeHandleKind::ControlPoint(0)),
    ];
    assert_ne!(
        edge_handle_identity_sequence(&straight),
        edge_handle_identity_sequence(&curved)
    );
}

/// `portal_identity_sequence` reflects the visible-portal order
/// emitted by `portal_pair_data`. Folded endpoints drop their
/// pair from the sequence — the in-place mutator path uses this
/// to detect when a fold/unfold has changed the structure and
/// trigger a full rebuild instead.
#[test]
fn portal_identity_sequence_drops_folded_pairs() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0, 0.0, 0.0),
            synthetic_node("b", None, 1, 200.0, 0.0),
            synthetic_node("parent", None, 2, 400.0, 0.0),
            synthetic_node("child", Some("parent"), 0, 0.0, 100.0),
        ],
        vec![],
    );
    map.portals.push(synthetic_portal("X", "a", "b", "#ff0000"));
    map.portals
        .push(synthetic_portal("Y", "b", "child", "#00ff00"));

    let pairs_before = portal_pair_data(&map, &HashMap::new(), None, None);
    assert_eq!(
        portal_identity_sequence(&pairs_before),
        vec![
            ("X".into(), "a".into(), "b".into()),
            ("Y".into(), "b".into(), "child".into()),
        ]
    );

    map.nodes.get_mut("parent").unwrap().folded = true;
    let pairs_after = portal_pair_data(&map, &HashMap::new(), None, None);
    assert_eq!(
        portal_identity_sequence(&pairs_after),
        vec![("X".into(), "a".into(), "b".into())]
    );
}

