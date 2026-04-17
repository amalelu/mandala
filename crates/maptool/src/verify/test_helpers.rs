//! Small helpers for constructing `MindNode` and `MindEdge` values in
//! verify unit tests. Kept out of the public surface — only the test
//! modules in this crate use them.

#![cfg(test)]

use baumhard::mindmap::model::{
    MindEdge, MindNode, NodeLayout, NodeStyle, Position, Size,
};

pub fn node(id: &str, parent_id: Option<&str>) -> MindNode {
    MindNode {
        id: id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        position: Position { x: 0.0, y: 0.0 },
        size: Size { width: 100.0, height: 40.0 },
        text: String::new(),
        text_runs: vec![],
        style: NodeStyle {
            background_color: "#141414".into(),
            frame_color: "#30b082".into(),
            text_color: "#ffffff".into(),
            shape: "rectangle".into(),
            corner_radius_percent: 0.0,
            frame_thickness: 0.0,
            show_frame: false,
            show_shadow: false,
            border: None,
        },
        layout: NodeLayout {
            layout_type: "map".into(),
            direction: "auto".into(),
            spacing: 0.0,
        },
        folded: false,
        notes: String::new(),
        color_schema: None,
        channel: 0,
        trigger_bindings: vec![],
        inline_mutations: vec![],
    }
}

pub fn edge(from: &str, to: &str) -> MindEdge {
    MindEdge {
        from_id: from.to_string(),
        to_id: to.to_string(),
        edge_type: "cross_link".to_string(),
        color: "#ffffff".into(),
        width: 2,
        line_style: "solid".into(),
        visible: true,
        label: None,
        label_position_t: None,
        anchor_from: "auto".into(),
        anchor_to: "auto".into(),
        control_points: vec![],
        glyph_connection: None,
        display_mode: None,
    }
}
