// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::font::metrics`] — the pure-arithmetic
//! `monospace_advance` helper + its underlying calibration
//! constant. The `do_*()` / `test_*()` split is the §B8
//! benchmark-reuse pattern: `do_*` bodies are reachable from
//! `benches/test_bench.rs` so a regression in the multiply
//! (e.g. someone bumps it to a per-face lookup) shows up in
//! `cargo bench`.

use crate::font::metrics::{monospace_advance, MONOSPACE_ADVANCE_RATIO};
use crate::util::geometry::almost_equal;

#[test]
fn test_monospace_advance_scales_linearly() {
    do_monospace_advance_scales_linearly();
}

/// Output is a straight `f32` multiply by [`MONOSPACE_ADVANCE_RATIO`].
/// Three sample points covering integer, integer, and fractional
/// inputs confirm linearity with no rounding surprises within
/// `almost_equal`'s 1e-5 tolerance.
pub fn do_monospace_advance_scales_linearly() {
    assert!(almost_equal(monospace_advance(10.0), 6.0));
    assert!(almost_equal(monospace_advance(20.0), 12.0));
    assert!(almost_equal(
        monospace_advance(33.3),
        33.3 * MONOSPACE_ADVANCE_RATIO,
    ));
}
