//! Render-side graph structures — the `GfxElement` tree, its
//! mutators, the predicate language that steers walker traversal, and
//! the `Scene` that composes multiple trees into one frame. Mindmap
//! nodes, borders, connections, and overlay UI all compile down to
//! `GfxElement`s living in one of these trees; mutations flow through
//! the walker to reshape them without rebuilding from scratch. The
//! `camera`, `scene`, and `tree_walker` modules sit at the top of this
//! stack; `element`, `model`, and `area` own the data each node holds;
//! `util` provides the spatial indexing and hit-test primitives the
//! walker and scene need.

/// Arena-backed `Tree` of elements and its `MutatorTree` sibling —
/// the mutation-first substrate (§B2).
pub mod tree;
/// Predicate language steering walker traversal — the conditions
/// in `Instruction::RepeatWhile` and siblings.
pub mod predicate;
/// Glyph-model data types: `GlyphModel` wrapping a `GlyphMatrix`
/// of `GlyphLine`s of `GlyphComponent`s.
pub mod model;
/// Field-level delta types for `GlyphArea` — the mutation
/// vocabulary the pipeline uses to target a single facet.
pub mod area_fields;
/// Per-node background / hit-test shape enum shared by the renderer
/// (SDF fragment path) and the BVH hit test (point-in-shape check).
pub mod shape;
/// `Applicable` implementations for `GlyphArea` commands and
/// deltas, dispatched by the tree walker.
pub mod area_mutators;
/// `GlyphArea` — the text-region element variant: text, scale,
/// position, colour-font regions, hit-box.
pub mod area;
/// Walker that aligns a `MutatorTree` against a target `Tree` by
/// channel — the `apply_to` engine.
pub mod tree_walker;
/// `GfxElement` — the tree-node variant (GlyphArea / GlyphModel /
/// Void) plus its field enum, flags, and AABB caching.
pub mod element;
/// `GfxMutator` — the top-level mutator enum (Single / Void /
/// Instruction / Macro) the walker applies.
pub mod mutator;
pub mod tests;
/// `Scene` — composes multiple `Tree`s at per-layer offsets into a
/// single rendered frame.
pub mod scene;
/// Spatial bookkeeping: grid-bucket region index, grid parameters,
/// per-model hit-box bags.
pub mod util;
/// 2D pan/zoom camera — canvas ↔ screen-space projection.
pub mod camera;