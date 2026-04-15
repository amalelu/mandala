//! `PickerDynamicContext` — the slim per-frame `SectionContext`
//! impl that replaces the full `compute_picker_areas` pass on the
//! dynamic mutator path. Computes each requested `GlyphAreaField`
//! directly from `(geometry, layout, section, index)` so no
//! intermediate `GlyphArea` table is allocated per cell per frame.

use std::sync::OnceLock;

use baumhard::core::primitives::ColorFontRegions;
use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::area::GlyphAreaField;
use baumhard::util::color::{hsv_to_hex, hsv_to_rgb};
use baumhard::util::grapheme_chad::count_grapheme_clusters;

use crate::application::color_picker::{
    arm_bottom_font, hue_slot_to_degrees, sat_cell_to_value, val_cell_to_value, ColorPickerLayout,
    ColorPickerOverlayGeometry, PickerHit, CROSSHAIR_CENTER_CELL, HUE_SLOT_COUNT, SAT_CELL_COUNT,
    VAL_CELL_COUNT,
};
use crate::application::color_picker_overlay::color::{
    highlight_hovered_cell_color, highlight_selected_cell_color, rgb_to_cosmic_color,
};
use crate::application::mutator_builder::{CellField, SectionContext};
use crate::application::widgets::color_picker_widget::load_spec;

use super::areas::PickerSection;

/// Per-section base-color tables precomputed in
/// [`PickerDynamicContext::new`]. Each entry pairs the float RGB
/// (the shape `highlight_hovered_cell_color` wants as input) with the
/// already-quantized `cosmic_text::Color` (for the common non-hovered,
/// non-selected case). `field()` becomes an array-index lookup plus
/// an optional highlight mix for the 1-3 cells per apply that are
/// actually hovered or the currently-selected sat/val cell.
#[derive(Clone, Copy)]
struct CellColor {
    rgb: [f32; 3],
    color: cosmic_text::Color,
}

impl CellColor {
    const fn zero() -> Self {
        Self {
            rgb: [0.0; 3],
            color: cosmic_text::Color(0),
        }
    }

    fn new(rgb: [f32; 3]) -> Self {
        Self {
            rgb,
            color: rgb_to_cosmic_color(rgb),
        }
    }
}

/// Hue-ring cell colors — a pure function of slot index (no HSV
/// state), so we compute them once at first picker use and share the
/// table across every subsequent `PickerDynamicContext::new`. 24
/// entries × 16 bytes = 384 bytes kept warm in the binary's data
/// segment. This is the main cheap-per-apply win for mouse scrubs
/// across the hue ring: hover-only changes no longer do 24 × `hsv_to_rgb`.
static HUE_RING_COLORS: OnceLock<[CellColor; HUE_SLOT_COUNT]> = OnceLock::new();

fn hue_ring_colors() -> &'static [CellColor; HUE_SLOT_COUNT] {
    HUE_RING_COLORS.get_or_init(|| {
        let mut table = [CellColor::zero(); HUE_SLOT_COUNT];
        let mut i = 0;
        while i < HUE_SLOT_COUNT {
            table[i] = CellColor::new(hsv_to_rgb(hue_slot_to_degrees(i), 1.0, 1.0));
            i += 1;
        }
        table
    })
}

/// Slim per-frame context for the picker's dynamic phase. Builds each
/// requested `GlyphAreaField` directly from
/// `(geometry, layout, section, index)` without allocating a full
/// `GlyphArea` per cell.
///
/// Construction captures the handful of derived values that are
/// genuinely shared across cells (preview color in both cosmic and
/// RGB form, the precomputed sat/val base-color tables, the
/// currently-selected sat/val cells, title/hint/hex text grapheme
/// counts, the hex text itself). Each `field` call then dispatches
/// on section name and looks the color up by index — no per-cell
/// `hsv_to_rgb`.
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
    /// (when not commit-hovered).
    preview_color: cosmic_text::Color,
    /// Preview color in float RGB — input to the `highlight_*` mixes
    /// for the preview-commit-hover branch.
    preview_rgb: [f32; 3],
    /// Sat-bar base colors: for each cell `i`, the non-hovered,
    /// non-selected cosmic color and its source RGB. Crosshair center
    /// cell (`CROSSHAIR_CENTER_CELL`) is zero — the dynamic spec
    /// never queries it.
    sat_colors: [CellColor; SAT_CELL_COUNT],
    /// Val-bar base colors — same layout as `sat_colors`.
    val_colors: [CellColor; VAL_CELL_COUNT],
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

        // Precompute sat-bar base colors. Skip the crosshair center
        // cell (the dynamic spec never queries it — see the
        // `debug_assert_ne!` guard in `field()`).
        let mut sat_colors = [CellColor::zero(); SAT_CELL_COUNT];
        for i in 0..SAT_CELL_COUNT {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            sat_colors[i] =
                CellColor::new(hsv_to_rgb(geometry.hue_deg, sat_cell_to_value(i), geometry.val));
        }
        // Precompute val-bar base colors. Same skip-center rule.
        let mut val_colors = [CellColor::zero(); VAL_CELL_COUNT];
        for i in 0..VAL_CELL_COUNT {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            val_colors[i] =
                CellColor::new(hsv_to_rgb(geometry.hue_deg, geometry.sat, val_cell_to_value(i)));
        }

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
            sat_colors,
            val_colors,
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
    ///
    /// Converts the `&str` section name through [`PickerSection::from_name`]
    /// so the exhaustive enum match below stays total — unknown-section
    /// panics are centralised in `from_name`.
    fn scale_for(&self, section: PickerSection, index: usize) -> f32 {
        let g = self.geometry;
        let layout = self.layout;
        let hover = |matches_hover: bool| if matches_hover { self.hover_scale } else { 1.0 };
        match section {
            PickerSection::Title | PickerSection::Hex => layout.font_size,
            PickerSection::Hint => layout.font_size * 0.85,
            PickerSection::HueRing => {
                let hovered = matches!(g.hovered_hit, Some(PickerHit::Hue(h)) if h == index);
                layout.ring_font_size * hover(hovered)
            }
            PickerSection::SatBar => {
                let hovered = matches!(g.hovered_hit, Some(PickerHit::SatCell(h)) if h == index);
                layout.cell_font_size * hover(hovered)
            }
            PickerSection::ValBar => {
                let hovered = matches!(g.hovered_hit, Some(PickerHit::ValCell(h)) if h == index);
                layout.cell_font_size * hover(hovered)
            }
            PickerSection::Preview => {
                let commit_hovered = matches!(g.hovered_hit, Some(PickerHit::Commit));
                layout.preview_size * hover(commit_hovered)
            }
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
        // Resolve the section string to the typed enum up front so
        // every `match` below is exhaustive on the enum rather than
        // repeating the unknown-section panic per branch. Invalid
        // section names surface at this single call — JSON / Rust
        // drift is a programming error, not a recoverable state, and
        // `from_name` is the canonical place to fail loudly.
        let section = PickerSection::from_name(section);
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
                debug_assert!(
                    matches!(section, PickerSection::Hex),
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
        // Every branch reads its base color from a precomputed table
        // and only runs a highlight mix for the at-most-3 cells per
        // apply that are hovered or the currently-selected sat/val
        // cell. No `hsv_to_rgb` calls on this hot loop.
        let g = self.geometry;
        let (count, color, font): (usize, cosmic_text::Color, Option<AppFont>) = match section {
            PickerSection::Title => (self.title_count, self.preview_color, None),
            PickerSection::Hint => (self.hint_count, self.preview_color, None),
            PickerSection::Hex => (self.hex_count, self.preview_color, None),
            PickerSection::HueRing => {
                let entry = hue_ring_colors()[index];
                let hovered = matches!(g.hovered_hit, Some(PickerHit::Hue(h)) if h == index);
                let color = if hovered {
                    highlight_hovered_cell_color(entry.rgb)
                } else {
                    entry.color
                };
                (1, color, None)
            }
            PickerSection::SatBar => {
                debug_assert_ne!(index, CROSSHAIR_CENTER_CELL);
                let entry = self.sat_colors[index];
                let hovered = matches!(g.hovered_hit, Some(PickerHit::SatCell(h)) if h == index);
                let color = if hovered {
                    highlight_hovered_cell_color(entry.rgb)
                } else if index == self.current_sat_cell {
                    highlight_selected_cell_color(entry.rgb)
                } else {
                    entry.color
                };
                (1, color, None)
            }
            PickerSection::ValBar => {
                debug_assert_ne!(index, CROSSHAIR_CENTER_CELL);
                let entry = self.val_colors[index];
                let hovered = matches!(g.hovered_hit, Some(PickerHit::ValCell(h)) if h == index);
                let color = if hovered {
                    highlight_hovered_cell_color(entry.rgb)
                } else if index == self.current_val_cell {
                    highlight_selected_cell_color(entry.rgb)
                } else {
                    entry.color
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
            PickerSection::Preview => {
                let commit_hovered = matches!(g.hovered_hit, Some(PickerHit::Commit));
                let color = if commit_hovered {
                    highlight_hovered_cell_color(self.preview_rgb)
                } else {
                    self.preview_color
                };
                (1, color, Some(AppFont::NotoSerifTibetanRegular))
            }
        };

        GlyphAreaField::ColorFontRegions(ColorFontRegions::single_span(
            count,
            Some(cosmic_to_rgba(color)),
            font,
        ))
    }
}
