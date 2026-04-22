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
//!
//! The ordering predicate lives on
//! [`ThrottledInteraction::should_perform_drain`], independent of
//! the `DrainContext` and the GPU resources it transitively reaches.
//! `drive` is a thin wrapper around that predicate plus the timing
//! envelope, which lets per-implementor tests exercise the
//! ordering against real interaction values without standing up a
//! renderer (see §T8).

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

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::model::MindEdge;
    use baumhard::mindmap::scene_builder::EdgeHandleKind;
    use glam::Vec2;
    use std::time::Duration;

    use crate::application::document::EdgeRef;

    fn fixture_edge() -> MindEdge {
        MindEdge {
            from_id: "a".to_string(),
            to_id: "b".to_string(),
            edge_type: "parent_child".to_string(),
            color: "#888888".to_string(),
            width: 4,
            line_style: "solid".to_string(),
            visible: true,
            label: None,
            label_config: None,
            anchor_from: "auto".to_string(),
            anchor_to: "auto".to_string(),
            control_points: Vec::new(),
            glyph_connection: None,
            display_mode: None,
            portal_from: None,
            portal_to: None,
            min_zoom_to_render: None,
            max_zoom_to_render: None,
        }
    }

    /// Push the throttle's average over-budget until `n > 1`. Returns
    /// the final drain divisor for assertion plumbing.
    fn drive_throttle_over_budget(t: &mut MutationFrequencyThrottle) -> u32 {
        for _ in 0..80 {
            if t.should_drain() {
                t.record_work_duration(Duration::from_micros(50_000));
            }
        }
        t.current_n()
    }

    #[test]
    fn test_as_dyn_mut_routes_to_moving_node() {
        let mut inner = MovingNodeInteraction::new(vec!["x".into()], false);
        // Non-zero pending flips `has_pending` to true; if dispatch
        // reached the wrong struct the bit wouldn't survive.
        inner.pending_delta = Vec2::new(1.0, 0.0);
        let mut drag = ThrottledDrag::MovingNode(inner);
        assert!(drag.as_dyn_mut().has_pending());
    }

    #[test]
    fn test_as_dyn_mut_routes_to_edge_handle() {
        let mut inner = EdgeHandleInteraction::new(
            EdgeRef::new("a", "b", "parent_child"),
            EdgeHandleKind::AnchorFrom,
            fixture_edge(),
            Vec2::ZERO,
        );
        inner.pending_delta = Vec2::new(0.0, 2.0);
        let mut drag = ThrottledDrag::EdgeHandle(inner);
        assert!(drag.as_dyn_mut().has_pending());
    }

    #[test]
    fn test_as_dyn_mut_routes_to_portal_label() {
        let mut inner = PortalLabelInteraction::new(
            EdgeRef::new("a", "b", "parent_child"),
            "a".to_string(),
            fixture_edge(),
        );
        inner.pending_cursor = Some(Vec2::new(10.0, 20.0));
        let mut drag = ThrottledDrag::PortalLabel(inner);
        assert!(drag.as_dyn_mut().has_pending());
    }

    #[test]
    fn test_as_dyn_mut_routes_to_edge_label() {
        let mut inner = EdgeLabelInteraction::new(
            EdgeRef::new("a", "b", "parent_child"),
            fixture_edge(),
        );
        inner.pending_cursor = Some(Vec2::new(5.0, 5.0));
        let mut drag = ThrottledDrag::EdgeLabel(inner);
        assert!(drag.as_dyn_mut().has_pending());
    }

    #[test]
    fn test_as_dyn_mut_throttle_mutations_reach_underlying_struct() {
        let inner = MovingNodeInteraction::new(vec!["x".into()], false);
        let mut drag = ThrottledDrag::MovingNode(inner);
        let n = drive_throttle_over_budget(drag.as_dyn_mut().throttle());
        assert!(n > 1, "expected n > 1 after over-budget work, got {}", n);
        // Unwrap the variant and confirm the mutation reached the real
        // struct's throttle, not a transient copy.
        let ThrottledDrag::MovingNode(real) = drag else {
            panic!("variant changed unexpectedly");
        };
        assert_eq!(real.throttle.current_n(), n);
    }

    #[test]
    fn test_default_reset_resets_throttle_only() {
        // The trait's default `reset` impl is "throttle().reset()" and
        // nothing else — pending / domain state must survive. Exercise
        // through a real implementor that does NOT override `reset`.
        let mut inner = MovingNodeInteraction::new(vec!["n".into()], true);
        inner.pending_delta = Vec2::new(3.0, 4.0);
        inner.total_delta = Vec2::new(7.0, 0.0);
        drive_throttle_over_budget(&mut inner.throttle);
        assert!(inner.throttle.current_n() > 1);

        // Default reset from the trait.
        (&mut inner as &mut dyn ThrottledInteraction).reset();

        assert_eq!(inner.throttle.current_n(), 1);
        // Pending and domain state are untouched — per the trait contract
        // they belong to the implementor and are expected to be empty
        // already or dropped with `self`.
        assert_eq!(inner.pending_delta, Vec2::new(3.0, 4.0));
        assert_eq!(inner.total_delta, Vec2::new(7.0, 0.0));
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
        let mut idle = MovingNodeInteraction::new(vec!["n".into()], false);
        let mut idle_drag = ThrottledDrag::MovingNode(idle);
        assert!(
            !idle_drag.as_dyn_mut().should_perform_drain(),
            "idle interaction through dyn_mut must report no drain"
        );

        idle = MovingNodeInteraction::new(vec!["n".into()], false);
        idle.pending_delta = Vec2::new(1.0, 0.0);
        let mut pending_drag = ThrottledDrag::MovingNode(idle);
        assert!(
            pending_drag.as_dyn_mut().should_perform_drain(),
            "pending interaction through dyn_mut must drain on fresh throttle"
        );
    }
}
