// SPDX-License-Identifier: MPL-2.0

//! Throttled interaction for dragging a portal label along its
//! owning node's border.
//!
//! Cursor events overwrite the `pending_cursor` field — unlike the
//! delta-accumulate drags, intermediate cursor positions carry no
//! information the final border projection needs, so discarding
//! them between drains is both correct and cheaper. The per-frame
//! `drain()` projects the latest cursor onto the node border and
//! writes the resulting `(border_t, perpendicular_offset)` into
//! the edge's `portal_from` / `portal_to`.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::document::EdgeRef;

use super::super::portal_label_drag::apply_portal_label_drag;
use super::super::scene_rebuild::{flush_canvas_scene_buffers, CanvasFrame};
use super::pending::ThrottledPending;
use super::release::{ReleaseCommit, ReleaseRefresh};
use super::{DrainContext, ThrottledDragInteraction, ThrottledInteraction};

/// Drag state for repositioning one portal endpoint along its
/// node's border.
pub(in crate::application::app) struct PortalLabelInteraction {
    pub edge_ref: EdgeRef,
    pub endpoint_node_id: String,
    /// Full pre-drag `MindEdge` snapshot, held for the
    /// `UndoAction::EditEdge` commit and for the no-op skip check
    /// on release (compares `portal_from` / `portal_to` only —
    /// whole-edge `PartialEq` would fold in float-fragile
    /// `control_points`).
    pub original: baumhard::mindmap::model::MindEdge,
    /// Cursor-latch pending state plus this gesture's adaptive
    /// throttle: the latest canvas-space cursor, overwritten per
    /// event and taken once per successful drain.
    pub pending: ThrottledPending,
}

impl PortalLabelInteraction {
    pub(in crate::application::app) fn new(
        edge_ref: EdgeRef,
        endpoint_node_id: String,
        original: baumhard::mindmap::model::MindEdge,
    ) -> Self {
        Self {
            edge_ref,
            endpoint_node_id,
            original,
            pending: ThrottledPending::latching_cursor(),
        }
    }
}

impl ThrottledInteraction for PortalLabelInteraction {
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
            ..
        } = ctx;

        // has_pending guarded the entry; `take` converts it to a
        // concrete `Vec2` and resets the pending slot to `None`
        // in the same step. No unwrap path needed.
        let Some(cursor) = self.pending.take_cursor() else {
            return;
        };

        if let Some(doc) = document.as_mut() {
            let changed = apply_portal_label_drag(doc, &self.edge_ref, &self.endpoint_node_id, cursor);
            if changed {
                let offsets = std::collections::HashMap::new();
                CanvasFrame::new(
                    doc,
                    &offsets,
                    // Portal markers ignore resize-handle /
                    // NodeEdit overrides, so the empty bundle keeps
                    // this drag off the mode-threading path.
                    baumhard::mindmap::tree_builder::InteractionModeOverrides::none(),
                    renderer.camera_zoom(),
                )
                .update_portal_tree(app_scene);
                flush_canvas_scene_buffers(app_scene, renderer);
            }
        }
    }
}

impl ThrottledDragInteraction for PortalLabelInteraction {
    /// Release-commit body, renderer-free. See [`super::release`].
    ///
    /// Flushes the final cursor if one is buffered. `None` means
    /// the last drain already consumed it; `Some` means the
    /// throttle skipped that cursor and release must commit the
    /// user's actual drop position rather than wherever the prior
    /// drain happened to land. Bypasses the throttle — there is no
    /// "next frame" after release.
    ///
    /// The no-op check compares only the two fields this drag
    /// touches (`portal_from` / `portal_to`) — whole-edge
    /// `PartialEq` would fold in float-fragile `control_points`.
    ///
    /// `original` is cloned rather than moved out so the body can
    /// take `&mut self` like every other release commit; one clone
    /// per gesture end.
    fn commit_on_release_core(&mut self, c: ReleaseCommit<'_>) -> ReleaseRefresh {
        let pending_cursor = self.pending.take_cursor();
        if let (Some(doc), Some(cursor)) = (c.document.as_mut(), pending_cursor) {
            apply_portal_label_drag(doc, &self.edge_ref, &self.endpoint_node_id, cursor);
        }
        let Some(doc) = c.document.as_mut() else {
            return ReleaseRefresh::None;
        };
        doc.commit_throttled_edge_drag(&self.edge_ref, self.original.clone(), |c, o| {
            c.portal_from != o.portal_from || c.portal_to != o.portal_to
        });
        ReleaseRefresh::All
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

    fn fixture_interaction() -> PortalLabelInteraction {
        PortalLabelInteraction::new(
            EdgeRef::new("a", "b", "parent_child"),
            "a".to_string(),
            fixture_edge(),
        )
    }

    #[test]
    fn test_new_initializes_pending_cursor_to_none() {
        let i = fixture_interaction();
        assert_eq!(i.edge_ref.from_id, "a");
        assert_eq!(i.endpoint_node_id, "a");
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
        i.accumulate(sample(Vec2::ZERO, Vec2::new(4.0, 5.0)));
        assert!(i.has_pending());
        assert_eq!(i.pending.peek_cursor(), Some(Vec2::new(4.0, 5.0)));
        assert_eq!(i.pending.total_delta(), Vec2::ZERO);
    }

    #[test]
    fn test_latest_cursor_overwrites_previous() {
        // Overwrite discipline — intermediate cursors carry no
        // information the border projection needs, so the pending
        // slot must hold the last write, not accumulate or queue.
        let mut i = fixture_interaction();
        i.accumulate(sample(Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0)));
        i.accumulate(sample(Vec2::new(8.0, 8.0), Vec2::new(9.0, 9.0)));
        assert_eq!(i.pending.peek_cursor(), Some(Vec2::new(9.0, 9.0)));
    }

    #[test]
    fn test_reset_preserves_pending_cursor() {
        // Reset is throttle-only per the trait default; pending state
        // lingers until drain `take`s it or the whole interaction is
        // dropped at drag release.
        let mut i = fixture_interaction();
        i.accumulate(sample(Vec2::new(1.0, 1.0), Vec2::new(2.0, 3.0)));
        drive_throttle_over_budget(&mut i.pending.throttle);
        assert!(i.pending.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.pending.throttle.current_n(), 1);
        assert_eq!(i.pending.peek_cursor(), Some(Vec2::new(2.0, 3.0)));
        assert_eq!(i.endpoint_node_id, "a");
    }

    trait_default_tests_for_throttled_interaction! {
        build = fixture_interaction,
        set_pending = |i: &mut PortalLabelInteraction| {
            i.accumulate(moved(1.0, 1.0));
        },
    }
}
