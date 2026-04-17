//! Picker sizing constants, JSON-backed glyph accessors, and the
//! discrete-cell ↔ continuous-HSV helpers that compute_layout /
//! hit_test / picker_glyph_areas all route through.
//!
//! Everything in here is `pub` and cached via `OnceLock` — the
//! accessors hand out `&'static [&'static str]` slices so the render
//! hot path never touches serde or allocates per frame.

use std::sync::OnceLock;

use crate::application::widgets::color_picker_widget::load_spec;

/// Number of hue slots on the outer ring. 24 slots = 15° per step. Fine
/// enough that adjacent slots feel continuous, coarse enough that each
/// glyph has a comfortable hit target.
pub const HUE_SLOT_COUNT: usize = 24;

// =============================================================
// Stable channel scheme for the picker tree
// =============================================================
//
// Every picker `GlyphArea` is appended to the tree at a deterministic
// channel — same channel across every rebuild for the same logical
// cell. Stable channels are what let the `MutatorTree` path target
// "the hue ring slot at index 7" without re-deriving an index from
// tree position. Without stability, swapping the picker's full
// rebuild for a mutator-driven update would silently misalign
// (Baumhard's `align_child_walks` pairs mutator children with target
// children by ascending channel — see `tree_walker.rs:226`).
//
// The channel layout now lives in `widgets/color_picker.json` under
// `mutator_spec.children[*].Repeat` — each `Repeat` declares a
// section name, `channel_base`, and `count`. The declaration order
// of sections in the JSON IS the picker's tree-insertion order; bands
// are 100 wide so a future addition (e.g. an extra ring of glyphs)
// can slot in without renumbering. Reading the layout here means the
// Rust side never has to restate it.
//
// `picker_channel(section, index)` below is the canonical lookup —
// it walks the spec once (cached via OnceLock) and returns the
// channel for a given section + iteration index. Layout-wise: title
// → hue ring → hint → sat bar → val bar → ࿕ preview → hex readout.

/// Resolve the stable channel for the picker's `(section, index)`
/// cell from `widgets/color_picker.json`'s `mutator_spec`. First
/// call walks the spec; subsequent calls hit an `OnceLock` cache.
/// Panics if the section isn't declared — that's a JSON/Rust drift
/// bug, not a recoverable state.
pub fn picker_channel(section: &str, index: usize) -> usize {
    static CACHE: OnceLock<std::collections::HashMap<(String, usize), usize>> = OnceLock::new();
    let map = CACHE.get_or_init(|| {
        use baumhard::mutator_builder::{iter_section_channels, SectionContext};
        struct NoCtx;
        impl SectionContext for NoCtx {}
        let mut out = Vec::new();
        iter_section_channels(&load_spec().mutator_spec, &NoCtx, &mut out);
        out.into_iter()
            .map(|(s, i, c)| ((s, i), c))
            .collect()
    });
    *map.get(&(section.to_string(), index))
        .unwrap_or_else(|| panic!("picker_channel: unknown section/index ({section:?}, {index})"))
}

/// Number of cells on each crosshair bar. Odd so the center cell sits
/// exactly on the bar's midpoint (sat=0.5 / val=0.5). Cell 8 is the
/// wheel center where ࿕ lives — it's counted in the HSV quantization
/// but not rendered as a bar cell.
pub const SAT_CELL_COUNT: usize = 17;
pub const VAL_CELL_COUNT: usize = 17;

/// The center cell index of each 17-cell crosshair bar — the wheel
/// center where ࿕ sits. Skipped during bar rendering so the ࿕ glyph
/// shows through cleanly; still counted in sat/val quantization. Its
/// value doubles as the number of rendered cells on each arm, and is
/// used as the fixed size of the per-glyph ink-offset arrays.
pub const CROSSHAIR_CENTER_CELL: usize = 8;

/// Hue ring font size multiplier over the picker's base font_size.
///
/// Backed by [`color_picker.json`](../widgets/color_picker.json).
/// The function form replaces the old `pub const HUE_RING_FONT_SCALE:
/// f32 = 1.7` — moving the value into the widget spec was the first
/// step of the "widget appearance lives in JSON" migration.
pub fn hue_ring_font_scale() -> f32 {
    load_spec().geometry.hue_ring_font_scale
}

// =============================================================
// Glyph accessors — all read from the JSON widget spec
// =============================================================
//
// The old `pub const HUE_RING_GLYPHS: [&str; 24]` (and its four
// crosshair-arm siblings) moved into `color_picker.json`. Runtime
// callers go through these accessors, which read from the spec and
// cache a leaked `&'static [&'static str]` so the existing
// `[&str]`-shaped call-sites keep working unchanged.

/// Cached `&'static [&'static str]` derived from a Vec<String> in the
/// spec. The spec is itself cached; leaking the per-glyph strings
/// costs one allocation per glyph per process, which is trivial
/// (~32 glyphs total) and avoids spreading `String` ownership
/// through the render hot path.
fn leak_glyphs(v: &[String]) -> &'static [&'static str] {
    let slice: Vec<&'static str> = v
        .iter()
        .map(|s| &*Box::leak(s.clone().into_boxed_str()))
        .collect();
    Box::leak(slice.into_boxed_slice())
}

/// Hue ring sacred-script glyphs, clockwise from 12 o'clock.
/// Backed by `color_picker.json`'s `hue_ring_glyphs`. Three 8-glyph
/// arcs today: Devanagari (top-right), Hebrew (bottom-right),
/// Tibetan (bottom-left → top-left). Each glyph indexes directly
/// into `hue_slot_positions[i]`.
pub fn hue_ring_glyphs() -> &'static [&'static str] {
    static CACHE: OnceLock<&'static [&'static str]> = OnceLock::new();
    CACHE.get_or_init(|| leak_glyphs(&load_spec().hue_ring_glyphs))
}

/// Val bar top arm glyphs (brightest → mid).
pub fn arm_top_glyphs() -> &'static [&'static str] {
    static CACHE: OnceLock<&'static [&'static str]> = OnceLock::new();
    CACHE.get_or_init(|| leak_glyphs(&load_spec().arm_top_glyphs))
}

/// Val bar bottom arm glyphs (mid → darkest). Typically Egyptian
/// hieroglyphs; cosmic-text needs an explicit font hint for these —
/// see [`arm_bottom_font`].
pub fn arm_bottom_glyphs() -> &'static [&'static str] {
    static CACHE: OnceLock<&'static [&'static str]> = OnceLock::new();
    CACHE.get_or_init(|| leak_glyphs(&load_spec().arm_bottom_glyphs))
}

/// Sat bar left arm glyphs (desaturated → mid).
pub fn arm_left_glyphs() -> &'static [&'static str] {
    static CACHE: OnceLock<&'static [&'static str]> = OnceLock::new();
    CACHE.get_or_init(|| leak_glyphs(&load_spec().arm_left_glyphs))
}

/// Sat bar right arm glyphs (mid → saturated).
pub fn arm_right_glyphs() -> &'static [&'static str] {
    static CACHE: OnceLock<&'static [&'static str]> = OnceLock::new();
    CACHE.get_or_init(|| leak_glyphs(&load_spec().arm_right_glyphs))
}

/// Central preview glyph — doubles as the commit button on the ࿕.
pub fn center_preview_glyph() -> &'static str {
    static CACHE: OnceLock<&'static str> = OnceLock::new();
    CACHE.get_or_init(|| {
        Box::leak(load_spec().center_preview_glyph.clone().into_boxed_str())
    })
}

/// Explicit font family the renderer should pin when shaping
/// `arm_bottom_glyphs`. `None` if the spec didn't set one — in which
/// case cosmic-text's default fallback picks a face.
pub fn arm_bottom_font() -> Option<baumhard::font::fonts::AppFont> {
    load_spec().arm_bottom_font
}

// =============================================================
// Discrete cell ↔ continuous HSV value mappings
// =============================================================

/// Convert a hue-ring slot index to its degrees value (0..360).
pub fn hue_slot_to_degrees(slot: usize) -> f32 {
    (slot as f32 / HUE_SLOT_COUNT as f32) * 360.0
}

/// Quantize a degrees value to the nearest hue slot.
pub fn degrees_to_hue_slot(deg: f32) -> usize {
    let normalized = deg.rem_euclid(360.0) / 360.0;
    let slot = (normalized * HUE_SLOT_COUNT as f32).round() as usize;
    slot % HUE_SLOT_COUNT
}

/// Convert a saturation-bar cell index to its `[0, 1]` value.
pub fn sat_cell_to_value(cell: usize) -> f32 {
    cell as f32 / (SAT_CELL_COUNT as f32 - 1.0)
}

/// Convert a value-bar cell index to its `[0, 1]` value. Top of the bar
/// (cell 0) is brightest (val=1.0); bottom is darkest (val=0.0).
pub fn val_cell_to_value(cell: usize) -> f32 {
    1.0 - cell as f32 / (VAL_CELL_COUNT as f32 - 1.0)
}
