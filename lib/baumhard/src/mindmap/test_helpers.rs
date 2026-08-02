// SPDX-License-Identifier: MPL-2.0

//! Shared synthetic-map fixtures for the mindmap-layer tests.
//!
//! Co-located with `mindmap/` (same directory as `model/`,
//! `tree_builder/`, `loader.rs`) so every
//! test inside the mindmap layer reaches the same primitives
//! through one canonical path. Pre-consolidation each
//! per-module `tests/fixtures.rs` carried its own
//! near-identical `synthetic_node` / `synthetic_map` /
//! `synthetic_portal_edge` / `test_map_path`, with the
//! only differences being the parameters callers passed in.
//! The bodies live here once; the per-module wrappers thin
//! down to delegations that preserve their existing
//! call-site signatures.
//!
//! Visibility: `pub(crate)` and `#[cfg(test)]`-gated at the
//! module declaration in `mindmap/mod.rs`. The benchmark-reuse
//! path (`pub mod tests;` per `TEST_CONVENTIONS.md §T2.2`)
//! does *not* apply here because none of these helpers are
//! `do_*` benchmark bodies — they are `synthetic_*`
//! constructors used purely from `#[test]` functions.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::mindmap::model::{
    Canvas, GlyphConnectionConfig, MindEdge, MindMap, MindNode, MindSection, NodeLayout, NodeStyle, Position,
    Size, DISPLAY_MODE_PORTAL,
};

/// Path to the canonical `maps/testament.mindmap.json` fixture
/// from the workspace root. Climbs from `lib/baumhard/` up two
/// levels (`lib/baumhard -> lib -> root`) and joins the relative
/// fixture path so tests work regardless of working directory.
///
/// Cost: one `PathBuf` heap allocation; trivial.
pub(crate) fn testament_map_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // lib/baumhard -> lib
    path.pop(); // lib -> root
    path.push("maps/testament.mindmap.json");
    path
}

/// Build a synthetic `MindNode` with explicit position, size,
/// optional parent, and frame visibility. The remaining
/// `NodeStyle` / `NodeLayout` fields are pinned to the test-
/// canonical defaults that every module's local fixture used
/// (`#000` background, `#fff` frame and text, rectangle shape,
/// `1.0` frame thickness, `map`/`auto`/`0.0` layout, no border).
///
/// Per-module wrappers in `tree_builder/tests/fixtures.rs` and
/// `tree_builder/tests/fixtures.rs` thins over this with its
/// preferred argument shape — neither call-site list rewrites
/// 161+ lines as a result.
///
/// Cost: ~12 `String` heap allocations (id, parent_id, defaulted
/// style strings) plus an empty `Vec` for `text_runs`. No font-
/// system lock; no measurement passes. Trivial.
pub(crate) fn synthetic_node_full(
    id: &str,
    parent: Option<&str>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    show_frame: bool,
) -> MindNode {
    MindNode {
        id: id.to_string(),
        parent_id: parent.map(|s| s.to_string()),
        position: Position { x, y },
        size: Size { width: w, height: h },
        sections: vec![MindSection::new_default(id.to_string(), vec![])],
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
        inline_macros: Vec::new(),
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

/// Build a synthetic `MindMap` from a list of nodes and edges.
/// Canvas is the test-canonical [`blank_canvas`]; palettes and
/// custom_mutations are empty. `name` is `"synthetic"`.
///
/// Cost: O(N) inserts into a fresh `HashMap<String, MindNode>`,
/// each clone-copying the node. Two `String` allocs for the
/// version + name fields, plus a `blank_canvas()` call.
pub(crate) fn synthetic_map(nodes_vec: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap {
    let mut nodes = HashMap::new();
    for n in nodes_vec {
        nodes.insert(n.id.clone(), n);
    }
    MindMap {
        version: "1.0".into(),
        name: "synthetic".into(),
        canvas: blank_canvas(),
        palettes: HashMap::new(),
        nodes,
        edges,
        custom_mutations: vec![],
        macros: vec![],
    }
}

/// Build a minimal cross-link `MindEdge` with explicit endpoints
/// and anchors. Color `#fff`, width 1, no label, no glyph
/// connection. The shape every connection-pass edge fixture wants.
///
/// Cost: ~7 `String` heap allocations (endpoints, anchors, plus
/// the static-defaulted edge_type / color / line_style). Empty
/// `Vec` for `control_points`. Trivial.
pub(crate) fn synthetic_edge(from: &str, to: &str, anchor_from: &str, anchor_to: &str) -> MindEdge {
    MindEdge {
        from_id: from.to_string(),
        to_id: to.to_string(),
        edge_type: "cross_link".to_string(),
        color: "#fff".to_string(),
        width: 1,
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_config: None,
        anchor_from: anchor_from.to_string(),
        anchor_to: anchor_to.to_string(),
        control_points: vec![],
        glyph_connection: None,
        display_mode: None,
        portal_from: None,
        portal_to: None,
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

/// Build a portal-mode `MindEdge` between `a` and `b` with the
/// given color. `display_mode = "portal"`, `glyph_connection.body
/// = "◈"`, 16pt portal-marker font size — the shape every
/// post-refactor portal fixture wanted.
///
/// Cost: ~10 `String` allocations across the edge's fields and the
/// nested `GlyphConnectionConfig`. Empty `Vec` for control points.
/// Trivial.
pub(crate) fn synthetic_portal_edge(a: &str, b: &str, color: &str) -> MindEdge {
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
            body: "\u{25C8}".into(),
            font_size_pt: 16.0,
            ..GlyphConnectionConfig::default()
        }),
        display_mode: Some(DISPLAY_MODE_PORTAL.into()),
        portal_from: None,
        portal_to: None,
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

/// Trivial `Canvas` with `#000` background, no defaults, empty
/// theme tables. Used by tests that need a Canvas placeholder
/// but don't exercise canvas behavior.
///
/// Cost: one `String` allocation for the background hex, two
/// empty `HashMap` allocations (theme variables + variants).
/// Trivial.
pub(crate) fn blank_canvas() -> Canvas {
    Canvas {
        background_color: "#000".into(),
        default_border: None,
        default_connection: None,
        default_section_frame_border: None,
        default_focused_section_frame_border: None,
        theme_variables: HashMap::new(),
        theme_variants: HashMap::new(),
    }
}
