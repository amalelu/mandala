// SPDX-License-Identifier: MPL-2.0

//! Initial-build path invariants — channel ordering, preview
//! centering, and the GlyphArea/GlyphModel pairing the picker tree's
//! mutator-walker safety relies on.

use baumhard::core::primitives::Discriminated;
use baumhard::gfx_structs::element::GfxElementType;
use baumhard::util::geometry::aabb_center;
use glam::Vec2;

use super::fixtures::{picker_glyph_areas_for, picker_sample_geometry};
use crate::application::color_picker::{compute_color_picker_layout, picker_channel};
use crate::application::color_picker_overlay::picker_glyph_areas::build_color_picker_overlay_tree;
use crate::application::widgets::color_picker_widget::load_spec;

/// Regression for the visible-glyph-off-center bug the glyph
/// alignment session surfaced: the ࿕ preview box is rendered with
/// `scaled_preview * 1.5` bounds for hover-grow slack, but it must
/// be **centered symmetrically** on the layout's intended point.
/// Previously the box was positioned as if bounds were
/// `preview_size × preview_size`, extending the extra 0.5× only to
/// the right — drifting the ࿕ right of the wheel center by
/// `preview_size / 4` (~15 px at the spec's 3× preview scale).
/// With `Align::Center` the glyph advance lands at the box center;
/// so `pos + bounds/2` must equal the layout's intended preview
/// center within rounding slack.
#[test]
fn picker_preview_box_centered_symmetrically_on_wheel() {
    let g = picker_sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let preview_size = layout.font_size * load_spec().geometry.preview_size_scale;
    let intended = (
        layout.preview_pos.0 + preview_size * 0.5,
        layout.preview_pos.1 + preview_size * 0.5,
    );
    let areas = picker_glyph_areas_for(&g);
    let preview_ch = picker_channel("preview", 0);
    let (_, preview_area) = areas
        .iter()
        .find(|(channel, _)| *channel == preview_ch)
        .expect("preview area must be emitted");
    let box_center = aabb_center(
        Vec2::new(preview_area.position.x.0, preview_area.position.y.0),
        Vec2::new(preview_area.render_bounds.x.0, preview_area.render_bounds.y.0),
    );
    assert!(
        (box_center.x - intended.0).abs() < 0.01,
        "preview box-center x {} drifts from intended {}",
        box_center.x,
        intended.0,
    );
    assert!(
        (box_center.y - intended.1).abs() < 0.01,
        "preview box-center y {} drifts from intended {}",
        box_center.y,
        intended.1,
    );
}

/// `picker_glyph_areas` must emit channels in strictly ascending
/// order — Baumhard's `align_child_walks` relies on this for the §B2
/// mutator path. Regression guard for any future band reordering or
/// skipped insertion.
#[test]
fn picker_glyph_areas_ascending_channels() {
    let g = picker_sample_geometry();
    let areas = picker_glyph_areas_for(&g);
    for window in areas.windows(2) {
        assert!(
            window[1].0 > window[0].0,
            "channel {} should follow {} strictly, got {} → {}",
            window[0].0,
            window[0].0,
            window[0].0,
            window[1].0,
        );
    }
}

/// Every direct child of the picker overlay tree's root is a
/// `GlyphArea`, and they're all leaves — Baumhard's
/// `align_child_walks` relies on the flat shape for the §B2 mutator
/// path.
#[test]
fn picker_overlay_tree_emits_flat_glyph_area_children() {
    let g = picker_sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let tree = build_color_picker_overlay_tree(&g, &layout);

    let mut area_count = 0usize;
    for area_id in tree.root.children(&tree.arena) {
        let area_elem = tree.arena.get(area_id).expect("area node in arena").get();
        assert!(
            matches!(area_elem.variant(), GfxElementType::GlyphArea),
            "every direct child of root must be a GlyphArea"
        );
        assert_eq!(
            area_id.children(&tree.arena).count(),
            0,
            "picker GlyphAreas are leaves",
        );
        area_count += 1;
    }
    assert!(area_count > 0, "picker tree must emit at least one piece");
}

/// Hex visibility flips on cursor enter/exit of the backdrop. The
/// element set must stay stable across that flip — same channels,
/// same count — so the mutator path can keep using the same
/// registered tree without unregistering / rebuilding. When
/// invisible, the hex emits empty text (walker shapes nothing).
#[test]
fn picker_glyph_areas_hex_channel_stable_when_visibility_flips() {
    let mut g = picker_sample_geometry();
    g.hex_visible = false;
    let invisible = picker_glyph_areas_for(&g);
    g.hex_visible = true;
    let visible = picker_glyph_areas_for(&g);
    assert_eq!(
        invisible.len(),
        visible.len(),
        "element count must stay stable across hex visibility"
    );
    let invisible_channels: Vec<usize> = invisible.iter().map(|(c, _)| *c).collect();
    let visible_channels: Vec<usize> = visible.iter().map(|(c, _)| *c).collect();
    assert_eq!(invisible_channels, visible_channels);
    // Hex itself: invisible → empty text, visible → hex string.
    let hex_ch = picker_channel("hex", 0);
    let hex_invisible = invisible
        .iter()
        .find(|(c, _)| *c == hex_ch)
        .expect("hex channel present");
    assert!(hex_invisible.1.text.is_empty());
    let hex_visible = visible
        .iter()
        .find(|(c, _)| *c == hex_ch)
        .expect("hex channel present");
    assert!(hex_visible.1.text.starts_with('#'));
}
