// SPDX-License-Identifier: MPL-2.0

//! Throttled interaction for the edge grab-handle drag gesture.
//!
//! The user drags one of a selected edge's handles (anchor,
//! midpoint, or control point). Accumulates canvas-space deltas
//! the same way `MovingNode` does; each drain folds the sum into
//! the edge's model state via
//! [`crate::application::app::edge_drag::apply_edge_handle_drag`]
//! and re-emits only the one dirty edge's geometry.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;

use glam::Vec2;

use crate::application::document::EdgeRef;

use super::super::edge_drag::apply_edge_handle_drag;
use super::super::scene_rebuild::{flush_canvas_scene_buffers, CanvasFrame};
use super::pending::ThrottledPending;
use super::release::{ReleaseCommit, ReleaseRefresh};
use super::{DrainContext, ThrottledDragInteraction, ThrottledInteraction};

/// Drag-to-reshape state for one edge's grab-handle.
pub(in crate::application::app) struct EdgeHandleInteraction {
    pub edge_ref: EdgeRef,
    /// Which handle is being dragged. `Midpoint` is only the
    /// initial kind — after the first drain frame inserts a
    /// fresh control point, this mutates in place to
    /// `ControlPoint(0)` so subsequent frames take the CP path.
    pub handle: baumhard::mindmap::tree_builder::EdgeHandleKind,
    /// Full snapshot of the edge at drag start, consumed by the
    /// release path for the `UndoAction::EditEdge` entry and the
    /// no-op skip check.
    pub original: baumhard::mindmap::model::MindEdge,
    /// Canvas-space handle position at drag start. Used to
    /// recompute the handle's new position from an absolute
    /// cursor location, which avoids accumulating drift on
    /// non-control-point handles.
    pub start_handle_pos: Vec2,
    /// Delta-accumulate pending state plus this gesture's adaptive
    /// throttle. The running total is added to `start_handle_pos`
    /// to give the handle's absolute position.
    pub pending: ThrottledPending,
}

impl EdgeHandleInteraction {
    pub(in crate::application::app) fn new(
        edge_ref: EdgeRef,
        handle: baumhard::mindmap::tree_builder::EdgeHandleKind,
        original: baumhard::mindmap::model::MindEdge,
        start_handle_pos: Vec2,
    ) -> Self {
        Self {
            edge_ref,
            handle,
            original,
            start_handle_pos,
            pending: ThrottledPending::accumulating_deltas(),
        }
    }
}

impl ThrottledInteraction for EdgeHandleInteraction {
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
            interaction_mode,
            ..
        } = ctx;

        self.pending.take_delta();
        if let Some(doc) = document.as_mut() {
            let new_handle = apply_edge_handle_drag(
                doc,
                &self.edge_ref,
                self.handle,
                self.start_handle_pos,
                self.pending.total_delta(),
            );
            self.handle = new_handle;

            let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from(&self.edge_ref);
            scene_cache.invalidate_edge(&edge_key);

            let offsets: HashMap<String, (f32, f32)> = HashMap::new();
            let frame = CanvasFrame::new(
                doc,
                &offsets,
                interaction_mode.resize_handle_overrides(),
                renderer.camera_zoom(),
            );
            frame.update_connection_trees(scene_cache, app_scene);
            frame.update_connection_label_tree(app_scene);
            frame.update_portal_tree(app_scene);
            flush_canvas_scene_buffers(app_scene, renderer);
        }
    }
}

impl ThrottledDragInteraction for EdgeHandleInteraction {
    /// Release-commit body, renderer-free. See [`super::release`].
    ///
    /// The drain loop has been writing each new edge state directly
    /// into the model. Before release, flush one last write using
    /// the full `total_delta` (independent of any throttled pending
    /// drain) so the final committed state matches the cursor
    /// position exactly. Reaching this path means the drag
    /// threshold was crossed, so the `EditEdge` undo entry carrying
    /// the pre-drag snapshot is pushed unconditionally.
    ///
    /// `original` is cloned rather than moved out so the body can
    /// take `&mut self` like every other release commit; one clone
    /// per gesture end.
    fn commit_on_release_core(&mut self, c: ReleaseCommit<'_>) -> ReleaseRefresh {
        let Some(doc) = c.document.as_mut() else {
            return ReleaseRefresh::None;
        };
        apply_edge_handle_drag(
            doc,
            &self.edge_ref,
            self.handle,
            self.start_handle_pos,
            self.pending.total_delta(),
        );
        // Crossing the drag threshold guarantees a state change, so
        // commit unconditionally.
        doc.commit_throttled_edge_drag(&self.edge_ref, self.original.clone(), |_, _| true);
        ReleaseRefresh::All
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::test_utils::{
        drive_throttle_over_budget, fixture_edge, moved, trait_default_tests_for_throttled_interaction,
    };
    use baumhard::mindmap::tree_builder::EdgeHandleKind;

    fn fixture_interaction() -> EdgeHandleInteraction {
        EdgeHandleInteraction::new(
            EdgeRef::new("a", "b", "parent_child"),
            EdgeHandleKind::AnchorFrom,
            fixture_edge(),
            Vec2::new(10.0, 20.0),
        )
    }

    #[test]
    fn test_new_initializes_fields_with_zero_deltas() {
        let i = fixture_interaction();
        assert_eq!(i.edge_ref.from_id, "a");
        assert_eq!(i.edge_ref.to_id, "b");
        assert_eq!(i.edge_ref.edge_type, "parent_child");
        assert_eq!(i.handle, EdgeHandleKind::AnchorFrom);
        assert_eq!(i.start_handle_pos, Vec2::new(10.0, 20.0));
        assert_eq!(i.pending.pending_delta(), Vec2::ZERO);
        assert_eq!(i.pending.total_delta(), Vec2::ZERO);
        assert_eq!(i.pending.throttle.current_n(), 1);
    }

    /// Delta-accumulate discipline — see the `MovingNode`
    /// counterpart for why the wiring is worth its own test.
    #[test]
    fn test_pending_uses_the_delta_accumulate_discipline() {
        let mut i = fixture_interaction();
        assert!(!i.has_pending());
        i.accumulate(moved(0.0, 3.0));
        assert!(i.has_pending());
        assert_eq!(i.pending.total_delta(), Vec2::new(0.0, 3.0));
        assert_eq!(i.pending.peek_cursor(), None);
    }

    #[test]
    fn test_reset_resets_only_throttle() {
        let mut i = fixture_interaction();
        i.accumulate(moved(1.0, 2.0));
        drive_throttle_over_budget(&mut i.pending.throttle);
        assert!(i.pending.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.pending.throttle.current_n(), 1);
        assert_eq!(i.pending.pending_delta(), Vec2::new(1.0, 2.0));
        assert_eq!(i.pending.total_delta(), Vec2::new(1.0, 2.0));
        assert_eq!(i.start_handle_pos, Vec2::new(10.0, 20.0));
        assert_eq!(i.handle, EdgeHandleKind::AnchorFrom);
    }

    trait_default_tests_for_throttled_interaction! {
        build = fixture_interaction,
        set_pending = |i: &mut EdgeHandleInteraction| {
            i.accumulate(moved(2.0, 0.0));
        },
    }

    #[test]
    fn test_handle_variant_round_trips_control_point() {
        // Midpoint is only the initial kind — the drag promotes it to
        // ControlPoint(0) on first drain. The constructor accepts any
        // variant; verify a non-trivial one round-trips through `new`.
        let i = EdgeHandleInteraction::new(
            EdgeRef::new("a", "b", "parent_child"),
            EdgeHandleKind::Midpoint,
            fixture_edge(),
            Vec2::ZERO,
        );
        assert_eq!(i.handle, EdgeHandleKind::Midpoint);
    }
}
