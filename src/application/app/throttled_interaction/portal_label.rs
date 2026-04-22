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

use glam::Vec2;

use crate::application::document::EdgeRef;
use crate::application::frame_throttle::MutationFrequencyThrottle;

use super::super::portal_label_drag::apply_portal_label_drag;
use super::super::scene_rebuild::{flush_canvas_scene_buffers, update_portal_tree};
use super::{DrainContext, ThrottledInteraction};

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
    /// Latest cursor position in canvas space. Overwritten on
    /// every `CursorMoved`; consumed (`None`) at the end of every
    /// successful drain.
    pub pending_cursor: Option<Vec2>,
    pub throttle: MutationFrequencyThrottle,
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
            pending_cursor: None,
            throttle: MutationFrequencyThrottle::with_default_budget(),
        }
    }
}

impl ThrottledInteraction for PortalLabelInteraction {
    fn has_pending(&self) -> bool {
        self.pending_cursor.is_some()
    }

    fn throttle(&mut self) -> &mut MutationFrequencyThrottle {
        &mut self.throttle
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
        let Some(cursor) = self.pending_cursor.take() else {
            return;
        };

        if let Some(doc) = document.as_mut() {
            let changed = apply_portal_label_drag(
                doc,
                &self.edge_ref,
                &self.endpoint_node_id,
                cursor,
            );
            if changed {
                update_portal_tree(
                    doc,
                    &std::collections::HashMap::new(),
                    app_scene,
                    renderer,
                );
                flush_canvas_scene_buffers(app_scene, renderer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::model::MindEdge;
    use std::time::Duration;

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

    fn fixture_interaction() -> PortalLabelInteraction {
        PortalLabelInteraction::new(
            EdgeRef::new("a", "b", "parent_child"),
            "a".to_string(),
            fixture_edge(),
        )
    }

    fn drive_throttle_over_budget(t: &mut MutationFrequencyThrottle) -> u32 {
        for _ in 0..80 {
            if t.should_drain() {
                t.record_work_duration(Duration::from_micros(50_000));
            }
        }
        t.current_n()
    }

    #[test]
    fn test_new_initialises_pending_cursor_to_none() {
        let i = fixture_interaction();
        assert_eq!(i.edge_ref.from_id, "a");
        assert_eq!(i.endpoint_node_id, "a");
        assert!(i.pending_cursor.is_none());
        assert_eq!(i.throttle.current_n(), 1);
    }

    #[test]
    fn test_has_pending_false_when_pending_cursor_is_none() {
        let i = fixture_interaction();
        assert!(!i.has_pending());
    }

    #[test]
    fn test_has_pending_true_when_pending_cursor_is_some() {
        let mut i = fixture_interaction();
        i.pending_cursor = Some(Vec2::new(4.0, 5.0));
        assert!(i.has_pending());
    }

    #[test]
    fn test_latest_cursor_overwrites_previous() {
        // Overwrite discipline — intermediate cursors carry no
        // information the border projection needs, so the pending
        // slot must hold the last write, not accumulate or queue.
        let mut i = fixture_interaction();
        i.pending_cursor = Some(Vec2::new(1.0, 1.0));
        i.pending_cursor = Some(Vec2::new(9.0, 9.0));
        assert_eq!(i.pending_cursor, Some(Vec2::new(9.0, 9.0)));
    }

    #[test]
    fn test_reset_preserves_pending_cursor() {
        // Reset is throttle-only per the trait default; pending state
        // lingers until drain `take`s it or the whole interaction is
        // dropped at drag release.
        let mut i = fixture_interaction();
        i.pending_cursor = Some(Vec2::new(2.0, 3.0));
        drive_throttle_over_budget(&mut i.throttle);
        assert!(i.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.throttle.current_n(), 1);
        assert_eq!(i.pending_cursor, Some(Vec2::new(2.0, 3.0)));
        assert_eq!(i.endpoint_node_id, "a");
    }

    #[test]
    fn test_should_perform_drain_false_when_idle() {
        let mut i = fixture_interaction();
        assert!(!i.should_perform_drain());
    }

    #[test]
    fn test_should_perform_drain_true_when_pending_and_throttle_fresh() {
        let mut i = fixture_interaction();
        i.pending_cursor = Some(Vec2::new(0.0, 0.0));
        assert!(i.should_perform_drain());
    }

    #[test]
    fn test_should_perform_drain_false_when_throttle_skipping() {
        let mut i = fixture_interaction();
        drive_throttle_over_budget(&mut i.throttle);
        assert!(i.throttle.current_n() > 1);

        let n = i.throttle.current_n() as usize;
        let mut saw_skip = false;
        for _ in 0..(n * 2) {
            i.pending_cursor = Some(Vec2::new(0.0, 0.0));
            if !i.should_perform_drain() {
                saw_skip = true;
            }
            i.throttle.record_work_duration(Duration::from_micros(50_000));
        }
        assert!(saw_skip);
    }

    #[test]
    fn test_idle_should_perform_drain_does_not_advance_throttle() {
        let mut i = fixture_interaction();
        for _ in 0..5 {
            assert!(!i.should_perform_drain());
        }
        i.pending_cursor = Some(Vec2::new(1.0, 1.0));
        assert!(i.should_perform_drain());
    }
}
