// SPDX-License-Identifier: MPL-2.0

//! Throttled drag-to-move state for one section's `offset`.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::mindmap::tree_builder::build_section_resize_handles;
use glam::Vec2;

use crate::application::document::apply_section_drag_delta_and_collect_patches;

use super::super::scene_rebuild::{flush_canvas_scene_buffers, update_section_resize_handle_tree_from_slice};
use super::pending::ThrottledPending;
use super::release::{ReleaseCommit, ReleaseRefresh};
use super::{DrainContext, ThrottledDragInteraction, ThrottledInteraction};

/// Per-frame drains mutate the section's tree subtree only; the
/// model is unchanged until release-commit, where
/// `set_section_offset` writes the final offset and pushes a
/// single `EditNodeStyle` undo entry. Drag callers that bypass
/// this discipline and call the setter per-frame would explode
/// the undo stack.
pub(in crate::application::app) struct MovingSectionInteraction {
    pub node_id: String,
    pub section_idx: usize,
    /// Section's `offset` at drag start; release-commit adds
    /// `total_delta` to this and writes the result via
    /// `set_section_offset`.
    pub start_offset: (f64, f64),
    /// Delta-accumulate pending state plus this gesture's adaptive
    /// throttle. The running total is what release folds into
    /// `start_offset`.
    pub pending: ThrottledPending,
    /// Scratch `(unique_id, new_position)` buffer handed to
    /// [`apply_section_drag_delta_and_collect_patches`] and then to
    /// `Renderer::patch_drag_positions`. Lives on the interaction so
    /// the one allocation is made at drag start and reused for the
    /// rest of the gesture — the reuse the out-param shape exists
    /// for, which a `Vec::new()` per drained frame did not honor.
    patches: Vec<(usize, (f32, f32))>,
}

impl MovingSectionInteraction {
    pub(in crate::application::app) fn new(
        node_id: String,
        section_idx: usize,
        start_offset: (f64, f64),
    ) -> Self {
        Self {
            node_id,
            section_idx,
            start_offset,
            pending: ThrottledPending::accumulating_deltas(),
            patches: Vec::new(),
        }
    }
}

impl ThrottledInteraction for MovingSectionInteraction {
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

        let pending_delta = self.pending.take_delta();
        let total_delta = self.pending.total_delta();
        if let Some(tree) = mindmap_tree.as_mut() {
            // The collector appends rather than clearing, so the
            // reused buffer is emptied here before it is refilled.
            self.patches.clear();
            apply_section_drag_delta_and_collect_patches(
                tree,
                &self.node_id,
                self.section_idx,
                pending_delta.x,
                pending_delta.y,
                &mut self.patches,
            );
            renderer.patch_drag_positions(&self.patches);
            // The section was Section-selected at threshold-cross
            // (otherwise no `MovingSection` promotion), so the
            // selection-gated resize handles render at the
            // pre-drag offset. Refresh their positions in lockstep
            // with the section's tree-side movement; without this
            // the 8 handles freeze in place while the section
            // visibly slides under them.
            if let Some(doc) = document.as_ref() {
                if let Some(node) = doc.mindmap.nodes.get(&self.node_id) {
                    if let Some(section) = node.sections.get(self.section_idx) {
                        let canvas_pos = Vec2::new(
                            node.position.x as f32 + section.offset.x as f32 + total_delta.x,
                            node.position.y as f32 + section.offset.y as f32 + total_delta.y,
                        );
                        let canvas_size = section
                            .size
                            .as_ref()
                            .map(|s| Vec2::new(s.width as f32, s.height as f32));
                        let elements = build_section_resize_handles(
                            &self.node_id,
                            self.section_idx,
                            canvas_pos,
                            canvas_size,
                        );
                        update_section_resize_handle_tree_from_slice(&elements, app_scene);
                    }
                }
            }
            // Container/connections/borders/portals untouched —
            // those anchor to `node.position` which the section
            // drag doesn't change.
            //
            // Section frames are deliberately also untouched here:
            // they're built from `MindSection.offset`, which the
            // section drag patches in the rendered tree but not in
            // the model (the patch path is `patch_drag_positions`,
            // not a model setter). A per-frame rebuild would either
            // need a transient model write — unsafe per-tick — or a
            // dedicated tree-side patcher mirroring
            // `apply_section_drag_delta_and_collect_patches`. The
            // dragged section's frame visibly catches up at drag
            // release when the model write lands and the next full
            // `build_section_frames` runs. For sibling sections the
            // drag is a no-op anyway. Pre-MovingNode-fix the same
            // gap existed for whole-node drags too; the MovingNode
            // drain now calls `update_section_frame_tree` because
            // there `offsets` is a real, in-flight map.
            flush_canvas_scene_buffers(app_scene, renderer);
        }
    }
}

impl ThrottledDragInteraction for MovingSectionInteraction {
    /// Release-commit body, renderer-free. See [`super::release`].
    ///
    /// A single setter call; AABB overflow rejection logs and falls
    /// through to [`ReleaseRefresh::All`], which rebuilds the tree
    /// from the unchanged model and snaps the section back.
    fn commit_on_release_core(&mut self, c: ReleaseCommit<'_>) -> ReleaseRefresh {
        let Some(doc) = c.document.as_mut() else {
            return ReleaseRefresh::None;
        };
        let total_delta = self.pending.total_delta();
        let new_x = self.start_offset.0 + total_delta.x as f64;
        let new_y = self.start_offset.1 + total_delta.y as f64;
        match doc.set_section_offset(&self.node_id, self.section_idx, new_x, new_y) {
            Ok(true) => {}
            Ok(false) => {
                log::debug!(
                    "section drag committed no-op offset on '{}' section[{}]",
                    self.node_id,
                    self.section_idx
                );
            }
            Err(msg) => {
                log::info!("section drag release rejected: {} (snapping back)", msg);
            }
        }
        // Unconditional clear so the rebuild resamples from the
        // authoritative model — the per-frame drain mutated the
        // tree (and therefore stale scene-cache samples) regardless
        // of which arm above ran.
        c.scene_cache.clear();
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
        let i = MovingSectionInteraction::new("0".to_string(), 1, (10.0, 10.0));
        assert_eq!(i.node_id, "0");
        assert_eq!(i.section_idx, 1);
        assert_eq!(i.start_offset, (10.0, 10.0));
        assert_eq!(i.pending.pending_delta(), Vec2::ZERO);
        assert_eq!(i.pending.total_delta(), Vec2::ZERO);
        assert_eq!(i.pending.throttle.current_n(), 1);
    }

    /// Delta-accumulate discipline — see the `MovingNode`
    /// counterpart for why the wiring is worth its own test.
    #[test]
    fn test_pending_uses_the_delta_accumulate_discipline() {
        let mut i = MovingSectionInteraction::new("n".into(), 0, (0.0, 0.0));
        assert!(!i.has_pending());
        i.accumulate(moved(3.0, -2.0));
        assert!(i.has_pending());
        assert_eq!(i.pending.total_delta(), Vec2::new(3.0, -2.0));
        assert_eq!(i.pending.peek_cursor(), None);
    }

    #[test]
    fn test_reset_resets_only_throttle() {
        let mut i = MovingSectionInteraction::new("n".into(), 1, (5.0, 7.0));
        i.accumulate(moved(11.0, 13.0));
        drive_throttle_over_budget(&mut i.pending.throttle);
        assert!(i.pending.throttle.current_n() > 1);

        i.reset();

        assert_eq!(i.pending.throttle.current_n(), 1);
        // Pending / total / identity survive — reset is throttle-only.
        assert_eq!(i.pending.pending_delta(), Vec2::new(11.0, 13.0));
        assert_eq!(i.pending.total_delta(), Vec2::new(11.0, 13.0));
        assert_eq!(i.node_id, "n");
        assert_eq!(i.section_idx, 1);
        assert_eq!(i.start_offset, (5.0, 7.0));
    }

    #[test]
    fn test_throttle_accessor_reaches_owned_instance() {
        let mut i = MovingSectionInteraction::new("n".into(), 0, (0.0, 0.0));
        drive_throttle_over_budget(i.throttle());
        assert!(i.pending.throttle.current_n() > 1);
    }

    trait_default_tests_for_throttled_interaction! {
        build = || MovingSectionInteraction::new("n".into(), 0, (0.0, 0.0)),
        set_pending = |i: &mut MovingSectionInteraction| {
            i.accumulate(moved(1.0, 0.0));
        },
    }
}
