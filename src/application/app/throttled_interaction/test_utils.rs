// SPDX-License-Identifier: MPL-2.0

//! Shared helpers and the trait-default test macro for the
//! throttled-interaction implementors.
//!
//! Seven impls under this module (`MovingNodeInteraction`,
//! `MovingSectionInteraction`, `SectionResizeInteraction`,
//! `EdgeHandleInteraction`, `EdgeLabelInteraction`,
//! `PortalLabelInteraction`, `ColorPickerHoverInteraction`) all
//! inherit the `should_perform_drain` ordering invariant from
//! [`super::ThrottledInteraction`]'s default method. Pre-macro,
//! every impl carried its own four copies of the same four tests
//! exercising that invariant — the only varying parts being a
//! fixture constructor and a per-impl "set pending" hook. Those
//! four tests now live once, inside
//! [`trait_default_tests_for_throttled_interaction!`], and each
//! impl invokes the macro with its own `build` and `set_pending`
//! closures.
//!
//! Per-impl-specific tests (e.g. `test_handle_variant_round_trips_control_point`
//! on `EdgeHandleInteraction`, `test_canvas_needs_rebuild_*` on
//! `ColorPickerHoverInteraction`) are not in the macro's scope —
//! they stay inline at their respective sites.

use crate::application::document::defaults::default_parent_child_edge;
use crate::application::frame_throttle::MutationFrequencyThrottle;
use baumhard::mindmap::model::MindEdge;
use glam::Vec2;
use std::time::Duration;

use super::pending::DragInput;

/// One cursor sample, both halves stated. The production
/// constructor is `DragInput::between(prev, cursor)`, where the two
/// halves are unrelated numbers; a test that wants to tell the
/// disciplines apart has to say so explicitly, which is what this is
/// for.
///
/// `sample(Vec2::ZERO, c)` is the sharpest of them: a stationary
/// pointer still reports an absolute position, so a cursor-latch
/// gesture goes pending on it and a delta-accumulate gesture does
/// not. Nothing else separates the two on a single sample.
pub fn sample(delta: Vec2, cursor: Vec2) -> DragInput {
    DragInput { delta, cursor }
}

/// Offset between the two halves of a [`moved`] sample.
///
/// Non-zero on purpose. When the halves were equal, an assertion
/// reading back `(x, y)` could not say *which* half it had read, so
/// a gesture wired to the wrong discipline satisfied it either way.
const MOVED_CURSOR_OFFSET: f32 = 100.0;

/// One cursor sample that moved `(x, y)` and now sits at
/// `(x, y) + MOVED_CURSOR_OFFSET` — the two halves deliberately
/// distinguishable, so an assertion about one of them cannot be
/// satisfied by the other.
///
/// Use [`sample`] where the delta and the absolute position need to
/// be pinned independently.
pub fn moved(x: f32, y: f32) -> DragInput {
    sample(
        Vec2::new(x, y),
        Vec2::new(x + MOVED_CURSOR_OFFSET, y + MOVED_CURSOR_OFFSET),
    )
}

/// Push the throttle's average over-budget until `n > 1`. Returns
/// the final drain divisor for assertion plumbing.
pub fn drive_throttle_over_budget(t: &mut MutationFrequencyThrottle) -> u32 {
    for _ in 0..80 {
        if t.should_drain() {
            t.record_work_duration(Duration::from_micros(50_000));
        }
    }
    t.current_n()
}

/// Construct a minimally-valid `MindEdge` for the tests. The drag
/// state only references the snapshot for its pre-drag identity
/// bookkeeping; no field inside it is under test here. Routes
/// through `defaults::default_parent_child_edge` so the field-set
/// stays in lockstep with the production-side parent-child default
/// — they were byte-for-byte identical before this delegation.
pub fn fixture_edge() -> MindEdge {
    default_parent_child_edge("a", "b")
}

/// Emit the four trait-default `should_perform_drain` tests for a
/// [`super::ThrottledInteraction`] implementor.
///
/// The default method's contract is identical for every implementor:
///
/// - **Idle** → returns `false` without touching the throttle.
/// - **Pending + fresh throttle** → returns `true`.
/// - **Pending + skipping throttle (n > 1)** → returns `false` on the
///   skipped frames in the cadence.
/// - **Idle calls don't advance the throttle's skip counter.**
///
/// A new implementor invokes this macro once inside its
/// `#[cfg(test)] mod tests { ... }` block:
///
/// ```ignore
/// trait_default_tests_for_throttled_interaction! {
///     build = || MyInteraction::new(/* idle inputs */),
///     set_pending = |i: &mut MyInteraction| { i.dirty = true; },
/// }
/// ```
///
/// `build` returns a fresh idle instance (the throttle at `n = 1`,
/// no pending state). `set_pending` is `FnMut(&mut T)` and flips
/// the impl's `has_pending()` to true — the macro calls it
/// repeatedly inside the cadence-skip test, so it must be cheap
/// and idempotent.
///
/// The macro reaches the throttle through the trait's
/// [`ThrottledInteraction::throttle`] accessor — the same seam
/// production drain code uses. A future implementor that satisfies
/// the trait works without macro changes; a renamed `throttle` field
/// shows up as a clean compile error at the trait impl, not at every
/// macro expansion.
macro_rules! trait_default_tests_for_throttled_interaction {
    (
        build = $build:expr,
        set_pending = $set_pending:expr $(,)?
    ) => {
        #[test]
        fn test_should_perform_drain_false_when_idle() {
            use $crate::application::app::throttled_interaction::ThrottledInteraction;
            let mut i = ($build)();
            assert!(
                !i.should_perform_drain(),
                "idle interaction must report no drain"
            );
        }

        #[test]
        fn test_should_perform_drain_true_when_pending_and_throttle_fresh() {
            use $crate::application::app::throttled_interaction::ThrottledInteraction;
            let mut i = ($build)();
            ($set_pending)(&mut i);
            assert!(
                i.should_perform_drain(),
                "pending interaction with fresh throttle must drain"
            );
        }

        #[test]
        fn test_should_perform_drain_false_when_throttle_skipping() {
            // Throttle cadence under sustained over-budget load: at
            // n > 1, should_perform_drain must return false on the
            // skipped frames even when pending state is set.
            use $crate::application::app::throttled_interaction::ThrottledInteraction;
            let mut i = ($build)();
            $crate::application::app::throttled_interaction::test_utils::drive_throttle_over_budget(
                i.throttle(),
            );
            assert!(i.throttle().current_n() > 1);

            let n = i.throttle().current_n() as usize;
            ($set_pending)(&mut i);
            let mut saw_skip = false;
            for _ in 0..(n * 2) {
                if !i.should_perform_drain() {
                    saw_skip = true;
                }
                // Keep n stable while probing cadence.
                i.throttle()
                    .record_work_duration(::std::time::Duration::from_micros(50_000));
                ($set_pending)(&mut i);
            }
            assert!(saw_skip, "expected at least one skipped drain at n > 1");
        }

        #[test]
        fn test_idle_should_perform_drain_does_not_advance_throttle() {
            // Invariant — if should_perform_drain consulted
            // should_drain first, this would be off by n: several
            // idle calls would advance `frames_since_drain` and the
            // next pending tick would skip instead of drain.
            use $crate::application::app::throttled_interaction::ThrottledInteraction;
            let mut i = ($build)();
            for _ in 0..5 {
                assert!(!i.should_perform_drain());
            }
            ($set_pending)(&mut i);
            assert!(
                i.should_perform_drain(),
                "first pending tick after idles must drain"
            );
        }
    };
}

pub(crate) use trait_default_tests_for_throttled_interaction;
