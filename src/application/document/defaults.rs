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
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_position_t: None,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: Vec::new(),
        glyph_connection: None,
        display_mode: None,
        portal_from: None,
        portal_to: None,
    }
}

/// Build a fresh "orphan" MindNode with sensible defaults, positioned at
/// `position` and marked as a root (`parent_id = None`).
pub(super) fn default_orphan_node(id: &str, position: Vec2) -> MindNode {
    let text = "New node".to_string();
    let text_runs = vec![TextRun {
        start: 0,
        end: baumhard::util::grapheme_chad::count_grapheme_clusters(&text),
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
            shape: "rectangle".to_string(),
            corner_radius_percent: 10.0,
            frame_thickness: 4.0,
            show_frame: true,
            show_shadow: false,
            border: None,
        },
        layout: NodeLayout {
            layout_type: "map".to_string(),
            direction: "auto".to_string(),
            spacing: 50.0,
        },
        folded: false,
        notes: String::new(),
        color_schema: None,
        channel: 0,
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
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_position_t: None,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: Vec::new(),
        glyph_connection: None,
        display_mode: None,
        portal_from: None,
        portal_to: None,
    }
}

/// Build a default-styled portal-mode edge. Like `default_cross_link_edge`,
/// but with `display_mode = Some("portal")` and a `glyph_connection` that
/// carries the chosen marker glyph. Callers rotate `glyph_preset_index`
/// through `PORTAL_GLYPH_PRESETS.len()` to pick distinct glyphs per
/// portal without forcing the user to choose up front.
pub(super) fn default_portal_edge(
    from_id: &str,
    to_id: &str,
    glyph: &str,
) -> MindEdge {
    use baumhard::mindmap::model::{GlyphConnectionConfig, DISPLAY_MODE_PORTAL};
    MindEdge {
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        edge_type: "cross_link".to_string(),
        color: "#aa88cc".to_string(),
        width: 3,
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_position_t: None,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: Vec::new(),
        glyph_connection: Some(GlyphConnectionConfig {
            body: glyph.to_string(),
            font_size_pt: 16.0,
            ..GlyphConnectionConfig::default()
        }),
        display_mode: Some(DISPLAY_MODE_PORTAL.to_string()),
        portal_from: None,
        portal_to: None,
    }
}
