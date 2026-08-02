// SPDX-License-Identifier: MPL-2.0

//! Clip-AABB pass: the boxes the connection sampler drops glyphs
//! inside. A frameless node contributes its raw rect; a framed one
//! contributes the rect grown by the resolved border's extent, so
//! connection glyphs stop at the frame rather than under it. A
//! border preview participates, because a preview can change both
//! that extent and whether a frame exists at all.

use super::fixtures::*;
use crate::mindmap::border::BORDER_APPROX_CHAR_WIDTH_FRAC;
use crate::mindmap::tree_builder::{
    node_clip_aabbs, BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef, EditView,
};
use std::collections::HashMap;

/// The single AABB a one-node map produces.
fn only_aabb(
    map: &crate::mindmap::model::MindMap,
    preview: Option<BorderPreview<'_>>,
) -> (glam::Vec2, glam::Vec2) {
    let hidden = map.fold_hidden_set();
    let aabbs = node_clip_aabbs(map, &HashMap::new(), preview, &hidden);
    assert_eq!(aabbs.len(), 1, "one visible node → one clip box");
    aabbs[0]
}

#[test]
fn test_frameless_node_clips_at_its_raw_rect() {
    let map = synthetic_map(vec![sized_node("a", 10.0, 20.0, 80.0, 40.0, false)], vec![]);
    let (pos, size) = only_aabb(&map, None);
    assert!((pos.x - 10.0).abs() < 1e-3, "x: {}", pos.x);
    assert!((pos.y - 20.0).abs() < 1e-3, "y: {}", pos.y);
    assert!((size.x - 80.0).abs() < 1e-3, "w: {}", size.x);
    assert!((size.y - 40.0).abs() < 1e-3, "h: {}", size.y);
}

/// A framed node's clip box grows by one border `font_size`
/// vertically and one approx-char-width horizontally on each side,
/// so a connection glyph never lands inside the drawn frame.
#[test]
fn test_framed_node_clip_box_grows_by_the_border_extent() {
    let map = synthetic_map(vec![sized_node("a", 10.0, 20.0, 80.0, 40.0, true)], vec![]);
    let (pos, size) = only_aabb(&map, None);
    // The fixture has no per-node border config, so the cascade
    // lands on the 14 pt default.
    let bf = 14.0_f32;
    let bcw = bf * BORDER_APPROX_CHAR_WIDTH_FRAC;
    assert!((pos.x - (10.0 - bcw)).abs() < 1e-3, "x: {}", pos.x);
    assert!((pos.y - (20.0 - bf)).abs() < 1e-3, "y: {}", pos.y);
    assert!((size.x - (80.0 + 2.0 * bcw)).abs() < 1e-3, "w: {}", size.x);
    assert!((size.y - (40.0 + 2.0 * bf)).abs() < 1e-3, "h: {}", size.y);
}

#[test]
fn test_hidden_by_fold_node_contributes_no_clip_box() {
    let mut parent = synthetic_node("parent", None, 0.0, 0.0);
    parent.folded = true;
    let child = synthetic_node("child", Some("parent"), 0.0, 100.0);
    let map = synthetic_map(vec![parent, child], vec![]);
    let hidden = map.fold_hidden_set();
    let aabbs = node_clip_aabbs(&map, &HashMap::new(), None, &hidden);
    assert_eq!(
        aabbs.len(),
        1,
        "the folded parent still clips; its child does not"
    );
}

#[test]
fn test_drag_offset_moves_the_clip_box() {
    let map = synthetic_map(vec![sized_node("a", 10.0, 20.0, 80.0, 40.0, false)], vec![]);
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (5.0_f32, -7.0_f32));
    let hidden = map.fold_hidden_set();
    let aabbs = node_clip_aabbs(&map, &offsets, None, &hidden);
    assert!((aabbs[0].0.x - 15.0).abs() < 1e-3, "x: {}", aabbs[0].0.x);
    assert!((aabbs[0].0.y - 13.0).abs() < 1e-3, "y: {}", aabbs[0].0.y);
}

/// A `force_show_frame` preview on a frameless node has to grow the
/// clip box too — otherwise the previewed frame renders but
/// connection glyphs keep running underneath it, and the preview
/// looks broken in a way the committed state never would.
#[test]
fn test_force_show_frame_preview_grows_a_frameless_node_clip_box() {
    let map = synthetic_map(vec![sized_node("a", 0.0, 0.0, 80.0, 40.0, false)], vec![]);
    let target_ids = [String::from("a")];
    let preview = BorderPreview {
        target: BorderPreviewTargetRef::Nodes(&target_ids),
        edits: BorderConfigEditsView {
            preset: EditView::Set("heavy"),
            ..Default::default()
        },
        force_show_frame: true,
    };
    let (bare_pos, bare_size) = only_aabb(&map, None);
    let (preview_pos, preview_size) = only_aabb(&map, Some(preview));
    assert!(
        preview_pos.x < bare_pos.x && preview_pos.y < bare_pos.y,
        "previewed frame must push the clip origin outward: {:?} vs {:?}",
        preview_pos,
        bare_pos
    );
    assert!(
        preview_size.x > bare_size.x && preview_size.y > bare_size.y,
        "previewed frame must grow the clip extent: {:?} vs {:?}",
        preview_size,
        bare_size
    );
}

/// A preview that changes the border font size changes the clip
/// extent with it — the box tracks what is actually on screen.
#[test]
fn test_preview_font_size_drives_the_clip_extent() {
    let map = synthetic_map(vec![sized_node("a", 0.0, 0.0, 80.0, 40.0, true)], vec![]);
    let target_ids = [String::from("a")];
    let preview = BorderPreview {
        target: BorderPreviewTargetRef::Nodes(&target_ids),
        edits: BorderConfigEditsView {
            font_size_pt: EditView::Set(28.0),
            ..Default::default()
        },
        force_show_frame: false,
    };
    let (_, committed) = only_aabb(&map, None);
    let (_, previewed) = only_aabb(&map, Some(preview));
    assert!(
        previewed.y > committed.y,
        "doubling the border font size must widen the vertical clip margin: {} vs {}",
        previewed.y,
        committed.y
    );
}
