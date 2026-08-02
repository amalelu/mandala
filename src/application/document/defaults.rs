// SPDX-License-Identifier: MPL-2.0

//! Default constructors for `MindEdge` and `MindNode` values —
//! the shapes new orphan nodes, new parent→child edges, and new
//! cross-link edges inherit when the user creates them. Keeps the
//! field lists in one place so visual defaults (colour, font,
//! cap glyphs) don't drift across call sites.

use glam::Vec2;

use baumhard::mindmap::model::{
    MindEdge, MindNode, MindSection, NodeLayout, NodeStyle, Position, Size, TextRun,
};

/// Font family a freshly-authored [`TextRun`] carries when
/// nothing else in the cascade supplies one.
pub(in crate::application) const DEFAULT_RUN_FONT_FAMILY: &str = "LiberationSans";

/// Point size a freshly-authored [`TextRun`] carries.
///
/// **Not the same number as the renderer's no-runs fallback.**
/// [`baumhard::mindmap::tree_builder::DEFAULT_SECTION_FONT_SCALE`]
/// (14) is what the tree builder measures a section with when it
/// has *no* runs at all; this 24 is what the authoring layer
/// writes when it *creates* a run. The two answer different
/// questions and are deliberately different values: a new node's
/// text reads at 24pt, while a run-less legacy section keeps
/// rendering at the historical 14pt until something authors a
/// run onto it.
pub(in crate::application) const DEFAULT_RUN_SIZE_PT: u32 = 24;

/// Color a freshly-authored [`TextRun`] carries — the same
/// fall-through-to-white floor the renderer applies to a node
/// with no explicit `style.text_color`.
pub(in crate::application) const DEFAULT_RUN_COLOR: &str = "#ffffff";

/// One unstyled [`TextRun`] covering `[0, end)` graphemes, in the
/// authoring defaults above.
///
/// The single template every "this section has no run to inherit
/// from" path reaches for — node/section text setters, the
/// range-setter gap filler, the clipboard cut/paste splice
/// templates. Callers that need one field different use struct
/// update syntax, e.g.
/// `TextRun { color: node.style.text_color.clone(), ..default_text_run(0) }`.
///
/// `end == 0` is legal here because most callers use the result
/// purely as a *template* (they overwrite `start` / `end` from
/// the text they are about to write). A caller installing the
/// run directly must pass a non-zero `end`: `text_run_ops`
/// requires `start < end` and panics in debug builds on a
/// degenerate run.
pub(in crate::application) fn default_text_run(end: usize) -> TextRun {
    TextRun {
        start: 0,
        end,
        bold: false,
        italic: false,
        underline: false,
        font: DEFAULT_RUN_FONT_FAMILY.to_string(),
        size_pt: DEFAULT_RUN_SIZE_PT,
        color: DEFAULT_RUN_COLOR.to_string(),
        hyperlink: None,
    }
}

pub(in crate::application) fn default_parent_child_edge(from_id: &str, to_id: &str) -> MindEdge {
    MindEdge {
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        edge_type: "parent_child".to_string(),
        color: "#888888".to_string(),
        width: 4,
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_config: None,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: Vec::new(),
        glyph_connection: None,
        display_mode: None,
        portal_from: None,
        portal_to: None,
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

/// Build a fresh "orphan" MindNode with sensible defaults, positioned at
/// `position` and marked as a root (`parent_id = None`).
pub(in crate::application) fn default_orphan_node(id: &str, position: Vec2) -> MindNode {
    let text = "New node".to_string();
    let text_runs = vec![default_text_run(
        baumhard::util::grapheme_chad::count_grapheme_clusters(&text),
    )];
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
        // Fresh orphan: one default section so the user can edit
        // immediately. `MindSection::new_default` covers the
        // (offset 0, fill the node, channel 0) shape.
        sections: vec![MindSection::new_default(text, text_runs)],
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
        inline_macros: Vec::new(),
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

/// Build a default-styled cross_link edge from `from_id` to `to_id`.
/// Used by connect mode (Ctrl+D) to create non-hierarchical connections.
/// Cross-links don't affect the tree structure.
pub(in crate::application) fn default_cross_link_edge(from_id: &str, to_id: &str) -> MindEdge {
    MindEdge {
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        edge_type: "cross_link".to_string(),
        color: "#aa88cc".to_string(),
        width: 3,
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_config: None,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: Vec::new(),
        glyph_connection: None,
        display_mode: None,
        portal_from: None,
        portal_to: None,
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

/// Build a default-styled portal-mode edge. Like `default_cross_link_edge`,
/// but with `display_mode = Some("portal")` and a `glyph_connection` that
/// carries the chosen marker glyph. Callers rotate `glyph_preset_index`
/// through `PORTAL_GLYPH_PRESETS.len()` to pick distinct glyphs per
/// portal without forcing the user to choose up front.
pub(super) fn default_portal_edge(from_id: &str, to_id: &str, glyph: &str) -> MindEdge {
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
        label_config: None,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: Vec::new(),
        glyph_connection: Some(GlyphConnectionConfig {
            body: glyph.to_string(),
            // Portal markers are labels, not body glyphs — 50pt
            // reads as a clearly-legible badge next to the node,
            // in line with the (bumped) `DEFAULT_PORTAL_MARKER_FONT_SIZE_PT`
            // fallback the scene builder uses for edges flipped
            // into portal mode without an explicit connection
            // override.
            font_size_pt: 50.0,
            ..GlyphConnectionConfig::default()
        }),
        display_mode: Some(DISPLAY_MODE_PORTAL.to_string()),
        portal_from: None,
        portal_to: None,
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}
