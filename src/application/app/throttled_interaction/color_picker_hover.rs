//! Throttled interaction for glyph-wheel color-picker hover.
//!
//! Unlike the four drag interactions, this one coexists with other
//! state — the picker is open while the user interacts with nodes
//! and edges underneath it. That's why it lives as a sibling field
//! on `InitState` instead of inside `ThrottledDrag`. The pending
//! discipline is two flags:
//!
//! - `dirty`: something the picker *overlay* draws has changed —
//!   gesture repositioning, HSV cursor move, hovered-cell change.
//!   Every drain rebuilds the overlay.
//! - `canvas_dirty`: the document's `color_picker_preview` changed
//!   (only `apply_picker_preview` writes to it). Gates the canvas
//!   rebuild — gesture-only frames leave this flag clear and skip
//!   the expensive scene walk.
//!
//! Both clear at the end of each drain.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::frame_throttle::MutationFrequencyThrottle;

use super::super::color_picker_flow::rebuild_color_picker_overlay;
use super::super::scene_rebuild::rebuild_scene_only;
use super::{DrainContext, ThrottledInteraction};

/// Hover-update state for the color picker. Two independently-set
/// dirty flags drive the drain's split-rebuild decision — see the
/// module docs.
pub(in crate::application::app) struct ColorPickerHoverInteraction {
    pub dirty: bool,
    pub canvas_dirty: bool,
    pub throttle: MutationFrequencyThrottle,
}

impl ColorPickerHoverInteraction {
    pub(in crate::application::app) fn new() -> Self {
        Self {
            dirty: false,
            canvas_dirty: false,
            throttle: MutationFrequencyThrottle::with_default_budget(),
        }
    }

    /// True iff the document's `color_picker_preview` changed since
    /// the last drain — the sole signal that gates the canvas
    /// rebuild. Gesture frames (Move / Resize) leave this clear.
    /// Nudge keys, swatch hover, and any other path that goes
    /// through `apply_picker_preview` set it.
    ///
    /// Pulled out as a method so the drain's branching predicate is
    /// unit-testable without standing up a full `DrainContext`.
    fn canvas_needs_rebuild(&self) -> bool {
        self.canvas_dirty
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
            scene_cache,
            color_picker_state,
            ..
        } = ctx;

        // If the picker closed between the last event and this
        // drain, drop both dirty flags without doing the rebuild —
        // there's nothing on-screen to update.
        if !color_picker_state.is_open() {
            self.dirty = false;
            self.canvas_dirty = false;
            return;
        }

        // Split the rebuilds: canvas only when something visible on
        // the map actually changed (via `apply_picker_preview`),
        // overlay whenever the picker's own paint changed. Gesture
        // Move / Resize frames leave `canvas_dirty` clear and skip
        // the O(edges) canvas walk; keyboard nudges during a gesture
        // still set `canvas_dirty` through `apply_picker_preview` so
        // the targeted edge's preview color repaints as expected.
        let canvas_needs_rebuild = self.canvas_needs_rebuild();

        if let Some(doc) = document.as_mut() {
            if canvas_needs_rebuild {
                rebuild_scene_only(doc, app_scene, renderer, scene_cache);
            }
            rebuild_color_picker_overlay(color_picker_state, doc, app_scene, renderer);
        }
        self.dirty = false;
        self.canvas_dirty = false;
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
        assert!(!i.canvas_dirty);
        assert_eq!(i.throttle.current_n(), 1);
    }

    #[test]
    fn test_new_equals_default() {
        // `new` is the canonical constructor; `Default` must mirror it
        // so call sites that reach for either land in the same state.
        let a = ColorPickerHoverInteraction::new();
        let b = ColorPickerHoverInteraction::default();
        assert_eq!(a.dirty, b.dirty);
        assert_eq!(a.canvas_dirty, b.canvas_dirty);
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
    fn test_canvas_needs_rebuild_false_by_default() {
        // Fresh interaction: nothing has called `apply_picker_preview`,
        // so the canvas is clean and the drain should skip
        // `rebuild_scene_only`. Baseline for the gesture path — a
        // Move / Resize drag sets only `dirty`, never `canvas_dirty`.
        let i = ColorPickerHoverInteraction::new();
        assert!(!i.canvas_needs_rebuild());
    }

    #[test]
    fn test_canvas_needs_rebuild_when_canvas_dirty_set() {
        // `apply_picker_preview` sets `canvas_dirty = true` — e.g.
        // keyboard nudge keys pressed mid-drag still change the
        // document's color_picker_preview and must not be dropped
        // by the gesture-only gate.
        let mut i = ColorPickerHoverInteraction::new();
        i.canvas_dirty = true;
        assert!(i.canvas_needs_rebuild());
    }

    #[test]
    fn test_canvas_needs_rebuild_independent_of_dirty() {
        // `dirty` drives the overlay rebuild; `canvas_dirty` drives
        // the canvas rebuild. A gesture-only frame sets `dirty`
        // without `canvas_dirty` — regression guard for the original
        // performance fix.
        let mut i = ColorPickerHoverInteraction::new();
        i.dirty = true;
        assert!(!i.canvas_needs_rebuild());
    }
}
