// SPDX-License-Identifier: MPL-2.0

//! Per-frame drain helpers for the non-throttled paths in
//! `AboutToWait`. Throttled drains (drag, hover) live under
//! [`super::throttled_interaction`]; what's here are the three paths
//! that deliberately skip the throttle: rect-select overlay,
//! camera-driven geometry rebuild, animation tick.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;

use glam::Vec2;

use super::now_ms;
use super::scene_rebuild::{
    flush_canvas_scene_buffers, rebuild_all, update_connection_label_tree, update_connection_tree,
    update_edge_handle_tree, update_portal_tree,
};
use crate::application::document::{
    apply_tree_highlights, rect_select, MindMapDocument, SelectionState, HIGHLIGHT_COLOR,
};
use crate::application::renderer::Renderer;

pub(super) fn drain_selecting_rect(
    start_canvas: Vec2,
    current_canvas: Vec2,
    document: &Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let sc = start_canvas;
    let cc = current_canvas;
    let min = Vec2::new(sc.x.min(cc.x), sc.y.min(cc.y));
    let max = Vec2::new(sc.x.max(cc.x), sc.y.max(cc.y));
    renderer.rebuild_selection_rect_overlay(min, max);

    // Preview: rebuild tree with intersecting nodes highlighted
    if let Some(doc) = document.as_ref() {
        let mut new_tree = doc.build_tree();
        let hits = rect_select(sc, cc, &new_tree);
        let preview_selection = SelectionState::from_ids(hits);
        // Rect-select preview always whole-node; `from_ids` never
        // produces a `Section` selection so the section narrowing
        // would be `None` here.
        apply_tree_highlights(
            &mut new_tree,
            preview_selection
                .selected_ids()
                .into_iter()
                .map(|id| (id, None, HIGHLIGHT_COLOR)),
        );
        renderer.rebuild_buffers_from_tree(&new_tree.tree);
        *mindmap_tree = Some(new_tree);
    }
}

/// Camera (pan/zoom/resize) changed — rebuild
/// connection buffers against the new viewport. On
/// zoom, the document-side scene cache is also stale
/// because effective font size depends on zoom, so
/// clear it before the rebuild re-samples.
///
/// Skipped when a node drag is in progress: the
/// `MovingNode` drain rebuilds with the drag offsets
/// on its next non-zero `pending_delta` using the
/// current camera, and rebuilding here with empty
/// offsets would flicker dragged connections back to
/// their pre-drag positions for one frame. Wheel-zoom
/// during an active drag with zero `pending_delta`
/// leaves connections stale for one frame until the
/// next mouse-move flush — an acceptable tradeoff to
/// keep the two dirty sources separate. Always take
/// the flags (even when skipped) so they don't leak
/// across drag frames.
pub(super) fn drain_camera_geometry_rebuild(
    is_moving_node: bool,
    document: &Option<MindMapDocument>,
    interaction_mode: &super::InteractionMode,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let geometry_dirty = renderer.take_connection_geometry_dirty();
    if geometry_dirty && !is_moving_node {
        if let Some(doc) = document.as_ref() {
            // `ensure_zoom` inside `build_scene_with_cache` would
            // also catch this, but clearing explicitly here keeps
            // the ordering readable next to the rebuild.
            scene_cache.clear();
            let scene = doc.build_scene_with_cache(
                &HashMap::new(),
                scene_cache,
                renderer.camera_zoom(),
                interaction_mode.resize_handle_overrides(),
            );
            update_connection_tree(&scene, app_scene);
            update_connection_label_tree(&scene, app_scene, renderer);
            update_portal_tree(doc, &HashMap::new(), app_scene, renderer);
            // Edge handles (if an edge is selected) must
            // also follow camera changes — scroll-wheel
            // zoom with a selected edge used to leave
            // the handles pinned to stale screen
            // positions until the next full rebuild.
            update_edge_handle_tree(&scene, app_scene);
            flush_canvas_scene_buffers(app_scene, renderer);
        }
    }
}

/// Tick any active animations. Each tick lerps the from / to
/// snapshots into the model and (on completion) routes the final
/// state through `apply_custom_mutation` so the standard model-sync
/// + undo-push runs once. Drives `rebuild_all` only when something
/// actually advanced. The event loop's `ControlFlow::Wait` /
/// `ControlFlow::Poll` choice is decided in `NativeApp::about_to_wait`
/// from `InitState::needs_continuation`, which factors in
/// `has_active_animations` — so when no animations are active and
/// no other source needs continuation, the loop parks until the
/// next OS event.
pub(super) fn drain_animation_tick(
    document: &mut Option<MindMapDocument>,
    interaction_mode: &super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let animation_advanced = match document.as_mut() {
        Some(doc) if doc.has_active_animations() => {
            doc.tick_animations(now_ms() as u64, mindmap_tree.as_mut())
        }
        _ => false,
    };
    if animation_advanced {
        if let Some(doc) = document.as_ref() {
            // Animation ticks lerp positions (and on completion
            // route through `apply_custom_mutation`) in place; the
            // cache's `pre_clip_positions` go stale under both
            // paths. Clear before re-sampling.
            scene_cache.clear();
            rebuild_all(
                doc,
                interaction_mode,
                mindmap_tree,
                app_scene,
                renderer,
                scene_cache,
            );
        }
    }
}
