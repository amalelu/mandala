// SPDX-License-Identifier: MPL-2.0

//! Unified shell for continuous, high-frequency-input-driven
//! mutations, covering the whole lifecycle of one:
//!
//! 1. **Accumulate** — [`ThrottledDragInteraction::accumulate`]
//!    folds a cursor sample into the gesture's pending state, per
//!    the discipline its [`ThrottledPending`] was built with.
//! 2. **Drain** — [`ThrottledInteraction::drive`] packages the
//!    has-pending → should-drain → record-duration dance around the
//!    per-gesture `drain` body.
//! 3. **Commit on release** —
//!    [`ThrottledDragInteraction::commit_on_release_core`] flushes
//!    what the throttle left pending, writes the model with its undo
//!    entry, and *names* the canvas decree it owes rather than
//!    running it; [`release::ReleaseRefresh::execute`] is what runs
//!    it, off the gesture trait entirely (see [`release`]).
//!
//! An implementor supplies four things: the accessor pair for its
//! [`ThrottledPending`], a `drain` body, and — if it is a drag — a
//! `commit_on_release_core` body. `has_pending`, `throttle`,
//! `accumulate` and the whole drive shell follow. Adding a new throttled drag is
//! one struct, one trait impl and one [`ThrottledDrag`] variant;
//! neither event-file dispatcher grows.
//!
//! Scope: drags and hover effects that fire a flood of cursor
//! events. One-shots and self-gated paths stay on their existing
//! call paths.
//!
//! The has-pending check before should_drain is load-bearing —
//! advancing the skip counter on an idle interaction would push
//! the first real drain out of cadence.

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
pub(in crate::application::app) mod moving_section;
pub(in crate::application::app) mod node_resize;
pub(in crate::application::app) mod pending;
pub(in crate::application::app) mod portal_label;
pub(in crate::application::app) mod release;
pub(in crate::application::app) mod section_resize;
#[cfg(test)]
mod test_utils;
#[cfg(test)]
mod tests_lifecycle;

pub(in crate::application::app) use color_picker_hover::ColorPickerHoverInteraction;
pub(in crate::application::app) use edge_handle::EdgeHandleInteraction;
pub(in crate::application::app) use edge_label::EdgeLabelInteraction;
pub(in crate::application::app) use moving_node::MovingNodeInteraction;
pub(in crate::application::app) use moving_section::MovingSectionInteraction;
pub(in crate::application::app) use node_resize::NodeResizeInteraction;
pub(in crate::application::app) use pending::{DragInput, ThrottledPending};

pub(in crate::application::app) use portal_label::PortalLabelInteraction;
pub(in crate::application::app) use release::{ReleaseCommit, ReleaseRefresh};
pub(in crate::application::app) use section_resize::SectionResizeInteraction;

/// Mutable references into the persistent app state every drain
/// body reaches for. Built once in
/// [`super::run_native::InitState::drain_inputs`] and handed to the
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
    /// Active interaction mode — drives `InteractionModeOverrides` for
    /// every per-frame `CanvasFrame` built here. Read-only
    /// because per-frame drains never mutate mode (mode transitions
    /// are discrete actions, not gestures).
    pub interaction_mode: &'a super::InteractionMode,
}

impl DrainContext<'_> {
    /// The renderer-free slice a release commit body works against.
    /// Reborrows, so the caller keeps the full context for the
    /// decree that follows.
    pub(in crate::application::app) fn release_commit(&mut self) -> ReleaseCommit<'_> {
        ReleaseCommit {
            document: &mut *self.document,
            mindmap_tree: &mut *self.mindmap_tree,
            scene_cache: &mut *self.scene_cache,
        }
    }
}

/// The mutually-exclusive throttled drag variants. Only one can be
/// active at any instant, which is why they live behind the same
/// `DragState::Throttled` tag. Picker hover, which coexists with
/// other state, is a sibling field on `InitState` and does not
/// appear here.
pub(in crate::application::app) enum ThrottledDrag {
    MovingNode(MovingNodeInteraction),
    MovingSection(MovingSectionInteraction),
    SectionResize(SectionResizeInteraction),
    NodeResize(NodeResizeInteraction),
    EdgeHandle(EdgeHandleInteraction),
    PortalLabel(PortalLabelInteraction),
    EdgeLabel(EdgeLabelInteraction),
}

impl ThrottledDrag {
    /// Widen the active variant to a trait-object borrow so every
    /// dispatcher — accumulate, drain, commit-on-release — can reach
    /// it without naming the kind. One match arm per variant, and
    /// this is the only place in the crate that has one; the three
    /// dispatchers themselves stay shapeless.
    ///
    /// Widened to [`ThrottledDragInteraction`] rather than to
    /// [`ThrottledInteraction`]: the drag trait has the other as a
    /// supertrait, so `drive()` is reachable through it and a second
    /// ladder is not needed.
    pub(in crate::application::app) fn as_dyn_mut(&mut self) -> &mut dyn ThrottledDragInteraction {
        match self {
            Self::MovingNode(i) => i,
            Self::MovingSection(i) => i,
            Self::SectionResize(i) => i,
            Self::NodeResize(i) => i,
            Self::EdgeHandle(i) => i,
            Self::PortalLabel(i) => i,
            Self::EdgeLabel(i) => i,
        }
    }

    /// `&self` widening counterpart of [`Self::as_dyn_mut`]. Predicates
    /// like [`ThrottledInteraction::needs_continuation`] read state
    /// without mutation; an immutable borrow lets callers ask the
    /// active variant whether the event loop should keep iterating
    /// without holding `&mut self.drag_state`.
    pub(in crate::application::app) fn as_dyn(&self) -> &dyn ThrottledDragInteraction {
        match self {
            Self::MovingNode(i) => i,
            Self::MovingSection(i) => i,
            Self::SectionResize(i) => i,
            Self::NodeResize(i) => i,
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
    /// This interaction's pending input plus the throttle that
    /// gates its drain. The whole of the required state surface:
    /// [`has_pending`](Self::has_pending),
    /// [`throttle`](Self::throttle) and
    /// [`ThrottledDragInteraction::accumulate`] are all provided
    /// from it, so a new implementor writes the accessor pair once
    /// instead of restating its pending discipline in three places.
    fn pending(&self) -> &ThrottledPending;

    /// `&mut` counterpart of [`Self::pending`].
    fn pending_mut(&mut self) -> &mut ThrottledPending;

    /// True iff the interaction has accumulated state waiting to
    /// be applied. When false, [`drive`](Self::drive) returns
    /// without touching the throttle — consulting `should_drain()`
    /// on an idle interaction would advance the skip counter and
    /// push the first real drain out of cadence.
    fn has_pending(&self) -> bool {
        self.pending().has_pending()
    }

    /// Access to this interaction's adaptive throttle. Each
    /// interaction owns its own instance; per-gesture cost
    /// profiles (a 500-node move-node delta vs a single-glyph
    /// label reposition) tune independently and do not bias each
    /// other's moving-average windows.
    fn throttle(&mut self) -> &mut MutationFrequencyThrottle {
        &mut self.pending_mut().throttle
    }

    /// The per-component body: apply the accumulated pending state
    /// to the model and rebuild whichever scene trees the
    /// mutation touched. Implementations must clear their own
    /// pending state before returning so skipped frames continue
    /// to fold new input into a single subsequent drain.
    fn drain(&mut self, ctx: DrainContext<'_>);

    /// End-of-interaction cleanup. Called from
    /// `super::event_mouse_click` on drag release (and
    /// the picker-close path for the hover variant) before the
    /// owning enum transitions away. The default resets only the
    /// throttle — pending state is expected to be empty already
    /// or about to be discarded with `self`, so explicit pending
    /// clearing is left to the few implementors that need it.
    fn reset(&mut self) {
        self.throttle().reset();
    }

    /// Pure predicate: true iff this tick should perform the drain
    /// body. Encodes the `has_pending`-before-`should_drain`
    /// ordering invariant (see module docs) as a standalone check.
    /// Callable without a [`DrainContext`]; separated from
    /// [`drive`](Self::drive) so tests can exercise the ordering
    /// against real interaction values.
    fn should_perform_drain(&mut self) -> bool {
        if !self.has_pending() {
            return false;
        }
        self.throttle().should_drain()
    }

    /// True iff the loop must keep iterating to flush pending state
    /// even without further input. Consulted by the idle-CPU
    /// gating: under `ControlFlow::Wait` the loop would otherwise
    /// park indefinitely after the last cursor event, leaving a
    /// throttle-deferred drain stuck on its accumulated pending
    /// delta. The default mirrors `has_pending` — if there's
    /// pending state, a future drive() will eventually drain it.
    fn needs_continuation(&self) -> bool {
        self.has_pending()
    }

    /// The unified six-step shell. Not meant to be overridden —
    /// overriding defeats the purpose of the trait.
    fn drive(&mut self, ctx: DrainContext<'_>) {
        if !self.should_perform_drain() {
            return;
        }
        let work_start = Instant::now();
        self.drain(ctx);
        self.throttle().record_work_duration(work_start.elapsed());
    }
}

/// The two lifecycle phases a *drag* has and the picker-hover
/// interaction does not: a cursor sample arriving, and a button
/// coming up.
///
/// Split from [`ThrottledInteraction`] rather than folded into it so
/// `commit_on_release_core` stays a required method. A default that
/// did nothing would let a new drag variant compile with no release
/// behavior at all — silently losing the user's gesture at mouse-up,
/// which is the least visible way this could break.
pub(in crate::application::app) trait ThrottledDragInteraction:
    ThrottledInteraction
{
    /// Fold one `CursorMoved` sample into the pending state.
    ///
    /// Provided, and not meant to be overridden: which half of the
    /// sample survives is a property of the
    /// [`ThrottledPending`] discipline the gesture chose at
    /// construction, not of the gesture's own code.
    fn accumulate(&mut self, input: DragInput) {
        self.pending_mut().accumulate(input);
    }

    /// Commit the gesture at button-up, without touching the
    /// renderer: flush whatever the throttle left pending, write the
    /// model with its undo entry, clear the scene cache if the
    /// per-frame drains left stale samples in it, and return the
    /// canvas work that leaves owing.
    ///
    /// This is the *only* release entry point on the trait, which is
    /// what keeps the release ladder testable (see [`release`]):
    /// [`ReleaseCommit`] carries no renderer, so a commit body has
    /// nothing to reach it with. The one thing on the release path
    /// still holding `&mut Renderer` is
    /// [`ReleaseRefresh::execute`], which lives on the decree and
    /// not on a gesture at all.
    ///
    /// Requiring it rather than defaulting it means a new variant
    /// cannot compile with no release behavior at all. What is left
    /// as convention is the *drain* half: [`drain`] and its
    /// [`drive`](ThrottledInteraction::drive) shell take a
    /// [`DrainContext`] because a per-frame drain genuinely
    /// repaints, and [`accumulate`](Self::accumulate) is provided
    /// rather than sealed. Neither is meant to write the model —
    /// nothing enforces that.
    ///
    /// [`drain`]: ThrottledInteraction::drain
    fn commit_on_release_core(&mut self, commit: ReleaseCommit<'_>) -> ReleaseRefresh;

    /// True iff the right mouse button started this gesture.
    ///
    /// The right-button release path finalizes only gestures it
    /// started; a stray right-click during a left-button drag must
    /// not terminate it, because the left-button release is the
    /// rightful finalizer. Defaults to `false` — a gesture that
    /// cannot be started with the right button says nothing.
    fn started_with_right(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::test_utils::{
        drive_throttle_over_budget, fixture_edge, moved, sample,
    };
    use crate::application::document::EdgeRef;
    use baumhard::mindmap::model::{Position, Size};
    use baumhard::mindmap::tree_builder::{EdgeHandleKind, ResizeHandleSide};
    use glam::Vec2;

    /// One of each variant, in declaration order. Built through the
    /// real constructors so a variant that picks the wrong pending
    /// discipline shows up here, not in production.
    fn every_variant() -> Vec<ThrottledDrag> {
        vec![
            ThrottledDrag::MovingNode(MovingNodeInteraction::new(
                vec!["x".into()],
                false,
                std::collections::HashSet::new(),
            )),
            ThrottledDrag::MovingSection(MovingSectionInteraction::new("x".into(), 0, (0.0, 0.0))),
            ThrottledDrag::SectionResize(SectionResizeInteraction::new(
                "x".into(),
                0,
                ResizeHandleSide::SE,
                Position { x: 0.0, y: 0.0 },
                Size {
                    width: 10.0,
                    height: 10.0,
                },
                false,
            )),
            ThrottledDrag::NodeResize(NodeResizeInteraction::new(
                "x".into(),
                ResizeHandleSide::SE,
                Position { x: 0.0, y: 0.0 },
                Size {
                    width: 10.0,
                    height: 10.0,
                },
                false,
            )),
            ThrottledDrag::EdgeHandle(EdgeHandleInteraction::new(
                EdgeRef::new("a", "b", "parent_child"),
                EdgeHandleKind::AnchorFrom,
                fixture_edge(),
                Vec2::ZERO,
            )),
            ThrottledDrag::PortalLabel(PortalLabelInteraction::new(
                EdgeRef::new("a", "b", "parent_child"),
                "a".to_string(),
                fixture_edge(),
            )),
            ThrottledDrag::EdgeLabel(EdgeLabelInteraction::new(
                EdgeRef::new("a", "b", "parent_child"),
                fixture_edge(),
            )),
        ]
    }

    /// Name each variant through an exhaustive `match`, so adding one
    /// to the enum is a *build* error here. Paired with
    /// [`test_every_variant_is_covered`], that turns "someone added a
    /// throttled drag and forgot the fixture" from a silent coverage
    /// hole into a red build.
    fn variant_name(drag: &ThrottledDrag) -> &'static str {
        match drag {
            ThrottledDrag::MovingNode(_) => "MovingNode",
            ThrottledDrag::MovingSection(_) => "MovingSection",
            ThrottledDrag::SectionResize(_) => "SectionResize",
            ThrottledDrag::NodeResize(_) => "NodeResize",
            ThrottledDrag::EdgeHandle(_) => "EdgeHandle",
            ThrottledDrag::PortalLabel(_) => "PortalLabel",
            ThrottledDrag::EdgeLabel(_) => "EdgeLabel",
        }
    }

    #[test]
    fn test_every_variant_is_covered() {
        let names: Vec<&str> = every_variant().iter().map(variant_name).collect();
        assert_eq!(
            names,
            vec![
                "MovingNode",
                "MovingSection",
                "SectionResize",
                "NodeResize",
                "EdgeHandle",
                "PortalLabel",
                "EdgeLabel",
            ],
            "every ThrottledDrag variant must appear in the shared fixture"
        );
    }

    /// The accumulate half of the lifecycle, through the same single
    /// `as_dyn_mut()` call the cursor-move dispatcher makes.
    ///
    /// What this pins is the *routing*: every variant is reachable
    /// through the widening match, and one sample through it lands
    /// in the real struct's pending state rather than a temporary.
    /// A variant left out of `as_dyn_mut` — or wired to a sibling's
    /// interaction — fails here.
    ///
    /// It deliberately does **not** pin the pending discipline: the
    /// `moved` sample carries both halves, so a gesture wired either
    /// way reports pending on it. Discipline is
    /// [`test_a_stationary_pointer_separates_the_two_disciplines`]
    /// below, plus the per-impl
    /// `test_pending_uses_the_*_discipline` pair.
    #[test]
    fn test_accumulate_through_as_dyn_mut_flips_every_variant_to_pending() {
        for mut drag in every_variant() {
            let name = variant_name(&drag);
            assert!(!drag.as_dyn().has_pending(), "{name} must start idle");
            drag.as_dyn_mut().accumulate(moved(3.0, -2.0));
            assert!(
                drag.as_dyn().has_pending(),
                "{name} did not accept a cursor sample through accumulate()"
            );
        }
    }

    /// Which discipline each variant is wired to, named through an
    /// exhaustive `match` so a new variant has to answer the
    /// question rather than inherit an answer. `true` is
    /// cursor-latch, `false` delta-accumulate; the picker-hover's
    /// dirty-flag discipline is not a [`ThrottledDrag`].
    fn latches_the_cursor(drag: &ThrottledDrag) -> bool {
        match drag {
            ThrottledDrag::MovingNode(_) => false,
            ThrottledDrag::MovingSection(_) => false,
            ThrottledDrag::SectionResize(_) => false,
            ThrottledDrag::NodeResize(_) => false,
            ThrottledDrag::EdgeHandle(_) => false,
            ThrottledDrag::PortalLabel(_) => true,
            ThrottledDrag::EdgeLabel(_) => true,
        }
    }

    /// The entry-point pin for a gesture wired to the wrong pending
    /// discipline.
    ///
    /// A stationary pointer is the one sample that separates them:
    /// the delta is `ZERO` but the absolute position is not, so a
    /// cursor-latch gesture goes pending and a delta-accumulate one
    /// does not. Swap any variant's `ThrottledPending` constructor
    /// and this fails on that variant's row — which is the failure
    /// the production symptom would otherwise be: a drag that is
    /// handed every sample, compiles, and silently never drains.
    #[test]
    fn test_a_stationary_pointer_separates_the_two_disciplines() {
        for mut drag in every_variant() {
            let name = variant_name(&drag);
            let latches = latches_the_cursor(&drag);
            drag.as_dyn_mut()
                .accumulate(sample(Vec2::ZERO, Vec2::new(5.0, 5.0)));
            assert_eq!(
                drag.as_dyn().has_pending(),
                latches,
                "{name} answered the stationary-pointer sample as the wrong discipline"
            );
        }
    }

    /// ...and the loop-continuation predicate agrees with it, for
    /// every variant. Under `ControlFlow::Wait` a variant that
    /// reported `false` here would park the loop with a
    /// throttle-deferred drain still queued.
    #[test]
    fn test_needs_continuation_follows_accumulate_for_every_variant() {
        for mut drag in every_variant() {
            let name = variant_name(&drag);
            assert!(!drag.as_dyn().needs_continuation(), "{name} must start idle");
            drag.as_dyn_mut().accumulate(moved(1.0, 0.0));
            assert!(
                drag.as_dyn().needs_continuation(),
                "{name} left a pending drain unreachable under ControlFlow::Wait"
            );
        }
    }

    /// Only the two resize gestures can be started by the right
    /// button, and only when they were told they were. The
    /// right-release path finalizes on this predicate alone now, so a
    /// variant answering `true` by accident would have its gesture
    /// terminated by a stray right-click.
    #[test]
    fn test_started_with_right_defaults_to_false_for_every_variant() {
        for drag in every_variant() {
            assert!(
                !drag.as_dyn().started_with_right(),
                "{} claimed a right-button origin it was not given",
                variant_name(&drag)
            );
        }
    }

    #[test]
    fn test_as_dyn_mut_throttle_mutations_reach_underlying_struct() {
        let inner = MovingNodeInteraction::new(vec!["x".into()], false, std::collections::HashSet::new());
        let mut drag = ThrottledDrag::MovingNode(inner);
        let n = drive_throttle_over_budget(drag.as_dyn_mut().throttle());
        assert!(n > 1, "expected n > 1 after over-budget work, got {}", n);
        // Unwrap the variant and confirm the mutation reached the real
        // struct's throttle, not a transient copy.
        let ThrottledDrag::MovingNode(real) = drag else {
            panic!("variant changed unexpectedly");
        };
        assert_eq!(real.pending.throttle.current_n(), n);
    }

    #[test]
    fn test_default_reset_resets_throttle_only() {
        // The trait's default `reset` impl is "throttle().reset()" and
        // nothing else — pending / domain state must survive. Exercise
        // through a real implementor that does NOT override `reset`.
        let mut inner = MovingNodeInteraction::new(vec!["n".into()], true, std::collections::HashSet::new());
        inner.accumulate(moved(3.0, 4.0));
        drive_throttle_over_budget(&mut inner.pending.throttle);
        assert!(inner.pending.throttle.current_n() > 1);

        // Default reset from the trait.
        (&mut inner as &mut dyn ThrottledInteraction).reset();

        assert_eq!(inner.pending.throttle.current_n(), 1);
        // Pending and domain state are untouched — per the trait contract
        // they belong to the implementor and are expected to be empty
        // already or dropped with `self`.
        assert_eq!(inner.pending.pending_delta(), Vec2::new(3.0, 4.0));
        assert_eq!(inner.pending.total_delta(), Vec2::new(3.0, 4.0));
        assert_eq!(inner.node_ids, vec!["n".to_string()]);
        assert!(inner.individual);
    }

    #[test]
    fn test_should_perform_drain_through_dyn_mut_reflects_underlying_state() {
        // The default `should_perform_drain` body reads through
        // `has_pending()` and `throttle()` — two trait methods that the
        // enum routes via `as_dyn_mut`. If routing returned a stale or
        // wrong-variant borrow the predicate would disagree with the
        // real struct's state.
        let idle = MovingNodeInteraction::new(vec!["n".into()], false, std::collections::HashSet::new());
        let mut idle_drag = ThrottledDrag::MovingNode(idle);
        assert!(
            !idle_drag.as_dyn_mut().should_perform_drain(),
            "idle interaction through dyn_mut must report no drain"
        );

        let mut pending =
            MovingNodeInteraction::new(vec!["n".into()], false, std::collections::HashSet::new());
        pending.accumulate(moved(1.0, 0.0));
        let mut pending_drag = ThrottledDrag::MovingNode(pending);
        assert!(
            pending_drag.as_dyn_mut().should_perform_drain(),
            "pending interaction through dyn_mut must drain on fresh throttle"
        );
    }
}
