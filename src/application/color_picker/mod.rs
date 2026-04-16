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
//! v1 wires the picker to two color-bearing fields whose document setters
//! already exist: `MindEdge.color` (via `set_edge_color`) and
//! `PortalPair.color` (via `set_portal_color`). Node colors and
//! theme-variable editing become a follow-up session.
//!
//! Live preview uses direct in-place model mutation during hover —
//! mirroring `apply_edge_handle_drag` in `app.rs`. The pre-picker snapshot
//! is captured at open time, so cancel restores it without touching the
//! undo stack and commit pushes a single `EditEdge` / `EditPortal` entry.
//!
//! Pure-function layout (`compute_color_picker_layout`) and hit-testing
//! (`hit_test_picker`) are extracted so unit tests don't need a GPU.
//!
//! WASM status: this module compiles on wasm32 (it's pure Rust + data
//! types, no native-only deps), but the `open_*` / `handle_*` entry
//! points in `app.rs` are gated behind `#[cfg(not(target_arch =
//! "wasm32"))]` like the palette and label-edit modals. Picker keyboard
//! / mouse dispatch for WASM is deferred as part of the broader WASM
//! input gap tracked in the roadmap.
//!
//! Split between files:
//! - [`glyph_tables`] — constants, JSON-backed glyph accessors,
//!   HSV↔cell helpers, `picker_channel`.
//! - [`targets`] — `NodeColorAxis` / `ColorTarget` / `PickerHandle` /
//!   `TargetKind`, `current_color_at`, `current_hsv_at`.
//! - [`state`] — `PickerMode`, `PickerGesture`, `ColorPickerState`,
//!   `FlashKind`, `request_error_flash`.
//! - [`geometry`] — `ColorPickerOverlayGeometry` (the pre-render
//!   plain-data struct handed to the renderer).
//! - [`layout`] — `ColorPickerLayout` (pure-function layout output).
//! - [`compute`] / [`compute_sizing`] / [`compute_positions`] — the
//!   three-stage `compute_color_picker_layout` pipeline.
//! - [`clipboard`] — `HandlesCopy` / `HandlesPaste` / `HandlesCut`
//!   implementations on `ColorPickerState`.
//! - [`hit`] — `PickerHit`, `hit_test_picker`.

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
mod tests;

pub use compute::compute_color_picker_layout;
pub use geometry::ColorPickerOverlayGeometry;
pub use glyph_tables::{
    arm_bottom_font, arm_bottom_glyphs, arm_left_glyphs, arm_right_glyphs, arm_top_glyphs,
    center_preview_glyph, hue_ring_font_scale, hue_ring_glyphs, hue_slot_to_degrees,
    picker_channel, sat_cell_to_value, val_cell_to_value, CROSSHAIR_CENTER_CELL,
    HUE_SLOT_COUNT, SAT_CELL_COUNT, VAL_CELL_COUNT,
};
pub use hit::{hit_test_picker, PickerHit};
pub use layout::ColorPickerLayout;
pub use state::{
    request_error_flash, ColorPickerState, FlashKind, PickerDynamicApplyKey, PickerGesture,
    PickerMode,
};
pub use targets::{current_hsv_at, ColorTarget, NodeColorAxis, PickerHandle};
