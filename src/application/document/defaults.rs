//! Default constructors for `MindEdge` and `MindNode` values —
//! the shapes new orphan nodes, new parent→child edges, and new
//! cross-link edges inherit when the user creates them. Keeps the
//! field lists in one place so visual defaults (colour, font,
//! cap glyphs) don't drift across call sites.

use glam::Vec2;

use baumhard::mindmap::model::{MindEdge, MindNode, NodeLayout, NodeStyle, Position, Size, TextRun};

pub(super) fn default_parent_child_edge(from_id: &str, to_id: &str) -> MindEdge {
    MindEdge {
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        edge_type: "parent_child".to_string(),
        color: "#888888".to_string(),
        width: 4,
        line_style: 0,
        visible: true,
        label: None,
        label_position_t: None,
        anchor_from: 0,
        anchor_to: 0,
        control_points: Vec::new(),
        glyph_connection: None,
    }
}

/// Build a fresh "orphan" MindNode with sensible defaults, positioned at
/// `position` and marked as a root (`parent_id = None`). The node has
/// placeholder text so it's visible on the canvas until the user edits it
/// (text editing is still WIP; see roadmap M7).
pub(super) fn default_orphan_node(id: &str, position: Vec2, index: i32) -> MindNode {
    let text = "New node".to_string();
    let text_runs = vec![TextRun {
        start: 0,
        end: text.chars().count(),
        bold: false,
        italic: false,
        underline: false,
        font: "LiberationSans".to_string(),
        size_pt: 24,
        color: "#ffffff".to_string(),
        hyperlink: None,
    }];
    MindNode {
        id: id.to_string(),
        parent_id: None,
        index,
        position: Position {
            x: position.x as f64,
            y: position.y as f64,
        },
        size: Size {
            width: 240.0,
            height: 60.0,
        },
        text,
        text_runs,
        style: NodeStyle {
            background_color: "#141414".to_string(),
            frame_color: "#30b082".to_string(),
            text_color: "#ffffff".to_string(),
            shape_type: 0,
            corner_radius_percent: 10.0,
            frame_thickness: 4.0,
            show_frame: true,
            show_shadow: false,
            border: None,
        },
        layout: NodeLayout {
            layout_type: 0,
            direction: 0,
            spacing: 50.0,
        },
        folded: false,
        notes: String::new(),
        color_schema: None,
        trigger_bindings: Vec::new(),
        inline_mutations: Vec::new(),
    }
}

/// Build a default-styled cross_link edge from `from_id` to `to_id`.
/// Used by connect mode (Ctrl+D) to create non-hierarchical connections.
/// Cross-links don't affect the tree structure.
pub(super) fn default_cross_link_edge(from_id: &str, to_id: &str) -> MindEdge {
    MindEdge {
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        edge_type: "cross_link".to_string(),
        color: "#aa88cc".to_string(),
        width: 3,
        line_style: 0,
        visible: true,
        label: None,
        label_position_t: None,
        anchor_from: 0,
        anchor_to: 0,
        control_points: Vec::new(),
        glyph_connection: None,
    }
}
