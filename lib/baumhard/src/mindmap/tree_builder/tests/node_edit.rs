// SPDX-License-Identifier: MPL-2.0

//! NodeEdit-mode chrome on the border pass: when one node is the
//! active NodeEdit target, every *other* visible node's frame
//! renders at [`INACTIVE_NODE_ALPHA_MULTIPLIER`] alpha.
//!
//! The matching **text-run** dimming is a render-layer overlay
//! applied to the node tree by the application crate — the node
//! tree itself is a pure model projection (see
//! `MindMapDocument::build_tree`), so the dimming cannot live here.
//! Its tests sit next to that overlay in
//! `src/application/document/hit_test.rs`.
//!
//! Border-preview integration rides along in this file because it
//! reaches the border pass through the same
//! [`BorderChromeOverrides`] bundle.

use super::fixtures::*;
use crate::mindmap::model::MindSection;
use crate::mindmap::tree_builder::{SceneSelectionContext, INACTIVE_NODE_ALPHA_MULTIPLIER};
use std::collections::HashMap;

/// Parse an `#RRGGBB` / `#RRGGBBAA` hex into the alpha channel as a
/// float in `[0.0, 1.0]`. Lifted into a helper so the assertions
/// below stay short and read like prose.
fn alpha_of(hex: &str) -> f32 {
    let stripped = hex.trim_start_matches('#');
    if stripped.len() == 8 {
        let bytes = u8::from_str_radix(&stripped[6..8], 16).expect("hex digits");
        bytes as f32 / 255.0
    } else {
        // No alpha byte → opaque.
        1.0
    }
}

fn ctx_with_node_edit_for<'a>(active: &'a str) -> SceneSelectionContext<'a> {
    SceneSelectionContext {
        node_edit_for: Some(active),
        ..SceneSelectionContext::default()
    }
}

fn node_with_text(id: &str, x: f64, y: f64, text: &str) -> crate::mindmap::model::MindNode {
    let mut node = sized_node(id, x, y, 200.0, 200.0, true);
    node.sections = vec![MindSection::new_default(text.into(), vec![])];
    // Pin a known opaque text color so the half-alpha is unambiguous.
    let grapheme_count = text.chars().count();
    node.sections[0].text_runs = vec![crate::mindmap::model::TextRun {
        start: 0,
        end: grapheme_count,
        bold: false,
        italic: false,
        underline: false,
        font: String::new(),
        size_pt: 14,
        color: "#ffffff".into(),
        hyperlink: None,
    }];
    // Pin the frame color (the resolved border color cascades from
    // `style.frame_color` when no `border.color` override is set) so
    // the assertion has a stable starting alpha to halve.
    node.style.frame_color = "#ff8800".into();
    node
}

#[test]
fn test_node_edit_dim_off_renders_full_alpha() {
    let map = synthetic_map(
        vec![
            node_with_text("a", 0.0, 0.0, "alpha"),
            node_with_text("b", 400.0, 0.0, "beta"),
        ],
        vec![],
    );
    let scene = project_with_overrides(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        1.0,
    );
    assert_eq!(scene.border_nodes.len(), 2, "both framed nodes emit a border");
    for b in &scene.border_nodes {
        assert!(
            (alpha_of(&b.border_style.color) - 1.0).abs() < 1e-3,
            "default mode: {} border alpha = {} (expected 1.0)",
            b.node_id,
            alpha_of(&b.border_style.color),
        );
    }
}

#[test]
fn test_node_edit_active_node_keeps_full_alpha() {
    let map = synthetic_map(
        vec![
            node_with_text("a", 0.0, 0.0, "alpha"),
            node_with_text("b", 400.0, 0.0, "beta"),
        ],
        vec![],
    );
    let scene = project_with_overrides(
        &map,
        &HashMap::new(),
        ctx_with_node_edit_for("a"),
        None,
        None,
        None,
        1.0,
    );
    let active_border = scene
        .border_nodes
        .iter()
        .find(|b| b.node_id == "a")
        .expect("active node 'a' must emit a border");
    assert!(
        (alpha_of(&active_border.border_style.color) - 1.0).abs() < 1e-3,
        "active node border alpha must stay 1.0; got {}",
        alpha_of(&active_border.border_style.color),
    );
}

#[test]
fn test_node_edit_inactive_node_dims_to_half_alpha() {
    let map = synthetic_map(
        vec![
            node_with_text("a", 0.0, 0.0, "alpha"),
            node_with_text("b", 400.0, 0.0, "beta"),
        ],
        vec![],
    );
    let scene = project_with_overrides(
        &map,
        &HashMap::new(),
        ctx_with_node_edit_for("a"),
        None,
        None,
        None,
        1.0,
    );
    let inactive_border = scene
        .border_nodes
        .iter()
        .find(|b| b.node_id == "b")
        .expect("inactive node 'b' must emit a border");
    assert!(
        (alpha_of(&inactive_border.border_style.color) - INACTIVE_NODE_ALPHA_MULTIPLIER).abs() < 1e-2,
        "inactive node 'b' border alpha must be {} (got {})",
        INACTIVE_NODE_ALPHA_MULTIPLIER,
        alpha_of(&inactive_border.border_style.color),
    );
    // The dimmed color must still be a resolvable hex, not a
    // pass-through of an unparsed input — the tree builder feeds it
    // straight to `hex_to_rgba_safe` for the region colors.
    assert!(
        inactive_border.color_rgba[3] < 0.75,
        "dimmed border rgba alpha must follow the hex; got {:?}",
        inactive_border.color_rgba
    );
}

// ─── border preview integration: per-node target ────────────────

/// Per-node `Nodes(ids)` preview substitutes the previewed border
/// config for the matching node's resolved style, leaving other
/// visible nodes' borders unchanged.
#[test]
fn test_border_preview_node_target_renders_through_border_pass() {
    use crate::mindmap::tree_builder::{
        BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef, EditView,
    };
    let map = synthetic_map(
        vec![
            node_with_text("a", 0.0, 0.0, "alpha"),
            node_with_text("b", 400.0, 0.0, "beta"),
        ],
        vec![],
    );
    let target_ids = [String::from("a")];
    let edits = BorderConfigEditsView {
        preset: EditView::Set("heavy"),
        ..Default::default()
    };
    let preview = BorderPreview {
        target: BorderPreviewTargetRef::Nodes(&target_ids),
        edits,
        force_show_frame: true,
    };
    let scene = project_with_overrides(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        Some(preview),
        1.0,
    );
    let target = scene
        .border_nodes
        .iter()
        .find(|b| b.node_id == "a")
        .expect("a's border emitted (force_show_frame)");
    assert_eq!(
        target.border_style.corners.top_left, "\u{250F}",
        "preview-targeted node 'a' resolves to heavy preset"
    );
    if let Some(other) = scene.border_nodes.iter().find(|b| b.node_id == "b") {
        // 'b' isn't in the preview target — its border resolves
        // through the committed cascade (light floor / fixture's
        // configured shape).
        assert_ne!(
            other.border_style.corners.top_left, "\u{250F}",
            "non-targeted node 'b' must NOT pick up the heavy preview"
        );
    }
}

/// `force_show_frame = true` lets a preview render against a node
/// whose committed `style.show_frame == false`. Without it, the
/// emitter skips the border entirely and `border preview
/// preset=heavy` on a frameless node would render nothing — the
/// user would think the verb was broken.
#[test]
fn test_border_preview_force_show_frame_on_hidden_node() {
    use crate::mindmap::tree_builder::{
        BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef, EditView,
    };
    // Build a node with `show_frame = false`.
    let mut hidden_node = sized_node("hidden", 0.0, 0.0, 200.0, 100.0, false);
    hidden_node.sections = vec![MindSection::new_default("hidden text".into(), vec![])];
    let map = synthetic_map(vec![hidden_node], vec![]);

    // Sanity: with no preview, the node emits no border data.
    let baseline = project_with_overrides(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        1.0,
    );
    assert!(
        baseline.border_nodes.iter().all(|b| b.node_id != "hidden"),
        "no preview + show_frame=false → no border emitted"
    );

    // With a preview that has `force_show_frame = true`, the node
    // emits a previewed border even though the committed
    // `show_frame` is still `false`.
    let target_ids = [String::from("hidden")];
    let edits = BorderConfigEditsView {
        preset: EditView::Set("heavy"),
        ..Default::default()
    };
    let preview = BorderPreview {
        target: BorderPreviewTargetRef::Nodes(&target_ids),
        edits,
        force_show_frame: true,
    };
    let scene = project_with_overrides(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        Some(preview),
        1.0,
    );
    let target = scene
        .border_nodes
        .iter()
        .find(|b| b.node_id == "hidden")
        .expect("force_show_frame must materialize the border");
    assert_eq!(
        target.border_style.corners.top_left, "\u{250F}",
        "preview's heavy preset rendered through"
    );
}
