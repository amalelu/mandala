//! Leaf utilities shared across baumhard: small-scale geometry,
//! grapheme-aware string ops, colour math, prime sieve, hashable
//! vectors, and arena-tree helpers. Nothing here depends on the
//! renderer, the GPU, or the mindmap model.

/// Small-scale 2D geometry: pivot rotation, epsilon float compare,
/// pixel-space ordering.
pub mod geometry;
/// Grapheme-cluster aware text primitives — reach for these from
/// the app crate rather than byte-indexing a `String`, per §B3.
pub mod grapheme_chad;
/// Colour-space conversions: hex ↔ RGB ↔ HSV, theme-variable
/// resolution.
pub mod color_conversion;
/// Core `Color` type and arithmetic, plus compile-time
/// colour-literal macros.
pub mod color;
/// Reference palettes — internal seeds and example constants.
pub mod palettes;
/// Arena-wide subtree copy helpers built on `indextree`.
pub mod arena_utils;
/// Hashable, `Eq`-able 2D float vector — each axis wrapped in
/// `OrderedFloat` so instances can key hash maps and ordered sets.
pub mod ordered_vec2;
/// Test bodies exposed through the `pub mod tests;` pattern so
/// `benches/test_bench.rs` can reuse the `do_*()` function bodies as
/// micro-benchmarks. See §B8.
pub mod tests;
/// Lazy Sieve of Eratosthenes — the prime table the region-params
/// grid chooser consults to avoid prime dimension factors.
pub mod primes;