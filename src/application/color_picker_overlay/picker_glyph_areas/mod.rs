//! Single source of truth for the picker's per-section
//! `GlyphArea` set, plus the tree- and mutator-builders that wrap it
//! into the shapes the renderer registers. Everything that turns
//! `(geometry, layout)` into a registered overlay or an in-place
//! mutator lives here; the initial-build path and the §B2 mutator
//! paths cannot drift because they all read from the same
//! [`PickerAreas`] table built by [`compute_picker_areas`].
//!
//! The section names ("title", "hue_ring", "hint", "sat_bar",
//! "val_bar", "preview", "hex") must match the
//! `mutator_spec.sections[*].section` strings in
//! `widgets/color_picker.json` — the spec's channel layout is
//! authoritative and the per-section builders below fill the cells
//! it asks for.
//!
//! Module split:
//! - [`areas`] — `PickerAreas` table + `PickerSection` enum.
//! - [`make_area`] — the single `GlyphArea` constructor used by the
//!   layout-phase per-section builders.
//! - [`compute`] — the layout-phase `compute_picker_areas` dispatcher
//!   that calls each section builder in turn.
//! - [`sections`] — one file per picker section, each owning the
//!   per-cell layout math for that section.
//! - [`trees`] — the tree- and mutator-tree builders the renderer
//!   calls from `apply_color_picker_overlay_*`.
//! - [`dynamic_context`] — `PickerDynamicContext`, the slim per-frame
//!   `SectionContext` impl that bypasses `compute_picker_areas` for
//!   the dynamic mutator phase.

mod areas;
mod compute;
mod dynamic_context;
mod make_area;
mod sections;
mod trees;

pub(super) use trees::{
    build_color_picker_overlay_dynamic_mutator, build_color_picker_overlay_mutator,
    build_color_picker_overlay_tree,
};

#[cfg(test)]
pub(super) use compute::picker_glyph_areas;
