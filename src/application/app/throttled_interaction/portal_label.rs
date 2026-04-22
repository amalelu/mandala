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
