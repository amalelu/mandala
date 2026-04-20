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

pub mod tree;
pub mod predicate;
pub mod model;
pub mod area_fields;
pub mod area_mutators;
pub mod area;
pub mod tree_walker;
pub mod element;
pub mod mutator;
pub mod tests;
pub mod scene;
pub mod util;
pub mod camera;