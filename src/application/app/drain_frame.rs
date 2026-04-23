//! Per-frame drain helpers for the non-throttled paths in the
//! `AboutToWait` arm of the native event loop. Each function
//! handles one self-contained drain block and takes its mutable
//! state dependencies as parameters so the event-loop body stays a
//! thin dispatcher.
//!
//! Throttled, continuous-input-driven drains (node drag, edge-
//! handle drag, portal-label drag, edge-label drag, color-picker
//! hover) moved under [`super::throttled_interaction`] behind the
//! unified [`ThrottledInteraction`](super::throttled_interaction::ThrottledInteraction)
//! trait; what remains here are the three paths that deliberately
//! skip the throttle:
//!
//! - [`drain_selecting_rect`] — rubber-band overlay + preview
//!   highlight. Lightweight enough to run every frame.
//! - [`drain_camera_geometry_rebuild`] — gated by its own
//!   `take_connection_geometry_dirty` / viewport-dirty flags on
//!   the renderer; layering the mutation throttle on top would
//!   add gating without reducing work.
//! - [`drain_animation_tick`] — paced by `now_ms()` and the
//!   animation's own timing envelope, not mutation frequency.

#![cfg(not(target_arch = "wasm32"))]

use super::*;

/// Update selection rectangle overlay + preview highlight (once per frame)
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
        apply_tree_highlights(
            &mut new_tree,
            preview_selection
                .selected_ids()
                .into_iter()
                .map(|id| (id, HIGHLIGHT_COLOR)),
        );
        renderer.rebuild_buffers_from_tree(&new_tree.tree);
        *mindmap_tree = Some(new_tree);
    }
}

/// Camera (pan/zoom/resize) changed -- rebuild
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
/// next mouse-move flush -- an acceptable tradeoff to
/// keep the two dirty sources separate. Always take
/// the flags (even when skipped) so they don't leak
/// across drag frames.
pub(super) fn drain_camera_geometry_rebuild(
    is_moving_node: bool,
    document: &Option<MindMapDocument>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let geometry_dirty = renderer.take_connection_geometry_dirty();
    let viewport_dirty = renderer.take_connection_viewport_dirty();
    if (geometry_dirty || viewport_dirty) && !is_moving_node {
        if let Some(doc) = document.as_ref() {
            if geometry_dirty {
                // `ensure_zoom` inside
                // `build_scene_with_cache` would also
                // catch this, but clearing explicitly
                // here keeps the ordering readable
                // next to the rebuild.
                scene_cache.clear();
            }
            let scene = doc.build_scene_with_cache(
                &HashMap::new(),
                scene_cache,
                renderer.camera_zoom(),
            );
            update_connection_tree(&scene, app_scene);
            update_connection_label_tree(&scene, app_scene, renderer);
            update_portal_tree(
                doc,
                &HashMap::new(),
                app_scene,
                renderer,
            );
            // Edge handles (if an edge is selected) must
            // also follow camera changes -- scroll-wheel
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
/// actually advanced -- sleeping in Poll mode when no animations
/// are active is automatic.
pub(super) fn drain_animation_tick(
    document: &mut Option<MindMapDocument>,
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
            rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
        }
    }
}
