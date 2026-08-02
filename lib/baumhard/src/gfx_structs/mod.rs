// SPDX-License-Identifier: MPL-2.0

//! Render-side graph structures — the `GfxElement` tree, its
//! mutators, the predicate language that steers walker traversal,
//! and the `Scene` that composes multiple trees into one frame.
//! Mindmap nodes, borders, connections, and overlay UI all compile
//! down to `GfxElement`s; mutations flow through the walker to
//! reshape them without rebuilding from scratch.

/// `GlyphArea` — text-region element variant: text, scale,
/// position, colour-font regions, hit-box.
pub mod area;
/// Field-level delta types for `GlyphArea` — the mutation
/// vocabulary that targets a single facet.
pub mod area_fields;
/// `Applicable` impls for `GlyphArea` commands and deltas,
/// dispatched by the tree walker.
pub mod area_mutators;
/// 2D pan/zoom camera — canvas ↔ screen-space projection.
pub mod camera;
/// `Delta` — the field-set container and `DeltaField` vocabulary
/// trait shared by the area-side and model-side mutation surfaces.
pub mod delta;
/// `GfxElement` — tree-node variant (`GlyphArea` / `GlyphModel` /
/// `Void`) plus its field enum, flags, and AABB caching.
pub mod element;
/// `GlyphModel` wrapping a `GlyphMatrix` of `GlyphLine`s of
/// `GlyphComponent`s.
pub mod model;
/// `GfxMutator` — top-level mutator enum (`Single` / `Void` /
/// `Instruction` / `Macro`) the walker applies.
pub mod mutator;
/// Predicate language steering walker traversal (the conditions in
/// `Instruction::RepeatWhile` and siblings).
pub mod predicate;
/// `Scene` — composes multiple `Tree`s at per-layer offsets into a
/// single rendered frame.
pub mod scene;
/// Per-node background / hit-test shape enum shared by the
/// renderer (SDF) and the BVH hit test.
pub mod shape;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
pub mod tests;
/// Arena-backed `Tree` plus `MutatorTree` — the mutation-first
/// substrate (§B2).
pub mod tree;
/// Walker that aligns a `MutatorTree` against a target `Tree` by
/// channel — the `apply_to` engine.
pub mod tree_walker;
/// Spatial bookkeeping: grid-bucket region index, grid parameters,
/// per-model hit-box bags.
pub mod util;
/// `ZoomVisibility` — per-`GlyphArea` lower/upper bound on
/// `camera.zoom` controlling whether the element renders. Orthogonal
/// to the connection / portal font-size clamps that reshape *size*
/// with zoom.
pub mod zoom_visibility;
