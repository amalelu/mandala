//! Tree- and mutator-builders for the glyph-wheel color picker
//! overlay. Both paths route through
//! [`super::picker_glyph_areas::compute_picker_areas`] so the initial
//! build and the §B2 in-place update can never drift on the element
//! set. The mutator path additionally delegates tree-shape decisions
//! to [`crate::application::mutator_builder`], whose input is
//! the declarative `mutator_spec` block in
//! `widgets/color_picker.json`.

use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::{MutatorTree, Tree};

use super::glyph_model::glyph_model_from_picker_area;
use super::picker_glyph_areas::{compute_picker_areas, PickerAreas};
use crate::application::mutator_builder::{self, SectionContext};

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
/// `rebuild_color_picker_overlay_buffers` call, which is the hover
/// hot path. The §B2 mutator path ([`build_color_picker_overlay_mutator`])
/// reuses the same arena and updates field values in place.
pub(super) fn build_color_picker_overlay_tree(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> Tree<GfxElement, GfxMutator> {
    let areas = compute_picker_areas(geometry, layout);
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    for (channel, area) in &areas.ordered {
        let model = glyph_model_from_picker_area(area);
        let area_element = GfxElement::new_area_non_indexed_with_id(area.clone(), *channel, *channel);
        let area_id = tree.arena.new_node(area_element);
        tree.root.append(area_id, &mut tree.arena);
        let model_element =
            GfxElement::new_model_non_indexed_with_id(model, *channel, *channel);
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
pub(super) fn build_color_picker_overlay_mutator(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> MutatorTree<GfxMutator> {
    let spec = crate::application::widgets::color_picker_widget::load_spec();
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
/// **Cost note**: this still routes through [`compute_picker_areas`]
/// today, so the per-frame work is dominated by full GlyphArea
/// construction even though the mutator only reads a subset of
/// fields. Closing that gap is the next consolidation step (slim
/// per-section context + `SectionContext::dynamic_field`); pinned by
/// the `dynamic_mutator_spec_per_section_fields_are_slim` test in
/// `widgets::color_picker_widget`.
pub(super) fn build_color_picker_overlay_dynamic_mutator(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> MutatorTree<GfxMutator> {
    let spec = crate::application::widgets::color_picker_widget::load_spec();
    let areas = compute_picker_areas(geometry, layout);
    let ctx = PickerSectionContext { areas: &areas };
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

