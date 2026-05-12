// SPDX-License-Identifier: MPL-2.0

//! Magical glyph-wheel color picker — a custom modal overlay for choosing
//! colors that fits Mandala's "everything is a positioned glyph" aesthetic.
//!
//! Layout: a 24-glyph hue ring forming a mandala, a crosshair sat/value
//! selector formed by two perpendicular glyph bars meeting at the wheel
//! center, a central preview glyph showing the currently-previewed color,
//! and a row of theme-variable quick-pick chips below. Mouse hover updates
//! the preview live; click commits, click outside cancels. Keyboard
//! fallback: h/H nudges hue, s/S sat, v/V value, Tab cycles chips, Enter
//! commits, Esc cancels.
//!
//! The picker wires to `MindEdge.color` (via `set_edge_color`) and the
//! three node colour axes. Portal-mode edges flow through the same
//! `set_edge_color` sink as line-mode edges — portals are a render mode
//! on the same entity, so there's no separate portal setter. Node
//! colors and theme-variable editing become a follow-up session.
//!
//! Live preview uses direct in-place model mutation during hover —
//! mirroring `apply_edge_handle_drag` in `app.rs`. The pre-picker
//! snapshot is captured at open time, so cancel restores it without
//! touching the undo stack and commit pushes a single `EditEdge` entry.
//!
//! Pure-function layout (`compute_color_picker_layout`) and hit-testing
//! (`hit_test_picker`) are extracted so unit tests don't need a GPU.

mod clipboard;
mod compute;
mod compute_positions;
mod compute_sizing;
mod geometry;
mod glyph_tables;
mod hit;
mod layout;
mod state;
mod targets;

#[cfg(test)]
pub(in crate::application) mod tests;

// Cross-platform surface — consumed by the picker widget on both
// native and WASM (the spec-driven dynamic-context tree builder is
// cross-platform).
pub use geometry::ColorPickerOverlayGeometry;
pub use glyph_tables::{
    arm_bottom_font, arm_bottom_glyphs, arm_left_glyphs, arm_right_glyphs, arm_top_glyphs,
    center_preview_glyph, hue_ring_glyphs, hue_slot_to_degrees, picker_channel, sat_cell_to_value,
    sat_value_to_cell, val_cell_to_value, val_value_to_cell, CROSSHAIR_CENTER_CELL, HUE_SLOT_COUNT,
    SAT_CELL_COUNT, VAL_CELL_COUNT,
};
pub use hit::PickerHit;
pub use layout::ColorPickerLayout;
pub use targets::{ColorTarget, NodeColorAxis, SectionColorAxis};

// Native-only surface — consumed exclusively by `app/color_picker_flow/`
// (mouse / open / commit / rebuild) and `run_native.rs`. WASM has
// no inline color-picker modal yet, so the symbols below have no
// reachable consumer there.
#[cfg(not(target_arch = "wasm32"))]
pub use compute::compute_color_picker_layout;
#[cfg(not(target_arch = "wasm32"))]
pub use glyph_tables::hue_ring_font_scale;
#[cfg(not(target_arch = "wasm32"))]
pub use hit::hit_test_picker;
#[cfg(not(target_arch = "wasm32"))]
pub use state::{
    request_error_flash, ColorPickerState, FlashKind, PickerDynamicApplyKey, PickerGesture, PickerMode,
};
#[cfg(not(target_arch = "wasm32"))]
pub use targets::{current_color_at, current_hsv_at, PickerHandle};
