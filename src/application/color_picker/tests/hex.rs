//! Hex readout position coverage: visibility-coupled `Option`,
//! and horizontal centering when visible.

use super::fixtures::{sample_geometry, sample_geometry_with_hex};
use crate::application::color_picker::compute_color_picker_layout;

/// `hex_pos` must be `Some` when geometry declares it visible and
/// `None` otherwise.
#[test]
fn hex_pos_is_some_iff_hex_visible() {
    let invisible = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
    assert!(
        invisible.hex_pos.is_none(),
        "hex_pos must be None when hex_visible=false",
    );
    let visible = compute_color_picker_layout(&sample_geometry_with_hex(), 1280.0, 720.0);
    assert!(
        visible.hex_pos.is_some(),
        "hex_pos must be Some when hex_visible=true",
    );
}

/// When visible, the hex readout must be horizontally centered on
/// the wheel center.
#[test]
fn hex_pos_horizontally_centered_on_wheel_center() {
    let layout = compute_color_picker_layout(&sample_geometry_with_hex(), 1280.0, 720.0);
    let (hx, _) = layout.hex_pos.expect("hex_pos should be Some");
    let hex_width = layout.char_width * 7.0;
    let hex_center_x = hx + hex_width * 0.5;
    assert!(
        (hex_center_x - layout.center.0).abs() < 1.0,
        "hex readout center {hex_center_x} not aligned with wheel center {}",
        layout.center.0,
    );
}
