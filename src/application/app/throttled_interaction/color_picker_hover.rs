// SPDX-License-Identifier: MPL-2.0

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

use super::super::color_picker_flow::rebuild_color_picker_overlay;
use super::super::scene_rebuild::rebuild_scene_only;
use super::pending::ThrottledPending;
use super::{DrainContext, ThrottledInteraction};

/// Hover-update state for the color picker. Two independently-set
/// dirty flags drive the drain's split-rebuild decision — see the
/// module docs.
pub(in crate::application::app) struct ColorPickerHoverInteraction {
    /// Dirty-flag pending state plus this interaction's adaptive
    /// throttle. Written through [`Self::mark_dirty`] /
    /// [`Self::mark_canvas_dirty`] rather than as bare fields, so
    /// the "canvas dirty implies overlay dirty" pairing cannot be
    /// half-set from a caller.
    pub pending: ThrottledPending,
}

impl ColorPickerHoverInteraction {
    pub(in crate::application::app) fn new() -> Self {
        Self {
            pending: ThrottledPending::dirty_flags(),
        }
    }

    /// Something the picker *overlay* draws has changed — gesture
    /// repositioning, HSV cursor move, hovered-cell change. The next
    /// drain rebuilds the overlay and nothing else.
    pub(in crate::application::app) fn mark_dirty(&mut self) {
        self.pending.mark_dirty();
    }

    /// The document's `color_picker_preview` changed, so the canvas
    /// walk is owed too. Only `apply_picker_preview` gets here.
    pub(in crate::application::app) fn mark_canvas_dirty(&mut self) {
        self.pending.mark_canvas_dirty();
    }

    /// True iff the document's `color_picker_preview` changed since
    /// the last drain — the sole signal that gates the canvas
    /// rebuild. Gesture frames (Move / Resize) leave this clear.
    /// Nudge keys, swatch hover, and any other path that goes
    /// through `apply_picker_preview` set it.
    ///
    /// Pulled out as a method so the drain's branching predicate is
    /// unit-testable without standing up a full `DrainContext`.
    pub(in crate::application::app) fn canvas_needs_rebuild(&self) -> bool {
        self.pending.canvas_is_dirty()
    }

    /// Drop both flags without doing the drain's work. The
    /// picker-closed path in `drain_inputs` uses it to unstick a
    /// throttle-deferred hover drain that has nothing left to
    /// repaint.
    pub(in crate::application::app) fn clear_pending(&mut self) {
        self.pending.clear_flags();
    }
}

impl Default for ColorPickerHoverInteraction {
    fn default() -> Self {
        Self::new()
    }
}

impl ThrottledInteraction for ColorPickerHoverInteraction {
    fn pending(&self) -> &ThrottledPending {
        &self.pending
    }

    fn pending_mut(&mut self) -> &mut ThrottledPending {
        &mut self.pending
    }

    fn drain(&mut self, ctx: DrainContext<'_>) {
        let DrainContext {
            document,
            app_scene,
            renderer,
            scene_cache,
            color_picker_state,
            interaction_mode,
            ..
        } = ctx;

        // If the picker closed between the last event and this
        // drain, drop both dirty flags without doing the rebuild —
        // there's nothing on-screen to update.
        if !color_picker_state.is_open() {
            self.pending.clear_flags();
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

        if canvas_needs_rebuild {
            if let Some(doc) = document.as_mut() {
                rebuild_scene_only(doc, interaction_mode, app_scene, renderer, scene_cache);
            }
        }
        // Deliberately outside the `document` guard, which is a
        // behavior change: the overlay rebuild used to sit inside
        // `if let Some(doc)` and was skipped on a documentless
        // drain. It no longer reads the document at all — the
        // picker's paint comes from `color_picker_state` plus the
        // renderer's surface size — so skipping it there dropped a
        // frame of the picker for no reason. The independence is
        // enforced by the signature, not by this comment:
        // `rebuild_color_picker_overlay` has no document parameter
        // to pass. Unreachable in practice (`startup_load` installs
        // a document before any drain), which is why it is
        // described here rather than covered by a test — driving
        // `drain` needs a live `Renderer`, and TEST_CONVENTIONS §T8
        // / §T10 rule out GPU test infrastructure.
        rebuild_color_picker_overlay(color_picker_state, app_scene, renderer);
        self.pending.clear_flags();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::test_utils::{
        drive_throttle_over_budget, trait_default_tests_for_throttled_interaction,
    };

    #[test]
    fn test_default_is_not_dirty() {
        let i = ColorPickerHoverInteraction::default();
        assert!(!i.has_pending());
        assert!(!i.canvas_needs_rebuild());
        assert_eq!(i.pending.throttle.current_n(), 1);
    }

    #[test]
    fn test_new_equals_default() {
        // `new` is the canonical constructor; `Default` must mirror it
        // so call sites that reach for either land in the same state.
        let a = ColorPickerHoverInteraction::new();
        let b = ColorPickerHoverInteraction::default();
        assert_eq!(a.has_pending(), b.has_pending());
        assert_eq!(a.canvas_needs_rebuild(), b.canvas_needs_rebuild());
        assert_eq!(a.pending.throttle.current_n(), b.pending.throttle.current_n());
    }

    #[test]
    fn test_has_pending_matches_dirty_flag() {
        let mut i = ColorPickerHoverInteraction::new();
        assert!(!i.has_pending());
        i.mark_dirty();
        assert!(i.has_pending());
        i.clear_pending();
        assert!(!i.has_pending());
    }

    #[test]
    fn test_reset_does_not_clear_dirty() {
        // Reset is throttle-only; the dirty flag is cleared inside
        // `drain` (or when the picker closes), not here.
        let mut i = ColorPickerHoverInteraction::new();
        i.mark_dirty();
        drive_throttle_over_budget(&mut i.pending.throttle);
        assert!(i.pending.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.pending.throttle.current_n(), 1);
        assert!(i.has_pending());
    }

    trait_default_tests_for_throttled_interaction! {
        build = ColorPickerHoverInteraction::new,
        set_pending = |i: &mut ColorPickerHoverInteraction| {
            i.mark_dirty();
        },
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
        // `apply_picker_preview` marks the canvas dirty — e.g.
        // keyboard nudge keys pressed mid-drag still change the
        // document's color_picker_preview and must not be dropped
        // by the gesture-only gate.
        let mut i = ColorPickerHoverInteraction::new();
        i.mark_canvas_dirty();
        assert!(i.canvas_needs_rebuild());
    }

    #[test]
    fn test_canvas_needs_rebuild_independent_of_dirty() {
        // `dirty` drives the overlay rebuild; `canvas_dirty` drives
        // the canvas rebuild. A gesture-only frame sets `dirty`
        // without `canvas_dirty` — regression guard for the original
        // performance fix.
        let mut i = ColorPickerHoverInteraction::new();
        i.mark_dirty();
        assert!(!i.canvas_needs_rebuild());
    }

    /// The canvas-dirty mark implies the overlay-dirty mark: every
    /// path that changes the document's preview also changes what
    /// the overlay draws, and a canvas-only mark would leave the
    /// drain gated off entirely (`has_pending` reads `dirty`).
    #[test]
    fn test_marking_the_canvas_dirty_also_flags_the_drain() {
        let mut i = ColorPickerHoverInteraction::new();
        i.mark_canvas_dirty();
        assert!(i.has_pending());
        assert!(i.canvas_needs_rebuild());
    }
}
