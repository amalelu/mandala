//! Glyph-model data types — the render-time shape of a tree node's
//! content. Split into four leaf modules so each concern stays
//! skimmable:
//!
//! - [`component`] — `GlyphComponent` + `GlyphComponentField`: the
//!   leaf text+font+color triplet.
//! - [`line`] — `GlyphLine`: a horizontal run of components with
//!   `overriding_insert` / `expanding_insert` mutation primitives.
//! - [`matrix`] — `GlyphMatrix`: a vertical stack of lines plus the
//!   `place_in` painter that renders into a shared
//!   `String` + `ColorFontRegions` pair.
//! - [`glyph_model`] — `GlyphModel`: the outermost wrapper, carrying
//!   a matrix plus position / layer / hitbox.
//! - [`mutator`] — `GlyphModelField`, `DeltaGlyphModel`,
//!   `GlyphModelCommand`: the mutation surface that reaches every
//!   field of the above, applied via [`Applicable`][crate::core::primitives::Applicable].

pub mod component;
pub mod glyph_model;
pub mod line;
pub mod matrix;
pub mod mutator;

pub use component::{GlyphComponent, GlyphComponentField};
pub use glyph_model::GlyphModel;
pub use line::GlyphLine;
pub(crate) use line::GlyphLineOp;
pub use matrix::GlyphMatrix;
pub use mutator::{
    DeltaGlyphModel, GlyphModelCommand, GlyphModelCommandType, GlyphModelField,
    GlyphModelFieldType,
};
