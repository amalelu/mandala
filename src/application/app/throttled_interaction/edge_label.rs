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

use glam::Vec2;

use crate::application::document::EdgeRef;
use crate::application::frame_throttle::MutationFrequencyThrottle;

use super::super::edge_label_drag::apply_edge_label_drag;
use super::super::scene_rebuild::{
    flush_canvas_scene_buffers, update_connection_label_tree,
};
use super::{DrainContext, ThrottledInteraction};

/// Drag state for repositioning one line-mode edge's label along
/// its edge path.
pub(in crate::application::app) struct EdgeLabelInteraction {
    pub edge_ref: EdgeRef,
    /// Full pre-drag `MindEdge` snapshot, held for the
    /// `UndoAction::EditEdge` commit and for the no-op skip check
    /// on release (compares `label_config` only).
    pub original: baumhard::mindmap::model::MindEdge,
    /// Latest cursor position in canvas space; overwritten per
    /// event, taken once per successful drain.
    pub pending_cursor: Option<Vec2>,
    pub throttle: MutationFrequencyThrottle,
}

impl EdgeLabelInteraction {
    pub(in crate::application::app) fn new(
        edge_ref: EdgeRef,
        original: baumhard::mindmap::model::MindEdge,
    ) -> Self {
        Self {
            edge_ref,
            original,
            pending_cursor: None,
            throttle: MutationFrequencyThrottle::with_default_budget(),
        }
    }
}

impl ThrottledInteraction for EdgeLabelInteraction {
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
            scene_cache,
            ..
        } = ctx;

        let Some(cursor) = self.pending_cursor.take() else {
            return;
        };

        if let Some(doc) = document.as_mut() {
            let changed = apply_edge_label_drag(doc, &self.edge_ref, cursor);
            if changed {
                // A label move never invalidates connection-path
                // geometry, so every edge hits the cache's fast
                // path and only `build_label_elements` (which
                // runs per-frame regardless) produces new work.
                let scene = doc.build_scene_with_cache(
                    &HashMap::new(),
                    scene_cache,
                    renderer.camera_zoom(),
                );
                update_connection_label_tree(&scene, app_scene, renderer);
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

    fn fixture_interaction() -> EdgeLabelInteraction {
        EdgeLabelInteraction::new(
            EdgeRef::new("a", "b", "parent_child"),
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
        assert_eq!(i.edge_ref.to_id, "b");
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
        i.pending_cursor = Some(Vec2::new(0.5, 0.5));
        assert!(i.has_pending());
    }

    #[test]
    fn test_latest_cursor_overwrites_previous() {
        // Label target is a function of the final cursor only, so
        // intermediate writes must be discarded rather than queued.
        let mut i = fixture_interaction();
        i.pending_cursor = Some(Vec2::new(1.0, 1.0));
        i.pending_cursor = Some(Vec2::new(42.0, -7.0));
        assert_eq!(i.pending_cursor, Some(Vec2::new(42.0, -7.0)));
    }

    #[test]
    fn test_reset_preserves_pending_cursor() {
        let mut i = fixture_interaction();
        i.pending_cursor = Some(Vec2::new(8.0, 9.0));
        drive_throttle_over_budget(&mut i.throttle);
        assert!(i.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.throttle.current_n(), 1);
        assert_eq!(i.pending_cursor, Some(Vec2::new(8.0, 9.0)));
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
