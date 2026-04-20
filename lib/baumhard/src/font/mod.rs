//! Cosmic-text font integration — the single blessed boundary
//! between baumhard's styled-region data model and the underlying
//! font system. `fonts` owns the compiled font table and the shared
//! `FONT_SYSTEM`; `attrs` translates `ColorFontRegions` into
//! cosmic-text `AttrsList`s.

pub mod attrs;
pub mod fonts;
pub mod tests;
