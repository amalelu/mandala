//! Spatial bookkeeping used by the gfx tree: a grid-bucket region
//! index for O(1) screen-region lookup, the grid-parameter helper
//! that picks non-prime subdivisions, and the hit-test rectangle bag
//! carried by each `GlyphModel`. These are the primitives the scene
//! and tree walker lean on to turn a screen-space point into a
//! `(SceneTreeId, NodeId)` hit without scanning every element.

pub mod region_indexer;
pub mod regions;
pub mod hitbox;