//! Glyph-wheel color picker overlay: tree / mutator / area builders
//! for the picker the user opens from an edge or portal context menu
//! (modal) or via the `color picker` console command (standalone
//! palette).
//!
//! Public surface is two functions and a [`ColorPickerOverlayBuild`]
//! result: [`build`] produces a fresh `(tree, backdrop)` from a
//! geometry + viewport, [`build_mutator`] produces an in-place
//! `MutatorTree<GfxMutator>` that updates the same tree's channels
//! without rebuilding the arena. Layout, picker spec, and the
//! `(GlyphArea, GlyphModel)` pair shape stay internal to the module.

use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::{MutatorTree, Tree};

use crate::application::color_picker::{compute_color_picker_layout, ColorPickerOverlayGeometry};

mod color;
mod glyph_model;
mod picker_glyph_areas;
mod tree_builder;

/// Result of [`build`] — the picker tree plus the opaque-backdrop
/// rectangle the renderer needs to draw underneath it.
///
/// `backdrop` is `None` when the picker spec's `transparent_backdrop`
/// flag is set (no opaque rect drawn; per-glyph halos handle
/// legibility) or when the layout yields no backdrop for this
/// geometry — the renderer treats both cases the same: skip the
/// fill-rect pass.
pub(crate) struct ColorPickerOverlayBuild {
    pub tree: Tree<GfxElement, GfxMutator>,
    pub backdrop: Option<(f32, f32, f32, f32)>,
}

/// Build the picker's overlay tree and its backdrop rect from the
/// current `geometry` at the given viewport size. Consumes the
/// picker spec internally to decide whether to emit an opaque
/// backdrop or leave it transparent.
pub(crate) fn build(
    geometry: &ColorPickerOverlayGeometry,
    viewport_w: f32,
    viewport_h: f32,
) -> ColorPickerOverlayBuild {
    let layout = compute_color_picker_layout(geometry, viewport_w, viewport_h);
    let spec = crate::application::widgets::color_picker_widget::load_spec();
    let backdrop = if spec.geometry.transparent_backdrop {
        None
    } else {
        Some(layout.backdrop)
    };
    let tree = tree_builder::build_color_picker_overlay_tree(geometry, &layout);
    ColorPickerOverlayBuild { tree, backdrop }
}

/// Build an in-place [`MutatorTree`] for the picker's
/// already-registered overlay tree. The resulting mutator updates
/// every picker GlyphArea's variable fields at its stable channel;
/// the arena is reused.
pub(crate) fn build_mutator(
    geometry: &ColorPickerOverlayGeometry,
    viewport_w: f32,
    viewport_h: f32,
) -> MutatorTree<GfxMutator> {
    let layout = compute_color_picker_layout(geometry, viewport_w, viewport_h);
    tree_builder::build_color_picker_overlay_mutator(geometry, &layout)
}

#[cfg(test)]
mod tests {
    use super::picker_glyph_areas::picker_glyph_areas;
    use super::tree_builder::{
        build_color_picker_overlay_mutator, build_color_picker_overlay_tree,
    };
    use crate::application::color_picker::CROSSHAIR_CENTER_CELL;
    use baumhard::gfx_structs::area::GlyphArea;

    /// Helpers for the picker mutator tests below.
    fn picker_sample_geometry() -> crate::application::color_picker::ColorPickerOverlayGeometry {
        crate::application::color_picker::ColorPickerOverlayGeometry {
            target_label: "edge",
            hue_deg: 0.0,
            sat: 1.0,
            val: 1.0,
            preview_hex: "#ff0000".to_string(),
            hex_visible: false,
            max_cell_advance: 16.0,
            max_ring_advance: 24.0,
            measurement_font_size: 16.0,
            size_scale: 1.0,
            center_override: None,
            hovered_hit: None,
            arm_top_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_bottom_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_left_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_right_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            preview_ink_offset: (0.0, 0.0),
        }
    }

    fn picker_glyph_areas_for(
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    ) -> Vec<(usize, GlyphArea)> {
        use crate::application::color_picker::compute_color_picker_layout;
        let layout = compute_color_picker_layout(geometry, 1280.0, 720.0);
        picker_glyph_areas(geometry, &layout)
    }

    /// Regression for the visible-glyph-off-centre bug the glyph
    /// alignment session surfaced: the ࿕ preview box is rendered
    /// with `scaled_preview * 1.5` bounds for hover-grow slack, but
    /// it must be **centred symmetrically** on the layout's
    /// intended point. Previously the box was positioned as if
    /// bounds were `preview_size × preview_size`, extending the
    /// extra 0.5× only to the right — drifting the ࿕ right of the
    /// wheel centre by `preview_size / 4` (~15 px at the spec's 3×
    /// preview scale). With `Align::Center` the glyph advance lands
    /// at the box centre; so `pos + bounds/2` must equal the
    /// layout's intended preview centre within rounding slack.
    #[test]
    fn picker_preview_box_centered_symmetrically_on_wheel() {
        use crate::application::color_picker::{
            compute_color_picker_layout, PICKER_CHANNEL_PREVIEW,
        };
        use crate::application::widgets::color_picker_widget::load_spec;
        let g = picker_sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let preview_size = layout.font_size * load_spec().geometry.preview_size_scale;
        let intended = (
            layout.preview_pos.0 + preview_size * 0.5,
            layout.preview_pos.1 + preview_size * 0.5,
        );
        let areas = picker_glyph_areas_for(&g);
        let (_, preview_area) = areas
            .iter()
            .find(|(channel, _)| *channel == PICKER_CHANNEL_PREVIEW)
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

    /// `picker_glyph_areas` must emit channels in strictly
    /// ascending order — Baumhard's `align_child_walks` relies on
    /// this for the §B2 mutator path. Regression guard for any
    /// future band reordering or skipped insertion.
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

    /// Each picker `GlyphArea` node in the overlay tree has exactly
    /// one `GlyphModel` child, sharing the parent's channel — the
    /// architectural pattern the color-picker restructure asked for.
    /// The model node is structural source-of-truth (today's renderer
    /// reads the area; the model is "stamped into" the area at build
    /// time via `glyph_model_from_picker_area`); future per-glyph
    /// mutation / animation work can target the model and re-stamp.
    ///
    /// Walker safety: this regression also implicitly verifies the
    /// §B2 mutator path stays viable — Baumhard's `align_child_walks`
    /// returns immediately when a mutator node has no children
    /// (`tree_walker.rs:237-240`), so adding GlyphModel children
    /// under each GlyphArea doesn't trip the existing flat
    /// `build_color_picker_overlay_mutator` (it produces no mutator
    /// children for these models, so the walk terminates correctly
    /// at each GlyphArea level). The `picker_overlay_mutator_round_trips`
    /// test below exercises the round-trip end-to-end.
    #[test]
    fn picker_overlay_tree_pairs_each_area_with_a_model_child() {
        use baumhard::gfx_structs::element::GfxElementType;
        use baumhard::gfx_structs::tree::BranchChannel;
        use crate::application::color_picker::compute_color_picker_layout;

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

    /// Mutator round-trip: the §B2 in-place update path keeps working
    /// across the tree-shape change in `build_color_picker_overlay_tree`.
    /// Build a tree at state A, apply a mutator computed from state B
    /// (a different hue / sat / val), and verify the resulting
    /// GlyphArea fields match a freshly-built state-B tree. Confirms
    /// the new GlyphModel children don't interfere with channel
    /// alignment (per `picker_overlay_tree_pairs_each_area_with_a_model_child`'s
    /// walker note).
    #[test]
    fn picker_overlay_mutator_round_trips_across_paired_tree() {
        use baumhard::core::primitives::Applicable;
        use baumhard::gfx_structs::tree::BranchChannel;
        use crate::application::color_picker::compute_color_picker_layout;

        let mut g_a = picker_sample_geometry();
        g_a.hue_deg = 0.0;
        g_a.sat = 1.0;
        g_a.val = 1.0;
        let layout_a = compute_color_picker_layout(&g_a, 1280.0, 720.0);

        let mut g_b = picker_sample_geometry();
        g_b.hue_deg = 120.0;
        g_b.sat = 0.6;
        g_b.val = 0.4;
        let layout_b = compute_color_picker_layout(&g_b, 1280.0, 720.0);

        let mut tree = build_color_picker_overlay_tree(&g_a, &layout_a);
        let mutator = build_color_picker_overlay_mutator(&g_b, &layout_b);
        mutator.apply_to(&mut tree);

        let expected = picker_glyph_areas(&g_b, &layout_b);
        let mut got: Vec<(usize, GlyphArea)> = Vec::new();
        for descendant_id in tree.root().descendants(&tree.arena) {
            let node = tree.arena.get(descendant_id).expect("arena node");
            let element = node.get();
            if let Some(area) = element.glyph_area() {
                got.push((element.channel(), area.clone()));
            }
        }

        assert_eq!(got.len(), expected.len(), "post-mutation area count");
        for ((c_got, a_got), (c_exp, a_exp)) in got.iter().zip(expected.iter()) {
            assert_eq!(c_got, c_exp, "channel mismatch on round-trip");
            assert_eq!(a_got.text, a_exp.text, "text on ch {c_got}");
            assert_eq!(a_got.position, a_exp.position, "position on ch {c_got}");
            assert_eq!(a_got.regions, a_exp.regions, "regions on ch {c_got}");
        }
    }

    /// Hex visibility flips on cursor enter/exit of the backdrop.
    /// The element set must stay stable across that flip — same
    /// channels, same count — so the mutator path can keep using
    /// the same registered tree without unregistering / rebuilding.
    /// When invisible, the hex emits empty text (walker shapes
    /// nothing).
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
        let invisible_channels: Vec<usize> =
            invisible.iter().map(|(c, _)| *c).collect();
        let visible_channels: Vec<usize> = visible.iter().map(|(c, _)| *c).collect();
        assert_eq!(invisible_channels, visible_channels);
        // Hex itself: invisible → empty text, visible → hex string.
        let hex_invisible = invisible
            .iter()
            .find(|(c, _)| *c == crate::application::color_picker::PICKER_CHANNEL_HEX)
            .expect("hex channel present");
        assert!(hex_invisible.1.text.is_empty());
        let hex_visible = visible
            .iter()
            .find(|(c, _)| *c == crate::application::color_picker::PICKER_CHANNEL_HEX)
            .expect("hex channel present");
        assert!(hex_visible.1.text.starts_with('#'));
    }

    /// Round-trip: applying the mutator to a freshly-built tree
    /// should leave every GlyphArea's variable state matching what
    /// a fresh `picker_glyph_areas` call would emit. Pins the
    /// promise that the §B2 in-place update path produces the same
    /// observable state as a from-scratch rebuild.
    ///
    /// Strategy: build a tree with state A, build a mutator from
    /// state B, apply the mutator, then verify the tree's
    /// per-channel GlyphAreas equal what `picker_glyph_areas(B)`
    /// would have produced.
    #[test]
    fn picker_mutator_round_trips_to_fresh_build() {
        use crate::application::color_picker::{compute_color_picker_layout, PickerHit};
        use baumhard::core::primitives::Applicable;
        use baumhard::gfx_structs::tree::BranchChannel;

        let g_a = picker_sample_geometry();
        let mut g_b = picker_sample_geometry();
        g_b.hue_deg = 120.0;
        g_b.sat = 0.5;
        g_b.val = 0.7;
        g_b.hovered_hit = Some(PickerHit::Hue(3));

        let layout_a = compute_color_picker_layout(&g_a, 1280.0, 720.0);
        let layout_b = compute_color_picker_layout(&g_b, 1280.0, 720.0);

        // Build the picker tree at state A, then apply the mutator
        // computed from state B.
        let mut tree = build_color_picker_overlay_tree(&g_a, &layout_a);
        let mutator = build_color_picker_overlay_mutator(&g_b, &layout_b);
        mutator.apply_to(&mut tree);

        // Fresh build at state B, for comparison.
        let expected = picker_glyph_areas(&g_b, &layout_b);

        // Walk the mutated tree, gather (channel, area) pairs, and
        // compare to `expected`. Since the mutator uses Assign on
        // every variable field, the pairs should match.
        let mut got: Vec<(usize, GlyphArea)> = Vec::new();
        for descendant_id in tree.root().descendants(&tree.arena) {
            let node = tree.arena.get(descendant_id).expect("arena node");
            let element = node.get();
            if let Some(area) = element.glyph_area() {
                got.push((element.channel(), area.clone()));
            }
        }

        assert_eq!(
            got.len(),
            expected.len(),
            "post-mutation tree element count mismatch"
        );
        for ((c_got, a_got), (c_exp, a_exp)) in got.iter().zip(expected.iter()) {
            assert_eq!(c_got, c_exp, "channel mismatch");
            assert_eq!(
                a_got.text, a_exp.text,
                "text mismatch on channel {c_got}"
            );
            assert_eq!(
                a_got.position, a_exp.position,
                "position mismatch on channel {c_got}"
            );
            assert_eq!(
                a_got.render_bounds, a_exp.render_bounds,
                "bounds mismatch on channel {c_got}"
            );
            assert_eq!(
                a_got.scale, a_exp.scale,
                "scale mismatch on channel {c_got}"
            );
            assert_eq!(
                a_got.line_height, a_exp.line_height,
                "line_height mismatch on channel {c_got}"
            );
            assert_eq!(
                a_got.outline, a_exp.outline,
                "outline mismatch on channel {c_got}"
            );
            // Regions equality compares the inner Vec — a single
            // mismatch on any region field (range, font, color)
            // surfaces here.
            assert_eq!(
                a_got.regions, a_exp.regions,
                "regions mismatch on channel {c_got}"
            );
        }
    }
}
