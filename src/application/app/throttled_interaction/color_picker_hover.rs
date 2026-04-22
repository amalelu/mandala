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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn drive_throttle_over_budget(t: &mut MutationFrequencyThrottle) -> u32 {
        for _ in 0..80 {
            if t.should_drain() {
                t.record_work_duration(Duration::from_micros(50_000));
            }
        }
        t.current_n()
    }

    #[test]
    fn test_default_is_not_dirty() {
        let i = ColorPickerHoverInteraction::default();
        assert!(!i.dirty);
        assert_eq!(i.throttle.current_n(), 1);
    }

    #[test]
    fn test_new_equals_default() {
        // `new` is the canonical constructor; `Default` must mirror it
        // so call sites that reach for either land in the same state.
        let a = ColorPickerHoverInteraction::new();
        let b = ColorPickerHoverInteraction::default();
        assert_eq!(a.dirty, b.dirty);
        assert_eq!(a.throttle.current_n(), b.throttle.current_n());
    }

    #[test]
    fn test_has_pending_matches_dirty_flag() {
        let mut i = ColorPickerHoverInteraction::new();
        assert!(!i.has_pending());
        i.dirty = true;
        assert!(i.has_pending());
        i.dirty = false;
        assert!(!i.has_pending());
    }

    #[test]
    fn test_reset_does_not_clear_dirty() {
        // Reset is throttle-only; the dirty flag is cleared inside
        // `drain` (or when the picker closes), not here.
        let mut i = ColorPickerHoverInteraction::new();
        i.dirty = true;
        drive_throttle_over_budget(&mut i.throttle);
        assert!(i.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.throttle.current_n(), 1);
        assert!(i.dirty);
    }

    #[test]
    fn test_should_perform_drain_false_when_idle() {
        let mut i = ColorPickerHoverInteraction::new();
        assert!(!i.should_perform_drain());
    }

    #[test]
    fn test_should_perform_drain_true_when_dirty_and_throttle_fresh() {
        let mut i = ColorPickerHoverInteraction::new();
        i.dirty = true;
        assert!(i.should_perform_drain());
    }

    #[test]
    fn test_should_perform_drain_false_when_throttle_skipping() {
        let mut i = ColorPickerHoverInteraction::new();
        drive_throttle_over_budget(&mut i.throttle);
        assert!(i.throttle.current_n() > 1);

        let n = i.throttle.current_n() as usize;
        i.dirty = true;
        let mut saw_skip = false;
        for _ in 0..(n * 2) {
            if !i.should_perform_drain() {
                saw_skip = true;
            }
            i.throttle.record_work_duration(Duration::from_micros(50_000));
            i.dirty = true;
        }
        assert!(saw_skip);
    }

    #[test]
    fn test_idle_should_perform_drain_does_not_advance_throttle() {
        let mut i = ColorPickerHoverInteraction::new();
        for _ in 0..5 {
            assert!(!i.should_perform_drain());
        }
        i.dirty = true;
        assert!(i.should_perform_drain());
    }
}
