//! Initial-build path invariants — channel ordering, preview
//! centering, and the GlyphArea/GlyphModel pairing the picker tree's
//! mutator-walker safety relies on.

use baumhard::gfx_structs::element::GfxElementType;
use baumhard::gfx_structs::tree::BranchChannel;

use super::fixtures::{picker_glyph_areas_for, picker_sample_geometry};
use crate::application::color_picker::{compute_color_picker_layout, picker_channel};
use crate::application::color_picker_overlay::picker_glyph_areas::build_color_picker_overlay_tree;
use crate::application::widgets::color_picker_widget::load_spec;

/// Regression for the visible-glyph-off-centre bug the glyph
/// alignment session surfaced: the ࿕ preview box is rendered with
/// `scaled_preview * 1.5` bounds for hover-grow slack, but it must
/// be **centred symmetrically** on the layout's intended point.
/// Previously the box was positioned as if bounds were
/// `preview_size × preview_size`, extending the extra 0.5× only to
/// the right — drifting the ࿕ right of the wheel centre by
/// `preview_size / 4` (~15 px at the spec's 3× preview scale).
/// With `Align::Center` the glyph advance lands at the box centre;
/// so `pos + bounds/2` must equal the layout's intended preview
/// centre within rounding slack.
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
    let box_center = (
        preview_area.position.x.0 + preview_area.render_bounds.x.0 * 0.5,
        preview_area.position.y.0 + preview_area.render_bounds.y.0 * 0.5,
    );
    assert!(
        (box_center.0 - intended.0).abs() < 0.01,
        "preview box-centre x {} drifts from intended {}",
        box_center.0,
        intended.0,
    );
    assert!(
        (box_center.1 - intended.1).abs() < 0.01,
        "preview box-centre y {} drifts from intended {}",
        box_center.1,
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

/// Each picker `GlyphArea` node in the overlay tree has exactly one
/// `GlyphModel` child, sharing the parent's channel — the
/// architectural pattern the color-picker restructure asked for. The
/// model node is structural source-of-truth (today's renderer reads
/// the area; the model is "stamped into" the area at build time via
/// `glyph_model_from_picker_area`); future per-glyph mutation /
/// animation work can target the model and re-stamp.
///
/// Walker safety: this regression also implicitly verifies the §B2
/// mutator path stays viable — Baumhard's `align_child_walks`
/// returns immediately when a mutator node has no children
/// (`tree_walker.rs:237-240`), so adding GlyphModel children under
/// each GlyphArea doesn't trip the existing flat
/// `build_color_picker_overlay_mutator` (it produces no mutator
/// children for these models, so the walk terminates correctly at
/// each GlyphArea level).
#[test]
fn picker_overlay_tree_pairs_each_area_with_a_model_child() {
    let g = picker_sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let tree = build_color_picker_overlay_tree(&g, &layout);

    let mut area_count = 0usize;
    let mut model_count = 0usize;
    for area_id in tree.root.children(&tree.arena) {
        let area_node = tree.arena.get(area_id).expect("area node in arena");
        let area_elem = area_node.get();
        // `GfxElementType` doesn't implement `Debug`, so assert via
        // `matches!` rather than `assert_eq!` — same coverage, no
        // upstream-derive change needed.
        assert!(
            matches!(area_elem.get_type(), GfxElementType::GlyphArea),
            "every direct child of root must be a GlyphArea"
        );
        area_count += 1;
        let area_channel = area_elem.channel();

        let mut children = area_id.children(&tree.arena);
        let model_id = children
            .next()
            .expect("each picker GlyphArea must have a paired GlyphModel child");
        assert!(
            children.next().is_none(),
            "each picker GlyphArea has exactly one child (the GlyphModel pair)"
        );
        let model_node = tree.arena.get(model_id).expect("model node in arena");
        let model_elem = model_node.get();
        assert!(
            matches!(model_elem.get_type(), GfxElementType::GlyphModel),
            "the area's child must be a GlyphModel"
        );
        assert_eq!(
            model_elem.channel(),
            area_channel,
            "model child shares its parent area's channel"
        );
        model_count += 1;
    }
    assert!(area_count > 0, "picker tree must emit at least one piece");
    assert_eq!(area_count, model_count, "every area has its model");
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
