//! Tests for [`crate::mutator_builder`]. `pub mod` (not
//! `#[cfg(test)] mod`) per §T2.2 so criterion benches can reach the
//! `do_*()` bodies.

pub mod mutator_builder_tests;
