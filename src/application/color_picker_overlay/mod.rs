//! Glyph-wheel color picker overlay: tree / mutator / area builders
//! for the picker the user opens from an edge or portal context menu
//! (modal) or via the `color picker` console command (standalone
//! palette).
//!
//! Public surface is two functions and a [`ColorPickerOverlayBuild`]
//! result: [`build`] produces a fresh `(tree, backdrop)` from a
//! geometry + layout, [`build_mutator`] produces an in-place
//! `MutatorTree<GfxMutator>` that updates the same tree's channels
//! without rebuilding the arena. Layout is computed once at the
//! dispatch site (app.rs `compute_picker_geometry`) and threaded
//! through to every builder so the per-frame hot path doesn't
//! recompute font-size derivation, cell anchor math, or backdrop
//! geometry. Picker spec and the `(GlyphArea, GlyphModel)` pair shape
//! stay internal to the module.

use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::{MutatorTree, Tree};

use crate::application::color_picker::{ColorPickerLayout, ColorPickerOverlayGeometry};

mod color;
mod glyph_model;
mod picker_glyph_areas;

#[cfg(test)]
mod tests;

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
/// current `geometry` + pre-computed `layout`. Consumes the picker
/// spec internally to decide whether to emit an opaque backdrop or
/// leave it transparent.
pub(crate) fn build(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> ColorPickerOverlayBuild {
    let spec = crate::application::widgets::color_picker_widget::load_spec();
    let backdrop = if spec.geometry.transparent_backdrop {
        None
    } else {
        Some(layout.backdrop)
    };
    let tree = picker_glyph_areas::build_color_picker_overlay_tree(geometry, layout);
    ColorPickerOverlayBuild { tree, backdrop }
}

/// Build an in-place [`MutatorTree`] for the picker's
/// already-registered overlay tree — the **layout** phase. Carries
/// every variable field on every cell; meant to run on layout-change
/// events (initial open, viewport resize, RMB size_scale drag), not
/// on every hover/HSV frame.
///
/// Per-frame updates go through [`build_dynamic_mutator`] instead.
pub(crate) fn build_mutator(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> MutatorTree<GfxMutator> {
    picker_glyph_areas::build_color_picker_overlay_mutator(geometry, layout)
}

/// Build an in-place [`MutatorTree`] for the picker — the **dynamic**
/// phase, applied every hover / HSV / drag frame. Same channel layout
/// as [`build_mutator`] but slimmer per-section field lists, driven by
/// the JSON's `dynamic_mutator_spec`. Position / bounds / line_height
/// / outline are *not* touched here — those come from the layout
/// phase and stay valid across dynamic applies.
pub(crate) fn build_dynamic_mutator(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> MutatorTree<GfxMutator> {
    picker_glyph_areas::build_color_picker_overlay_dynamic_mutator(geometry, layout)
}

