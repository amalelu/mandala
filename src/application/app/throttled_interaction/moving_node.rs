// SPDX-License-Identifier: MPL-2.0

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

use std::collections::{HashMap, HashSet};

use glam::Vec2;

use super::super::scene_rebuild::{flush_canvas_scene_buffers, CanvasFrame};
use super::pending::ThrottledPending;
use super::release::{ReleaseCommit, ReleaseRefresh};
use super::{DrainContext, ThrottledDragInteraction, ThrottledInteraction};
use crate::application::document::{apply_drag_delta, apply_drag_delta_and_collect_patches, UndoAction};

/// Drag-to-move state for one or more nodes. `individual = true`
/// (Alt-drag) moves only the anchor nodes; false moves each
/// anchor and all its descendants.
pub(in crate::application::app) struct MovingNodeInteraction {
    /// The node IDs being moved. Single node, or every selected
    /// node (shift+drag).
    pub node_ids: Vec<String>,
    /// Alt-drag: move only the anchor node(s), not their subtrees.
    pub individual: bool,
    /// Delta-accumulate pending state plus this gesture's adaptive
    /// throttle.
    pub pending: ThrottledPending,
    /// Snapshot of every descendant of every anchor node. Captured
    /// once at drag start because the descendant set cannot change
    /// mid-drag; reused in every `drain` instead of re-walking the
    /// model with `all_descendants` per frame.
    pub descendant_ids: HashSet<String>,
}

impl MovingNodeInteraction {
    /// Start a fresh move-node drag. The throttle begins at the
    /// default budget and both accumulators start zeroed — the
    /// `CursorMoved` handler fills them through `accumulate`.
    ///
    /// `descendant_ids` must contain every descendant of every node
    /// in `node_ids` when `individual` is `false`; it is ignored when
    /// `individual` is `true`.
    pub(in crate::application::app) fn new(
        node_ids: Vec<String>,
        individual: bool,
        descendant_ids: HashSet<String>,
    ) -> Self {
        Self {
            node_ids,
            individual,
            pending: ThrottledPending::accumulating_deltas(),
            descendant_ids,
        }
    }
}

impl ThrottledInteraction for MovingNodeInteraction {
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
            scene_cache,
            interaction_mode,
            ..
        } = ctx;

        let pending_delta = self.pending.take_delta();
        if let Some(tree) = mindmap_tree.as_mut() {
            // Position-only patch: move the dragged nodes in the
            // arena and patch the renderer's existing text buffers
            // in place. No text reshaping, no font-system lock.
            let mut patches = Vec::new();
            for nid in &self.node_ids {
                apply_drag_delta_and_collect_patches(
                    tree,
                    nid,
                    pending_delta.x,
                    pending_delta.y,
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
            let total = self.pending.total_delta();
            let delta = (total.x, total.y);
            for nid in &self.node_ids {
                offsets.insert(nid.clone(), delta);
            }
            if !self.individual {
                for desc_id in &self.descendant_ids {
                    offsets.insert(desc_id.clone(), delta);
                }
            }

            let frame = CanvasFrame::new(
                doc,
                &offsets,
                interaction_mode.resize_handle_overrides(),
                renderer.camera_zoom(),
            );
            frame.update_connection_trees(scene_cache, app_scene);
            frame.update_border_tree(app_scene);
            frame.update_connection_label_tree(app_scene);
            frame.update_portal_tree(app_scene);
            // The 8 resize handles for a Single-selected dragged
            // node anchor on the node's AABB, so they have to be
            // re-emitted against the in-flight offsets. Without
            // this the handles freeze at the pre-drag position
            // while the node visibly slides under them.
            frame.update_node_resize_handle_tree(app_scene);
            // Section frames render only while in NodeEdit mode.
            // Without this drain call, dragging the active node
            // mid-edit leaves every frame stuck at the pre-drag
            // origin while the node and its borders slide under
            // them — the no-NodeEdit case is already a no-op (the
            // pass emits nothing).
            frame.update_section_frame_tree(app_scene);
            flush_canvas_scene_buffers(app_scene, renderer);
        }
    }
}

impl ThrottledDragInteraction for MovingNodeInteraction {
    /// Release-commit body, renderer-free. See
    /// [`super::release`] for why the canvas work comes back as a
    /// decree instead of running here.
    ///
    /// The pending flush is unconditional on the throttle: on
    /// release the final position must be committed in full even
    /// if the throttle was mid-stretch skipping intermediate
    /// drains.
    ///
    /// The pending slot is *taken*, not peeked, like every other
    /// release core's. `commit_on_release_core` is `&mut self`, so
    /// a second call is expressible — the lifecycle harness can
    /// script two `Release` steps — and peeking would flush the
    /// same delta into the tree twice. This does not make the body
    /// idempotent (the model write below reads the running total,
    /// which no release consumes); it removes the one asymmetry
    /// among the seven cores about whether the pending slot
    /// survives a release. `take_delta` leaves the running total
    /// alone by contract.
    fn commit_on_release_core(&mut self, c: ReleaseCommit<'_>) -> ReleaseRefresh {
        let pending_delta = self.pending.take_delta();
        let had_pending = pending_delta != Vec2::ZERO;
        if had_pending {
            if let Some(tree) = c.mindmap_tree.as_mut() {
                for nid in &self.node_ids {
                    apply_drag_delta(tree, nid, pending_delta.x, pending_delta.y, !self.individual);
                }
            }
        }
        let Some(doc) = c.document.as_mut() else {
            return ReleaseRefresh::None;
        };
        // Sync to model, push undo.
        let total_delta = self.pending.total_delta();
        let dx = total_delta.x as f64;
        let dy = total_delta.y as f64;
        let undo_data = doc.apply_move_multiple(&self.node_ids, dx, dy, self.individual);
        doc.undo_stack.push(UndoAction::MoveNodes {
            original_positions: undo_data,
        });
        doc.dirty = true;

        // Under rapid drag the throttle can skip the last drain or
        // two, leaving `pending_delta` stranded in the accumulator.
        // The flush above syncs the tree and `apply_move_multiple`
        // syncs the model, but the `SceneConnectionCache`'s
        // `pre_clip_positions` still reflect the second-to-last
        // drain's `offsets` (short of the committed position by
        // `pending_delta`). The subsequent rebuild runs with empty
        // offsets, so the cache's fast path returns those stale
        // samples and the edges appear glued to the node's
        // pre-flush position until the next cache-invalidating
        // event (mutation or zoom). Clearing here forces a resample
        // from the now-authoritative model.
        if had_pending {
            c.scene_cache.clear();
        }
        ReleaseRefresh::All
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::test_utils::{
        drive_throttle_over_budget, moved, trait_default_tests_for_throttled_interaction,
    };

    #[test]
    fn test_new_initializes_fields_with_zero_deltas() {
        let i = MovingNodeInteraction::new(vec!["a".to_string(), "b".to_string()], true, HashSet::new());
        assert_eq!(i.node_ids, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(i.pending.pending_delta(), Vec2::ZERO);
        assert_eq!(i.pending.total_delta(), Vec2::ZERO);
        assert!(i.individual);
        assert_eq!(i.pending.throttle.current_n(), 1);
    }

    /// This gesture is wired to the delta-accumulate discipline, not
    /// the cursor latch. Picking the wrong constructor would leave
    /// `has_pending` permanently false under the samples the
    /// cursor-move dispatcher sends, and the drag would silently
    /// never drain.
    #[test]
    fn test_pending_uses_the_delta_accumulate_discipline() {
        let mut i = MovingNodeInteraction::new(vec!["n".into()], false, HashSet::new());
        assert!(!i.has_pending());
        i.accumulate(moved(3.0, -2.0));
        assert!(i.has_pending());
        assert_eq!(i.pending.total_delta(), Vec2::new(3.0, -2.0));
        assert_eq!(i.pending.pending_delta(), Vec2::new(3.0, -2.0));
        assert_eq!(i.pending.peek_cursor(), None);
    }

    #[test]
    fn test_throttle_accessor_reaches_owned_instance() {
        let mut i = MovingNodeInteraction::new(vec!["n".into()], false, HashSet::new());
        drive_throttle_over_budget(i.throttle());
        // The accessor must hand out the field, not a transient copy —
        // mutations through it have to survive into the struct.
        assert!(i.pending.throttle.current_n() > 1);
    }

    #[test]
    fn test_reset_resets_only_throttle() {
        let mut i = MovingNodeInteraction::new(vec!["a".into(), "b".into()], true, HashSet::new());
        i.accumulate(moved(5.0, 7.0));
        drive_throttle_over_budget(&mut i.pending.throttle);
        assert!(i.pending.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.pending.throttle.current_n(), 1);
        // Pending / total / identity survive — reset is throttle-only
        // per the trait's default impl.
        assert_eq!(i.pending.pending_delta(), Vec2::new(5.0, 7.0));
        assert_eq!(i.pending.total_delta(), Vec2::new(5.0, 7.0));
        assert_eq!(i.node_ids, vec!["a".to_string(), "b".to_string()]);
        assert!(i.individual);
    }

    trait_default_tests_for_throttled_interaction! {
        build = || MovingNodeInteraction::new(vec!["n".into()], false, HashSet::new()),
        set_pending = |i: &mut MovingNodeInteraction| {
            i.accumulate(moved(1.0, 0.0));
        },
    }
}
