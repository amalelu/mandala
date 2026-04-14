//! Test suite for the color picker overlay. Split by concern so each
//! file stays focused on one promise the picker has to honour.
//!
//! - [`fixtures`] — shared test geometry + helper accessors.
//! - [`build_shape`] — invariants of the initial-build path
//!   (preview centering, channel ordering, paired GlyphModel children).
//! - [`mutator_round_trip`] — layout-mutator and full round-trip
//!   between a fresh build and an in-place apply.
//! - [`dynamic_compose`] — composition of layout-then-dynamic apply.

mod build_shape;
mod dynamic_compose;
mod fixtures;
mod mutator_round_trip;
