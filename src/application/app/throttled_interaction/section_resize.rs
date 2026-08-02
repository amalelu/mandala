// SPDX-License-Identifier: MPL-2.0

//! Throttled drag-to-resize state for one section's `(offset, size)`.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::mindmap::model::{Position, Size};
use baumhard::mindmap::tree_builder::{build_section_resize_handles, ResizeHandleSide};
use glam::Vec2;

use crate::application::document::apply_section_resize_to_tree;

use super::super::scene_rebuild::{flush_canvas_scene_buffers, update_section_resize_handle_tree_from_slice};
use super::pending::ThrottledPending;
use super::release::{ReleaseCommit, ReleaseRefresh};
use super::{DrainContext, ThrottledDragInteraction, ThrottledInteraction};

/// Per-frame drains apply a side-aware delta to the section's
/// `offset` and `size` in the tree only; the model is unchanged
/// until release-commit, where `set_section_size` (and possibly
/// `set_section_offset` for N/W/NW/NE/SW handles) writes the
/// final state under a single `EditNodeStyle` undo entry.
pub(in crate::application::app) struct SectionResizeInteraction {
    pub node_id: String,
    pub section_idx: usize,
    pub side: ResizeHandleSide,
    /// Section's `offset` at drag start; release-commit folds
    /// `total_delta` through `axis_factors` to compute the final
    /// offset.
    pub start_offset: Position,
    /// Section's `size` at drag start. Always `Some` — the
    /// pre-drag selection-gate filtered out fill-parent
    /// (`None`-sized) sections, since they have no AABB to
    /// resize.
    pub start_size: Size,
    /// Delta-accumulate pending state plus this gesture's adaptive
    /// throttle. The running total is what `resolve` folds into the
    /// start AABB.
    pub pending: ThrottledPending,
    /// `true` when the drag was started by the right mouse
    /// button (fast-resize gesture); `false` for left-button
    /// handle drags. Right-release finalize gates on this so
    /// a stray right-click during a left-button drag doesn't
    /// terminate the resize prematurely.
    pub started_with_right: bool,
}

impl SectionResizeInteraction {
    pub(in crate::application::app) fn new(
        node_id: String,
        section_idx: usize,
        side: ResizeHandleSide,
        start_offset: Position,
        start_size: Size,
        started_with_right: bool,
    ) -> Self {
        Self {
            node_id,
            section_idx,
            side,
            start_offset,
            start_size,
            pending: ThrottledPending::accumulating_deltas(),
            started_with_right,
        }
    }

    pub fn resolve(&self, total_delta: Vec2) -> (Position, Size) {
        self.side
            .resolve_aabb(self.start_offset, self.start_size, total_delta)
    }

    /// Which gesture produced this drag — see
    /// [`super::node_resize::NodeResizeInteraction`]'s counterpart.
    fn gesture_label(&self) -> &'static str {
        if self.started_with_right {
            "fast-resize section"
        } else {
            "section resize"
        }
    }
}

impl ThrottledInteraction for SectionResizeInteraction {
    fn pending(&self) -> &ThrottledPending {
        &self.pending
    }

    fn pending_mut(&mut self) -> &mut ThrottledPending {
        &mut self.pending
    }

    fn drain(&mut self, ctx: DrainContext<'_>) {
        let DrainContext {
            document,
            mindmap_tree,
            app_scene,
            renderer,
            ..
        } = ctx;

        // Per-frame: write the in-progress (offset, size) to the
        // section-area's `GlyphArea`, refresh the 8 handle
        // positions to track the new AABB, and rebuild buffers
        // so cosmic-text reflows the section content against
        // the new bounds. Tree-only — model writes happen at
        // release via `set_section_aabb`. The section's
        // canvas-space position derives from the *current
        // model* node.position plus the in-progress offset, so
        // a concurrent node move doesn't desynchronize mid-drag.
        self.pending.take_delta();
        if let (Some(doc), Some(tree)) = (document.as_ref(), mindmap_tree.as_mut()) {
            let (new_offset, new_size) = self.resolve(self.pending.total_delta());
            let node_pos = doc
                .mindmap
                .nodes
                .get(&self.node_id)
                .map(|n| (n.position.x as f32, n.position.y as f32));
            if let Some((nx, ny)) = node_pos {
                let canvas_pos = Vec2::new(nx + new_offset.x as f32, ny + new_offset.y as f32);
                // Clamp at MIN_DRAG_SIZE_PX. The user can drag past
                // the start size on N/W/NW/SW handles, producing
                // negative size from `resolve_aabb`. Per-frame
                // writes to GlyphArea use the clamped value so
                // cosmic-text doesn't see a negative bound; the
                // release-commit's `set_section_aabb` validator
                // remains the source of truth for what the model
                // accepts.
                const MIN_DRAG_SIZE_PX: f32 = 1.0;
                let canvas_size = Vec2::new(
                    (new_size.width as f32).max(MIN_DRAG_SIZE_PX),
                    (new_size.height as f32).max(MIN_DRAG_SIZE_PX),
                );
                apply_section_resize_to_tree(tree, &self.node_id, self.section_idx, canvas_pos, canvas_size);
                renderer.rebuild_buffers_from_tree(&tree.tree);
                let elements = build_section_resize_handles(
                    &self.node_id,
                    self.section_idx,
                    canvas_pos,
                    Some(canvas_size),
                );
                update_section_resize_handle_tree_from_slice(&elements, app_scene);
                flush_canvas_scene_buffers(app_scene, renderer);
            }
        }
    }
}

impl ThrottledDragInteraction for SectionResizeInteraction {
    /// Release-commit body, renderer-free. See [`super::release`].
    ///
    /// Routes through `set_section_aabb`, which validates the
    /// post-mutation `(offset, size)` against the parent in one
    /// step, so a W-grow gesture (shrink offset, grow width) passes
    /// the right-edge guard the two-step `set_section_size` +
    /// `set_section_offset` path rejected (intermediate state had
    /// new size at old offset, overflowing). Pushes an
    /// `EditNodeStyle` undo entry via the section's parent node
    /// (sections share their owning node's style undo envelope —
    /// see `mutate_section_with_style_undo` in `nodes/`).
    fn commit_on_release_core(&mut self, c: ReleaseCommit<'_>) -> ReleaseRefresh {
        let Some(doc) = c.document.as_mut() else {
            return ReleaseRefresh::None;
        };
        let (new_offset, new_size) = self.resolve(self.pending.total_delta());
        match doc.set_section_aabb(&self.node_id, self.section_idx, new_offset, new_size) {
            Ok(true) => {}
            Ok(false) => {
                log::debug!(
                    "{} release committed no-op on '{}' section[{}]",
                    self.gesture_label(),
                    self.node_id,
                    self.section_idx
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

    /// See [`super::node_resize::NodeResizeInteraction`]'s
    /// counterpart.
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

    fn fixture(side: ResizeHandleSide) -> SectionResizeInteraction {
        SectionResizeInteraction::new(
            "n".to_string(),
            0,
            side,
            Position { x: 10.0, y: 20.0 },
            Size {
                width: 100.0,
                height: 50.0,
            },
            false,
        )
    }

    #[test]
    fn test_new_initializes_fields_with_zero_deltas() {
        let i = fixture(ResizeHandleSide::SE);
        assert_eq!(i.node_id, "n");
        assert_eq!(i.section_idx, 0);
        assert_eq!(i.side, ResizeHandleSide::SE);
        assert_eq!(i.start_offset.x, 10.0);
        assert_eq!(i.start_offset.y, 20.0);
        assert_eq!(i.start_size.width, 100.0);
        assert_eq!(i.start_size.height, 50.0);
        assert_eq!(i.pending.pending_delta(), Vec2::ZERO);
        assert_eq!(i.pending.total_delta(), Vec2::ZERO);
        assert_eq!(i.pending.throttle.current_n(), 1);
    }

    /// SE handle: drag right + down → size grows, offset
    /// unchanged.
    #[test]
    fn test_resolve_se_grows_size_only() {
        let i = fixture(ResizeHandleSide::SE);
        let (off, size) = i.resolve(Vec2::new(20.0, 10.0));
        assert_eq!(off.x, 10.0);
        assert_eq!(off.y, 20.0);
        assert_eq!(size.width, 120.0);
        assert_eq!(size.height, 60.0);
    }

    /// NW handle: drag right + down → offset grows, size
    /// shrinks (so the SE corner stays put).
    #[test]
    fn test_resolve_nw_shifts_offset_and_shrinks_size() {
        let i = fixture(ResizeHandleSide::NW);
        let (off, size) = i.resolve(Vec2::new(5.0, 4.0));
        assert_eq!(off.x, 15.0);
        assert_eq!(off.y, 24.0);
        assert_eq!(size.width, 95.0);
        assert_eq!(size.height, 46.0);
    }

    /// N handle: only y axis moves.
    #[test]
    fn test_resolve_n_moves_y_axis_only() {
        let i = fixture(ResizeHandleSide::N);
        let (off, size) = i.resolve(Vec2::new(10.0, 5.0));
        // x axis is untouched (factor 0).
        assert_eq!(off.x, 10.0);
        assert_eq!(size.width, 100.0);
        // y: offset grows by dy, size shrinks by dy.
        assert_eq!(off.y, 25.0);
        assert_eq!(size.height, 45.0);
    }

    /// E handle: only x axis grows.
    #[test]
    fn test_resolve_e_grows_x_axis_only() {
        let i = fixture(ResizeHandleSide::E);
        let (off, size) = i.resolve(Vec2::new(7.0, 3.0));
        assert_eq!(off.x, 10.0);
        assert_eq!(off.y, 20.0);
        assert_eq!(size.width, 107.0);
        assert_eq!(size.height, 50.0);
    }

    /// NE handle: x grows, y shifts offset and shrinks.
    #[test]
    fn test_resolve_ne_combines_x_grow_and_y_shrink() {
        let i = fixture(ResizeHandleSide::NE);
        let (off, size) = i.resolve(Vec2::new(8.0, 6.0));
        assert_eq!(off.x, 10.0);
        assert_eq!(off.y, 26.0);
        assert_eq!(size.width, 108.0);
        assert_eq!(size.height, 44.0);
    }

    /// SW handle: x shifts offset and shrinks, y grows.
    #[test]
    fn test_resolve_sw_combines_x_shrink_and_y_grow() {
        let i = fixture(ResizeHandleSide::SW);
        let (off, size) = i.resolve(Vec2::new(5.0, 7.0));
        assert_eq!(off.x, 15.0);
        assert_eq!(off.y, 20.0);
        assert_eq!(size.width, 95.0);
        assert_eq!(size.height, 57.0);
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
        assert_eq!(i.side, ResizeHandleSide::SE);
    }

    /// The right-release path finalizes only gestures the right
    /// button started; the trait predicate must report the field.
    #[test]
    fn test_started_with_right_reports_the_origin_button() {
        assert!(!ThrottledDragInteraction::started_with_right(&fixture(
            ResizeHandleSide::SE
        )));
        let fast = SectionResizeInteraction::new(
            "n".to_string(),
            0,
            ResizeHandleSide::SE,
            Position { x: 0.0, y: 0.0 },
            Size {
                width: 100.0,
                height: 50.0,
            },
            true,
        );
        assert!(ThrottledDragInteraction::started_with_right(&fast));
    }

    trait_default_tests_for_throttled_interaction! {
        build = || fixture(ResizeHandleSide::SE),
        set_pending = |i: &mut SectionResizeInteraction| {
            i.accumulate(moved(1.0, 0.0));
        },
    }
}
