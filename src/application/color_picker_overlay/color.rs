//! RGB → cosmic-text conversion and highlight mixes shared by the
//! picker's tree / mutator / area builders.

/// Convert a normalized `[0, 1]` RGB triple into an opaque
/// `cosmic_text::Color`. Used by the glyph-wheel color picker render
/// path to paint each hue-ring slot, sat/val cell, and preview glyph
/// at its own HSV coordinate without per-frame closure allocation.
///
/// Clamps each channel before the `* 255.0` cast — `as u8` wraps on
/// out-of-range floats, so `baumhard::util::color::convert_f32_to_u8`
/// isn't a drop-in replacement for this path.
#[inline]
pub(super) fn rgb_to_cosmic_color(rgb: [f32; 3]) -> cosmic_text::Color {
    let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    cosmic_text::Color::rgba(to_u8(rgb[0]), to_u8(rgb[1]), to_u8(rgb[2]), 255)
}

/// Linear mix of `rgb` toward white by `t` ∈ `[0, 1]`. `t = 0` is the
/// input untouched; `t = 1` is pure white. Shared by the picker's
/// hover / selected highlight mixes so the two differ only in the
/// mix constant — the UI choice — not in the math.
#[inline]
fn mix_toward_white(rgb: [f32; 3], t: f32) -> [f32; 3] {
    let mix = |c: f32| (c + (1.0 - c) * t).clamp(0.0, 1.0);
    [mix(rgb[0]), mix(rgb[1]), mix(rgb[2])]
}

/// Highlight a crosshair-arm cell's color to mark it as "currently
/// selected". The picker used to swap glyphs (■ → ◆) to indicate
/// selection, but with sacred-script glyphs that approach would lose
/// the per-cell script identity. Instead we brighten the cell 60%
/// toward white, which reads as a subtle glow on top of the
/// hue-saturated base color.
#[inline]
pub(super) fn highlight_selected_cell_color(rgb: [f32; 3]) -> cosmic_text::Color {
    rgb_to_cosmic_color(mix_toward_white(rgb, 0.6))
}

/// Highlight a cell under the cursor. Distinct from the selected-
/// cell mix (which marks the HSV-current cell) so the hovered + the
/// already-selected cell can both be visually distinguishable — the
/// hovered one reads "whitest" because of the scale bump AND a
/// deeper mix, while the selected one stays subtly glowing behind
/// the hover cursor. A 40% mix toward white is enough to pop against
/// the hue-saturated background but not so saturated that the glyph
/// character becomes hard to read.
#[inline]
pub(super) fn highlight_hovered_cell_color(rgb: [f32; 3]) -> cosmic_text::Color {
    rgb_to_cosmic_color(mix_toward_white(rgb, 0.4))
}
