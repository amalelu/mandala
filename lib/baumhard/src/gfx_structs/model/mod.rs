// SPDX-License-Identifier: MPL-2.0

//! Glyph-model data types — the render-time shape of a tree node's
//! content. `GlyphModel` wraps a `GlyphMatrix` of `GlyphLine`s of
//! `GlyphComponent`s; the matrix's `place_in` paints them into a
//! shared text + regions pair. Mutations land via `DeltaGlyphModel`
//! / `GlyphModelCommand` through
//! [`Applicable`](crate::core::primitives::Applicable).

/// `GlyphComponent` — leaf of the glyph-model hierarchy: one
/// contiguous text run sharing a font and colour.
pub mod component;
/// `GlyphModel` — outermost wrapper carrying a matrix plus
/// position / layer / hitbox.
pub mod glyph_model;
/// `GlyphLine` — horizontal run of `GlyphComponent`s plus the
/// `overriding_insert` / `expanding_insert` mutation primitives.
pub mod line;
/// `GlyphMatrix` — vertical stack of `GlyphLine`s plus `place_in`,
/// the painter that writes into a shared text + regions pair.
pub mod matrix;
/// `DeltaGlyphModel` (field-level deltas) and `GlyphModelCommand`
/// (high-level commands).
pub mod mutator;

pub use component::GlyphComponent;
pub use glyph_model::GlyphModel;
pub use line::GlyphLine;
pub use matrix::GlyphMatrix;
pub use mutator::{
    DeltaGlyphModel, GlyphModelCommand, GlyphModelCommandType, GlyphModelField, GlyphModelFieldType,
};
