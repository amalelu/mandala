//! Core data types and animation primitives shared across baumhard.
//!
//! `primitives` defines the model-level vocabulary (ranges, styled
//! regions, apply operations, anchors, flags). `animation` defines
//! the animation timeline + mutator traits.

pub mod animation;
pub mod primitives;
pub mod tests;
