//! Cell-index ↔ HSV-value round trips: hue slot ↔ degrees (incl.
//! wrap-around and rounding), sat/val cell ↔ [0,1] endpoints.

use crate::application::color_picker::{
    hue_slot_to_degrees, sat_cell_to_value, val_cell_to_value, HUE_SLOT_COUNT,
    SAT_CELL_COUNT, VAL_CELL_COUNT,
};

// `degrees_to_hue_slot` is private to the color_picker module — reach
// into `glyph_tables` via `super::super::` (tests live one level
// deeper than the rest of the module tree).
use super::super::glyph_tables::degrees_to_hue_slot;

#[test]
fn hue_slot_to_degrees_round_trip() {
    for slot in 0..HUE_SLOT_COUNT {
        let deg = hue_slot_to_degrees(slot);
        assert_eq!(degrees_to_hue_slot(deg), slot);
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

/// Hue wrap: `degrees_to_hue_slot` must wrap correctly across the
/// 0/360 boundary in both directions.
#[test]
fn degrees_to_hue_slot_wraps_at_boundary() {
    assert_eq!(degrees_to_hue_slot(0.0), 0);
    assert_eq!(degrees_to_hue_slot(360.0), 0);
    assert_eq!(degrees_to_hue_slot(720.0), 0);
    assert_eq!(degrees_to_hue_slot(-15.0), 23);
    assert_eq!(degrees_to_hue_slot(-360.0), 0);
    // 357° rounds to slot 0 (since 24 % 24 = 0).
    assert_eq!(degrees_to_hue_slot(357.0), 0);
    // 352° rounds to slot 23.
    assert_eq!(degrees_to_hue_slot(352.0), 23);
}

/// Quantization: every input degree in `[0, 360)` must fall into
/// the slot whose center is closest. Walk the range in 1° steps
/// and check that no input is more than 7.5° (half a slot) from
/// its resolved slot's canonical degree.
#[test]
fn degrees_to_hue_slot_quantizes_to_nearest() {
    for d in 0..360 {
        let deg = d as f32;
        let slot = degrees_to_hue_slot(deg);
        let canonical = hue_slot_to_degrees(slot);
        let diff = ((deg - canonical).rem_euclid(360.0)).min(
            (canonical - deg).rem_euclid(360.0),
        );
        assert!(diff <= 7.5 + 1e-4,
            "deg {deg} → slot {slot} (canonical {canonical}°) distance {diff} > 7.5");
    }
}

/// Boundary rounding: 7.4° rounds to slot 0 (closer), 7.6° rounds
/// to slot 1.
#[test]
fn degrees_to_hue_slot_mid_slot_rounding() {
    assert_eq!(degrees_to_hue_slot(7.4), 0);
    assert_eq!(degrees_to_hue_slot(7.6), 1);
    assert_eq!(degrees_to_hue_slot(22.4), 1);
    assert_eq!(degrees_to_hue_slot(22.6), 2);
}
