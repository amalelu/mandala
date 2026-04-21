//! Test modules for [`crate::gfx_structs`]. Each module covers one
//! conceptual area; see individual module docs for scope.
//!
//! Declared `pub mod` (not `#[cfg(test)] mod`) per §T2.2 so the
//! criterion bench harness at `lib/baumhard/benches/test_bench.rs`
//! can reach the `do_*()` bodies. Removing that gate is load-bearing
//! — see TEST_CONVENTIONS.md §T2.2 and §B8.

pub mod area_tests;
pub mod zoom_visibility_tests;
pub mod shape_tests;
pub mod model_tests;
pub mod mutator_tests;
pub mod tree_tests;
pub mod tree_walker_tests;
pub mod region_indexer_tests;
pub mod region_params_tests;
pub mod region_rect_tests;
pub mod scene_tests;
pub mod camera_tests;
pub mod element_tests;
pub mod predicate_tests;
pub mod subtree_aabb_tests;
pub mod bvh_descent_tests;
pub mod spatial_descend_tests;
pub mod map_children_tests;
