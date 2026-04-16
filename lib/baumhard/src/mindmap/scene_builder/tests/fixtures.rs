//! Shared fixtures for the scene_builder tests. Exposed as
//! `pub(super)` so themed sibling modules can reuse them without
//! per-file duplication.

use std::path::PathBuf;

use crate::mindmap::model::{
    Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, PortalPair, Position, Size,
};

pub(super) fn test_map_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // lib/baumhard -> lib
    path.pop(); // lib -> root
    path.push("maps/testament.mindmap.json");
    path
}

pub(super) fn synthetic_node(id: &str, x: f64, y: f64, w: f64, h: f64, show_frame: bool) -> MindNode {
    MindNode {
        id: id.to_string(),
        parent_id: None,
        position: Position { x, y },
        size: Size { width: w, height: h },
        text: id.to_string(),
        text_runs: vec![],
        style: NodeStyle {
            background_color: "#000".into(),
            frame_color: "#fff".into(),
            text_color: "#fff".into(),
            shape: "rectangle".into(),
            corner_radius_percent: 0.0,
            frame_thickness: 1.0,
            show_frame,
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

pub(super) fn synthetic_edge(from: &str, to: &str, anchor_from: &str, anchor_to: &str) -> MindEdge {
    MindEdge {
        from_id: from.to_string(),
        to_id: to.to_string(),
        edge_type: "cross_link".to_string(),
        color: "#fff".to_string(),
        width: 1,
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_position_t: None,
        anchor_from: anchor_from.to_string(),
        anchor_to: anchor_to.to_string(),
        control_points: vec![],
        glyph_connection: None,
    }
}

pub(super) fn synthetic_map(nodes_vec: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap {
    use std::collections::HashMap;
    let mut nodes = HashMap::new();
    for n in nodes_vec {
        nodes.insert(n.id.clone(), n);
    }
    MindMap {
        version: "1.0".into(),
        name: "test".into(),
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
        portals: vec![],
    }
}

pub(super) fn themed_node(id: &str, bg: &str, frame: &str, text: &str) -> MindNode {
    let mut n = synthetic_node(id, 0.0, 0.0, 40.0, 40.0, true);
    n.style.background_color = bg.to_string();
    n.style.frame_color = frame.to_string();
    n.style.text_color = text.to_string();
    n
}

pub(super) fn two_node_edge_map() -> MindMap {
    synthetic_map(
        vec![
            synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
            synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
        ],
        vec![synthetic_edge("a", "b", "right", "left")],
    )
}

pub(super) fn synthetic_portal(label: &str, a: &str, b: &str, color: &str) -> PortalPair {
    PortalPair {
        endpoint_a: a.to_string(),
        endpoint_b: b.to_string(),
        label: label.to_string(),
        glyph: "\u{25C8}".to_string(),
        color: color.to_string(),
        font_size_pt: 16.0,
        font: None,
    }
}
