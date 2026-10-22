// SPDX-License-Identifier: MPL-2.0

//! Cell-index → HSV-value mappings: hue slot → degrees, sat/val
//! cell → `[0, 1]` endpoints.
//!
//! There is no hue-degrees → slot direction to round-trip against.
//! The hue ring paints each of its 24 cells at its own fixed hue and
//! has no "selected slot" highlight, so unlike the sat and val bars
//! (whose highlights drive `sat_value_to_cell` / `val_value_to_cell`)
//! it never needs to quantize a continuous hue back to a cell. The
//! inverse `degrees_to_hue_slot` existed and was tested here without
//! any production caller; it went in #41.

use crate::application::color_picker::{
    hue_slot_to_degrees, sat_cell_to_value, val_cell_to_value, HUE_SLOT_COUNT, SAT_CELL_COUNT, VAL_CELL_COUNT,
};

/// Every slot maps into `[0, 360)` at an even 15° spacing, in
/// ascending order — the property the ring's per-cell coloring
/// depends on.
#[test]
fn hue_slot_to_degrees_spans_the_circle_evenly() {
    let step = 360.0 / HUE_SLOT_COUNT as f32;
    for slot in 0..HUE_SLOT_COUNT {
        let deg = hue_slot_to_degrees(slot);
        assert!((0.0..360.0).contains(&deg), "slot {slot} → {deg}° out of range");
        assert!(
            (deg - slot as f32 * step).abs() < 1e-3,
            "slot {slot} → {deg}°, expected {}°",
            slot as f32 * step
        );
    }
}

#[test]
fn sat_and_val_cell_value_endpoints() {
    assert!((sat_cell_to_value(0) - 0.0).abs() < 1e-6);
    assert!((sat_cell_to_value(SAT_CELL_COUNT - 1) - 1.0).abs() < 1e-6);
    // Val bar inverted: top cell = brightest.
    assert!((val_cell_to_value(0) - 1.0).abs() < 1e-6);
    assert!((val_cell_to_value(VAL_CELL_COUNT - 1) - 0.0).abs() < 1e-6);
}
