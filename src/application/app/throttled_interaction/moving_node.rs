//! Throttled interaction for the `MovingNode` drag gesture.
//!
//! Accumulates incremental canvas-space deltas from every
//! `CursorMoved` event; the per-frame `drain()` body applies the
//! summed delta to the tree's node positions and rebuilds the
//! edges, borders, labels, portals, and edge-handles that track
//! the moving subtrees. Pending state is an accumulating
//! `Vec2`; skipped frames leave the accumulator intact so the
//! next successful drain folds in everything that arrived in the
//! meantime.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;

use glam::Vec2;

use crate::application::document::apply_drag_delta_and_collect_patches;
use crate::application::frame_throttle::MutationFrequencyThrottle;

use super::super::scene_rebuild::{
    flush_canvas_scene_buffers, update_border_tree_with_offsets,
    update_connection_label_tree, update_connection_tree, update_edge_handle_tree,
    update_portal_tree,
};
use super::{DrainContext, ThrottledInteraction};

/// Drag-to-move state for one or more nodes. `individual = true`
/// (Alt-drag) moves only the anchor nodes; false moves each
/// anchor and all its descendants.
pub(in crate::application::app) struct MovingNodeInteraction {
    /// The node IDs being moved. Single node, or every selected
    /// node (shift+drag).
    pub node_ids: Vec<String>,
    /// Accumulated total delta in canvas coords. Used on release
    /// to sync the final position back to the model.
    pub total_delta: Vec2,
    /// Delta accumulated since the last successful drain. Folded
    /// into the tree and reset to `Vec2::ZERO` in `drain`.
    pub pending_delta: Vec2,
    /// Alt-drag: move only the anchor node(s), not their subtrees.
    pub individual: bool,
    /// Per-interaction adaptive throttle. See
    /// [`crate::application::frame_throttle`].
    pub throttle: MutationFrequencyThrottle,
}

impl MovingNodeInteraction {
    /// Start a fresh move-node drag. The throttle begins at the
    /// default budget; `pending_delta` and `total_delta` start
    /// zeroed — the `CursorMoved` handler fills them.
    pub(in crate::application::app) fn new(
        node_ids: Vec<String>,
        individual: bool,
    ) -> Self {
        Self {
            node_ids,
            total_delta: Vec2::ZERO,
            pending_delta: Vec2::ZERO,
            individual,
            throttle: MutationFrequencyThrottle::with_default_budget(),
        }
    }
}

impl ThrottledInteraction for MovingNodeInteraction {
    fn has_pending(&self) -> bool {
        self.pending_delta != Vec2::ZERO
    }

    fn throttle(&mut self) -> &mut MutationFrequencyThrottle {
        &mut self.throttle
    }

    fn drain(&mut self, ctx: DrainContext<'_>) {
        let DrainContext {
            document,
            mindmap_tree,
            app_scene,
            renderer,
            scene_cache,
            ..
        } = ctx;

        if let Some(tree) = mindmap_tree.as_mut() {
            // Position-only patch: move the dragged nodes in the
            // arena and patch the renderer's existing text buffers
            // in place. No text reshaping, no font-system lock.
            let mut patches = Vec::new();
            for nid in &self.node_ids {
                apply_drag_delta_and_collect_patches(
                    tree,
                    nid,
                    self.pending_delta.x,
                    self.pending_delta.y,
                    !self.individual,
                    &mut patches,
                );
            }
            renderer.patch_drag_positions(&patches);
            renderer.rebuild_node_backgrounds_from_tree(&tree.tree);
        }

        // Rebuild connections and borders with position offsets.
        //
        // Use the cache-aware scene build so only edges whose
        // endpoints appear in `offsets` get re-sampled. The
        // renderer's keyed rebuild methods then only re-shape the
        // buffers for dirty elements; stable elements have just
        // their `pos` patched in place.
        if let Some(doc) = document.as_ref() {
            let mut offsets: HashMap<String, (f32, f32)> = HashMap::new();
            let delta = (self.total_delta.x, self.total_delta.y);
            for nid in &self.node_ids {
                offsets.insert(nid.clone(), delta);
                if !self.individual {
                    for desc_id in doc.mindmap.all_descendants(nid) {
                        offsets.insert(desc_id, delta);
                    }
                }
            }

            let scene = doc.build_scene_with_cache(
                &offsets,
                scene_cache,
                renderer.camera_zoom(),
            );

            update_connection_tree(&scene, app_scene);
            update_border_tree_with_offsets(doc, &offsets, app_scene);
            update_connection_label_tree(&scene, app_scene, renderer);
            update_portal_tree(doc, &offsets, app_scene, renderer);
            update_edge_handle_tree(&scene, app_scene);
            flush_canvas_scene_buffers(app_scene, renderer);
        }

        self.pending_delta = Vec2::ZERO;
    }
}
