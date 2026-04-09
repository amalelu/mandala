use std::collections::HashMap;

use glam::Vec2;
use indextree::NodeId;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::model::{MindMap, MindNode};
use crate::util::color;

/// Result of building a Baumhard tree from a MindMap.
/// The tree mirrors the MindMap's parent-child hierarchy,
/// with each MindNode represented as a GlyphArea element.
pub struct MindMapTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// Maps MindNode ID → indextree NodeId for later lookup.
    pub node_map: HashMap<String, NodeId>,
}

/// Builds a `Tree<GfxElement, GfxMutator>` from a MindMap's hierarchy.
///
/// The tree structure mirrors the MindMap's parent-child relationships:
/// - A Void root node at the top
/// - Each root MindNode (parent_id is None) as a child of the Void root
/// - Children nested recursively following parent_id
/// - Nodes hidden by fold state are excluded
///
/// Each MindNode becomes a GlyphArea element with its text, position,
/// size, and color regions.
pub fn build_mindmap_tree(map: &MindMap) -> MindMapTree {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut node_map: HashMap<String, NodeId> = HashMap::new();
    let mut id_counter: usize = 1; // 0 is reserved for the Void root

    let roots = map.root_nodes();
    for root in &roots {
        if map.is_hidden_by_fold(root) {
            continue;
        }
        let area = mindnode_to_glyph_area(root);
        let element = GfxElement::new_area_non_indexed_with_id(area, 0, id_counter);
        id_counter += 1;

        let node_id = tree.arena.new_node(element);
        tree.root.append(node_id, &mut tree.arena);
        node_map.insert(root.id.clone(), node_id);

        build_children_recursive(map, &root.id, node_id, &mut tree, &mut node_map, &mut id_counter);
    }

    MindMapTree { tree, node_map }
}

fn build_children_recursive(
    map: &MindMap,
    parent_mind_id: &str,
    parent_node_id: NodeId,
    tree: &mut Tree<GfxElement, GfxMutator>,
    node_map: &mut HashMap<String, NodeId>,
    id_counter: &mut usize,
) {
    let children = map.children_of(parent_mind_id);
    for child in &children {
        if map.is_hidden_by_fold(child) {
            continue;
        }
        let area = mindnode_to_glyph_area(child);
        let element = GfxElement::new_area_non_indexed_with_id(area, 0, *id_counter);
        *id_counter += 1;

        let child_node_id = tree.arena.new_node(element);
        parent_node_id.append(child_node_id, &mut tree.arena);
        node_map.insert(child.id.clone(), child_node_id);

        build_children_recursive(map, &child.id, child_node_id, tree, node_map, id_counter);
    }
}

/// Converts a MindNode's data into a Baumhard GlyphArea.
fn mindnode_to_glyph_area(node: &MindNode) -> GlyphArea {
    let scale = node
        .text_runs
        .first()
        .map(|r| r.size_pt as f32)
        .unwrap_or(14.0);
    let line_height = scale * 1.2;
    let position = Vec2::new(node.position.x as f32, node.position.y as f32);
    let bounds = Vec2::new(node.size.width as f32, node.size.height as f32);

    let mut area = GlyphArea::new_with_str(&node.text, scale, line_height, position, bounds);

    // Convert text runs to ColorFontRegions
    let mut regions = ColorFontRegions::new_empty();
    for run in &node.text_runs {
        let rgba = color::hex_to_rgba(&run.color);
        regions.submit_region(ColorFontRegion::new(
            Range::new(run.start, run.end),
            None, // Font: use default (cosmic-text resolves family names at render time)
            Some(rgba),
        ));
    }
    area.regions = regions;

    area
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
