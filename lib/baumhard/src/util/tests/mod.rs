// SPDX-License-Identifier: MPL-2.0

pub mod arena_utils_tests;
pub mod color_tests;
pub mod geometry_tests;
pub mod grapheme_chad_tests;
pub mod ordered_vec2_tests;
pub mod primes_test;
// Follows `util::rust_source`'s own gate: it reads files off the
// filesystem, which wasm32 does not have.
#[cfg(not(target_arch = "wasm32"))]
pub mod rust_source_tests;
