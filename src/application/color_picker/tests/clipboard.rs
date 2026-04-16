//! Clipboard trait tests for `ColorPickerState`.

use crate::application::color_picker::{
    ColorPickerState, PickerMode, CROSSHAIR_CENTER_CELL,
};
use crate::application::console::traits::{
    ClipboardContent, HandlesCopy, HandlesCut, HandlesPaste, Outcome,
};

/// Build a `ColorPickerState::Open` in Standalone mode with given HSV.
/// Only the fields the clipboard traits read/write are meaningful;
/// the rest are zero/None stubs that let the enum construct.
fn make_standalone_open(hue: f32, sat: f32, val: f32) -> ColorPickerState {
    ColorPickerState::Open {
        mode: PickerMode::Standalone,
        hue_deg: hue,
        sat,
        val,
        last_cursor_pos: None,
        max_cell_advance: 16.0,
        max_ring_advance: 24.0,
        measurement_font_size: 16.0,
        arm_top_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
        arm_bottom_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
        arm_left_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
        arm_right_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
        preview_ink_offset: (0.0, 0.0),
        layout: None,
        center_override: None,
        size_scale: 1.0,
        gesture: None,
        hovered_hit: None,
        hover_preview: None,
        pending_error_flash: false,
        last_dynamic_apply: None,
    }
}

#[test]
fn copy_produces_hex_when_open() {
    let state = make_standalone_open(0.0, 1.0, 1.0);
    match state.clipboard_copy() {
        ClipboardContent::Text(hex) => {
            assert!(hex.starts_with('#'), "expected hex string, got {hex}");
            assert_eq!(hex.len(), 7, "expected #RRGGBB format");
        }
        other => panic!("expected Text, got {other:?}"),
    }
}

#[test]
fn copy_returns_not_applicable_when_closed() {
    let state = ColorPickerState::Closed;
    assert_eq!(state.clipboard_copy(), ClipboardContent::NotApplicable);
}

#[test]
fn paste_valid_hex_sets_hsv() {
    let mut state = make_standalone_open(0.0, 0.0, 0.0);
    let result = state.clipboard_paste("#ff0000");
    assert_eq!(result, Outcome::Applied);

    // After pasting red, hue should be near 0, sat and val near 1.
    if let ColorPickerState::Open {
        hue_deg, sat, val, ..
    } = &state
    {
        assert!((*hue_deg - 0.0).abs() < 1.0, "hue should be ~0 for red, got {hue_deg}");
        assert!(*sat > 0.9, "sat should be ~1 for red, got {sat}");
        assert!(*val > 0.9, "val should be ~1 for red, got {val}");
    } else {
        panic!("state should still be Open after paste");
    }
}

#[test]
fn paste_valid_hex_with_whitespace() {
    let mut state = make_standalone_open(0.0, 0.0, 0.0);
    let result = state.clipboard_paste("  #00ff00  ");
    assert_eq!(result, Outcome::Applied);
}

#[test]
fn paste_invalid_string_returns_invalid() {
    let mut state = make_standalone_open(0.0, 1.0, 1.0);
    let result = state.clipboard_paste("not a color");
    assert!(matches!(result, Outcome::Invalid(_)));
}

#[test]
fn paste_returns_not_applicable_when_closed() {
    let mut state = ColorPickerState::Closed;
    assert_eq!(state.clipboard_paste("#ff0000"), Outcome::NotApplicable);
}

#[test]
fn paste_same_color_returns_unchanged() {
    // Open with pure red (hue=0, sat=1, val=1), paste #ff0000 which
    // is the same color — should report Unchanged.
    let mut state = make_standalone_open(0.0, 1.0, 1.0);
    // First paste to set to a known hex-round-tripped state.
    state.clipboard_paste("#ff0000");
    let result = state.clipboard_paste("#ff0000");
    assert_eq!(result, Outcome::Unchanged);
}

#[test]
fn cut_equals_copy() {
    let mut state = make_standalone_open(120.0, 0.8, 0.6);
    let copy_result = state.clipboard_copy();
    let cut_result = state.clipboard_cut();
    assert_eq!(copy_result, cut_result);
}

#[test]
fn cut_returns_not_applicable_when_closed() {
    let mut state = ColorPickerState::Closed;
    assert_eq!(state.clipboard_cut(), ClipboardContent::NotApplicable);
}
