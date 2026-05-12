// SPDX-License-Identifier: MPL-2.0

//! Throttled drag-to-resize state for one node's `(position, size)`.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::mindmap::model::{Position, Size};
use baumhard::mindmap::scene_builder::{build_node_resize_handles, ResizeHandleSide};
use glam::Vec2;

use crate::application::document::apply_node_resize_to_tree;
use crate::application::frame_throttle::MutationFrequencyThrottle;

use super::super::scene_rebuild::{flush_canvas_scene_buffers, update_node_resize_handle_tree_from_slice};
use super::{DrainContext, ThrottledInteraction};

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
    /// Accumulated total delta across the entire drag.
    pub total_delta: Vec2,
    /// Delta accumulated since the last successful drain.
    pub pending_delta: Vec2,
    /// Per-interaction adaptive throttle.
    pub throttle: MutationFrequencyThrottle,
    /// `true` when the drag was started by the right mouse
    /// button (fast-resize gesture from `PendingRight`); `false`
    /// when it was started by the left button (handle drag).
    /// The right-button release path uses this to skip
    /// finalizing left-button drags when the user accidentally
    /// clicks the right button mid-drag.
    pub started_with_right: bool,
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
            total_delta: Vec2::ZERO,
            pending_delta: Vec2::ZERO,
            throttle: MutationFrequencyThrottle::with_default_budget(),
            started_with_right,
        }
    }

    pub fn resolve(&self, total_delta: Vec2) -> (Position, Size) {
        self.side
            .resolve_aabb(self.start_position, self.start_size, total_delta)
    }
}

impl ThrottledInteraction for NodeResizeInteraction {
    fn has_pending(&self) -> bool {
        self.pending_delta != Vec2::ZERO
    }

    fn throttle(&mut self) -> &mut MutationFrequencyThrottle {
        &mut self.throttle
    }

    fn drain(&mut self, ctx: DrainContext<'_>) {
        let DrainContext {
            mindmap_tree,
            app_scene,
            renderer,
            ..
        } = ctx;

        if let Some(tree) = mindmap_tree.as_mut() {
            let (new_position, new_size) = self.resolve(self.total_delta);
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
                if fx == -1 { self.pending_delta.x } else { 0.0 },
                if fy == -1 { self.pending_delta.y } else { 0.0 },
            );
            apply_node_resize_to_tree(tree, &self.node_id, canvas_pos, canvas_size, pending_pos_delta);
            renderer.rebuild_buffers_from_tree(&tree.tree);
            let elements = build_node_resize_handles(&self.node_id, canvas_pos, canvas_size);
            update_node_resize_handle_tree_from_slice(&elements, app_scene);
            flush_canvas_scene_buffers(app_scene, renderer);
        }

        self.pending_delta = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::test_utils::{
        drive_throttle_over_budget, trait_default_tests_for_throttled_interaction,
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
    fn test_new_initialises_fields() {
        let i = fixture(ResizeHandleSide::SE);
        assert_eq!(i.node_id, "n");
        assert_eq!(i.side, ResizeHandleSide::SE);
        assert_eq!(i.start_position.x, 100.0);
        assert_eq!(i.start_size.width, 200.0);
        assert_eq!(i.pending_delta, Vec2::ZERO);
        assert_eq!(i.total_delta, Vec2::ZERO);
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

    #[test]
    fn test_has_pending_false_for_zero_delta() {
        let i = fixture(ResizeHandleSide::SE);
        assert!(!i.has_pending());
    }

    #[test]
    fn test_has_pending_true_for_nonzero_delta() {
        let mut i = fixture(ResizeHandleSide::SE);
        i.pending_delta = Vec2::new(1.0, 0.0);
        assert!(i.has_pending());
    }

    #[test]
    fn test_reset_resets_only_throttle() {
        let mut i = fixture(ResizeHandleSide::SE);
        i.pending_delta = Vec2::new(11.0, 13.0);
        i.total_delta = Vec2::new(17.0, 19.0);
        drive_throttle_over_budget(&mut i.throttle);
        assert!(i.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.throttle.current_n(), 1);
        assert_eq!(i.pending_delta, Vec2::new(11.0, 13.0));
        assert_eq!(i.total_delta, Vec2::new(17.0, 19.0));
    }

    trait_default_tests_for_throttled_interaction! {
        build = || fixture(ResizeHandleSide::SE),
        set_pending = |i: &mut NodeResizeInteraction| {
            i.pending_delta = Vec2::new(1.0, 0.0);
        },
    }

    /// Whole-PR opus review T2: pin the origin-button gate.
    /// A stray right-click during a left-button resize must not
    /// terminate the resize prematurely (gated in
    /// `event_mouse_click.rs`'s right-release path).
    #[test]
    fn left_handle_drag_marks_started_with_right_false() {
        let i = fixture(ResizeHandleSide::SE);
        assert!(!i.started_with_right);
    }

    #[test]
    fn fast_resize_marks_started_with_right_true() {
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
    }
}
