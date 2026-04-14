//! Tree- and mutator-builders for the glyph-wheel color picker
//! overlay. Both paths consume the same
//! [`super::picker_glyph_areas::picker_glyph_areas`] so the initial
//! build and the §B2 in-place update can never drift on what the
//! picker's element set looks like.

use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::{MutatorTree, Tree};

use super::glyph_model::glyph_model_from_picker_area;
use super::picker_glyph_areas::picker_glyph_areas;

/// Build the color-picker overlay tree from a geometry +
/// pre-computed layout. Mirrors what
/// `Renderer::rebuild_color_picker_overlay_buffers_legacy` did
/// across its static + dynamic halves, but as one
/// `Tree<GfxElement, GfxMutator>` instead of two parallel buffer
/// lists.
///
/// Tree shape (each GlyphArea has a paired GlyphModel child built
/// by [`glyph_model_from_picker_area`] — see that function's docs
/// for why the model child exists and how it interacts with the
/// mutator path):
///
/// ```text
/// Void (root)
/// ├── GlyphArea title bar
/// │   └── GlyphModel mirror
/// ├── GlyphArea hue ring slot 0
/// │   └── GlyphModel mirror
/// │   ...
/// ├── GlyphArea hue ring slot 23
/// │   └── GlyphModel mirror
/// ├── GlyphArea hint footer
/// │   └── GlyphModel mirror
/// ├── GlyphArea sat-bar cell 0..N (skipping centre)
/// │   └── GlyphModel mirror
/// ├── GlyphArea val-bar cell 0..N (skipping centre)
/// │   └── GlyphModel mirror
/// ├── GlyphArea preview glyph (࿕ at 2× font size)
/// │   └── GlyphModel mirror
/// ├── GlyphArea hex readout (when geometry.hex_visible)
/// │   └── GlyphModel mirror
/// └── GlyphArea theme chip 0..N
///     └── GlyphModel mirror
/// ```
///
/// **Performance note**: this rebuilds every glyph on every
/// `rebuild_color_picker_overlay_buffers` call, which is the hover
/// hot path. The legacy split skipped the hue-ring shape on hover.
/// A follow-up will introduce a `MutatorTree`-based incremental
/// path (per §B2 of `lib/baumhard/CONVENTIONS.md`) that mutates
/// only the cells whose colors changed and the indicator's
/// position, leaving the static hue ring alone. The user
/// explicitly asked to land the migration first and address
/// picker sluggishness afterwards.
pub(crate) fn build_color_picker_overlay_tree(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    for (channel, area) in picker_glyph_areas(geometry, layout) {
        // GlyphModel mirror is built *before* the area is moved into
        // the GlyphArea node so we can read the area's text + regions
        // without juggling references across the move.
        let model = glyph_model_from_picker_area(&area);
        let area_element = GfxElement::new_area_non_indexed_with_id(area, channel, channel);
        let area_id = tree.arena.new_node(area_element);
        tree.root.append(area_id, &mut tree.arena);
        // The model child shares its parent area's channel — there's
        // no sibling to collide with (each area has exactly one model
        // child) and the channel match makes future mutator-tree
        // routing trivial if/when the §B2 path starts targeting models
        // directly.
        let model_element =
            GfxElement::new_model_non_indexed_with_id(model, channel, channel);
        let model_id = tree.arena.new_node(model_element);
        area_id.append(model_id, &mut tree.arena);
    }
    tree
}

/// Build a [`MutatorTree`] that updates an already-registered picker
/// tree to the current `(geometry, layout)` state without rebuilding
/// the arena. Pairs with [`build_color_picker_overlay_tree`] —
/// channels are stable across both, so the walker's
/// `align_child_walks` matches each mutator child against the
/// existing GlyphArea at the same channel.
///
/// Every entry is an `Assign` `DeltaGlyphArea` carrying the full set
/// of variable fields (text, position, bounds, scale, line_height,
/// regions, outline). `align_center` stays at whatever the initial
/// tree build set; it's never mutated through this path because the
/// picker's per-element alignment is constant.
///
/// This is the §B2 "mutation, not rebuild" path for picker hover /
/// HSV / chip / drag updates. The arena is reused; only field values
/// change. The walker still re-shapes every cell — that's the
/// remaining §B1 perf gap, tracked in `ROADMAP.md` as the
/// hash-keyed shape cache follow-up.
pub(crate) fn build_color_picker_overlay_mutator(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> MutatorTree<GfxMutator> {
    use baumhard::core::primitives::ApplyOperation;
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use baumhard::gfx_structs::mutator::Mutation;

    let mut mt = MutatorTree::new_with(GfxMutator::new_void(0));
    for (channel, area) in picker_glyph_areas(geometry, layout) {
        let delta = DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(area.text),
            GlyphAreaField::position(area.position.x.0, area.position.y.0),
            GlyphAreaField::bounds(area.render_bounds.x.0, area.render_bounds.y.0),
            GlyphAreaField::scale(area.scale.0),
            GlyphAreaField::line_height(area.line_height.0),
            GlyphAreaField::ColorFontRegions(area.regions),
            GlyphAreaField::Outline(area.outline),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]);
        let mutator = GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), channel);
        let id = mt.arena.new_node(mutator);
        mt.root.append(id, &mut mt.arena);
    }
    mt
}
