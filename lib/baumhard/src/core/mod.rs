//! Core data types and animation primitives shared across baumhard.
//!
//! `primitives` defines the model-level vocabulary (ranges, styled
//! regions, apply operations, anchors, flags). `animation` defines
//! the animation timeline + mutator traits.

/// Animation timeline + mutator traits — the vocabulary for
/// sequencing time-varying changes over a tree.
pub mod animation;
/// Core data types every higher-level abstraction rests on: ranges,
/// styled regions, `ApplyOperation`, anchors, flags, and the
/// `Applicable` trait.
pub mod primitives;
pub mod tests;
