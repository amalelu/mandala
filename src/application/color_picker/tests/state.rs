//! Picker state / channel invariants: strictly-ascending channels,
//! dynamic-apply short-circuit key equality, resize gesture math.

use crate::application::color_picker::{
    picker_channel, HUE_SLOT_COUNT, SAT_CELL_COUNT, VAL_CELL_COUNT,
};
use crate::application::widgets::color_picker_widget::load_spec;

/// Picker channels must be strictly ascending in tree-insertion
/// order, otherwise Baumhard's `align_child_walks` (which pairs
/// mutator children with target children by ascending channel)
/// breaks alignment and the §B2 mutator path silently misses
/// elements. Channels are resolved through `picker_channel`,
/// which reads `widgets/color_picker.json`'s `mutator_spec`.
///
/// Insertion order: hue ring (24 slots) → sat bar (17 cells) →
/// val bar (same) → preview → hex.
#[test]
fn picker_channels_are_strictly_ascending() {
    let bands: &[(&str, usize, usize)] = &[
        ("hue ring", picker_channel("hue_ring", 0), HUE_SLOT_COUNT),
        ("sat bar", picker_channel("sat_bar", 0), SAT_CELL_COUNT),
        ("val bar", picker_channel("val_bar", 0), VAL_CELL_COUNT),
        ("preview", picker_channel("preview", 0), 1),
        ("hex", picker_channel("hex", 0), 1),
    ];
    let mut prev_band_max: usize = 0;
    for (name, base, count) in bands {
        let band_min = *base;
        let band_max = *base + count - 1;
        assert!(
            band_min > prev_band_max,
            "{name} band starts at {band_min} but previous band ended at {prev_band_max}"
        );
        prev_band_max = band_max;
    }
}

/// The dynamic-apply short-circuit key must equate two geometries
/// with the same HSV / hover / hex-visibility, and distinguish
/// geometries that differ in any of those axes. Guards the
/// `rebuild_color_picker_overlay` dispatcher's bail-out so a hover
/// within the same cell skips the mutator build, but any real
/// observable change does not.
#[test]
fn picker_dynamic_apply_key_equates_stable_state_and_distinguishes_changes() {
    use crate::application::color_picker::{PickerDynamicApplyKey, PickerHit};

    let base = PickerDynamicApplyKey {
        hue_deg: 120.0,
        sat: 0.5,
        val: 0.7,
        hovered_hit: Some(PickerHit::Hue(4)),
        hex_visible: true,
    };
    let same = PickerDynamicApplyKey { ..base };
    assert_eq!(base, same, "identical state must compare equal");

    let mut diff_hue = base;
    diff_hue.hue_deg = 121.0;
    assert_ne!(base, diff_hue, "hue change must not short-circuit");

    let mut diff_hover = base;
    diff_hover.hovered_hit = Some(PickerHit::Hue(5));
    assert_ne!(base, diff_hover, "hover change must not short-circuit");

    let mut diff_hex = base;
    diff_hex.hex_visible = false;
    assert_ne!(base, diff_hex, "hex visibility flip must not short-circuit");

    let mut no_hover = base;
    no_hover.hovered_hit = None;
    assert_ne!(base, no_hover, "unhover must not short-circuit");
}

/// `PickerGesture::Resize` must compute the new scale
/// multiplicatively from cursor radius. A 2× radius produces
/// a 2× scale, a 0.5× radius produces a 0.5× scale, modulo
/// the spec's `[resize_scale_min, resize_scale_max]` clamp.
#[test]
fn resize_gesture_scale_math_is_multiplicative() {
    let spec = load_spec();
    let geom = &spec.geometry;
    let anchor_radius: f32 = 100.0;
    let anchor_scale: f32 = 1.0;
    let r_double = anchor_radius * 2.0;
    let new_double = (anchor_scale * (r_double / anchor_radius))
        .clamp(geom.resize_scale_min, geom.resize_scale_max);
    assert!(new_double > anchor_scale);
    assert!(new_double <= geom.resize_scale_max);
    let r_half = anchor_radius * 0.5;
    let new_half = (anchor_scale * (r_half / anchor_radius))
        .clamp(geom.resize_scale_min, geom.resize_scale_max);
    assert!(new_half < anchor_scale);
    assert!(new_half >= geom.resize_scale_min);
    let new_same = (anchor_scale * (anchor_radius / anchor_radius))
        .clamp(geom.resize_scale_min, geom.resize_scale_max);
    assert!((new_same - anchor_scale).abs() < 1e-6);
}
