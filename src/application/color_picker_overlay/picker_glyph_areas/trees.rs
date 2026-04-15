//! Tree- and mutator-tree builders the renderer calls from
//! `apply_color_picker_overlay_*`. The initial-build path produces a
//! freshly-allocated `Tree<GfxElement, GfxMutator>`; the layout and
//! dynamic mutator paths produce in-place `MutatorTree<GfxMutator>`s
//! that target an already-registered overlay tree.

use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::{MutatorTree, Tree};

use super::areas::PickerAreas;
use super::compute::compute_picker_areas;
use super::dynamic_context::PickerDynamicContext;
use crate::application::color_picker::{ColorPickerLayout, ColorPickerOverlayGeometry};
use crate::application::color_picker_overlay::glyph_model::glyph_model_from_picker_area;
use crate::application::mutator_builder::{self, SectionContext};
use crate::application::widgets::color_picker_widget::load_spec;

/// Build the color-picker overlay tree from a geometry +
/// pre-computed layout. Iterates the
/// [channel-ascending][PickerAreas::ordered] list so the registered
/// tree's channels match what the mutator path will later target.
///
/// Tree shape (each GlyphArea has a paired GlyphModel child built by
/// [`glyph_model_from_picker_area`]):
///
/// ```text
/// Void (root)
/// ├── GlyphArea title bar / hue ring / hint / sat bar / val bar / preview / hex
/// │   └── GlyphModel mirror
/// └── …
/// ```
///
/// **Performance note**: this rebuilds every glyph on every
/// `rebuild_color_picker_overlay_buffers` call, which is reserved
/// for picker open / close and tree-shape changes. Per-frame
/// updates go through [`build_color_picker_overlay_dynamic_mutator`]
/// — same arena, slim per-cell delta.
pub(in crate::application::color_picker_overlay) fn build_color_picker_overlay_tree(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> Tree<GfxElement, GfxMutator> {
    let areas = compute_picker_areas(geometry, layout);
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    // Consume the ordered vector so each GlyphArea is moved into its
    // GfxElement rather than cloned. The model is derived from the
    // area by reference before the move so both children share the
    // same text / color / position without a second allocation.
    for (channel, area) in areas.ordered {
        let model = glyph_model_from_picker_area(&area);
        let area_element =
            GfxElement::new_area_non_indexed_with_id(area, channel, channel);
        let area_id = tree.arena.new_node(area_element);
        tree.root.append(area_id, &mut tree.arena);
        let model_element =
            GfxElement::new_model_non_indexed_with_id(model, channel, channel);
        let model_id = tree.arena.new_node(model_element);
        area_id.append(model_id, &mut tree.arena);
    }
    tree
}

/// Build a [`MutatorTree`] that updates an already-registered picker
/// tree to the current `(geometry, layout)` state without rebuilding
/// the arena. The tree shape is declared in
/// `widgets/color_picker.json`'s `mutator_spec` (the **layout** spec
/// — full per-cell field set); the per-cell `GlyphArea` values come
/// from [`PickerAreas`] via a [`PickerSectionContext`] adapter.
///
/// This is the §B2 "mutation, not rebuild" path for layout-change
/// events: initial open, viewport resize, and RMB size_scale drag.
/// Hover / HSV / chip frames go through
/// [`build_color_picker_overlay_dynamic_mutator`] instead — same
/// channel layout, slimmer per-section field lists.
pub(in crate::application::color_picker_overlay) fn build_color_picker_overlay_mutator(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> MutatorTree<GfxMutator> {
    let spec = load_spec();
    let areas = compute_picker_areas(geometry, layout);
    let ctx = PickerSectionContext { areas: &areas };
    mutator_builder::build(&spec.mutator_spec, &ctx)
}

/// Per-frame [`MutatorTree`] for the picker — the **dynamic** phase.
/// Walked from `widgets/color_picker.json`'s `dynamic_mutator_spec`,
/// which carries the same channel layout as `mutator_spec` but only
/// the per-section `CellField`s that actually change between hover /
/// HSV / drag frames (color, hover scale, hex text). Position,
/// bounds, line_height, and outline come from the layout phase and
/// stay untouched here.
///
/// Uses a [`PickerDynamicContext`] that overrides
/// `SectionContext::field` directly — each dynamic field is
/// computed from `(geometry, section, index)` on demand, so no
/// per-frame `Vec<(usize, GlyphArea)>`, no 60× `GlyphArea::new_with_str`,
/// no full-layout position/bounds math, and no per-cell outline
/// struct. The only clones that remain are the inevitable ones on
/// the mutator tree's `Vec<GlyphAreaField>` and its `Box<DeltaGlyphArea>`.
pub(in crate::application::color_picker_overlay) fn build_color_picker_overlay_dynamic_mutator(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> MutatorTree<GfxMutator> {
    let spec = load_spec();
    let ctx = PickerDynamicContext::new(geometry, layout);
    mutator_builder::build(&spec.dynamic_mutator_spec, &ctx)
}

/// Adapter implementing [`SectionContext`] on top of a precomputed
/// [`PickerAreas`] table. Only `area(section, index)` is wired — the
/// picker spec uses no runtime counts, runtime mutations, or macros,
/// so the other trait methods keep their `unreachable!()` defaults.
struct PickerSectionContext<'a> {
    areas: &'a PickerAreas,
}

impl<'a> SectionContext for PickerSectionContext<'a> {
    fn area(&self, section: &str, index: usize) -> &GlyphArea {
        self.areas.area(section, index)
    }
}
