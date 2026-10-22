// SPDX-License-Identifier: MPL-2.0

//! Core data types shared across baumhard.

/// Ranges, styled regions, `ApplyOperation`, anchors, flags, and the
/// `Applicable` trait.
pub mod primitives;
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
