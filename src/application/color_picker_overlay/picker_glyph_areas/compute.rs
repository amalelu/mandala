//! Layout-phase dispatcher: walks each section in declaration order,
//! delegating to the per-section builders in [`super::sections`]. The
//! returned [`PickerAreas`] is what the layout-mutator path and the
//! initial-build path consume — both go through this function so they
//! cannot drift.

use baumhard::gfx_structs::area::{GlyphArea, OutlineStyle};

use super::areas::PickerAreas;
use super::sections;
use crate::application::color_picker::{ColorPickerLayout, ColorPickerOverlayGeometry};
use crate::application::widgets::color_picker_widget::load_spec;

/// Compute the full per-section picker area table for the given
/// `(geometry, layout)` state. Both the initial-build path and the
/// mutator path route through this to avoid drift — see the
/// module-level docs.
///
/// **Channel ordering invariant**: `ordered` is channel-ascending.
/// Baumhard's `align_child_walks` pairs mutator children with target
/// children by ascending channel, so the insertion order here and
/// the mutator-builder's traversal order of the spec must agree.
pub(super) fn compute_picker_areas(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> PickerAreas {
    let spec = load_spec();
    let outline = build_outline(spec, layout);
    let mut areas = PickerAreas::new();

    sections::title::build(&mut areas, geometry, layout, outline);
    sections::hue_ring::build(&mut areas, geometry, layout, outline);
    sections::hint::build(&mut areas, geometry, layout, outline);
    sections::sat_bar::build(&mut areas, geometry, layout, outline);
    sections::val_bar::build(&mut areas, geometry, layout, outline);
    sections::preview::build(&mut areas, geometry, layout, outline);
    sections::hex::build(&mut areas, geometry, layout, outline);

    areas
}

/// Outline style for every picker glyph. Sized at the spec's
/// `font_max` baseline and scaled linearly to the actual layout
/// `font_size`. `None` when the spec sets `outline_px = 0`.
pub(super) fn build_outline(
    spec: &crate::application::widgets::color_picker_widget::ColorPickerWidgetSpec,
    layout: &ColorPickerLayout,
) -> Option<OutlineStyle> {
    if spec.geometry.outline_px > 0.0 {
        Some(OutlineStyle {
            color: [0, 0, 0, 255],
            px: spec.geometry.outline_px * (layout.font_size / spec.geometry.font_max),
        })
    } else {
        None
    }
}

/// Backward-compat shim — returns the channel-ordered list for
/// callers that don't need the section-keyed lookup (initial-build
/// path + some existing tests).
#[cfg(test)]
pub(in crate::application::color_picker_overlay) fn picker_glyph_areas(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> Vec<(usize, GlyphArea)> {
    compute_picker_areas(geometry, layout).ordered
}
