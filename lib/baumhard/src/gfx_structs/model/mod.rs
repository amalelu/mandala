//! Glyph-model data types — the render-time shape of a tree node's
//! content. Split into five leaf modules so each concern stays
//! skimmable:
//!
//! - `component` — `GlyphComponent` + `GlyphComponentField`: the
//!   leaf text+font+color triplet.
//! - `line` — `GlyphLine`: a horizontal run of components with
//!   `overriding_insert` / `expanding_insert` mutation primitives.
//! - `matrix` — `GlyphMatrix`: a vertical stack of lines plus the
//!   `place_in` painter that renders into a shared
//!   `String` + `ColorFontRegions` pair.
//! - `glyph_model` — `GlyphModel`: the outermost wrapper, carrying
//!   a matrix plus position / layer / hitbox.
//! - `mutator` — `GlyphModelField`, `DeltaGlyphModel`,
//!   `GlyphModelCommand`: the mutation surface that reaches every
//!   field of the above, applied via
//!   [`Applicable`](crate::core::primitives::Applicable).

/// Leaf of the glyph-model hierarchy — `GlyphComponent`: one
/// contiguous text run sharing a font and colour.
pub mod component;
/// Renderable glyph matrix + positional metadata (layer, origin,
/// hit box). One `GlyphModel` per tree node.
pub mod glyph_model;
/// `GlyphLine` — horizontal run of `GlyphComponent`s forming one
/// visual line, plus the `overriding_insert` / `expanding_insert`
/// mutation primitives.
pub mod line;
/// `GlyphMatrix` — vertical stack of `GlyphLine`s plus `place_in`,
/// the painter that writes into a shared text + regions pair.
pub mod matrix;
/// Glyph-model mutators: `DeltaGlyphModel` (field-level deltas)
/// and `GlyphModelCommand` (high-level commands).
pub mod mutator;

pub use component::{GlyphComponent, GlyphComponentField};
pub use glyph_model::GlyphModel;
pub use line::GlyphLine;
pub use matrix::GlyphMatrix;
pub use mutator::{
    DeltaGlyphModel, GlyphModelCommand, GlyphModelCommandType, GlyphModelField,
    GlyphModelFieldType,
};
