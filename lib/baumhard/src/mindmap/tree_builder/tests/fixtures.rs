//! Shared fixtures for the tree_builder tests. All helpers exposed
//! as `pub(super)` so sibling test modules can reuse them without
//! per-file duplication.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::mindmap::model::{
    Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, Position, Size,
    DISPLAY_MODE_PORTAL, GlyphConnectionConfig,
};

pub(super) fn test_map_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // lib/baumhard -> lib
    path.pop(); // lib -> root
    path.push("maps/testament.mindmap.json");
    path
}

pub(super) fn synthetic_node(id: &str, parent: Option<&str>, x: f64, y: f64) -> MindNode {
    MindNode {
        id: id.to_string(),
        parent_id: parent.map(|s| s.to_string()),
        position: Position { x, y },
        size: Size { width: 80.0, height: 40.0 },
        text: id.to_string(),
        text_runs: vec![],
        style: NodeStyle {
            background_color: "#000".into(),
            frame_color: "#fff".into(),
            text_color: "#fff".into(),
            shape: "rectangle".into(),
            corner_radius_percent: 0.0,
            frame_thickness: 1.0,
            show_frame: true,
            show_shadow: false,
            border: None,
        },
        layout: NodeLayout { layout_type: "map".into(), direction: "auto".into(), spacing: 0.0 },
        folded: false,
        notes: String::new(),
        color_schema: None,
        channel: 0,
        trigger_bindings: vec![],
        inline_mutations: vec![],
    }
}

pub(super) fn synthetic_map(nodes_vec: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap {
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
        palettes: HashMap::new(),
        nodes,
        edges,
        custom_mutations: vec![],
    }
}

/// Builds an N-node linear spine: `n0 -> n1 -> n2 -> ... -> n{N-1}`.
/// Useful for depth-stress tests and O(N²) regression guards.
pub(super) fn mk_chain_map(n: usize) -> MindMap {
    assert!(n >= 1);
    let mut nodes = Vec::with_capacity(n);
    nodes.push(synthetic_node("c0", None, 0.0, 0.0));
    for i in 1..n {
        let parent = format!("c{}", i - 1);
        let id = format!("c{}", i);
        nodes.push(synthetic_node(&id, Some(&parent), 0.0, i as f64 * 50.0));
    }
    synthetic_map(nodes, vec![])
}

/// Builds a star: one root and `n - 1` sibling children.
pub(super) fn mk_star_map(n: usize) -> MindMap {
    assert!(n >= 1);
    let mut nodes = Vec::with_capacity(n);
    nodes.push(synthetic_node("root", None, 0.0, 0.0));
    for i in 1..n {
        let id = format!("s{}", i);
        nodes.push(synthetic_node(
            &id,
            Some("root"),
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

/// Build a portal-mode edge from `a` to `b` with the given color.
/// Mirrors the `synthetic_portal` helper that seeded the pre-refactor
/// `PortalPair` fixtures — now returns a `MindEdge` with
/// `display_mode = "portal"`, `glyph_connection.body = "◈"`, and the
/// 16pt portal marker size.
pub(super) fn synthetic_portal_edge(a: &str, b: &str, color: &str) -> MindEdge {
    MindEdge {
        from_id: a.into(),
        to_id: b.into(),
        edge_type: "cross_link".into(),
        color: color.into(),
        width: 3,
        line_style: "solid".into(),
        visible: true,
        label: None,
        label_config: None,
        anchor_from: "auto".into(),
        anchor_to: "auto".into(),
        control_points: vec![],
        glyph_connection: Some(GlyphConnectionConfig {
            body: "◈".into(),
            font_size_pt: 16.0,
            ..GlyphConnectionConfig::default()
        }),
        display_mode: Some(DISPLAY_MODE_PORTAL.into()),
        portal_from: None,
        portal_to: None,
    }
}

pub(super) fn glyph_area_of<'a>(
    tree: &'a crate::gfx_structs::tree::Tree<
        crate::gfx_structs::element::GfxElement,
        crate::gfx_structs::mutator::GfxMutator,
    >,
    node_id: indextree::NodeId,
) -> &'a crate::gfx_structs::area::GlyphArea {
    tree.arena.get(node_id).unwrap().get().glyph_area().unwrap()
}
