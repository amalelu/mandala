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

    // Resolve the node's background color through theme variables and
    // pack it as u8 RGBA onto the tree element. The renderer's rect
    // pipeline reads it back out during `rebuild_buffers_from_tree`
    // and emits a solid quad behind the text glyphs.
    //
    // `None` means "no fill" — the canvas background shows through.
    // Both an empty string and a fully-transparent alpha ("#00000000"
    // / "#0000") map to `None`. Bad hex degrades to `None` as well,
    // so a theme typo leaves the node transparent rather than
    // painting it opaque black.
    area.background_color = {
        let raw = &node.style.background_color;
        if raw.is_empty() {
            None
        } else {
            let resolved = color::resolve_var(raw, vars);
            // Sentinel alpha = 0 means "parse failed" here because
            // the fallback is fully transparent. Authors can also
            // opt out with an explicit `#00000000` / `#0000`, which
            // lands on the same sentinel for free.
            let rgba = color::hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 0.0]);
            if rgba[3] <= 0.0 {
                None
            } else {
                Some(color::convert_f32_to_u8(&rgba))
            }
        }
    };

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

// =====================================================================
// Border tree builder
//
// Emits one baumhard `Tree<GfxElement, GfxMutator>` that, when walked
// into cosmic-text buffers, reproduces the same four box-drawing runs
// per framed node that the legacy `scene_builder::BorderElement` +
// `renderer::rebuild_border_buffers_keyed` pair produces.
//
// Layout constants (`BORDER_CORNER_OVERLAP_FRAC`,
// `BORDER_APPROX_CHAR_WIDTH_FRAC`) live on `crate::mindmap::border`
// so the renderer's keyed-buffer rebuild and this builder share one
// source of truth.
// =====================================================================

use crate::mindmap::border::{BORDER_APPROX_CHAR_WIDTH_FRAC, BORDER_CORNER_OVERLAP_FRAC};

/// Build a baumhard tree representing every framed node's border
/// glyphs. The tree's shape is:
///
/// ```text
/// Void (root)
/// ├── Void (per node — channel = id_counter)
/// │   ├── GlyphArea (top run, channel = 1)
/// │   ├── GlyphArea (bottom run, channel = 2)
/// │   ├── GlyphArea (left column, channel = 3)
/// │   └── GlyphArea (right column, channel = 4)
/// ├── Void (next node)
/// │   └── ...
/// ```
///
/// The per-node Void parent is not strictly necessary for rendering
/// but it gives mutator trees a natural target for whole-node
/// border changes (e.g. color change across all four runs).
///
/// Iteration order is the lexicographic order of `MindNode.id` —
/// stable across runs so per-node Void parents always land in the
/// same arena slot. Without this, `MindMap.nodes` (a `HashMap`)
/// would yield nondeterministic order, making mutator-tree
/// authoring against "the third framed node" unreliable.
///
/// # Costs
///
/// O(N log N) where N is the visible framed-node count (the sort
/// dominates for large maps). Allocates one tree arena, one
/// `Vec<&str>` for the sort, and one `String` per run. Uses the
/// same `BorderStyle` defaults as `scene_builder::build_scene` so
/// the two paths can't drift on style choices.
pub fn build_border_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
) -> Tree<GfxElement, GfxMutator> {
    use crate::mindmap::border::BorderStyle;

    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let vars = &map.canvas.theme_variables;
    let mut id_counter: usize = 1;

    let mut sorted_ids: Vec<&String> = map.nodes.keys().collect();
    sorted_ids.sort();

    for node_id in sorted_ids {
        let Some(node) = map.nodes.get(node_id) else {
            continue;
        };
        if map.is_hidden_by_fold(node) {
            continue;
        }
        if !node.style.show_frame {
            continue;
        }

        let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        let pos_x = node.position.x as f32 + ox;
        let pos_y = node.position.y as f32 + oy;
        let size_x = node.size.width as f32;
        let size_y = node.size.height as f32;

        let frame_color_hex = color::resolve_var(&node.style.frame_color, vars);
        let border_style = BorderStyle::default_with_color(frame_color_hex);
        let color_rgba = color::hex_to_rgba_safe(&border_style.color, [1.0, 1.0, 1.0, 1.0]);

        append_border_sub_tree(
            &mut tree,
            &border_style,
            color_rgba,
            pos_x,
            pos_y,
            size_x,
            size_y,
            &mut id_counter,
        );
    }

    tree
}

/// Build one per-node sub-tree (Void parent + 4 GlyphArea runs) and
/// append it under `tree.root`. Kept as a private helper so
/// `build_border_tree` stays readable.
fn append_border_sub_tree(
    tree: &mut Tree<GfxElement, GfxMutator>,
    border_style: &crate::mindmap::border::BorderStyle,
    color_rgba: [f32; 4],
    pos_x: f32,
    pos_y: f32,
    size_x: f32,
    size_y: f32,
    id_counter: &mut usize,
) {
    let font_size = border_style.font_size_pt;
    let approx_char_width = font_size * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let char_count = ((size_x / approx_char_width) + 2.0).ceil().max(3.0) as usize;
    let right_corner_x =
        pos_x - approx_char_width + (char_count - 1) as f32 * approx_char_width;
    let corner_overlap = font_size * BORDER_CORNER_OVERLAP_FRAC;
    let top_y = pos_y - font_size + corner_overlap;
    let bottom_y = pos_y + size_y - corner_overlap;
    let h_width = (char_count as f32 + 1.0) * approx_char_width;
    let v_width = approx_char_width * 2.0;
    let row_count = (size_y / font_size).round().max(1.0) as usize;

    let glyph_set = &border_style.glyph_set;
    let top_text = glyph_set.top_border(char_count);
    let bottom_text = glyph_set.bottom_border(char_count);
    let left_text: String =
        std::iter::repeat_n(format!("{}\n", glyph_set.left_char()), row_count).collect();
    let right_text: String =
        std::iter::repeat_n(format!("{}\n", glyph_set.right_char()), row_count).collect();

    // Per-node Void parent — groups the four runs for targeted
    // mutation. The parent's channel is the counter so distinct
    // nodes never collide.
    let parent_channel = *id_counter;
    let parent_id = tree
        .arena
        .new_node(GfxElement::new_void_with_id(parent_channel, parent_channel));
    tree.root.append(parent_id, &mut tree.arena);
    *id_counter += 1;

    // Stable channels 1..=4 inside each border sub-tree. The
    // per-node Void parent already disambiguates across nodes.
    append_border_run(
        tree,
        parent_id,
        1,
        *id_counter,
        &top_text,
        font_size,
        (pos_x - approx_char_width, top_y),
        (h_width, font_size * 1.5),
        color_rgba,
    );
    *id_counter += 1;
    append_border_run(
        tree,
        parent_id,
        2,
        *id_counter,
        &bottom_text,
        font_size,
        (pos_x - approx_char_width, bottom_y),
        (h_width, font_size * 1.5),
        color_rgba,
    );
    *id_counter += 1;
    append_border_run(
        tree,
        parent_id,
        3,
        *id_counter,
        &left_text,
        font_size,
        (pos_x - approx_char_width, pos_y),
        (v_width, size_y),
        color_rgba,
    );
    *id_counter += 1;
    append_border_run(
        tree,
        parent_id,
        4,
        *id_counter,
        &right_text,
        font_size,
        (right_corner_x, pos_y),
        (v_width, size_y),
        color_rgba,
    );
    *id_counter += 1;
}

fn append_border_run(
    tree: &mut Tree<GfxElement, GfxMutator>,
    parent_id: NodeId,
    channel: usize,
    unique_id: usize,
    text: &str,
    font_size: f32,
    position: (f32, f32),
    bounds: (f32, f32),
    color_rgba: [f32; 4],
) {
    let mut area = GlyphArea::new_with_str(
        text,
        font_size,
        font_size,
        Vec2::new(position.0, position.1),
        Vec2::new(bounds.0, bounds.1),
    );

    // Single ColorFontRegion covering the whole run — the renderer
    // walker translates this into a cosmic-text `Attrs::color`
    // span. Grapheme cluster count matches `chars().count()` here
    // because box-drawing glyphs are all single-scalar ASCII-range
    // codepoints, but using the grapheme counter is cheap and
    // future-proof.
    let cluster_count = text.chars().count();
    if cluster_count > 0 {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, cluster_count),
            None,
            Some(color_rgba),
        ));
        area.regions = regions;
    }

    let element = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
    let node = tree.arena.new_node(element);
    parent_id.append(node, &mut tree.arena);
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
}
