// SPDX-License-Identifier: MPL-2.0

//! The pending-input half of a throttled interaction: what a fresh
//! cursor sample does to the state waiting to be drained, and
//! whether anything is waiting at all.
//!
//! Three disciplines cover every implementor. They are not
//! interchangeable — which one a gesture uses is a property of what
//! its drain computes, not a style choice:
//!
//! - **Delta-accumulate** — the drain applies an *incremental*
//!   movement, so skipped samples must sum rather than be discarded.
//! - **Cursor-latch** — the drain projects an *absolute* position
//!   (onto a node border, onto an edge path), so only the last
//!   sample carries information and the rest are dead weight.
//! - **Dirty flags** — nothing accumulates; a flag says the next
//!   drain has work. Used by the picker-hover interaction, which is
//!   not a drag and never sees a cursor sample.
//!
//! Before this type each implementor spelled its own discipline out
//! by hand — a `!= Vec2::ZERO` here, an `is_some()` there, and the
//! fold itself open-coded once per variant in the cursor-move
//! dispatcher. CODE_CONVENTIONS §5: the logic lives once.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;

use crate::application::frame_throttle::MutationFrequencyThrottle;

/// One `CursorMoved` sample, presented in both of the forms a
/// gesture's pending state can consume.
///
/// P1-30 names a `Delta(Vec2) | Cursor(Vec2)` enum here. A struct
/// carrying both is what the acceptance criterion actually needs:
/// with an enum the *caller* has to know which discipline the active
/// gesture uses in order to pick a variant, and that knowledge is
/// precisely the per-variant match ladder the change exists to
/// delete. Both halves fall out of the same two `screen_to_canvas`
/// projections the delta arms already performed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::application::app) struct DragInput {
    /// Canvas-space movement since the previous cursor event.
    pub delta: Vec2,
    /// Absolute canvas-space cursor position now.
    pub cursor: Vec2,
}

impl DragInput {
    /// Build a sample from the two canvas-space projections of one
    /// `CursorMoved` event.
    ///
    /// The delta is `cursor - prev`, not the reverse: a rightward
    /// move must produce a positive x so the delta drags move *with*
    /// the pointer. Split out of the dispatcher so the sign, and the
    /// fact that the absolute half is the newer of the two
    /// projections, are pinned without a live renderer — the
    /// dispatcher itself needs one and TEST_CONVENTIONS §T8 keeps
    /// that out of the harness.
    pub(in crate::application::app) fn between(prev: Vec2, cursor: Vec2) -> Self {
        Self {
            delta: cursor - prev,
            cursor,
        }
    }
}

/// The three pending disciplines. Private: a gesture picks one at
/// construction through [`ThrottledPending`]'s constructors and
/// never names the variant again.
#[derive(Debug)]
enum PendingInput {
    /// Delta-accumulate. `total` is the sum since drag start —
    /// release commits read it — and `pending` the sum since the
    /// last drain.
    Delta { total: Vec2, pending: Vec2 },
    /// Cursor-latch: the last absolute cursor, or `None` once a
    /// drain has taken it.
    Cursor(Option<Vec2>),
    /// Dirty flags. `dirty` is the pending signal; `canvas_dirty`
    /// rides along because the drain that consumes them clears both
    /// together and no third party may observe one without the
    /// other.
    Flags { dirty: bool, canvas_dirty: bool },
}

/// A throttled interaction's pending input plus the throttle that
/// gates its drain.
///
/// Every implementor of
/// [`super::ThrottledInteraction`] owns exactly one of these, and
/// exposing it is the whole of the trait's required state surface:
/// `has_pending`, `throttle` and `accumulate` are all provided from
/// it.
#[derive(Debug)]
pub(in crate::application::app) struct ThrottledPending {
    input: PendingInput,
    /// Per-interaction adaptive throttle. Each interaction owns its
    /// own instance; per-gesture cost profiles (a 500-node move-node
    /// delta vs a single-glyph label reposition) tune independently
    /// and do not bias each other's moving-average windows.
    pub throttle: MutationFrequencyThrottle,
}

impl ThrottledPending {
    /// A gesture whose drain applies an incremental delta.
    pub(in crate::application::app) fn accumulating_deltas() -> Self {
        Self {
            input: PendingInput::Delta {
                total: Vec2::ZERO,
                pending: Vec2::ZERO,
            },
            throttle: MutationFrequencyThrottle::with_default_budget(),
        }
    }

    /// A gesture whose drain projects the latest absolute cursor.
    pub(in crate::application::app) fn latching_cursor() -> Self {
        Self {
            input: PendingInput::Cursor(None),
            throttle: MutationFrequencyThrottle::with_default_budget(),
        }
    }

    /// An interaction whose pending signal is a dirty flag rather
    /// than cursor input.
    pub(in crate::application::app) fn dirty_flags() -> Self {
        Self {
            input: PendingInput::Flags {
                dirty: false,
                canvas_dirty: false,
            },
            throttle: MutationFrequencyThrottle::with_default_budget(),
        }
    }

    /// True iff there is state waiting to be applied.
    ///
    /// The delta comparison is strict `!= ZERO`: a sub-pixel
    /// accumulator from one high-frequency tick must still count,
    /// because the sum across skipped frames is the contract
    /// `drive()` relies on.
    pub(in crate::application::app) fn has_pending(&self) -> bool {
        match &self.input {
            PendingInput::Delta { pending, .. } => *pending != Vec2::ZERO,
            PendingInput::Cursor(latest) => latest.is_some(),
            PendingInput::Flags { dirty, .. } => *dirty,
        }
    }

    /// Fold one cursor sample in, per this gesture's discipline.
    ///
    /// A no-op under the dirty-flag discipline: that interaction is
    /// not a drag, is not reachable through
    /// [`super::ThrottledDragInteraction`], and never sees a sample.
    /// Silently ignoring one is the §9-correct posture for an
    /// interactive path — there is nothing a panic here could tell a
    /// user.
    pub(in crate::application::app) fn accumulate(&mut self, input: DragInput) {
        match &mut self.input {
            PendingInput::Delta { total, pending } => {
                *total += input.delta;
                *pending += input.delta;
            }
            PendingInput::Cursor(latest) => *latest = Some(input.cursor),
            PendingInput::Flags { .. } => {}
        }
    }

    /// Sum of every delta since drag start.
    ///
    /// `Vec2::ZERO` under the other two disciplines, which keep no
    /// total: a cursor-latch drain projects the latest absolute
    /// position and nothing about the path taken survives, and a
    /// dirty flag has no geometry at all.
    pub(in crate::application::app) fn total_delta(&self) -> Vec2 {
        match &self.input {
            PendingInput::Delta { total, .. } => *total,
            _ => Vec2::ZERO,
        }
    }

    /// Delta accumulated since the last drain. `Vec2::ZERO` under
    /// the other two disciplines — see [`Self::total_delta`].
    ///
    /// Test-gated: drains consume the value through
    /// [`Self::take_delta`]; only the tests need to look without
    /// taking.
    #[cfg(test)]
    pub(in crate::application::app) fn pending_delta(&self) -> Vec2 {
        match &self.input {
            PendingInput::Delta { pending, .. } => *pending,
            _ => Vec2::ZERO,
        }
    }

    /// Consume the pending delta, leaving the running total intact.
    /// The delta-discipline drains call this once their write has
    /// landed so skipped frames keep folding into a single
    /// subsequent drain.
    pub(in crate::application::app) fn take_delta(&mut self) -> Vec2 {
        match &mut self.input {
            PendingInput::Delta { pending, .. } => std::mem::replace(pending, Vec2::ZERO),
            _ => Vec2::ZERO,
        }
    }

    /// Consume the latched cursor. `None` under the other two
    /// disciplines, and `None` under this one once a drain has
    /// already taken it — the two are the same answer to the same
    /// question ("is there an absolute position waiting?").
    pub(in crate::application::app) fn take_cursor(&mut self) -> Option<Vec2> {
        match &mut self.input {
            PendingInput::Cursor(latest) => latest.take(),
            _ => None,
        }
    }

    /// Read the latched cursor without consuming it. Test-gated for
    /// the same reason as [`Self::pending_delta`]: drains use
    /// [`Self::take_cursor`].
    #[cfg(test)]
    pub(in crate::application::app) fn peek_cursor(&self) -> Option<Vec2> {
        match &self.input {
            PendingInput::Cursor(latest) => *latest,
            _ => None,
        }
    }

    /// Flag the next drain. Dirty-flag discipline only.
    pub(in crate::application::app) fn mark_dirty(&mut self) {
        if let PendingInput::Flags { dirty, .. } = &mut self.input {
            *dirty = true;
        }
    }

    /// Flag the next drain *and* the canvas rebuild it gates. Every
    /// canvas-dirtying path is also overlay-dirtying, so this sets
    /// both — the reverse (overlay without canvas) is the gesture-only
    /// frame [`Self::mark_dirty`] exists for.
    pub(in crate::application::app) fn mark_canvas_dirty(&mut self) {
        if let PendingInput::Flags { dirty, canvas_dirty } = &mut self.input {
            *dirty = true;
            *canvas_dirty = true;
        }
    }

    /// True iff the document's picker preview changed since the last
    /// drain — the sole signal that gates the canvas rebuild.
    /// Gesture frames leave it clear.
    pub(in crate::application::app) fn canvas_is_dirty(&self) -> bool {
        match &self.input {
            PendingInput::Flags { canvas_dirty, .. } => *canvas_dirty,
            _ => false,
        }
    }

    /// Clear both flags. Dirty-flag discipline only.
    pub(in crate::application::app) fn clear_flags(&mut self) {
        if let PendingInput::Flags { dirty, canvas_dirty } = &mut self.input {
            *dirty = false;
            *canvas_dirty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dx: f32, dy: f32, cx: f32, cy: f32) -> DragInput {
        DragInput {
            delta: Vec2::new(dx, dy),
            cursor: Vec2::new(cx, cy),
        }
    }

    /// The dispatcher's whole arithmetic contract: the delta points
    /// the way the pointer went, and the absolute half is where the
    /// pointer is *now*, not where it was.
    #[test]
    fn test_between_takes_the_delta_forwards_and_latches_the_new_cursor() {
        let input = DragInput::between(Vec2::new(10.0, 4.0), Vec2::new(13.0, 9.0));
        assert_eq!(input.delta, Vec2::new(3.0, 5.0));
        assert_eq!(input.cursor, Vec2::new(13.0, 9.0));
    }

    /// A cursor event that did not move produces no delta — the
    /// pending accumulator stays idle rather than being flipped to
    /// "drain me" by a zero.
    #[test]
    fn test_between_a_stationary_cursor_is_a_zero_delta() {
        let at = Vec2::new(7.0, -2.0);
        let mut p = ThrottledPending::accumulating_deltas();
        p.accumulate(DragInput::between(at, at));
        assert!(!p.has_pending());
    }

    #[test]
    fn test_delta_discipline_sums_both_accumulators() {
        let mut p = ThrottledPending::accumulating_deltas();
        assert!(!p.has_pending());
        p.accumulate(sample(3.0, 4.0, 3.0, 4.0));
        p.accumulate(sample(1.0, 1.0, 4.0, 5.0));
        assert!(p.has_pending());
        assert_eq!(p.total_delta(), Vec2::new(4.0, 5.0));
        assert_eq!(p.pending_delta(), Vec2::new(4.0, 5.0));
    }

    /// `take_delta` clears only the pending half — the running total
    /// is what the release commit reads, and a drain must not
    /// disturb it.
    #[test]
    fn test_take_delta_leaves_the_running_total_alone() {
        let mut p = ThrottledPending::accumulating_deltas();
        p.accumulate(sample(3.0, 4.0, 3.0, 4.0));
        assert_eq!(p.take_delta(), Vec2::new(3.0, 4.0));
        assert_eq!(p.pending_delta(), Vec2::ZERO);
        assert_eq!(p.total_delta(), Vec2::new(3.0, 4.0));
        assert!(!p.has_pending());
        p.accumulate(sample(1.0, 0.0, 4.0, 4.0));
        assert_eq!(p.total_delta(), Vec2::new(4.0, 4.0));
        assert_eq!(p.pending_delta(), Vec2::new(1.0, 0.0));
    }

    /// A sub-pixel accumulator still counts as pending.
    #[test]
    fn test_tiny_nonzero_delta_is_pending() {
        let mut p = ThrottledPending::accumulating_deltas();
        p.accumulate(sample(1e-6, 0.0, 1e-6, 0.0));
        assert!(p.has_pending());
    }

    #[test]
    fn test_cursor_discipline_latches_the_last_sample_only() {
        let mut p = ThrottledPending::latching_cursor();
        assert!(!p.has_pending());
        p.accumulate(sample(1.0, 1.0, 10.0, 10.0));
        p.accumulate(sample(1.0, 1.0, 42.0, -7.0));
        assert!(p.has_pending());
        assert_eq!(p.peek_cursor(), Some(Vec2::new(42.0, -7.0)));
        assert_eq!(p.take_cursor(), Some(Vec2::new(42.0, -7.0)));
        assert_eq!(p.take_cursor(), None);
        assert!(!p.has_pending());
    }

    /// The cursor discipline keeps no total: its drain projects an
    /// absolute position and the path taken is not information.
    #[test]
    fn test_cursor_discipline_reports_no_deltas() {
        let mut p = ThrottledPending::latching_cursor();
        p.accumulate(sample(5.0, 5.0, 10.0, 10.0));
        assert_eq!(p.total_delta(), Vec2::ZERO);
        assert_eq!(p.pending_delta(), Vec2::ZERO);
        assert_eq!(p.take_delta(), Vec2::ZERO);
    }

    /// ...and the delta discipline latches no cursor.
    #[test]
    fn test_delta_discipline_reports_no_cursor() {
        let mut p = ThrottledPending::accumulating_deltas();
        p.accumulate(sample(5.0, 5.0, 10.0, 10.0));
        assert_eq!(p.peek_cursor(), None);
        assert_eq!(p.take_cursor(), None);
    }

    #[test]
    fn test_flag_discipline_gates_pending_on_dirty_only() {
        let mut p = ThrottledPending::dirty_flags();
        assert!(!p.has_pending());
        assert!(!p.canvas_is_dirty());
        p.mark_dirty();
        assert!(p.has_pending());
        assert!(
            !p.canvas_is_dirty(),
            "a gesture-only frame must skip the canvas walk"
        );
        p.mark_canvas_dirty();
        assert!(p.has_pending());
        assert!(p.canvas_is_dirty());
        p.clear_flags();
        assert!(!p.has_pending());
        assert!(!p.canvas_is_dirty());
    }

    /// A cursor sample handed to the flag discipline is ignored
    /// rather than mistaken for pending work. Unreachable in
    /// production — the picker hover is not a `ThrottledDrag` — but
    /// the alternative is a panic on an interactive path.
    #[test]
    fn test_flag_discipline_ignores_a_cursor_sample() {
        let mut p = ThrottledPending::dirty_flags();
        p.accumulate(sample(5.0, 5.0, 10.0, 10.0));
        assert!(!p.has_pending());
        assert_eq!(p.total_delta(), Vec2::ZERO);
        assert_eq!(p.peek_cursor(), None);
    }
}
