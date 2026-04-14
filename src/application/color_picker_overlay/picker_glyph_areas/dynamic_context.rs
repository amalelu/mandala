//! `PickerDynamicContext` — the slim per-frame `SectionContext`
//! impl that replaces the full `compute_picker_areas` pass on the
//! dynamic mutator path. Computes each requested `GlyphAreaField`
//! directly from `(geometry, layout, section, index)` so no
//! intermediate `GlyphArea` table is allocated per cell per frame.

use baumhard::core::primitives::ColorFontRegions;
use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::area::GlyphAreaField;
use baumhard::util::color::{hsv_to_hex, hsv_to_rgb};
use baumhard::util::grapheme_chad::count_grapheme_clusters;

use crate::application::color_picker::{
    arm_bottom_font, hue_slot_to_degrees, sat_cell_to_value, val_cell_to_value, ColorPickerLayout,
    ColorPickerOverlayGeometry, PickerHit, CROSSHAIR_CENTER_CELL, SAT_CELL_COUNT, VAL_CELL_COUNT,
};
use crate::application::color_picker_overlay::color::{
    highlight_hovered_cell_color, highlight_selected_cell_color, rgb_to_cosmic_color,
};
use crate::application::mutator_builder::{CellField, SectionContext};
use crate::application::widgets::color_picker_widget::load_spec;

/// Slim per-frame context for the picker's dynamic phase. Builds each
/// requested `GlyphAreaField` directly from
/// `(geometry, layout, section, index)` without allocating a full
/// `GlyphArea` per cell.
///
/// Construction captures the handful of derived values that are
/// genuinely shared across cells (preview color in both cosmic and
/// RGB form, currently-selected sat/val cells, title/hint/hex text
/// grapheme counts, the hex text itself). Each `field` call then
/// dispatches on section name to produce just the requested field.
///
/// Panics on unsupported `CellField` variants — the dynamic spec is
/// declared in JSON and pinned by
/// `spec_dynamic_mutator_spec_per_section_fields_are_slim`, so any
/// drift surfaces there rather than as a silent no-op.
pub(super) struct PickerDynamicContext<'a> {
    geometry: &'a ColorPickerOverlayGeometry,
    /// Pre-computed layout threaded in from the dispatcher — holds
    /// per-font-size scales (`font_size`, `ring_font_size`,
    /// `cell_font_size`, `preview_size`) the dynamic mutator's `scale`
    /// field reads. Borrowed because the layout lives on
    /// `ColorPickerState::Open.layout` for the same frame.
    layout: &'a ColorPickerLayout,
    /// Preview color in cosmic form — reused by title/hint/hex/preview
    /// (when not commit-hovered). Going through cosmic preserves the
    /// u8 quantization round-trip the layout path bakes via
    /// `rgb_to_cosmic_color`, so regions produced here compare equal
    /// to regions produced by a fresh `compute_picker_areas` pass.
    preview_color: cosmic_text::Color,
    /// Preview color in float RGB — input to the `highlight_*` mixes
    /// for the preview-commit-hover branch.
    preview_rgb: [f32; 3],
    /// Currently-selected sat-bar cell (based on `geometry.sat`).
    /// Cells at this index render with the selected-cell highlight
    /// unless hovered — matches the layout-phase logic.
    current_sat_cell: usize,
    /// Currently-selected val-bar cell (based on `geometry.val`).
    current_val_cell: usize,
    /// Hover scale multiplier from the picker spec (typically 1.3×).
    /// Applied to the hovered cell's `scale` and the preview when
    /// commit-hovered.
    hover_scale: f32,
    /// Grapheme-cluster count of the title text set by the layout
    /// phase. Required so `ColorFontRegions::single_span` covers
    /// exactly the same glyph range the target area currently holds.
    title_count: usize,
    /// Grapheme-cluster count of the hint footer text.
    hint_count: usize,
    /// Current hex readout text (e.g. `#4af0a1`). Empty when the hex
    /// is invisible — `single_span(0, ...)` produces an empty region,
    /// same observable state as the layout path.
    hex_text: String,
    /// Grapheme-cluster count of `hex_text`.
    hex_count: usize,
}

impl<'a> PickerDynamicContext<'a> {
    pub(super) fn new(
        geometry: &'a ColorPickerOverlayGeometry,
        layout: &'a ColorPickerLayout,
    ) -> Self {
        let preview_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, geometry.val);
        let preview_color = rgb_to_cosmic_color(preview_rgb);

        let current_sat_cell = (geometry.sat * (SAT_CELL_COUNT as f32 - 1.0))
            .round()
            .clamp(0.0, (SAT_CELL_COUNT - 1) as f32) as usize;
        let current_val_cell = ((1.0 - geometry.val) * (VAL_CELL_COUNT as f32 - 1.0))
            .round()
            .clamp(0.0, (VAL_CELL_COUNT - 1) as f32) as usize;

        let spec = load_spec();
        let is_standalone = geometry.target_label.is_empty();
        // Title / hint grapheme counts mirror the strings the layout
        // phase writes into the target areas — we never produce the
        // strings ourselves on the dynamic path (the dynamic spec
        // doesn't include `Text` for these sections), only their
        // length so `single_span` covers the same range.
        let title_len = if is_standalone {
            count_grapheme_clusters(&spec.title_template_standalone)
        } else {
            count_grapheme_clusters(
                &spec
                    .title_template_contextual
                    .replace("{target_label}", geometry.target_label),
            )
        };
        let hint_len = if is_standalone {
            count_grapheme_clusters(&spec.hint_text_standalone)
        } else {
            count_grapheme_clusters(&spec.hint_text_contextual)
        };

        let hex_text = if geometry.hex_visible {
            hsv_to_hex(geometry.hue_deg, geometry.sat, geometry.val)
        } else {
            String::new()
        };
        let hex_count = count_grapheme_clusters(&hex_text);

        Self {
            geometry,
            layout,
            preview_color,
            preview_rgb,
            current_sat_cell,
            current_val_cell,
            hover_scale: spec.geometry.hover_scale,
            title_count: title_len,
            hint_count: hint_len,
            hex_text,
            hex_count,
        }
    }

    /// Cell font-size for the `(section, index)` cell — the value the
    /// layout phase feeds to `GlyphArea::new_with_str`'s `scale`
    /// parameter (first float). Mirrors `picker_glyph_areas::make_area`
    /// call-sites. Used by the dynamic spec's `scale` field.
    fn scale_for(&self, section: &str, index: usize) -> f32 {
        let g = self.geometry;
        let layout = self.layout;
        let hover = |matches_hover: bool| if matches_hover { self.hover_scale } else { 1.0 };
        match section {
            "title" => layout.font_size,
            "hint" => layout.font_size * 0.85,
            "hex" => layout.font_size,
            "hue_ring" => {
                let hovered = matches!(g.hovered_hit, Some(PickerHit::Hue(h)) if h == index);
                layout.ring_font_size * hover(hovered)
            }
            "sat_bar" => {
                let hovered = matches!(g.hovered_hit, Some(PickerHit::SatCell(h)) if h == index);
                layout.cell_font_size * hover(hovered)
            }
            "val_bar" => {
                let hovered = matches!(g.hovered_hit, Some(PickerHit::ValCell(h)) if h == index);
                layout.cell_font_size * hover(hovered)
            }
            "preview" => {
                let commit_hovered = matches!(g.hovered_hit, Some(PickerHit::Commit));
                layout.preview_size * hover(commit_hovered)
            }
            other => panic!("picker dynamic context scale_for: unknown section {other:?}"),
        }
    }
}

/// Convert a `cosmic_text::Color` to `[f32; 4]` in `[0, 1]` — same
/// shape as `make_area`'s float-region input so the two paths
/// produce bit-identical region values.
#[inline]
fn cosmic_to_rgba(color: cosmic_text::Color) -> [f32; 4] {
    [
        color.r() as f32 / 255.0,
        color.g() as f32 / 255.0,
        color.b() as f32 / 255.0,
        color.a() as f32 / 255.0,
    ]
}

impl<'a> SectionContext for PickerDynamicContext<'a> {
    fn field(&self, section: &str, index: usize, template: &CellField) -> GlyphAreaField {
        match template {
            CellField::Operation(op) => return GlyphAreaField::Operation(*op),
            // `area.scale` is the final font size the cell renders
            // at — not a 1.0-anchored multiplier. The layout path
            // bakes it as `base_font_size * (hover_scale if hovered
            // else 1.0)` via `GlyphArea::new_with_str`'s first float
            // argument (the baumhard parameter is literally named
            // `scale`). Mirror that exactly so fresh and
            // mutator-applied trees round-trip equal.
            CellField::scale => return GlyphAreaField::scale(self.scale_for(section, index)),
            CellField::Text => {
                debug_assert_eq!(
                    section, "hex",
                    "dynamic spec only writes Text on the hex section"
                );
                return GlyphAreaField::Text(self.hex_text.clone());
            }
            CellField::ColorFontRegions => {}
            other => panic!(
                "picker dynamic context does not produce field {other:?}; \
                 dynamic spec should only list Text / scale / ColorFontRegions / Operation"
            ),
        }

        // Color + grapheme count + optional font pin per section.
        let g = self.geometry;
        let (count, color, font): (usize, cosmic_text::Color, Option<AppFont>) = match section {
            "title" => (self.title_count, self.preview_color, None),
            "hint" => (self.hint_count, self.preview_color, None),
            "hex" => (self.hex_count, self.preview_color, None),
            "hue_ring" => {
                let hue = hue_slot_to_degrees(index);
                let rgb = hsv_to_rgb(hue, 1.0, 1.0);
                let hovered = matches!(g.hovered_hit, Some(PickerHit::Hue(h)) if h == index);
                let color = if hovered {
                    highlight_hovered_cell_color(rgb)
                } else {
                    rgb_to_cosmic_color(rgb)
                };
                (1, color, None)
            }
            "sat_bar" => {
                debug_assert_ne!(index, CROSSHAIR_CENTER_CELL);
                let cell_sat = sat_cell_to_value(index);
                let rgb = hsv_to_rgb(g.hue_deg, cell_sat, g.val);
                let hovered = matches!(g.hovered_hit, Some(PickerHit::SatCell(h)) if h == index);
                let color = if hovered {
                    highlight_hovered_cell_color(rgb)
                } else if index == self.current_sat_cell {
                    highlight_selected_cell_color(rgb)
                } else {
                    rgb_to_cosmic_color(rgb)
                };
                (1, color, None)
            }
            "val_bar" => {
                debug_assert_ne!(index, CROSSHAIR_CENTER_CELL);
                let cell_val = val_cell_to_value(index);
                let rgb = hsv_to_rgb(g.hue_deg, g.sat, cell_val);
                let hovered = matches!(g.hovered_hit, Some(PickerHit::ValCell(h)) if h == index);
                let color = if hovered {
                    highlight_hovered_cell_color(rgb)
                } else if index == self.current_val_cell {
                    highlight_selected_cell_color(rgb)
                } else {
                    rgb_to_cosmic_color(rgb)
                };
                // Top arm uses the default font fallback; bottom arm
                // pins `arm_bottom_font()` (Egyptian hieroglyphs need
                // an explicit face to avoid tofu). Mirrors the
                // layout-path `make_area` font argument.
                let font = if index < CROSSHAIR_CENTER_CELL {
                    None
                } else {
                    arm_bottom_font()
                };
                (1, color, font)
            }
            "preview" => {
                let commit_hovered = matches!(g.hovered_hit, Some(PickerHit::Commit));
                let color = if commit_hovered {
                    highlight_hovered_cell_color(self.preview_rgb)
                } else {
                    self.preview_color
                };
                (1, color, Some(AppFont::NotoSerifTibetanRegular))
            }
            other => panic!("picker dynamic context: unknown section {other:?}"),
        };

        GlyphAreaField::ColorFontRegions(ColorFontRegions::single_span(
            count,
            Some(cosmic_to_rgba(color)),
            font,
        ))
    }
}
