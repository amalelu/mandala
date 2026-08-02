// SPDX-License-Identifier: MPL-2.0

//! Section-frame emission rules. Pin the gating contract that
//! Plan §3.5 / §4.3 promises: frames appear only in NodeEdit on
//! a multi-section node, only on the active node, and one frame
//! tracks each section's effective AABB.

use std::collections::HashMap;

use super::fixtures::*;
use crate::mindmap::model::{MindSection, Position, Size};
use crate::mindmap::tree_builder::{build_section_frames, SectionFrameElement};

fn three_section_node() -> crate::mindmap::model::MindNode {
    let mut node = sized_node("active", 100.0, 200.0, 300.0, 90.0, true);
    // Three stacked sections — each 30 px tall, offset top → bottom.
    node.sections = vec![
        section("alpha", 0.0, 0.0, 300.0, 30.0),
        section("beta", 0.0, 30.0, 300.0, 30.0),
        section("gamma", 0.0, 60.0, 300.0, 30.0),
    ];
    node
}

fn section(text: &str, off_x: f64, off_y: f64, w: f64, h: f64) -> MindSection {
    let mut s = MindSection::new_default(text.into(), vec![]);
    s.offset = Position { x: off_x, y: off_y };
    s.size = Some(Size { width: w, height: h });
    s
}

fn other_node() -> crate::mindmap::model::MindNode {
    sized_node("other", 600.0, 200.0, 200.0, 90.0, true)
}

#[test]
fn test_section_frames_default_mode_emits_none() {
    let map = synthetic_map(vec![three_section_node(), other_node()], vec![]);
    let frames = build_section_frames(&map, &HashMap::new(), None, None, None, &map.fold_hidden_set());
    assert!(frames.is_empty(), "no NodeEdit target → no frames");
}

#[test]
fn test_section_frames_node_edit_on_multi_section_emits_per_section() {
    let map = synthetic_map(vec![three_section_node(), other_node()], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        None,
        None,
        &map.fold_hidden_set(),
    );
    assert_eq!(frames.len(), 3, "one frame per section");
    // Frames are emitted in section order.
    assert_eq!(frames[0].section_idx, 0);
    assert_eq!(frames[1].section_idx, 1);
    assert_eq!(frames[2].section_idx, 2);
    // All carry the active node id.
    for f in &frames {
        assert_eq!(f.node_id, "active");
    }
}

#[test]
fn test_section_frames_inactive_node_emits_no_frames() {
    let map = synthetic_map(vec![three_section_node(), other_node()], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        None,
        None,
        &map.fold_hidden_set(),
    );
    // Only sections of "active" appear; "other" never gets frames.
    assert!(frames.iter().all(|f| f.node_id == "active"));
}

#[test]
fn test_section_frames_single_section_node_skips_frames() {
    let mut node = sized_node("solo", 0.0, 0.0, 200.0, 50.0, true);
    node.sections = vec![section("only", 0.0, 0.0, 200.0, 50.0)];
    let map = synthetic_map(vec![node], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("solo"),
        None,
        None,
        &map.fold_hidden_set(),
    );
    assert!(
        frames.is_empty(),
        "single-section nodes skip frames (would duplicate the border)"
    );
}

#[test]
fn test_section_frames_missing_active_node_emits_no_frames() {
    let map = synthetic_map(vec![three_section_node()], vec![]);
    // Stale NodeEdit target after a custom mutation deletion.
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("nonexistent"),
        None,
        None,
        &map.fold_hidden_set(),
    );
    assert!(frames.is_empty(), "missing active node → no frames");
}

#[test]
fn test_section_frames_track_section_aabb() {
    let map = synthetic_map(vec![three_section_node(), other_node()], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        None,
        None,
        &map.fold_hidden_set(),
    );

    // Section 0 lives at node.position + section.offset = (100, 200).
    let f0 = &frames[0];
    assert!((f0.position.0 - 100.0).abs() < 1e-3, "x = {}", f0.position.0);
    assert!((f0.position.1 - 200.0).abs() < 1e-3, "y = {}", f0.position.1);
    assert!((f0.size.0 - 300.0).abs() < 1e-3, "w = {}", f0.size.0);
    assert!((f0.size.1 - 30.0).abs() < 1e-3, "h = {}", f0.size.1);

    // Section 1 sits below section 0 (offset.y = 30 → y = 230).
    let f1 = &frames[1];
    assert!((f1.position.1 - 230.0).abs() < 1e-3, "y = {}", f1.position.1);
}

#[test]
fn test_section_frames_focused_section_marks_only_matching_idx() {
    let map = synthetic_map(vec![three_section_node()], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("active", 1)),
        None,
        &map.fold_hidden_set(),
    );
    assert_eq!(frames.len(), 3);
    assert!(!frames[0].focused);
    assert!(frames[1].focused, "section 1 must be marked focused");
    assert!(!frames[2].focused);
}

/// Focused section pointing at a different node than the active
/// one (selection drift between editor open and rebuild) is
/// silently ignored — every frame stays unfocused.
#[test]
fn test_section_frames_focused_section_owner_mismatch_marks_none() {
    let map = synthetic_map(vec![three_section_node()], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("other", 0)),
        None,
        &map.fold_hidden_set(),
    );
    assert!(frames.iter().all(|f: &SectionFrameElement| !f.focused));
}

#[test]
fn test_section_frames_skip_zero_size_section() {
    let mut node = sized_node("active", 0.0, 0.0, 200.0, 200.0, true);
    node.sections = vec![
        section("ok", 0.0, 0.0, 200.0, 100.0),
        // Degenerate zero-height — skipped from frame emission to
        // mirror the section-area skip rule.
        {
            let mut s = MindSection::new_default("bad".into(), vec![]);
            s.offset = Position { x: 0.0, y: 100.0 };
            s.size = Some(Size {
                width: 200.0,
                height: 0.0,
            });
            s
        },
    ];
    let map = synthetic_map(vec![node], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        None,
        None,
        &map.fold_hidden_set(),
    );
    assert_eq!(frames.len(), 1, "degenerate section is skipped");
    assert_eq!(frames[0].section_idx, 0);
}

#[test]
fn test_section_frames_uses_selected_edge_color_when_no_override() {
    use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
    let map = synthetic_map(vec![three_section_node()], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        None,
        None,
        &map.fold_hidden_set(),
    );
    // With no per-section or canvas override, the resolver falls
    // through to the hardcoded floor (no `color` set) → resolved
    // BorderStyle.color is the SELECTION_HIGHLIGHT_HEX cyan the
    // caller passes in as `frame_color_resolved`.
    for f in &frames {
        assert_eq!(f.border_style.color, SELECTION_HIGHLIGHT_HEX);
    }
}

#[test]
fn test_section_frames_per_section_override_wins_over_canvas_default() {
    use crate::mindmap::model::GlyphBorderConfig;
    let mut node = three_section_node();
    // Section 1 carries a per-section override with a custom color.
    node.sections[1].frame_border = Some(GlyphBorderConfig {
        preset: "heavy".to_string(),
        font: None,
        font_size_pt: 12.0,
        color: Some("#ff8800".to_string()),
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });
    let mut map = synthetic_map(vec![node], vec![]);
    // Canvas-level default supplies a different color — the
    // per-section override should beat it.
    map.canvas.default_section_frame_border = Some(GlyphBorderConfig {
        preset: "double".to_string(),
        font: None,
        font_size_pt: 14.0,
        color: Some("#00ff00".to_string()),
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        None,
        None,
        &map.fold_hidden_set(),
    );
    assert_eq!(
        frames[0].border_style.color, "#00ff00",
        "section 0 uses canvas default"
    );
    assert_eq!(
        frames[1].border_style.color, "#ff8800",
        "section 1 uses per-section override"
    );
    assert_eq!(
        frames[2].border_style.color, "#00ff00",
        "section 2 uses canvas default"
    );
}

#[test]
fn test_section_frames_canvas_default_drives_unset_sections() {
    use crate::mindmap::model::GlyphBorderConfig;
    let node = three_section_node();
    let mut map = synthetic_map(vec![node], vec![]);
    map.canvas.default_section_frame_border = Some(GlyphBorderConfig {
        preset: "double".to_string(),
        font: None,
        font_size_pt: 14.0,
        color: Some("#abcdef".to_string()),
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        None,
        None,
        &map.fold_hidden_set(),
    );
    for f in &frames {
        assert_eq!(f.border_style.color, "#abcdef");
    }
}

#[test]
fn test_section_frames_focused_uses_focused_canvas_default() {
    use crate::mindmap::model::GlyphBorderConfig;
    let node = three_section_node();
    let mut map = synthetic_map(vec![node], vec![]);
    map.canvas.default_section_frame_border = Some(GlyphBorderConfig {
        preset: "light".to_string(),
        font: None,
        font_size_pt: 10.0,
        color: Some("#aaaaaa".to_string()),
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });
    map.canvas.default_focused_section_frame_border = Some(GlyphBorderConfig {
        preset: "heavy".to_string(),
        font: None,
        font_size_pt: 12.0,
        color: Some("#ffffff".to_string()),
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("active", 1)),
        None,
        &map.fold_hidden_set(),
    );
    assert_eq!(
        frames[0].border_style.color, "#aaaaaa",
        "section 0 unfocused → unfocused default"
    );
    assert_eq!(
        frames[1].border_style.color, "#ffffff",
        "section 1 focused → focused default"
    );
    assert_eq!(
        frames[2].border_style.color, "#aaaaaa",
        "section 2 unfocused → unfocused default"
    );
}

/// `focused = true` with only the unfocused canvas default set
/// falls through to the unfocused config (Plan §4.4 cascade
/// fallback). Pins the `default_focused.or(default_unfocused)`
/// branch in `resolve_section_frame_border`.
#[test]
fn test_section_frames_focused_falls_back_to_unfocused_canvas_default() {
    use crate::mindmap::model::GlyphBorderConfig;
    let node = three_section_node();
    let mut map = synthetic_map(vec![node], vec![]);
    map.canvas.default_section_frame_border = Some(GlyphBorderConfig {
        preset: "double".into(),
        font: None,
        font_size_pt: 14.0,
        color: Some("#abcdef".to_string()),
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });
    // No focused canvas default — focused should fall through.
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("active", 1)),
        None,
        &map.fold_hidden_set(),
    );
    assert_eq!(
        frames[1].border_style.color, "#abcdef",
        "focused section with no focused-canvas-default uses the unfocused default"
    );
}

/// `focused = false` with only the *focused* canvas default set
/// must NOT bleed into unfocused frames — those fall through to
/// the floor preset.
#[test]
fn test_section_frames_unfocused_does_not_bleed_focused_canvas_default() {
    use crate::mindmap::model::GlyphBorderConfig;
    use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
    let node = three_section_node();
    let mut map = synthetic_map(vec![node], vec![]);
    map.canvas.default_focused_section_frame_border = Some(GlyphBorderConfig {
        preset: "heavy".into(),
        font: None,
        font_size_pt: 12.0,
        color: Some("#ff00ff".to_string()),
        glyphs: None,
        padding: 0.0,
        color_palette: None,
        color_palette_field: None,
    });
    // No unfocused canvas default. Section 1 is focused → focused
    // canvas default; sections 0/2 unfocused → floor (cyan).
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("active", 1)),
        None,
        &map.fold_hidden_set(),
    );
    assert_eq!(frames[0].border_style.color, SELECTION_HIGHLIGHT_HEX);
    assert_eq!(frames[1].border_style.color, "#ff00ff");
    assert_eq!(frames[2].border_style.color, SELECTION_HIGHLIGHT_HEX);
}

/// Palette-cycling on a per-section `frame_border` resolves through
/// `resolve_palette_cycle` and lands on the emitted element's
/// `palette_cycle` vec. This is the headline creative-toolkit
/// capability — pins that authors can color-cycle a section frame
/// using the same palette infra node borders use.
#[test]
fn test_section_frames_palette_cycle_resolves_for_named_palette() {
    use crate::mindmap::model::{ColorGroup, GlyphBorderConfig, Palette};
    let mut node = three_section_node();
    node.sections[0].frame_border = Some(GlyphBorderConfig {
        preset: "light".into(),
        font: None,
        font_size_pt: 10.0,
        color: None,
        glyphs: None,
        padding: 0.0,
        color_palette: Some("rainbow".into()),
        color_palette_field: Some("frame".into()),
    });
    let mut map = synthetic_map(vec![node], vec![]);
    map.palettes.insert(
        "rainbow".into(),
        Palette {
            groups: vec![
                ColorGroup {
                    background: "#000000".into(),
                    frame: "#ff0000".into(),
                    text: "#ffffff".into(),
                    title: "#ffffff".into(),
                },
                ColorGroup {
                    background: "#000000".into(),
                    frame: "#00ff00".into(),
                    text: "#ffffff".into(),
                    title: "#ffffff".into(),
                },
                ColorGroup {
                    background: "#000000".into(),
                    frame: "#0000ff".into(),
                    text: "#ffffff".into(),
                    title: "#ffffff".into(),
                },
            ],
        },
    );
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        None,
        None,
        &map.fold_hidden_set(),
    );
    let palette = &frames[0].palette_cycle;
    assert_eq!(palette.len(), 3, "palette cycle has one entry per ColorGroup");
    // Spot-check the first entry is RGBA red.
    assert!((palette[0][0] - 1.0).abs() < 1e-3, "red channel = 1.0");
    assert!(palette[0][1] < 1e-3, "green channel = 0.0");
    assert!(palette[0][2] < 1e-3, "blue channel = 0.0");
}

/// Sections without `color_palette` set produce an empty
/// `palette_cycle` — single-color borders should not waste an
/// allocation on an N-element vec they won't read.
#[test]
fn test_section_frames_no_palette_yields_empty_cycle() {
    let map = synthetic_map(vec![three_section_node()], vec![]);
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        None,
        None,
        &map.fold_hidden_set(),
    );
    for f in &frames {
        assert!(
            f.palette_cycle.is_empty(),
            "single-color frame has no palette cycle"
        );
    }
}

// ─── border preview integration ────────────────────────────────
//
// The scene builder folds the staged preview edits into a clone
// of the affected slot before resolution. The committed model is
// untouched. These tests pin the integration end-to-end at the
// scene-builder layer; the document-side discipline (no undo, no
// dirty) is pinned in `tests_nodes.rs`.

/// Section-targeted preview reflects in the resolved
/// `SectionFrameElement.border_style` for the matching section
/// only — sibling sections fall back to their committed slot.
#[test]
fn test_border_preview_section_target_renders_through_frame_pass() {
    use crate::mindmap::tree_builder::{
        BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef, EditView,
    };
    let map = synthetic_map(vec![three_section_node()], vec![]);
    let target_pairs = [(String::from("active"), 1usize)];
    let edits = BorderConfigEditsView {
        preset: EditView::Set("heavy"),
        ..Default::default()
    };
    let preview = BorderPreview {
        target: BorderPreviewTargetRef::Sections(&target_pairs),
        edits,
        force_show_frame: true,
    };
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        None,
        Some(preview),
        &map.fold_hidden_set(),
    );
    // Section 1 should have the heavy preset's top corner glyph
    // (`┏` U+250F); sections 0 and 2 should retain the floor.
    assert_eq!(
        frames[1].border_style.corners.top_left, "\u{250F}",
        "preview-targeted section[1] resolves to heavy preset"
    );
    assert_eq!(
        frames[0].border_style.corners.top_left, "\u{250C}",
        "non-targeted section[0] keeps the light floor"
    );
    assert_eq!(
        frames[2].border_style.corners.top_left, "\u{250C}",
        "non-targeted section[2] keeps the light floor"
    );
}

/// Canvas-section-frame (unfocused) preview applies to unfocused
/// sections that have no per-section override. The focused
/// section falls through to the unfocused canvas slot per the
/// existing cascade (when no focused canvas default is set), so
/// pin a focused canvas default explicitly to isolate the
/// preview's reach to unfocused sections only.
#[test]
fn test_border_preview_canvas_section_frame_unfocused_branch() {
    use crate::mindmap::model::GlyphBorderConfig;
    use crate::mindmap::tree_builder::{
        BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef, EditView,
    };
    let mut map = synthetic_map(vec![three_section_node()], vec![]);
    // Pin the focused canvas default so focused sections don't
    // fall through to the unfocused slot — that way the preview's
    // effect on unfocused is observable in isolation.
    map.canvas.default_focused_section_frame_border = Some(GlyphBorderConfig {
        preset: "rounded".into(),
        font: None,
        font_size_pt: 14.0,
        color: None,
        glyphs: None,
        padding: 4.0,
        color_palette: None,
        color_palette_field: None,
    });
    let edits = BorderConfigEditsView {
        preset: EditView::Set("double"),
        ..Default::default()
    };
    let preview = BorderPreview {
        target: BorderPreviewTargetRef::CanvasSectionFrame,
        edits,
        force_show_frame: true,
    };
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("active", 1)),
        Some(preview),
        &map.fold_hidden_set(),
    );
    assert_eq!(
        frames[0].border_style.corners.top_left, "\u{2554}",
        "unfocused section[0] picks up the double-preset preview"
    );
    assert_eq!(
        frames[2].border_style.corners.top_left, "\u{2554}",
        "unfocused section[2] picks up the double-preset preview"
    );
    assert_eq!(
        frames[1].border_style.corners.top_left, "\u{256D}",
        "focused section[1] keeps the pinned focused canvas default (rounded)"
    );
}

/// Canvas-section-frame focused preview applies only to the
/// focused section; unfocused sections fall through to their own
/// floor (`light` preset).
#[test]
fn test_border_preview_canvas_section_frame_focused_branch() {
    use crate::mindmap::tree_builder::{
        BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef, EditView,
    };
    let map = synthetic_map(vec![three_section_node()], vec![]);
    let edits = BorderConfigEditsView {
        preset: EditView::Set("double"),
        ..Default::default()
    };
    let preview = BorderPreview {
        target: BorderPreviewTargetRef::CanvasSectionFrameFocused,
        edits,
        force_show_frame: true,
    };
    let frames = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("active", 1)),
        Some(preview),
        &map.fold_hidden_set(),
    );
    assert_eq!(
        frames[1].border_style.corners.top_left, "\u{2554}",
        "focused section[1] picks up the double-preset preview"
    );
    assert_eq!(
        frames[0].border_style.corners.top_left, "\u{250C}",
        "unfocused section[0] keeps the light floor"
    );
    assert_eq!(
        frames[2].border_style.corners.top_left, "\u{250C}",
        "unfocused section[2] keeps the light floor"
    );
}

/// Active preview that doesn't target this node-id / section
/// produces output byte-identical to `border_preview = None`.
/// Parity is the minimum bar — without it, the preview branch
/// would leak state into untargeted sections. The previous
/// version of this test compared the same `None` input twice
/// (`with_none` and `baseline` were identical inputs), proving
/// nothing; this one compares an active preview that misses
/// every target section against a `None` baseline.
#[test]
fn test_border_preview_inactive_target_matches_baseline() {
    use crate::mindmap::tree_builder::{
        BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef, EditView,
    };
    let map = synthetic_map(vec![three_section_node()], vec![]);
    // Preview targets a section nobody is rendering: the
    // node-id matches but the section index is out of range
    // for a three-section node, so the per-section "is this
    // target?" check fires false on every iteration.
    let target_pairs = [(String::from("active"), 999usize)];
    let edits = BorderConfigEditsView {
        preset: EditView::Set("heavy"),
        ..Default::default()
    };
    let preview = BorderPreview {
        target: BorderPreviewTargetRef::Sections(&target_pairs),
        edits,
        force_show_frame: false,
    };
    let with_preview = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("active", 1)),
        Some(preview),
        &map.fold_hidden_set(),
    );
    let baseline = build_section_frames(
        &map,
        &HashMap::new(),
        Some("active"),
        Some(("active", 1)),
        None,
        &map.fold_hidden_set(),
    );
    assert_eq!(with_preview.len(), baseline.len());
    for (a, b) in with_preview.iter().zip(baseline.iter()) {
        assert_eq!(
            a.border_style.color, b.border_style.color,
            "non-targeted section's color must match baseline"
        );
        assert_eq!(
            a.border_style.corners.top_left, b.border_style.corners.top_left,
            "non-targeted section's corner must match baseline"
        );
        assert_eq!(
            a.border_style.font_size_pt, b.border_style.font_size_pt,
            "non-targeted section's size must match baseline"
        );
        assert_eq!(a.focused, b.focused);
    }
}
