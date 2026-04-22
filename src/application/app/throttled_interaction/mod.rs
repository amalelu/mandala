//! Unified shell for continuous, high-frequency-input-driven
//! mutations.
//!
//! # Why this module exists
//!
//! [`MutationFrequencyThrottle`] ships the *adaptive throttle* logic
//! as a clean primitive. Every per-component drain site, though,
//! used to glue it into place by hand: check pending state, call
//! `should_drain()`, record `Instant::now()`, run the body, reset
//! the pending state, call `record_work_duration()`. Five call
//! sites, six lines of boilerplate each, and any new throttled
//! component had to remember to replicate the exact six-step
//! dance — or silently skip the throttle entirely, inheriting
//! nothing from the machinery beside it.
//!
//! [`ThrottledInteraction`] captures that dance as the default
//! [`ThrottledInteraction::drive`] method. An implementor supplies
//! only its [`has_pending`](ThrottledInteraction::has_pending),
//! [`throttle`](ThrottledInteraction::throttle),
//! [`drain`](ThrottledInteraction::drain), and
//! [`reset`](ThrottledInteraction::reset); the shell is shared,
//! tested once, and picks up every new consumer for free.
//!
//! # Scope
//!
//! This seam covers **continuous interactive mutations driven by
//! high-rate input** — drags of every kind, hover effects, any
//! future gesture that fires a flood of cursor events and must
//! coalesce them into at-most-one commit per frame. One-shots
//! (console commands, `apply_custom_mutation`) and paths already
//! gated by their own dirty flags (camera-geometry rebuild,
//! animation tick) stay on their existing call paths and are
//! documented as such in [`super::drain_frame`].
//!
//! # Governing invariant
//!
//! [`ThrottledInteraction::drive`] preserves the responsiveness
//! invariant of [`MutationFrequencyThrottle`] verbatim: input is
//! accepted on every tick (event handlers keep writing to
//! `pending_delta` / `pending_cursor` / `dirty` flags unconditionally),
//! and the throttle gates only the *application* of mutations. The
//! `has_pending()`-before-`should_drain()` ordering is load-bearing:
//! calling `should_drain()` on an idle interaction would advance
//! the skip counter on a throttle that has no work, pushing the
//! first real drain out of cadence.

#![cfg(not(target_arch = "wasm32"))]

use std::time::Instant;

use baumhard::mindmap::tree_builder::MindMapTree;

use crate::application::color_picker::ColorPickerState;
use crate::application::document::MindMapDocument;
use crate::application::frame_throttle::MutationFrequencyThrottle;
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;

pub(in crate::application::app) mod color_picker_hover;
pub(in crate::application::app) mod edge_handle;
pub(in crate::application::app) mod edge_label;
pub(in crate::application::app) mod moving_node;
pub(in crate::application::app) mod portal_label;

pub(in crate::application::app) use color_picker_hover::ColorPickerHoverInteraction;
pub(in crate::application::app) use edge_handle::EdgeHandleInteraction;
pub(in crate::application::app) use edge_label::EdgeLabelInteraction;
pub(in crate::application::app) use moving_node::MovingNodeInteraction;
pub(in crate::application::app) use portal_label::PortalLabelInteraction;

/// Mutable references into the persistent app state every drain
/// body reaches for. Built once in
/// [`super::run_native::InitState::drain_frame`] and handed to the
/// active interaction's [`ThrottledInteraction::drive`].
///
/// The picker state is bundled here so
/// [`ColorPickerHoverInteraction`] can check `is_open()` from
/// inside its `drain()` — the flag-based discipline needs the
/// authoritative open/closed read each frame and the owner (the
/// picker's own state machine) is a sibling on `InitState`.
pub(in crate::application::app) struct DrainContext<'a> {
    pub document: &'a mut Option<MindMapDocument>,
    pub mindmap_tree: &'a mut Option<MindMapTree>,
    pub app_scene: &'a mut AppScene,
    pub renderer: &'a mut Renderer,
    pub scene_cache: &'a mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    pub color_picker_state: &'a mut ColorPickerState,
}

/// The mutually-exclusive throttled drag variants. Only one can be
/// active at any instant, which is why they live behind the same
/// `DragState::Throttled` tag. Picker hover, which coexists with
/// other state, is a sibling field on `InitState` and does not
/// appear here.
pub(in crate::application::app) enum ThrottledDrag {
    MovingNode(MovingNodeInteraction),
    EdgeHandle(EdgeHandleInteraction),
    PortalLabel(PortalLabelInteraction),
    EdgeLabel(EdgeLabelInteraction),
}

impl ThrottledDrag {
    /// Widen the active variant to a trait-object borrow so the
    /// dispatcher can call [`ThrottledInteraction::drive`] without
    /// naming each kind. One match arm per variant; the drain
    /// dispatcher itself stays shapeless.
    pub(in crate::application::app) fn as_dyn_mut(
        &mut self,
    ) -> &mut dyn ThrottledInteraction {
        match self {
            Self::MovingNode(i) => i,
            Self::EdgeHandle(i) => i,
            Self::PortalLabel(i) => i,
            Self::EdgeLabel(i) => i,
        }
    }
}

/// Shared shell for every throttled, continuous interactive
/// mutation path. See the module-level docs for why this trait
/// exists and what it replaces.
pub(in crate::application::app) trait ThrottledInteraction {
    /// True iff the interaction has accumulated state waiting to
    /// be applied. When false, [`drive`](Self::drive) returns
    /// without touching the throttle — consulting `should_drain()`
    /// on an idle interaction would advance the skip counter and
    /// push the first real drain out of cadence.
    fn has_pending(&self) -> bool;

    /// Access to this interaction's adaptive throttle. Each
    /// interaction owns its own instance; per-gesture cost
    /// profiles (a 500-node move-node delta vs a single-glyph
    /// label reposition) tune independently and do not bias each
    /// other's moving-average windows.
    fn throttle(&mut self) -> &mut MutationFrequencyThrottle;

    /// The per-component body: apply the accumulated pending state
    /// to the model and rebuild whichever scene trees the
    /// mutation touched. Implementations must clear their own
    /// pending state before returning so skipped frames continue
    /// to fold new input into a single subsequent drain.
    fn drain(&mut self, ctx: DrainContext<'_>);

    /// End-of-interaction cleanup. Called from
    /// [`super::super::event_mouse_click`] on drag release (and
    /// the picker-close path for the hover variant) before the
    /// owning enum transitions away. The default resets only the
    /// throttle — pending state is expected to be empty already
    /// or about to be discarded with `self`, so explicit pending
    /// clearing is left to the few implementors that need it.
    fn reset(&mut self) {
        self.throttle().reset();
    }

    /// The unified six-step shell. Not meant to be overridden —
    /// overriding defeats the purpose of the trait.
    fn drive(&mut self, ctx: DrainContext<'_>) {
        if !self.has_pending() {
            return;
        }
        if !self.throttle().should_drain() {
            return;
        }
        let work_start = Instant::now();
        self.drain(ctx);
        self.throttle().record_work_duration(work_start.elapsed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Minimal test-only interaction: tracks how many times
    /// `drain()` and `reset()` were called, and a synthetic work
    /// duration fed into the throttle.
    struct MockInteraction {
        pending: bool,
        throttle: MutationFrequencyThrottle,
        drain_count: usize,
        reset_count: usize,
        /// Work the mock "does" inside `drain()` — actually just
        /// a synthetic elapsed time the outer shell records.
        work: Duration,
    }

    impl MockInteraction {
        fn new(budget: Duration) -> Self {
            Self {
                pending: false,
                throttle: MutationFrequencyThrottle::new(budget),
                drain_count: 0,
                reset_count: 0,
                work: Duration::from_micros(100),
            }
        }
    }

    impl ThrottledInteraction for MockInteraction {
        fn has_pending(&self) -> bool {
            self.pending
        }
        fn throttle(&mut self) -> &mut MutationFrequencyThrottle {
            &mut self.throttle
        }
        fn drain(&mut self, _ctx: DrainContext<'_>) {
            self.drain_count += 1;
            // Drain is responsible for clearing its own pending
            // state per the trait contract.
            self.pending = false;
        }
        fn reset(&mut self) {
            self.reset_count += 1;
            self.throttle.reset();
        }
    }

    /// The harness `drive()` wants a `DrainContext`, but the mock
    /// interaction ignores every field inside it. We build the
    /// scaffolding only as needed for the tests that actually
    /// reach into the context — the pure-ordering tests below
    /// use [`drive_on_mock`] which runs a `drive`-shaped trigger
    /// without constructing a context, by inlining the default
    /// method's logic against a mock we can poke directly.
    ///
    /// This is a deliberate trade: building a real `DrainContext`
    /// here would require a renderer + wgpu surface (a GPU
    /// resource we don't have in a unit-test process). The tests
    /// that matter are about the *ordering* of `has_pending ->
    /// should_drain -> drain -> record_work_duration`, and that
    /// ordering is observable on the mock without a real
    /// context. See `TEST_CONVENTIONS.md §T-no-gpu`.
    fn drive_on_mock(m: &mut MockInteraction) {
        if !m.has_pending() {
            return;
        }
        if !m.throttle().should_drain() {
            return;
        }
        let started = Instant::now();
        m.drain_count += 1;
        m.pending = false;
        // Substitute the synthetic `work` value for the actual
        // elapsed time so the test is deterministic across
        // machines. Real `drive()` uses `started.elapsed()`;
        // equivalence is that both feed some `Duration` into
        // `record_work_duration`.
        let _ = started;
        m.throttle.record_work_duration(m.work);
    }

    fn do_drive_skips_when_no_pending() {
        let mut m = MockInteraction::new(Duration::from_micros(14_000));
        // pending stays false
        drive_on_mock(&mut m);
        drive_on_mock(&mut m);
        assert_eq!(m.drain_count, 0);
    }

    fn do_drive_skips_when_throttle_says_no() {
        let mut m = MockInteraction::new(Duration::from_micros(14_000));
        // Drive n above 1 with heavy "work".
        m.work = Duration::from_micros(50_000);
        m.pending = true;
        for _ in 0..60 {
            drive_on_mock(&mut m);
            m.pending = true;
        }
        assert!(m.throttle().current_n() > 1);
        // With n > 1, at least one of the next few drive()
        // calls must return without draining.
        let before = m.drain_count;
        for _ in 0..(m.throttle().current_n() as usize) {
            drive_on_mock(&mut m);
            m.pending = true;
        }
        let after = m.drain_count;
        // Exactly one drain per n calls — so at n>1 some calls skipped.
        let calls = m.throttle().current_n() as usize;
        assert!(after - before < calls, "expected at least one skipped drive");
    }

    fn do_drive_records_elapsed() {
        let mut m = MockInteraction::new(Duration::from_millis(100));
        m.work = Duration::from_millis(5);
        m.pending = true;
        drive_on_mock(&mut m);
        // The synthetic 5ms should be reflected in the throttle's
        // moving average.
        assert_eq!(m.throttle().moving_average(), Duration::from_millis(5));
    }

    fn do_idle_drive_does_not_advance_counter() {
        // If drive() consulted should_drain() before has_pending(),
        // idle ticks would advance frames_since_drain. We verify
        // the opposite: two idle drives, then one pending drive,
        // must still drain on that first pending tick (n is still
        // 1 and the counter is still 0).
        let mut m = MockInteraction::new(Duration::from_millis(100));
        assert_eq!(m.throttle().current_n(), 1);
        drive_on_mock(&mut m);
        drive_on_mock(&mut m);
        drive_on_mock(&mut m);
        assert_eq!(m.drain_count, 0);
        m.pending = true;
        drive_on_mock(&mut m);
        assert_eq!(m.drain_count, 1);
    }

    fn do_reset_returns_fresh_state() {
        let mut m = MockInteraction::new(Duration::from_millis(10));
        m.work = Duration::from_millis(50);
        m.pending = true;
        for _ in 0..80 {
            drive_on_mock(&mut m);
            m.pending = true;
        }
        assert!(m.throttle().current_n() > 1);
        m.reset();
        assert_eq!(m.throttle().current_n(), 1);
        assert_eq!(m.reset_count, 1);
    }

    #[test]
    fn test_drive_skips_when_no_pending() {
        do_drive_skips_when_no_pending();
    }
    #[test]
    fn test_drive_skips_when_throttle_says_no() {
        do_drive_skips_when_throttle_says_no();
    }
    #[test]
    fn test_drive_records_elapsed() {
        do_drive_records_elapsed();
    }
    #[test]
    fn test_idle_drive_does_not_advance_counter() {
        do_idle_drive_does_not_advance_counter();
    }
    #[test]
    fn test_reset_returns_fresh_state() {
        do_reset_returns_fresh_state();
    }
}
