// SPDX-License-Identifier: MPL-2.0

//! Throttled interaction for dragging a line-mode edge's text
//! label along its connection path.
//!
//! Cursor overwrite discipline identical to `PortalLabelInteraction`
//! — the final `(position_t, perpendicular_offset)` depends only
//! on where the cursor lands, not on the path it took. The rebuild
//! narrows to `update_connection_label_tree` + buffer flush because
//! label moves don't touch node, border, portal, connection-body, or
//! edge-handle trees.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;

use crate::application::document::EdgeRef;

use super::super::edge_label_drag::apply_edge_label_drag;
use super::super::scene_rebuild::{flush_canvas_scene_buffers, CanvasFrame};
use super::pending::ThrottledPending;
use super::release::{ReleaseCommit, ReleaseRefresh};
use super::{DrainContext, ThrottledDragInteraction, ThrottledInteraction};

/// Drag state for repositioning one line-mode edge's label along
/// its edge path.
pub(in crate::application::app) struct EdgeLabelInteraction {
    pub edge_ref: EdgeRef,
    /// Full pre-drag `MindEdge` snapshot, held for the
    /// `UndoAction::EditEdge` commit and for the no-op skip check
    /// on release (compares `label_config` only).
    pub original: baumhard::mindmap::model::MindEdge,
    /// Cursor-latch pending state plus this gesture's adaptive
    /// throttle: the latest canvas-space cursor, overwritten per
    /// event and taken once per successful drain.
    pub pending: ThrottledPending,
}

impl EdgeLabelInteraction {
    pub(in crate::application::app) fn new(
        edge_ref: EdgeRef,
        original: baumhard::mindmap::model::MindEdge,
    ) -> Self {
        Self {
            edge_ref,
            original,
            pending: ThrottledPending::latching_cursor(),
        }
    }
}

impl ThrottledInteraction for EdgeLabelInteraction {
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
            interaction_mode,
            ..
        } = ctx;

        let Some(cursor) = self.pending.take_cursor() else {
            return;
        };

        if let Some(doc) = document.as_mut() {
            let changed = apply_edge_label_drag(doc, &self.edge_ref, cursor);
            if changed {
                // A label move never invalidates connection-path
                // geometry, so the connection role does not need
                // re-projecting at all — only `build_label_elements`
                // (which runs per-frame regardless) produces new
                // work.
                let offsets = HashMap::new();
                CanvasFrame::new(
                    doc,
                    &offsets,
                    interaction_mode.resize_handle_overrides(),
                    renderer.camera_zoom(),
                )
                .update_connection_label_tree(app_scene);
                flush_canvas_scene_buffers(app_scene, renderer);
            }
        }
    }
}

impl ThrottledDragInteraction for EdgeLabelInteraction {
    /// Release-commit body, renderer-free. See [`super::release`].
    ///
    /// Flushes the final cursor if one is buffered — see the portal
    /// counterpart for the rationale. The undo entry is skipped
    /// when nothing actually moved.
    ///
    /// The decree is [`ReleaseRefresh::SceneOnly`]: every per-frame
    /// drain already narrowed to the label tree because node trees
    /// are untouched by a label move, and the release commit is the
    /// same story.
    ///
    /// `original` is cloned rather than moved out so the body can
    /// take `&mut self` like every other release commit; one clone
    /// per gesture end.
    fn commit_on_release_core(&mut self, c: ReleaseCommit<'_>) -> ReleaseRefresh {
        let pending_cursor = self.pending.take_cursor();
        if let (Some(doc), Some(cursor)) = (c.document.as_mut(), pending_cursor) {
            apply_edge_label_drag(doc, &self.edge_ref, cursor);
        }
        let Some(doc) = c.document.as_mut() else {
            return ReleaseRefresh::None;
        };
        doc.commit_throttled_edge_drag(&self.edge_ref, self.original.clone(), |c, o| {
            c.label_config != o.label_config
        });
        ReleaseRefresh::SceneOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::test_utils::{
        drive_throttle_over_budget, fixture_edge, moved, sample,
        trait_default_tests_for_throttled_interaction,
    };
    use glam::Vec2;

    fn fixture_interaction() -> EdgeLabelInteraction {
        EdgeLabelInteraction::new(EdgeRef::new("a", "b", "parent_child"), fixture_edge())
    }

    #[test]
    fn test_new_initializes_pending_cursor_to_none() {
        let i = fixture_interaction();
        assert_eq!(i.edge_ref.from_id, "a");
        assert_eq!(i.edge_ref.to_id, "b");
        assert!(i.pending.peek_cursor().is_none());
        assert_eq!(i.pending.throttle.current_n(), 1);
    }

    /// This gesture is wired to the cursor-latch discipline, not
    /// the delta accumulator: its drain projects an absolute
    /// position, so a summed delta would be meaningless input.
    ///
    /// The sample is the discriminating one — zero delta, non-zero
    /// absolute position, the shape a stationary pointer produces.
    /// A delta-wired gesture reports no pending state at all on it,
    /// so this fails loudly rather than reading back the same
    /// numbers from the wrong half.
    #[test]
    fn test_pending_uses_the_cursor_latch_discipline() {
        let mut i = fixture_interaction();
        assert!(!i.has_pending());
        i.accumulate(sample(Vec2::ZERO, Vec2::new(0.5, 0.5)));
        assert!(i.has_pending());
        assert_eq!(i.pending.peek_cursor(), Some(Vec2::new(0.5, 0.5)));
        assert_eq!(i.pending.total_delta(), Vec2::ZERO);
    }

    #[test]
    fn test_latest_cursor_overwrites_previous() {
        // Label target is a function of the final cursor only, so
        // intermediate writes must be discarded rather than queued.
        let mut i = fixture_interaction();
        i.accumulate(sample(Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0)));
        i.accumulate(sample(Vec2::new(41.0, -8.0), Vec2::new(42.0, -7.0)));
        assert_eq!(i.pending.peek_cursor(), Some(Vec2::new(42.0, -7.0)));
    }

    #[test]
    fn test_reset_preserves_pending_cursor() {
        let mut i = fixture_interaction();
        i.accumulate(sample(Vec2::new(3.0, 3.0), Vec2::new(8.0, 9.0)));
        drive_throttle_over_budget(&mut i.pending.throttle);
        assert!(i.pending.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.pending.throttle.current_n(), 1);
        assert_eq!(i.pending.peek_cursor(), Some(Vec2::new(8.0, 9.0)));
    }

    trait_default_tests_for_throttled_interaction! {
        build = fixture_interaction,
        set_pending = |i: &mut EdgeLabelInteraction| {
            i.accumulate(moved(1.0, 1.0));
        },
    }
}
