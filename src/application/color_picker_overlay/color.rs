//! Color helpers for the glyph-wheel color picker.
//!
//! Factored out of `renderer.rs` so the tree / mutator / area builders
//! can share a single source of truth for RGB → cosmic-text conversion
//! and the hover / selected highlight mixes.

/// Convert a normalized `[0, 1]` RGB triple into an opaque
/// `cosmic_text::Color`. Used by the glyph-wheel color picker render
/// path to paint each hue-ring slot, sat/val cell, and preview glyph
/// at its own HSV coordinate without per-frame closure allocation.
#[inline]
pub(super) fn rgb_to_cosmic_color(rgb: [f32; 3]) -> cosmic_text::Color {
    cosmic_text::Color::rgba(
        (rgb[0] * 255.0).round() as u8,
        (rgb[1] * 255.0).round() as u8,
        (rgb[2] * 255.0).round() as u8,
        255,
    )
}

/// Highlight a crosshair-arm cell's color to mark it as "currently
/// selected". The picker used to swap glyphs (■ → ◆) to indicate
/// selection, but with sacred-script glyphs that approach would lose
/// the per-cell script identity. Instead we brighten the cell toward
/// white, which reads as a subtle glow on top of the hue-saturated
/// base color.
#[inline]
pub(super) fn highlight_selected_cell_color(rgb: [f32; 3]) -> cosmic_text::Color {
    // Mix 60% toward white.
    let mix = |c: f32| (c + (1.0 - c) * 0.6).clamp(0.0, 1.0);
    rgb_to_cosmic_color([mix(rgb[0]), mix(rgb[1]), mix(rgb[2])])
}

/// Highlight a cell under the cursor. Distinct from the selected-
/// cell mix (which marks the HSV-current cell) so the hovered + the
/// already-selected cell can both be visually distinguishable — the
/// hovered one reads "whitest" because of the scale bump AND this
/// deeper mix, while the selected one stays subtly glowing behind
/// the hover cursor. A 40% mix toward white is enough to pop against
/// the hue-saturated background but not so saturated that the glyph
/// character becomes hard to read.
#[inline]
pub(super) fn highlight_hovered_cell_color(rgb: [f32; 3]) -> cosmic_text::Color {
    let mix = |c: f32| (c + (1.0 - c) * 0.4).clamp(0.0, 1.0);
    rgb_to_cosmic_color([mix(rgb[0]), mix(rgb[1]), mix(rgb[2])])
}
