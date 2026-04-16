//! Tree-builder node tests — structure, root nodes, glyph_area properties, color regions, parent/child hierarchy, unique IDs.

use super::fixtures::*;
use super::super::*;
use crate::mindmap::loader;

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
    let lord_god = map.nodes.get("0").unwrap();
    let node_id = result.node_map.get("0").unwrap();
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
    let node_id = result.node_map.get("0").unwrap();
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
    let lord_god_tree_id = result.node_map.get("0").unwrap();
    let mindmap_children = map.children_of("0");

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
