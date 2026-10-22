// SPDX-License-Identifier: MPL-2.0

//! Throttled drag-to-resize state for one node's `(position, size)`.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::mindmap::model::{Position, Size};
use baumhard::mindmap::tree_builder::{build_node_resize_handles, ResizeHandleSide};
use glam::Vec2;

use crate::application::document::apply_node_resize_to_tree;

use super::super::scene_rebuild::{flush_canvas_scene_buffers, update_node_resize_handle_tree_from_slice};
use super::pending::ThrottledPending;
use super::release::{ReleaseCommit, ReleaseRefresh};
use super::{DrainContext, ThrottledDragInteraction, ThrottledInteraction};

/// Per-frame drains apply a side-aware delta to the node's
/// `position` and `size` in the tree only; the model is unchanged
/// until release-commit, where `set_node_aabb` writes the final
/// state under a single `EditNodeAabb` undo entry.
pub(in crate::application::app) struct NodeResizeInteraction {
    pub node_id: String,
    pub side: ResizeHandleSide,
    /// Node's `position` at drag start.
    pub start_position: Position,
    /// Node's `size` at drag start.
    pub start_size: Size,
    /// Delta-accumulate pending state plus this gesture's adaptive
    /// throttle. The running total is what `resolve` folds into the
    /// start AABB.
    pub pending: ThrottledPending,
    /// `true` when the drag was started by the right mouse
    /// button (fast-resize gesture from `PendingRight`); `false`
    /// when it was started by the left button (handle drag).
    /// The right-button release path uses this to skip
    /// finalizing left-button drags when the user accidentally
    /// clicks the right button mid-drag.
    pub started_with_right: bool,
    /// Scratch `(unique_id, new_position)` buffer handed to
    /// [`apply_node_resize_to_tree`] and then to
    /// `Renderer::patch_drag_positions`. Lives on the interaction so
    /// the one allocation is made at drag start and reused for the
    /// rest of the gesture — the reuse the out-param shape exists
    /// for, which a `Vec::new()` per drained frame did not honor.
    /// `apply_node_resize_to_tree` clears it on entry, so nothing
    /// from the previous frame survives into this one.
    patches: Vec<(usize, (f32, f32))>,
}

impl NodeResizeInteraction {
    pub(in crate::application::app) fn new(
        node_id: String,
        side: ResizeHandleSide,
        start_position: Position,
        start_size: Size,
        started_with_right: bool,
    ) -> Self {
        Self {
            node_id,
            side,
            start_position,
            start_size,
            pending: ThrottledPending::accumulating_deltas(),
            started_with_right,
            patches: Vec::new(),
        }
    }

    pub fn resolve(&self, total_delta: Vec2) -> (Position, Size) {
        self.side
            .resolve_aabb(self.start_position, self.start_size, total_delta)
    }

    /// Which gesture produced this drag, for the log lines below.
    /// Users grepping logs for "rejected" can tell a handle-driven
    /// left-button resize from a right-button corner-anchored one.
    fn gesture_label(&self) -> &'static str {
        if self.started_with_right {
            "fast-resize node"
        } else {
            "node resize"
        }
    }
}

impl ThrottledInteraction for NodeResizeInteraction {
    fn pending(&self) -> &ThrottledPending {
        &self.pending
    }

    fn pending_mut(&mut self) -> &mut ThrottledPending {
        &mut self.pending
    }

    fn drain(&mut self, ctx: DrainContext<'_>) {
        let DrainContext {
            mindmap_tree,
            app_scene,
            renderer,
            ..
        } = ctx;

        let pending_delta = self.pending.take_delta();
        if let Some(tree) = mindmap_tree.as_mut() {
            let (new_position, new_size) = self.resolve(self.pending.total_delta());
            let canvas_pos = Vec2::new(new_position.x as f32, new_position.y as f32);
            let canvas_size = Vec2::new(new_size.width as f32, new_size.height as f32);
            // Per-frame *incremental* position delta — the
            // section children store absolute canvas coords and
            // need to track the container's per-frame movement.
            // For pure E/S/SE drags `axis_factors == (>=0, >=0)`
            // and the cumulative position stays at `start_position`,
            // so this delta is `(0, 0)` and sections don't shift.
            let (fx, fy) = self.side.axis_factors();
            let pending_pos_delta = Vec2::new(
                if fx == -1 { pending_delta.x } else { 0.0 },
                if fy == -1 { pending_delta.y } else { 0.0 },
            );
            // What this frame wrote, and nothing else, is what gets
            // refreshed: the container's bounds changed so it is
            // re-shaped; the section elements only moved, so their
            // existing buffers get their positions patched with no
            // shaping and no font-system lock — the same split the
            // move-drag path uses. `rebuild_buffers_from_tree`, which
            // this replaced, re-shaped every text buffer on the map
            // per drained frame.
            let container = apply_node_resize_to_tree(
                tree,
                &self.node_id,
                canvas_pos,
                canvas_size,
                pending_pos_delta,
                &mut self.patches,
            );
            if let Some(container) = container {
                renderer.reshape_buffer_for(container, &tree.tree);
            }
            renderer.patch_drag_positions(&self.patches);
            // Section fills moved with their sections; the
            // container's own rect was re-collected by the reshape
            // above. This walk is position/color reads only — no
            // shaping — and is the same call the move-drag path
            // makes for the same reason.
            renderer.rebuild_node_backgrounds_from_tree(&tree.tree);
            let elements = build_node_resize_handles(&self.node_id, canvas_pos, canvas_size);
            update_node_resize_handle_tree_from_slice(&elements, app_scene);
            flush_canvas_scene_buffers(app_scene, renderer);
        }
    }
}

impl ThrottledDragInteraction for NodeResizeInteraction {
    /// Release-commit body, renderer-free. See [`super::release`].
    ///
    /// Writes the resolved `(position, size)` through
    /// `set_node_aabb` (atomic, single `EditNodeAabb` undo entry).
    /// Rejection (NaN, non-positive size, astronomical) logs and
    /// falls through to [`ReleaseRefresh::All`] from the unchanged
    /// model, so the node snaps back to its pre-drag AABB.
    ///
    /// Single-source for both the left-release and right-release
    /// finalization paths — pre-fix, the two arms held byte-near
    /// duplicates of this body. CODE_CONVENTIONS §5.
    fn commit_on_release_core(&mut self, c: ReleaseCommit<'_>) -> ReleaseRefresh {
        let Some(doc) = c.document.as_mut() else {
            return ReleaseRefresh::None;
        };
        let (new_position, new_size) = self.resolve(self.pending.total_delta());
        match doc.set_node_aabb(&self.node_id, new_position, new_size) {
            Ok(true) => {}
            Ok(false) => {
                log::debug!(
                    "{} release committed no-op on '{}'",
                    self.gesture_label(),
                    self.node_id
                );
            }
            Err(msg) => {
                log::info!(
                    "{} release rejected: {} (snapping back)",
                    self.gesture_label(),
                    msg
                );
            }
        }
        c.scene_cache.clear();
        ReleaseRefresh::All
    }

    /// Right-button fast-resize gestures are finalized by the
    /// right-button release; left-button handle drags are not.
    fn started_with_right(&self) -> bool {
        self.started_with_right
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::test_utils::{
        drive_throttle_over_budget, moved, trait_default_tests_for_throttled_interaction,
    };

    fn fixture(side: ResizeHandleSide) -> NodeResizeInteraction {
        NodeResizeInteraction::new(
            "n".to_string(),
            side,
            Position { x: 100.0, y: 50.0 },
            Size {
                width: 200.0,
                height: 80.0,
            },
            // Test fixture default: not started by right
            // mouse button (the released-tested path is the
            // common left-button-handle drag).
            false,
        )
    }

    #[test]
    fn test_new_initializes_fields() {
        let i = fixture(ResizeHandleSide::SE);
        assert_eq!(i.node_id, "n");
        assert_eq!(i.side, ResizeHandleSide::SE);
        assert_eq!(i.start_position.x, 100.0);
        assert_eq!(i.start_size.width, 200.0);
        assert_eq!(i.pending.pending_delta(), Vec2::ZERO);
        assert_eq!(i.pending.total_delta(), Vec2::ZERO);
    }

    #[test]
    fn test_resolve_se_grows_size_only() {
        let i = fixture(ResizeHandleSide::SE);
        let (pos, size) = i.resolve(Vec2::new(20.0, 10.0));
        assert_eq!(pos.x, 100.0);
        assert_eq!(pos.y, 50.0);
        assert_eq!(size.width, 220.0);
        assert_eq!(size.height, 90.0);
    }

    #[test]
    fn test_resolve_nw_shifts_position_and_shrinks_size() {
        let i = fixture(ResizeHandleSide::NW);
        let (pos, size) = i.resolve(Vec2::new(5.0, 4.0));
        assert_eq!(pos.x, 105.0);
        assert_eq!(pos.y, 54.0);
        assert_eq!(size.width, 195.0);
        assert_eq!(size.height, 76.0);
    }

    #[test]
    fn test_resolve_n_moves_y_axis_only() {
        let i = fixture(ResizeHandleSide::N);
        let (pos, size) = i.resolve(Vec2::new(10.0, 5.0));
        assert_eq!(pos.x, 100.0);
        assert_eq!(size.width, 200.0);
        assert_eq!(pos.y, 55.0);
        assert_eq!(size.height, 75.0);
    }

    /// NE handle: x grows, y shifts position and shrinks.
    #[test]
    fn test_resolve_ne_combines_x_grow_and_y_shrink() {
        let i = fixture(ResizeHandleSide::NE);
        let (pos, size) = i.resolve(Vec2::new(8.0, 6.0));
        assert_eq!(pos.x, 100.0);
        assert_eq!(pos.y, 56.0);
        assert_eq!(size.width, 208.0);
        assert_eq!(size.height, 74.0);
    }

    /// SW handle: x shifts position and shrinks, y grows.
    #[test]
    fn test_resolve_sw_combines_x_shrink_and_y_grow() {
        let i = fixture(ResizeHandleSide::SW);
        let (pos, size) = i.resolve(Vec2::new(5.0, 7.0));
        assert_eq!(pos.x, 105.0);
        assert_eq!(pos.y, 50.0);
        assert_eq!(size.width, 195.0);
        assert_eq!(size.height, 87.0);
    }

    #[test]
    fn test_resolve_e_grows_x_axis_only() {
        let i = fixture(ResizeHandleSide::E);
        let (pos, size) = i.resolve(Vec2::new(7.0, 3.0));
        assert_eq!(pos.x, 100.0);
        assert_eq!(pos.y, 50.0);
        assert_eq!(size.width, 207.0);
        assert_eq!(size.height, 80.0);
    }

    /// Delta-accumulate discipline — see the `MovingNode`
    /// counterpart for why the wiring is worth its own test.
    #[test]
    fn test_pending_uses_the_delta_accumulate_discipline() {
        let mut i = fixture(ResizeHandleSide::SE);
        assert!(!i.has_pending());
        i.accumulate(moved(1.0, 0.0));
        assert!(i.has_pending());
        assert_eq!(i.pending.total_delta(), Vec2::new(1.0, 0.0));
        assert_eq!(i.pending.peek_cursor(), None);
    }

    #[test]
    fn test_reset_resets_only_throttle() {
        let mut i = fixture(ResizeHandleSide::SE);
        i.accumulate(moved(11.0, 13.0));
        drive_throttle_over_budget(&mut i.pending.throttle);
        assert!(i.pending.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.pending.throttle.current_n(), 1);
        assert_eq!(i.pending.pending_delta(), Vec2::new(11.0, 13.0));
        assert_eq!(i.pending.total_delta(), Vec2::new(11.0, 13.0));
    }

    trait_default_tests_for_throttled_interaction! {
        build = || fixture(ResizeHandleSide::SE),
        set_pending = |i: &mut NodeResizeInteraction| {
            i.accumulate(moved(1.0, 0.0));
        },
    }

    /// Whole-PR opus review T2: pin the origin-button gate.
    /// A stray right-click during a left-button resize must not
    /// terminate the resize prematurely (gated in
    /// `event_mouse_click.rs`'s right-release path).
    #[test]
    fn test_left_handle_drag_marks_started_with_right_false() {
        let i = fixture(ResizeHandleSide::SE);
        assert!(!i.started_with_right);
        // ...and the trait predicate the right-release path reads
        // agrees with the field.
        assert!(!ThrottledDragInteraction::started_with_right(&i));
    }

    #[test]
    fn test_fast_resize_marks_started_with_right_true() {
        let i = NodeResizeInteraction::new(
            "n".to_string(),
            ResizeHandleSide::SE,
            Position { x: 0.0, y: 0.0 },
            Size {
                width: 100.0,
                height: 50.0,
            },
            true,
        );
        assert!(i.started_with_right);
        assert!(ThrottledDragInteraction::started_with_right(&i));
    }
}
