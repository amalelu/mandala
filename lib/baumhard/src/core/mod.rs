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
/// Test bodies exposed through the `pub mod tests;` pattern so
/// `benches/test_bench.rs` can reuse them as micro-benchmarks. See §B8.
pub mod tests;
