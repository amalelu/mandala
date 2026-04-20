//! Spatial bookkeeping used by the gfx tree: a grid-bucket region
//! index for O(1) screen-region lookup, the grid-parameter helper
//! that picks non-prime subdivisions, and the hit-test rectangle bag
//! carried by each `GlyphModel`. These are the primitives the scene
//! and tree walker lean on to turn a screen-space point into a
//! `(SceneTreeId, NodeId)` hit without scanning every element.

/// `RegionIndexer` — grid-bucket spatial index delivering O(1)
/// per-bucket lookup of "which elements occupy this screen
/// region".
pub mod region_indexer;
/// `RegionParams` — grid-parameter helper that picks non-prime
/// subdivisions of a pixel resolution for `RegionIndexer`; also
/// re-exports the indexer and its companion types.
pub mod regions;
/// `HitBox` — hit-test bounding-rectangle bag carried on every
/// `GlyphModel` (supports wrapped-line nodes with one rect per
/// visual line).
pub mod hitbox;