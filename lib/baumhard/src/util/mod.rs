// SPDX-License-Identifier: MPL-2.0

//! Leaf utilities shared across baumhard: small-scale geometry,
//! grapheme-aware string ops, color math, prime sieve, hashable
//! vectors, and arena-tree helpers. Nothing here depends on the
//! renderer, the GPU, or the mindmap model.

/// Arena-wide subtree copy helpers built on `indextree`.
pub mod arena_utils;
/// Core `Color` type, arithmetic, and compile-time color-literal
/// macros.
pub mod color;
/// Hex ↔ RGB ↔ HSV plus theme-variable resolution.
pub mod color_conversion;
/// Readers that pull published examples straight out of the
/// `format/` specs, so a doc pin tests the doc rather than a copy of
/// it. Native-only — the specs live on the filesystem.
#[cfg(not(target_arch = "wasm32"))]
pub mod doc_fixtures;
/// Small-scale 2D geometry: pivot rotation, epsilon compare,
/// pixel-space ordering.
pub mod geometry;
/// Grapheme-cluster aware text primitives — reach for these from
/// the app crate rather than byte-indexing a `String` (§B3).
pub mod grapheme_chad;
/// Logger initialization — `init()` selects the right backend per
/// target. Macro callsites keep using `log::warn!` etc. directly,
/// since `log` is the universal Rust facade.
pub mod log;
/// Reader over the workspace's own `Cargo.toml` files, so the
/// "one version, declared once in `[workspace.dependencies]`" rule
/// is enforced instead of merely written down. Test-only and
/// native-only — nothing in a shipped build parses manifests.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod manifests;
/// Hashable, `Eq`-able 2D float vector (each axis wrapped in
/// `OrderedFloat`).
pub mod ordered_vec2;
/// Reference palettes — internal seeds and example constants.
pub mod palettes;
/// Lazy Sieve of Eratosthenes — the prime table the region-params
/// grid chooser consults to avoid prime dimension factors.
pub mod primes;
/// Reachability walk over baumhard's own source, so a test can
/// enumerate the types a deserializer may be handed instead of
/// restating them in a list that drifts. Test-only: `syn` is a
/// dev-dependency and a shipped build has no business carrying a
/// parser for its own source.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod serde_coverage;
/// Collision-free scratch directories for filesystem tests, so
/// concurrent `cargo test` runs cannot race on a shared path.
/// Native-only — there is no filesystem to scratch on under wasm32.
#[cfg(not(target_arch = "wasm32"))]
pub mod test_temp;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
pub mod tests;
