//! Per-frame drain helpers extracted from the `AboutToWait` arm of
//! the native event loop in [`super::run_native`]. Each function
//! handles one self-contained drain block and takes its mutable
//! state dependencies as parameters so the event-loop body stays
//! a thin dispatcher.

#![cfg(not(target_arch = "wasm32"))]

use super::*;

/// Flush any accumulated drag delta (once per frame, not per mouse event),
/// gated by the mutation-frequency throttle. When the moving
/// average of this block's work duration exceeds the refresh
/// budget, `should_drain()` starts returning false on some
/// frames -- `pending_delta` stays intact and the next
/// successful drain folds in whatever motion arrived in the
/// meantime. This holds the governing invariant:
/// responsiveness is preserved at the cost of briefer
/// chunking in the visual update cadence.
pub(super) fn drain_moving_node(
    node_ids: &[String],
    pending_delta: &mut Vec2,
    total_delta: &Vec2,
    individual: bool,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    document: &Option<MindMapDocument>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    mutation_throttle: &mut MutationFrequencyThrottle,
) {
    if *pending_delta != Vec2::ZERO && mutation_throttle.should_drain() {
        let work_start = Instant::now();

        if let Some(tree) = mindmap_tree.as_mut() {
            for nid in node_ids {
                apply_drag_delta(tree, nid, pending_delta.x, pending_delta.y, !individual);
            }
            renderer.rebuild_buffers_from_tree(&tree.tree);
        }

        // Rebuild connections and borders with position offsets.
        //
        // Phase 4(B): use the cache-aware scene build so
        // only edges whose endpoints appear in `offsets`
        // get re-sampled. The renderer's keyed rebuild
        // methods then only re-shape the buffers for
        // dirty elements; stable elements have just
        // their `pos` patched in place.
        if let Some(doc) = document.as_ref() {
            let mut offsets: HashMap<String, (f32, f32)> = HashMap::new();
            let delta = (total_delta.x, total_delta.y);
            for nid in node_ids {
                offsets.insert(nid.clone(), delta);
                if !individual {
                    for desc_id in doc.mindmap.all_descendants(nid) {
                        offsets.insert(desc_id, delta);
                    }
                }
            }

            // The tree-path connection rebuild
            // currently re-shapes every edge each
            // frame. The per-edge incremental
            // shaping cache that would let us
            // narrow this to a "dirty" set
            // (computed from `scene_cache.
            // edges_touching(nid)` per moved node)
            // is a known regression tracked
            // separately; until it lands the full
            // rebuild below is correct, and we
            // skip the dead bookkeeping that would
            // feed it.
            let scene = doc.build_scene_with_cache(
                &offsets,
                scene_cache,
                renderer.camera_zoom(),
            );

            update_connection_tree(&scene, app_scene);
            // Borders go through the canvas-scene
            // tree path; drag offsets land on the
            // tree builder via this helper. The
            // legacy keyed border path used to
            // patch positions in place (cheap)
            // while the tree path re-shapes every
            // border (more expensive) -- that's a
            // known regression to address with a
            // tree-side incremental cache, see the
            // unified-rendering plan Session 2f.
            update_border_tree_with_offsets(doc, &offsets, app_scene);
            // Labels are emitted per frame (not
            // cached) so their positions track the
            // live drag.
            update_connection_label_tree(&scene, app_scene, renderer);
            // Portal markers also track the live drag.
            update_portal_tree(
                doc, &offsets, app_scene, renderer,
            );
            // Edge handles (anchor / midpoint /
            // control-point ◆ glyphs on a selected
            // edge) must also track the live drag.
            // Without this the handles stay pinned
            // to the pre-drag positions until mouse
            // release triggers a full rebuild.
            update_edge_handle_tree(&scene, app_scene);
            // Single buffer-walk after the batch.
            flush_canvas_scene_buffers(app_scene, renderer);
        }

        *pending_delta = Vec2::ZERO;
        mutation_throttle.record_work_duration(work_start.elapsed());
    }
}

/// Session 6C edge-handle drag drain. Mirrors the
/// MovingNode drain above but writes the edge in
/// place instead of moving nodes. The scene cache
/// is invalidated for the single dirty edge so the
/// next build re-samples just that edge; everything
/// else rides the incremental rebuild path.
pub(super) fn drain_edge_handle(
    edge_ref: &EdgeRef,
    handle: &mut baumhard::mindmap::scene_builder::EdgeHandleKind,
    pending_delta: &mut Vec2,
    total_delta: &Vec2,
    start_handle_pos: &Vec2,
    document: &mut Option<MindMapDocument>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    mutation_throttle: &mut MutationFrequencyThrottle,
) {
    if *pending_delta != Vec2::ZERO && mutation_throttle.should_drain() {
        let work_start = Instant::now();
        if let Some(doc) = document.as_mut() {
            let new_handle = apply_edge_handle_drag(
                doc,
                edge_ref,
                *handle,
                *start_handle_pos,
                *total_delta,
            );
            *handle = new_handle;

            let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
                &edge_ref.from_id,
                &edge_ref.to_id,
                &edge_ref.edge_type,
            );
            scene_cache.invalidate_edge(&edge_key);

            // Single dirty edge -- the one being
            // dragged. Once the tree-path gains
            // an incremental shaping cache (see
            // the matching note in the MovingNode
            // drain above), we'll thread this in
            // via `scene_cache.invalidate_edge`
            // already called above.
            let _ = edge_key;
            let offsets: HashMap<String, (f32, f32)> = HashMap::new();

            let scene = doc.build_scene_with_cache(
                &offsets,
                scene_cache,
                renderer.camera_zoom(),
            );
            update_connection_tree(&scene, app_scene);
            update_edge_handle_tree(&scene, app_scene);
            // Labels are rebuilt per frame so a
            // control-point drag keeps the label
            // correctly anchored to the live path.
            update_connection_label_tree(&scene, app_scene, renderer);
            update_portal_tree(
                doc,
                &std::collections::HashMap::new(),
                app_scene,
                renderer,
            );
            flush_canvas_scene_buffers(app_scene, renderer);
        }
        *pending_delta = Vec2::ZERO;
        mutation_throttle.record_work_duration(work_start.elapsed());
    }
}

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
        let preview_selection = match hits.len() {
            0 => SelectionState::None,
            1 => SelectionState::Single(hits.into_iter().next().unwrap()),
            _ => SelectionState::Multi(hits),
        };
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
/// MovingNode branch above rebuilds with the drag
/// offsets on its next non-zero `pending_delta` using
/// the current camera, and rebuilding here with
/// empty offsets would flicker dragged connections
/// back to their pre-drag positions for one frame.
/// Wheel-zoom during an active drag with zero
/// `pending_delta` leaves connections stale for one
/// frame until the next mouse-move flush -- an
/// acceptable tradeoff to keep the two dirty sources
/// separate. Always take the flags (even when
/// skipped) so they don't leak across drag frames.
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

/// Picker hover / chip drain. Mouse-move and
/// chip-focus handlers set `picker_dirty`
/// whenever HSV state changes; this gate runs
/// the actual rebuild at most once per
/// refresh budget. `picker_throttle` self-
/// tunes via the moving-average mechanism so
/// a heavy map (where rebuild_scene_only is
/// expensive) gets fewer drains per second
/// rather than dropping frames.
pub(super) fn drain_color_picker_hover(
    picker_dirty: &mut bool,
    picker_throttle: &mut MutationFrequencyThrottle,
    color_picker_state: &mut crate::application::color_picker::ColorPickerState,
    document: &mut Option<MindMapDocument>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    if *picker_dirty && picker_throttle.should_drain() {
        if let (Some(doc), true) =
            (document.as_mut(), color_picker_state.is_open())
        {
            let work_start = std::time::Instant::now();
            rebuild_scene_only(doc, app_scene, renderer);
            rebuild_color_picker_overlay(
                color_picker_state,
                doc,
                app_scene,
                renderer,
            );
            *picker_dirty = false;
            picker_throttle.record_work_duration(work_start.elapsed());
        } else {
            // Picker closed between event and drain -- drop the dirty flag.
            *picker_dirty = false;
        }
    }
}

/// Phase 4: tick any active animations. Each
/// tick lerps the from / to snapshots into the
/// model and (on completion) routes the final
/// state through `apply_custom_mutation` so the
/// standard model-sync + undo-push runs once.
/// Drives `rebuild_all` only when something
/// actually advanced -- sleeping in Poll mode
/// when no animations are active is automatic.
pub(super) fn drain_animation_tick(
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let animation_advanced = match document.as_mut() {
        Some(doc) if doc.has_active_animations() => {
            doc.tick_animations(now_ms() as u64, mindmap_tree.as_mut())
        }
        _ => false,
    };
    if animation_advanced {
        if let Some(doc) = document.as_ref() {
            rebuild_all(doc, mindmap_tree, app_scene, renderer);
        }
    }
}
