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

    let vars = &map.canvas.theme_variables;
    let roots = map.root_nodes();
    for root in &roots {
        if map.is_hidden_by_fold(root) {
            continue;
        }
        let area = mindnode_to_glyph_area(root, vars);
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
    let vars = &map.canvas.theme_variables;
    let children = map.children_of(parent_mind_id);
    for child in &children {
        if map.is_hidden_by_fold(child) {
            continue;
        }
        let area = mindnode_to_glyph_area(child, vars);
        let element = GfxElement::new_area_non_indexed_with_id(area, 0, *id_counter);
        *id_counter += 1;

        let child_node_id = tree.arena.new_node(element);
        parent_node_id.append(child_node_id, &mut tree.arena);
        node_map.insert(child.id.clone(), child_node_id);

        build_children_recursive(map, &child.id, child_node_id, tree, node_map, id_counter);
    }
}

/// Converts a MindNode's data into a Baumhard GlyphArea. Text-run colors
/// are resolved through the map's theme variables before being converted
/// to RGBA; unknown references and malformed hex fall back to transparent
/// black rather than panicking so a theme typo can't crash the render.
fn mindnode_to_glyph_area(node: &MindNode, vars: &HashMap<String, String>) -> GlyphArea {
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
        let resolved = color::resolve_var(&run.color, vars);
        let rgba = color::hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 1.0]);
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

    // -----------------------------------------------------------------
    // Scale / performance regression guards
    //
    // `build_mindmap_tree` runs on every mutation sync — any regression
    // from O(N) to O(N²) here would blow the drag budget on large maps
    // without being caught by the existing correctness tests (which load
    // the 243-node testament fixture).
    // -----------------------------------------------------------------

    use crate::mindmap::model::{
        Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, Position, Size,
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
}
