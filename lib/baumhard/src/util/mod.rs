// SPDX-License-Identifier: MPL-2.0

//! Leaf utilities shared across baumhard: small-scale geometry,
//! grapheme-aware string ops, color math, prime sieve, hashable
//! vectors, and arena-tree helpers. Nothing here depends on the
//! renderer, the GPU, or the mindmap model.

/// Arena-wide subtree copy helpers built on `indextree`.
pub mod arena_utils;
/// The §B8 bench-surface contract, checked: every `pub fn do_*()`
/// in a tests tree has an entry in `benches/test_bench.rs`.
/// Test-only and native-only for the same reasons `source_scan` is.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod bench_surface;
/// Core `Color` type and channel-wise arithmetic.
pub mod color;
/// Hex ↔ RGB ↔ HSV plus theme-variable resolution.
pub mod color_conversion;
/// The `cargo` commands this repository publishes, held against
/// the workspace's real manifests — a documented `--lib` names a
/// member that has one, a documented `-p` names a member that
/// exists, and a `--benches` anywhere in the tree selects a member
/// that owns a bench target instead of compiling nothing (#148).
/// Test-only and native-only for the same reasons `manifests`,
/// whose member list it reuses, is.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod doc_commands;
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
/// since `log` is the universal Rust facade. On native, `capture`
/// collects the `warn!` / `error!` records one stretch of code emits.
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
/// Lazy Sieve of Eratosthenes — the prime table the region-params
/// grid chooser consults to avoid prime dimension factors.
pub mod primes;
/// Comment-free, test-module-free reads of this workspace's Rust
/// source, for the pins no runtime assertion can make — a log
/// statement whose sink the suite does not install, a `wasm32`-only
/// branch `cargo test` never links. Native-only in effect; not
/// `cfg(test)`-gated because its callers live in `mandala`.
#[cfg(not(target_arch = "wasm32"))]
pub mod rust_source;
/// Reachability walk over baumhard's own source, so a test can
/// enumerate the types a deserializer may be handed instead of
/// restating them in a list that drifts. Test-only: `syn` is a
/// dev-dependency and a shipped build has no business carrying a
/// parser for its own source.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod serde_coverage;
/// The same question `serde_coverage` answers, asked of the
/// **generated** `Deserialize` impls instead of the source text that
/// produced them — so the published positional-array list has
/// something that can disagree with it. Test-only; unlike its
/// counterpart it reads no files and needs no `syn`, and it is gated
/// native-only only because every caller is.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod serde_probe;
/// Readers over the workspace's own source text — which bytes are
/// shipped code, and whether a `test_*` name a comment writes down is
/// one that exists. The single place `#[cfg(test)]` is reasoned
/// about. Test-only and native-only, for the same reasons
/// `serde_coverage` is.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod source_scan;
/// A recording `log::Log` sink, so a test can assert that a degrade
/// path emitted the `warn!` CODE_CONVENTIONS §9 promises.
/// Native-only — the browser build installs `console_log`; not
/// `cfg(test)`-gated because its callers live in `mandala` too,
/// the same reason `rust_source` and `test_temp` are not.
#[cfg(not(target_arch = "wasm32"))]
pub mod test_logger;
/// Collision-free scratch directories for filesystem tests, so
/// concurrent `cargo test` runs cannot race on a shared path.
/// Native-only — there is no filesystem to scratch on under wasm32.
#[cfg(not(target_arch = "wasm32"))]
pub mod test_temp;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
/// `#[allow(clippy::unwrap_used)]` because this is a tests tree
/// (§T2.2): it carries no `cfg(test)` gate — the criterion harness
/// imports its bodies — so the crate-root `warn(clippy::unwrap_used)`
/// sees it as shipped code. `unwrap()` in a test is the correct
/// spelling; the gate that matters for shipped code is
/// `util::unwrap_posture`, which excludes these trees by path.
#[allow(clippy::unwrap_used)]
pub mod tests;
/// CODE_CONVENTIONS §9's closing sentence, checked: no line a
/// shipped build compiles calls bare `unwrap()`. Test-only and
/// native-only for the same reasons `source_scan`, whose walkers it
/// is built on, is.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod unwrap_posture;
