//! `PickerDynamicContext` — the slim per-frame `SectionContext`
//! impl that replaces the full `compute_picker_areas` pass on the
//! dynamic mutator path. Computes each requested `GlyphAreaField`
//! directly from `(geometry, layout, section, index)` so no
//! intermediate `GlyphArea` table is allocated per cell per frame.

use std::cell::RefCell;
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
use baumhard::mutator_builder::{CellField, SectionContext};
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
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = CellColor::new(hsv_to_rgb(hue_slot_to_degrees(i), 1.0, 1.0));
        }
        table
    })
}

/// Two-axis HSV bit-pattern key for the per-table caches. `sat_colors`
/// is `hsv_to_rgb(hue, sat_cell_to_value(i), val)` — independent of
/// `geometry.sat`, so its key is `(hue, val)`. `val_colors` mirrors
/// the shape: `hsv_to_rgb(hue, sat, val_cell_to_value(i))`, keyed by
/// `(hue, sat)`. Splitting the key lets a single-axis scrub — the
/// user drags only the sat slider, say — hit the table that doesn't
/// depend on that axis and skip its rebuild entirely. Bit-exact
/// comparison means NaN fails to match itself (forcing a rebuild
/// rather than a corrupt read).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct AxisKey(u32, u32);

struct SatCache {
    /// `(hue, val)` — the axes `sat_colors` depends on.
    key: AxisKey,
    sat_colors: [CellColor; SAT_CELL_COUNT],
}

struct ValCache {
    /// `(hue, sat)` — the axes `val_colors` depends on.
    key: AxisKey,
    val_colors: [CellColor; VAL_CELL_COUNT],
}

thread_local! {
    /// Per-thread sat-bar base-color cache keyed by `(hue, val)`. The
    /// app is single-threaded so this is effectively global; `RefCell`
    /// borrows are short and mutually exclusive (one dynamic apply
    /// at a time). Stays live for the process lifetime so open/close
    /// cycles keep hitting the cache when the relevant axes haven't
    /// moved.
    static SAT_CACHE: RefCell<Option<SatCache>> = const { RefCell::new(None) };

    /// Per-thread val-bar base-color cache keyed by `(hue, sat)`.
    /// Same shape as `SAT_CACHE`; split so a user scrubbing only the
    /// sat axis leaves this table's key unchanged and the rebuild
    /// skips.
    static VAL_CACHE: RefCell<Option<ValCache>> = const { RefCell::new(None) };
}

/// Return the sat-bar base-color table for `geometry`. On hit (same
/// `(hue, val)` as the last call) hands back the cached array. On
/// miss rebuilds it — one `hsv_to_rgb` per live cell (centre slot
/// skipped, matching the layout spec's `skip_indices: [8]`).
fn sat_colors_for(geometry: &ColorPickerOverlayGeometry) -> [CellColor; SAT_CELL_COUNT] {
    let key = AxisKey(geometry.hue_deg.to_bits(), geometry.val.to_bits());
    SAT_CACHE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(cache) = slot.as_ref() {
            if cache.key == key {
                return cache.sat_colors;
            }
        }
        let mut sat_colors = [CellColor::zero(); SAT_CELL_COUNT];
        for (i, s) in sat_colors.iter_mut().enumerate() {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            *s = CellColor::new(hsv_to_rgb(geometry.hue_deg, sat_cell_to_value(i), geometry.val));
        }
        *slot = Some(SatCache { key, sat_colors });
        sat_colors
    })
}

/// Return the val-bar base-color table for `geometry`. Same cache
/// discipline as [`sat_colors_for`] but keyed by `(hue, sat)` — a
/// pure-sat drag hits the cache here even when it missed above.
fn val_colors_for(geometry: &ColorPickerOverlayGeometry) -> [CellColor; VAL_CELL_COUNT] {
    let key = AxisKey(geometry.hue_deg.to_bits(), geometry.sat.to_bits());
    VAL_CACHE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(cache) = slot.as_ref() {
            if cache.key == key {
                return cache.val_colors;
            }
        }
        let mut val_colors = [CellColor::zero(); VAL_CELL_COUNT];
        for (i, v) in val_colors.iter_mut().enumerate() {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            *v = CellColor::new(hsv_to_rgb(geometry.hue_deg, geometry.sat, val_cell_to_value(i)));
        }
        *slot = Some(ValCache { key, val_colors });
        val_colors
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
/// currently-selected sat/val cells, the hex text and its grapheme
/// count). Each `field` call then dispatches
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
    /// non-selected cosmic color and its source RGB. Populated for
    /// every slot including the crosshair center (see `new()`); a
    /// `debug_assert_ne!` in `field()` keeps the center slot off the
    /// hot path.
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

        // Sat-bar / val-bar base-color tables for every live cell.
        // The crosshair centre keeps its `CellColor::zero()` sentinel
        // — the dynamic spec never queries it (pinned by
        // `debug_assert_ne!` in `field()`) and `skip_indices` in the
        // layout spec lists `8`. Two axis-specific caches: sat_colors
        // depends on `(hue, val)`, val_colors depends on `(hue, sat)`,
        // so a user scrubbing only the sat slider leaves the val
        // table's cache key untouched (and vice versa) — doubling
        // the hit rate on single-axis drags vs a single combined key.
        let sat_colors = sat_colors_for(geometry);
        let val_colors = val_colors_for(geometry);

        let spec = load_spec();

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
            PickerSection::Title | PickerSection::Hint => {
                unreachable!("title/hint sections are not built")
            }
            PickerSection::Hex => layout.font_size,
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
            other => {
                // Per CODE_CONVENTIONS §7 the picker overlay is an
                // interactive path and must not abort the process.
                // The dynamic spec is pinned by a test
                // (`spec_dynamic_mutator_spec_per_section_fields_are_slim`)
                // so reaching here means a future spec drift —
                // log loudly, degrade the cell to an empty colour
                // region, and keep the frame alive.
                log::error!(
                    "picker dynamic context received unsupported field {other:?}; \
                     dynamic spec should only list Text / scale / ColorFontRegions / Operation"
                );
                return GlyphAreaField::ColorFontRegions(
                    ColorFontRegions::single_span(0, None, None),
                );
            }
        }

        // Color + grapheme count + optional font pin per section.
        // Every branch reads its base color from a precomputed table
        // and only runs a highlight mix for the at-most-3 cells per
        // apply that are hovered or the currently-selected sat/val
        // cell. No `hsv_to_rgb` calls on this hot loop.
        let g = self.geometry;
        let (count, color, font): (usize, cosmic_text::Color, Option<AppFont>) = match section {
            PickerSection::Title | PickerSection::Hint => {
                unreachable!("title/hint sections are not built")
            }
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

#[cfg(test)]
mod tests {
    //! Axis-split cache coverage for `sat_colors_for` and
    //! `val_colors_for`. These helpers are module-private so the
    //! test module sits inline here rather than in the
    //! `color_picker_overlay/tests/` tree.
    //!
    //! The caches live in thread-local statics; each test runs on
    //! its own thread under `cargo test`, but a single test can
    //! still observe prior state if it's the second call with the
    //! same key. To isolate cache-hit vs cache-miss we deliberately
    //! interpose a known-different HSV between probes so a stale
    //! cache entry can never match the expected output.

    use super::*;
    use crate::application::color_picker::CROSSHAIR_CENTER_CELL;

    fn geom(hue_deg: f32, sat: f32, val: f32) -> ColorPickerOverlayGeometry {
        ColorPickerOverlayGeometry {
            target_label: "edge",
            hue_deg,
            sat,
            val,
            preview_hex: String::new(),
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

    /// The sat-bar base-color cache is keyed on `(hue, val)` — the
    /// `sat` axis must NOT participate in the key, otherwise a
    /// sat-slider scrub would re-populate the cache on every frame
    /// (the perf regression the axis-split was introduced to fix).
    ///
    /// Previous versions of this test asserted that two calls with
    /// different `sat` produced identical output arrays. That's
    /// tautological: `sat_colors_for` never reads `geometry.sat` in
    /// the loop body (it uses `sat_cell_to_value(i)`), so the two
    /// arrays are equal regardless of whether the cache key includes
    /// `sat`. The real invariant is the key's *shape* — assert that
    /// directly by reading the `SAT_CACHE` thread-local after each
    /// call.
    #[test]
    fn sat_cache_key_excludes_sat_axis() {
        let hue = 60.0f32;
        let val = 0.75f32;

        // First call seats the cache with whatever HSV; inspect the
        // stored key to prove it's AxisKey(hue, val) only.
        let _ = sat_colors_for(&geom(hue, 0.25, val));
        let key_after_first = SAT_CACHE.with(|cell| cell.borrow().as_ref().unwrap().key);
        assert_eq!(
            key_after_first,
            AxisKey(hue.to_bits(), val.to_bits()),
            "sat cache key must be (hue, val) — not include sat"
        );

        // Second call with different `sat` but same (hue, val) must
        // not alter the cached key. If the cache were wrongly keyed
        // on (hue, val, sat), the key would now carry the new sat
        // bits and this assertion would fail.
        let _ = sat_colors_for(&geom(hue, 0.90, val));
        let key_after_second = SAT_CACHE.with(|cell| cell.borrow().as_ref().unwrap().key);
        assert_eq!(
            key_after_second, key_after_first,
            "sat-only change must leave the cache key untouched"
        );
    }

    /// Mirror guard for the val-bar cache: keyed on `(hue, sat)`,
    /// `val` axis absent. Same rationale as
    /// [`sat_cache_key_excludes_sat_axis`] — observable arrays don't
    /// depend on `geometry.val` either, so this has to assert the
    /// key shape directly to have any teeth.
    #[test]
    fn val_cache_key_excludes_val_axis() {
        let hue = 200.0f32;
        let sat = 0.4f32;

        let _ = val_colors_for(&geom(hue, sat, 0.1));
        let key_after_first = VAL_CACHE.with(|cell| cell.borrow().as_ref().unwrap().key);
        assert_eq!(
            key_after_first,
            AxisKey(hue.to_bits(), sat.to_bits()),
            "val cache key must be (hue, sat) — not include val"
        );

        let _ = val_colors_for(&geom(hue, sat, 0.9));
        let key_after_second = VAL_CACHE.with(|cell| cell.borrow().as_ref().unwrap().key);
        assert_eq!(
            key_after_second, key_after_first,
            "val-only change must leave the cache key untouched"
        );
    }

    /// Changing `hue_deg` invalidates the sat cache. A hue shift
    /// between two calls must produce a different cell colour at
    /// any saturated cell (cell 0 is sat=0 = pure grey regardless
    /// of hue, so probe the last cell where sat_cell_to_value = 1).
    #[test]
    fn sat_colors_for_invalidates_on_hue_change() {
        let a = sat_colors_for(&geom(0.0, 0.5, 0.5));
        let b = sat_colors_for(&geom(180.0, 0.5, 0.5));
        let probe = SAT_CELL_COUNT - 1;
        assert_ne!(
            a[probe].rgb, b[probe].rgb,
            "hue_deg shift must produce a different sat table at a saturated cell"
        );
    }

    /// The val axis is the one that matters for `sat_colors`
    /// (since sat_colors uses `hsv_to_rgb(hue, sat_cell, val)`).
    /// A val change between two calls must produce a different
    /// table. Guards against the reverse over-caching failure
    /// where the split somehow keyed sat_colors on (hue, sat).
    #[test]
    fn sat_colors_for_invalidates_on_val_change() {
        let a = sat_colors_for(&geom(45.0, 0.5, 0.2));
        let b = sat_colors_for(&geom(45.0, 0.5, 0.9));
        assert_ne!(
            a[0].rgb, b[0].rgb,
            "val shift must produce a different sat table"
        );
    }

    /// Symmetric guard for val_colors: val_colors depends on
    /// `(hue, sat)`, so a sat change must invalidate.
    #[test]
    fn val_colors_for_invalidates_on_sat_change() {
        let a = val_colors_for(&geom(45.0, 0.1, 0.5));
        let b = val_colors_for(&geom(45.0, 0.9, 0.5));
        assert_ne!(
            a[0].rgb, b[0].rgb,
            "sat shift must produce a different val table"
        );
    }

    /// The crosshair centre slot is never populated on the dynamic
    /// path — the layout spec's `skip_indices: [8]` keeps it off the
    /// mutator target set, and a `debug_assert_ne!` in
    /// `PickerDynamicContext::field` pins reads. Verify every
    /// HSV leaves cell 8 at the `CellColor::zero()` sentinel.
    #[test]
    fn crosshair_centre_stays_zero_across_hsv() {
        for (hue, sat, val) in [(0.0, 1.0, 1.0), (90.0, 0.3, 0.7), (270.0, 0.8, 0.2)] {
            let g = geom(hue, sat, val);
            let s = sat_colors_for(&g);
            let v = val_colors_for(&g);
            assert_eq!(s[CROSSHAIR_CENTER_CELL].rgb, [0.0; 3]);
            assert_eq!(v[CROSSHAIR_CENTER_CELL].rgb, [0.0; 3]);
        }
    }

    /// Hue-only change populates the maximally-saturated cell
    /// (SAT_CELL_COUNT - 1 → sat = 1.0) with a different colour
    /// on each call. Cheap regression that any future "cached the
    /// wrong axis" bug will trip. Cell 0 is pure grey (sat=0) so
    /// probe the saturated end.
    #[test]
    fn cache_produces_different_cell_colors_per_hue() {
        let a = sat_colors_for(&geom(0.0, 0.5, 1.0));
        let b = sat_colors_for(&geom(120.0, 0.5, 1.0));
        let c = sat_colors_for(&geom(240.0, 0.5, 1.0));
        let probe = SAT_CELL_COUNT - 1;
        // Three primaries — each must produce a distinct table at
        // the fully-saturated cell.
        assert!(
            a[probe].rgb != b[probe].rgb
                && b[probe].rgb != c[probe].rgb
                && a[probe].rgb != c[probe].rgb
        );
    }
}
