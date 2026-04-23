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

use crate::application::color_picker::ColorPickerState;
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

/// Gate for the canvas rebuild inside [`ColorPickerHoverInteraction::drain`].
///
/// Returns `true` when a picker-drag gesture (Move / Resize) is
/// active — those frames mutate only picker-local geometry and
/// leave document state untouched, so the canvas rebuild is
/// redundant. Returns `false` for the hover-preview path, where a
/// swatch change fans out to the document's color_picker_preview
/// and the canvas must be rebuilt to reflect the new preview.
///
/// Pulled out of `drain` so the shape of the decision is
/// independently testable without standing up a full `DrainContext`
/// (renderer, app_scene, scene_cache, …).
fn canvas_rebuild_is_redundant(state: &ColorPickerState) -> bool {
    matches!(
        state,
        ColorPickerState::Open { gesture: Some(_), .. },
    )
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
            scene_cache,
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

        // Gesture-active (Move / Resize) drains are picker-overlay
        // only — `color_picker_flow::mouse` writes just
        // `center_override` / `size_scale` on those frames and never
        // touches document state, so a canvas rebuild is pure waste.
        // The drag drains (MovingNode, EdgeHandle, …) already avoid
        // this path by walking their own targeted rebuild; the picker
        // is the one throttled consumer that was still paying
        // O(edges) for a screen-space overlay translation.
        let skip_canvas = canvas_rebuild_is_redundant(color_picker_state);

        if let Some(doc) = document.as_mut() {
            if !skip_canvas {
                rebuild_scene_only(doc, app_scene, renderer, scene_cache);
            }
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

    #[test]
    fn test_canvas_rebuild_redundant_for_closed_state() {
        // Closed: nothing on-screen to rebuild for. Drain returns
        // early before checking the gate, but the gate itself should
        // report "redundant" for safety.
        let state = ColorPickerState::Closed;
        assert!(!canvas_rebuild_is_redundant(&state));
    }

    #[test]
    fn test_canvas_rebuild_not_redundant_for_hover_open_state() {
        // Open with no active gesture → hover preview. The swatch
        // under the cursor changes the document's color_picker_preview,
        // so the canvas must be rebuilt to show the preview color.
        use crate::application::color_picker::{ColorPickerState, PickerMode};
        let state = ColorPickerState::Open {
            mode: PickerMode::Standalone,
            hue_deg: 0.0,
            sat: 1.0,
            val: 1.0,
            last_cursor_pos: None,
            max_cell_advance: 1.0,
            max_ring_advance: 1.0,
            measurement_font_size: 10.0,
            arm_top_ink_offsets: Default::default(),
            arm_bottom_ink_offsets: Default::default(),
            arm_left_ink_offsets: Default::default(),
            arm_right_ink_offsets: Default::default(),
            preview_ink_offset: (0.0, 0.0),
            layout: None,
            center_override: None,
            size_scale: 1.0,
            gesture: None,
            hovered_hit: None,
            hover_preview: None,
            pending_error_flash: false,
            last_dynamic_apply: None,
        };
        assert!(!canvas_rebuild_is_redundant(&state));
    }

    #[test]
    fn test_canvas_rebuild_redundant_during_move_gesture() {
        // Move gesture active: only `center_override` changes each
        // frame; no document-facing mutation. Canvas rebuild is
        // pure waste — the gate must report redundant.
        use crate::application::color_picker::{
            ColorPickerState, PickerGesture, PickerMode,
        };
        let state = ColorPickerState::Open {
            mode: PickerMode::Standalone,
            hue_deg: 0.0,
            sat: 1.0,
            val: 1.0,
            last_cursor_pos: None,
            max_cell_advance: 1.0,
            max_ring_advance: 1.0,
            measurement_font_size: 10.0,
            arm_top_ink_offsets: Default::default(),
            arm_bottom_ink_offsets: Default::default(),
            arm_left_ink_offsets: Default::default(),
            arm_right_ink_offsets: Default::default(),
            preview_ink_offset: (0.0, 0.0),
            layout: None,
            center_override: None,
            size_scale: 1.0,
            gesture: Some(PickerGesture::Move {
                grab_offset: (0.0, 0.0),
            }),
            hovered_hit: None,
            hover_preview: None,
            pending_error_flash: false,
            last_dynamic_apply: None,
        };
        assert!(canvas_rebuild_is_redundant(&state));
    }

    #[test]
    fn test_canvas_rebuild_redundant_during_resize_gesture() {
        // Resize gesture: only `size_scale` changes each frame.
        // Same rationale as Move — skip the canvas rebuild.
        use crate::application::color_picker::{
            ColorPickerState, PickerGesture, PickerMode,
        };
        let state = ColorPickerState::Open {
            mode: PickerMode::Standalone,
            hue_deg: 0.0,
            sat: 1.0,
            val: 1.0,
            last_cursor_pos: None,
            max_cell_advance: 1.0,
            max_ring_advance: 1.0,
            measurement_font_size: 10.0,
            arm_top_ink_offsets: Default::default(),
            arm_bottom_ink_offsets: Default::default(),
            arm_left_ink_offsets: Default::default(),
            arm_right_ink_offsets: Default::default(),
            preview_ink_offset: (0.0, 0.0),
            layout: None,
            center_override: None,
            size_scale: 1.0,
            gesture: Some(PickerGesture::Resize {
                anchor_radius: 1.0,
                anchor_scale: 1.0,
                anchor_center: (0.0, 0.0),
            }),
            hovered_hit: None,
            hover_preview: None,
            pending_error_flash: false,
            last_dynamic_apply: None,
        };
        assert!(canvas_rebuild_is_redundant(&state));
    }
}
