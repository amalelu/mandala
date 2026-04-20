//! Leaf utilities shared across baumhard: small-scale geometry,
//! grapheme-aware string ops, colour math, prime sieve, hashable
//! vectors, and arena-tree helpers. Nothing here depends on the
//! renderer, the GPU, or the mindmap model.

pub mod geometry;
pub mod grapheme_chad;
pub mod color_conversion;
pub mod color;
pub mod palettes;
pub mod arena_utils;
pub mod ordered_vec2;
pub mod tests;
pub mod primes;