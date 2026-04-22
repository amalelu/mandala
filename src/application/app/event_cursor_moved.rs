//! Cursor-move dispatch extracted from the native event loop in
//! [`super::run_native`]. Owns the drag-state transition logic
//! (pending → Panning / MovingNode / SelectingRect /
//! DraggingEdgeHandle / DraggingPortalLabel), plus hover
//! highlights for Reparent / Connect modes and the button-
//! cursor swap for trigger-bearing nodes. Persistent state flows
//! in through [`super::input_context::InputHandlerContext`].

#![cfg(not(target_arch = "wasm32"))]

use super::*;
use super::input_context::InputHandlerContext;
use winit::dpi::PhysicalPosition;
use winit::window::Window;

pub(super) fn handle_cursor_moved(
    position: PhysicalPosition<f64>,
    window: &Window,
    ctx: InputHandlerContext<'_>,
) {
    let InputHandlerContext {
        document,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
        cursor_pos,
        drag_state,
        app_mode,
        color_picker_state,
        hovered_node,
        cursor_is_hand,
        picker_dirty,
        modifiers,
        ..
    } = ctx;
    let prev_pos = *cursor_pos;
    *cursor_pos = (position.x, position.y);
    let cursor_pos_val = *cursor_pos;

    // Glyph-wheel color picker hover preview. Routes
    // mouse-over to the picker hit-test, updates the
    // current HSV in place, and lives-previews the
    // change on the affected edge/portal.
    //
    // Guard on `DragState::None`: if a canvas-side
    // drag is already in flight, do not route the
    // move to the picker.
    if color_picker_state.is_open() && matches!(*drag_state, DragState::None) {
        let consumed = if let Some(doc) = document.as_mut() {
            handle_color_picker_mouse_move(cursor_pos_val, color_picker_state, doc, picker_dirty)
        } else {
            true
        };
        if consumed {
            return;
        }
    }

    // Reparent or Connect mode: hit-test under cursor to update the hover
    // target highlight. Skip the regular drag-state handling.
    if matches!(*app_mode, AppMode::Reparent { .. } | AppMode::Connect { .. }) {
        let new_hover = mindmap_tree.as_mut().and_then(|tree| {
            let canvas_pos =
                renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            hit_test(canvas_pos, tree)
        });
        if new_hover != *hovered_node {
            *hovered_node = new_hover;
            if let Some(doc) = document.as_ref() {
                rebuild_all_with_mode(
                    doc,
                    app_mode,
                    hovered_node.as_deref(),
                    mindmap_tree,
                    app_scene,
                    renderer,
                );
            }
        }
        return;
    }

    // Hand cursor over button-like nodes (nodes with any
    // trigger bindings). Only recomputed when idle — during
    // a drag the cursor should stay as-is.
    if matches!(*drag_state, DragState::None) {
        let over_button = match (document.as_ref(), mindmap_tree.as_mut()) {
            (Some(doc), Some(tree)) => {
                let canvas_pos =
                    renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
                hit_test(canvas_pos, tree)
                    .and_then(|id| doc.mindmap.nodes.get(&id))
                    .map(|n| !n.trigger_bindings.is_empty())
                    .unwrap_or(false)
            }
            _ => false,
        };
        if over_button != *cursor_is_hand {
            window.set_cursor(if over_button {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            });
            *cursor_is_hand = over_button;
        }
    }

    match drag_state {
        DragState::Panning => {
            let dx = cursor_pos_val.0 - prev_pos.0;
            let dy = cursor_pos_val.1 - prev_pos.1;
            renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
        }
        DragState::MovingNode {
            total_delta,
            pending_delta,
            ..
        } => {
            // Convert screen delta to canvas delta and accumulate.
            // Actual mutation + rebuild happens in AboutToWait.
            let old_canvas = renderer.screen_to_canvas(prev_pos.0 as f32, prev_pos.1 as f32);
            let new_canvas =
                renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            let delta = new_canvas - old_canvas;

            *total_delta += delta;
            *pending_delta += delta;
        }
        DragState::DraggingEdgeHandle {
            total_delta,
            pending_delta,
            ..
        } => {
            // Same accumulation pattern as MovingNode —
            // actual edge mutation + buffer rebuild
            // happens in `AboutToWait`.
            let old_canvas = renderer.screen_to_canvas(prev_pos.0 as f32, prev_pos.1 as f32);
            let new_canvas =
                renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            let delta = new_canvas - old_canvas;

            *total_delta += delta;
            *pending_delta += delta;
        }
        DragState::DraggingEdgeLabel { edge_ref, .. } => {
            // Edge-label drag writes `position_t` +
            // `perpendicular_offset` per mouse-move event
            // directly — the state is two f32s on the edge,
            // and the only visual consumer is the label
            // pass, which rebuilds cheaply (one label per
            // labeled edge). Bypasses the per-frame drain /
            // undo push that `MovingNode` / `DraggingEdgeHandle`
            // use; release commits a single `EditEdge` with
            // the pre-drag snapshot.
            let cursor_canvas =
                renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            let edge_ref = edge_ref.clone();
            if let Some(doc) = document.as_mut() {
                let changed = super::edge_label_drag::apply_edge_label_drag(
                    doc,
                    &edge_ref,
                    cursor_canvas,
                );
                if changed {
                    // `rebuild_scene_only` — the label drag mutates
                    // `label_config` on one edge only; node text
                    // buffers, node backgrounds, and border trees
                    // are all untouched. Skipping the tree rebuild
                    // halves the per-drag-frame cost on maps with
                    // many nodes, matching the same "scene-only"
                    // discipline the color-picker hover uses.
                    rebuild_scene_only(doc, app_scene, renderer);
                }
            }
        }
        DragState::DraggingPortalLabel {
            edge_ref,
            endpoint_node_id,
            ..
        } => {
            // Portal label drag writes `border_t` per
            // mouse-move event directly — the state is
            // a single `f32` on the edge, and the only
            // visual consumer is the portal tree, which
            // rebuilds cheaply (O(portal-mode edges)).
            let cursor_canvas =
                renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            let edge_ref = edge_ref.clone();
            let endpoint_node_id = endpoint_node_id.clone();
            if let Some(doc) = document.as_mut() {
                let changed =
                    apply_portal_label_drag(doc, &edge_ref, &endpoint_node_id, cursor_canvas);
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
        DragState::Pending {
            start_pos,
            hit_node,
            hit_edge_handle,
            hit_portal_label,
            hit_edge_label,
        } => {
            let dist_x = cursor_pos_val.0 - start_pos.0;
            let dist_y = cursor_pos_val.1 - start_pos.1;
            if dist_x * dist_x + dist_y * dist_y > 25.0 {
                // Past threshold — promote `Pending` to the
                // appropriate drag variant. At most one of
                // `hit_edge_label` / `hit_portal_label` is set
                // at press time (see `event_mouse_click.rs`'s
                // click-hit chain), so the ordering here only
                // resolves the `hit_edge_handle`-vs-`hit_node`
                // overlap — a handle sits above its edge's
                // nodes, and a handle-grab drag should always
                // beat the node behind it. Consumption order:
                //   edge-label → portal-label → edge-handle →
                //   node (move) → shift-rect-select → pan.
                // Portal-text is intentionally missing: dragging
                // a portal's text sub-part isn't a supported
                // gesture — the icon carries the drag.
                if let Some(edge_key) = hit_edge_label.take() {
                    if let Some(doc) = document.as_mut() {
                        let edge_ref = crate::application::document::EdgeRef::new(
                            &edge_key.from_id,
                            &edge_key.to_id,
                            &edge_key.edge_type,
                        );
                        if let Some(original) = doc
                            .mindmap
                            .edges
                            .iter()
                            .find(|e| edge_ref.matches(e))
                            .cloned()
                        {
                            doc.selection = SelectionState::EdgeLabel(
                                crate::application::document::EdgeLabelSel::new(
                                    edge_ref.clone(),
                                ),
                            );
                            let prev = doc.selection.clone();
                            scene_cache.clear();
                            *drag_state = DragState::DraggingEdgeLabel {
                                edge_ref,
                                original,
                            };
                            // `rebuild_after_selection_change` picks
                            // `rebuild_scene_only` when both the
                            // previous and new selections are edge-
                            // adjacent (no node-tree highlight to
                            // shift). When the user was on a node
                            // before and drag-starts an edge-label
                            // in the same gesture, falls back to a
                            // full rebuild to clear the old node
                            // highlight from the tree's text buffer.
                            rebuild_after_selection_change(
                                &prev,
                                doc,
                                mindmap_tree,
                                app_scene,
                                renderer,
                            );
                            return;
                        }
                    }
                }
                if let Some((edge_key, endpoint)) = hit_portal_label.take() {
                    if let Some(doc) = document.as_mut() {
                        let edge_ref = crate::application::document::EdgeRef::new(
                            &edge_key.from_id,
                            &edge_key.to_id,
                            &edge_key.edge_type,
                        );
                        let original = doc
                            .mindmap
                            .edges
                            .iter()
                            .find(|e| edge_ref.matches(e))
                            .cloned();
                        if let Some(original) = original {
                            doc.selection = SelectionState::PortalLabel(
                                crate::application::document::PortalLabelSel {
                                    edge_key,
                                    endpoint_node_id: endpoint.clone(),
                                },
                            );
                            scene_cache.clear();
                            *drag_state = DragState::DraggingPortalLabel {
                                edge_ref,
                                endpoint_node_id: endpoint,
                                original,
                            };
                            rebuild_all(doc, mindmap_tree, app_scene, renderer);
                            return;
                        }
                    }
                }
                if let Some((edge_ref, handle_kind)) = hit_edge_handle.take() {
                    // Grab the pre-edit snapshot + start
                    // position so the drain loop can do
                    // absolute-positioning math.
                    if let Some(doc) = document.as_mut() {
                        if let Some(original) = doc
                            .mindmap
                            .edges
                            .iter()
                            .find(|e| edge_ref.matches(e))
                            .cloned()
                        {
                            let canvas_pos =
                                renderer.screen_to_canvas(start_pos.0 as f32, start_pos.1 as f32);
                            let start_handle_pos = doc
                                .hit_test_edge_handle(canvas_pos, &edge_ref, f32::INFINITY)
                                .map(|(_, p)| p)
                                .unwrap_or(canvas_pos);
                            scene_cache.clear();
                            *drag_state = DragState::DraggingEdgeHandle {
                                edge_ref,
                                handle: handle_kind,
                                original,
                                start_handle_pos,
                                total_delta: Vec2::ZERO,
                                pending_delta: Vec2::ZERO,
                            };
                            return;
                        }
                    }
                }
                if let Some(node_id) = hit_node.take() {
                    // Ensure the dragged node is selected
                    if let Some(doc) = document.as_mut() {
                        if !doc.selection.is_selected(&node_id) {
                            doc.selection = SelectionState::Single(node_id.clone());
                            if let Some(tree) = mindmap_tree.as_mut() {
                                let mut new_tree = doc.build_tree();
                                apply_tree_highlights(
                                    &mut new_tree,
                                    doc.selection
                                        .selected_ids()
                                        .into_iter()
                                        .map(|id| (id, HIGHLIGHT_COLOR)),
                                );
                                renderer.rebuild_buffers_from_tree(&new_tree.tree);
                                *tree = new_tree;
                            }
                        }
                    }
                    // Shift+drag: move all selected nodes together
                    let node_ids = if modifiers.shift_key() {
                        if let Some(doc) = document.as_ref() {
                            let mut ids: Vec<String> = doc
                                .selection
                                .selected_ids()
                                .iter()
                                .map(|s| s.to_string())
                                .collect();
                            if !ids.contains(&node_id) {
                                ids.push(node_id);
                            }
                            ids
                        } else {
                            vec![node_id]
                        }
                    } else {
                        vec![node_id]
                    };
                    // Start each drag with a clean scene cache so
                    // the keyed-edge rebuild picks up the moving
                    // node's edges from scratch.
                    scene_cache.clear();
                    *drag_state = DragState::MovingNode {
                        node_ids,
                        total_delta: Vec2::ZERO,
                        pending_delta: Vec2::ZERO,
                        individual: modifiers.alt_key(),
                    };
                } else if modifiers.shift_key() {
                    // Shift+drag on empty space: rubber-band selection
                    let start_canvas =
                        renderer.screen_to_canvas(start_pos.0 as f32, start_pos.1 as f32);
                    let current_canvas = renderer
                        .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
                    *drag_state = DragState::SelectingRect {
                        start_canvas,
                        current_canvas,
                    };
                } else {
                    *drag_state = DragState::Panning;
                    let dx = cursor_pos_val.0 - prev_pos.0;
                    let dy = cursor_pos_val.1 - prev_pos.1;
                    renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                }
            }
        }
        DragState::SelectingRect {
            current_canvas, ..
        } => {
            *current_canvas =
                renderer.screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
        }
        DragState::None => {}
    }
}
