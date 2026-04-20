//! Cosmic-text font integration — the single blessed boundary
//! between baumhard's styled-region data model and the underlying
//! font system. `fonts` owns the compiled font table and the shared
//! `FONT_SYSTEM`; `attrs` translates `ColorFontRegions` into
//! cosmic-text `AttrsList`s.

pub mod attrs;
pub mod fonts;
pub mod tests;

/// Packed-RGBA colour value (`u32` internally) in the shape
/// cosmic-text expects when building `Attrs`. Re-exported from
/// `cosmic_text::Color` so consumers outside the renderer can
/// reach a single blessed type without importing `cosmic_text`
/// directly (§1 keeps cosmic-text out of the app crate except
/// through the `baumhard::font` seam). Construct with
/// [`Color::rgba`].
pub use cosmic_text::Color;

/// Glyph-rasterization cache that `measure_glyph_ink_bounds` fills
/// in on demand. Owned by the caller (one per picker-open pass, not
/// one per glyph) so repeated measurements against the same glyphs
/// share rasterization work. Re-exported from `cosmic_text::SwashCache`
/// alongside [`Color`] so the app crate can construct one without
/// importing `cosmic_text` directly.
pub use cosmic_text::SwashCache;
