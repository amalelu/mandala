//! Throttled interaction for glyph-wheel color-picker hover.
//!
//! Unlike the four drag interactions, this one coexists with other
//! state — the picker is open while the user interacts with nodes
//! and edges underneath it. That's why it lives as a sibling field
//! on `InitState` instead of inside `ThrottledDrag`. The pending
//! discipline is a `dirty` flag set by the picker's own input
//! paths (mouse-move inside the wheel, chip focus); each drain
//! rebuilds the scene + picker overlay once per N frames and
//! clears the flag.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::frame_throttle::MutationFrequencyThrottle;

use super::super::color_picker_flow::rebuild_color_picker_overlay;
use super::super::scene_rebuild::rebuild_scene_only;
use super::{DrainContext, ThrottledInteraction};

/// Hover-update state for the color picker. `dirty` is set by
/// [`super::super::color_picker_flow::handle_color_picker_mouse_move`]
/// whenever HSV state changes; drained once per adaptive window by
/// [`ThrottledInteraction::drive`].
pub(in crate::application::app) struct ColorPickerHoverInteraction {
    pub dirty: bool,
    pub throttle: MutationFrequencyThrottle,
}

impl ColorPickerHoverInteraction {
    pub(in crate::application::app) fn new() -> Self {
        Self {
            dirty: false,
            throttle: MutationFrequencyThrottle::with_default_budget(),
        }
    }
}

impl Default for ColorPickerHoverInteraction {
    fn default() -> Self {
        Self::new()
    }
}

impl ThrottledInteraction for ColorPickerHoverInteraction {
    fn has_pending(&self) -> bool {
        self.dirty
    }

    fn throttle(&mut self) -> &mut MutationFrequencyThrottle {
        &mut self.throttle
    }

    fn drain(&mut self, ctx: DrainContext<'_>) {
        let DrainContext {
            document,
            app_scene,
            renderer,
            color_picker_state,
            ..
        } = ctx;

        // If the picker closed between the last event and this
        // drain, drop the dirty flag without doing the rebuild —
        // there's nothing on-screen to update.
        if !color_picker_state.is_open() {
            self.dirty = false;
            return;
        }

        if let Some(doc) = document.as_mut() {
            rebuild_scene_only(doc, app_scene, renderer);
            rebuild_color_picker_overlay(color_picker_state, doc, app_scene, renderer);
        }
        self.dirty = false;
    }
}
