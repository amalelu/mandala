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
