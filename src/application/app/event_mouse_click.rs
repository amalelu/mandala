//! Mouse-input event handler extracted from the native event loop in
//! [`super::run_native`]. Routes left / middle / right button press
//! and release through selection, click, double-click detection, drag
//! start, and drag end (MovingNode, DraggingEdgeHandle, SelectingRect).

#![cfg(not(target_arch = "wasm32"))]

use super::*;

/// Dispatch a `WindowEvent::MouseInput` event. Called from the native
/// event loop with the full mutable state tuple so the match arm in
/// `run_native` stays a one-line delegation.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_mouse_input(
    state: ElementState,
    button: MouseButton,
    cursor_pos: (f64, f64),
    modifiers: &ModifiersState,
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    drag_state: &mut DragState,
    app_mode: &mut AppMode,
    console_state: &mut ConsoleState,
    console_history: &[String],
    label_edit_state: &mut LabelEditState,
    text_edit_state: &mut TextEditState,
    color_picker_state: &mut crate::application::color_picker::ColorPickerState,
    last_click: &mut Option<LastClick>,
    hovered_node: &mut Option<String>,
    mutation_throttle: &mut MutationFrequencyThrottle,
    picker_dirty: &mut bool,
) {
    // The console swallows mouse clicks as a close
    // gesture. Clicking anywhere while open dismisses
    // the console without running a command, mirroring
    // Escape.
    if console_state.is_open() && state == ElementState::Pressed {
        save_console_history(console_history);
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
        return;
    }

    // Glyph-wheel color picker click handling. The
    // picker captures both left- and right-mouse
    // buttons:
    // - LMB on a `DragAnchor` → wheel-move gesture;
    //   on any other hit → preview / commit / chip
    //   focus.
    // - RMB on a `DragAnchor` → wheel-resize
    //   gesture (drag away to grow, toward to shrink).
    //   RMB elsewhere is currently a no-op — only
    //   the empty backdrop region acts as the resize
    //   handle, mirroring the LMB-move convention.
    // Release of either button ends any active
    // gesture. In **Standalone** (persistent
    // palette) mode, clicks outside the picker
    // backdrop fall through to normal dispatch —
    // otherwise the user couldn't select anything
    // else while the palette was open. In
    // **Contextual** mode the picker captures
    // everything; outside-click cancels.
    if color_picker_state.is_open()
        && matches!(button, MouseButton::Left | MouseButton::Right)
    {
        let consumed = if state == ElementState::Pressed {
            if let Some(doc) = document.as_mut() {
                handle_color_picker_click(
                    cursor_pos,
                    button,
                    color_picker_state,
                    doc,
                    mindmap_tree,
                    app_scene,
                    renderer,
                    picker_dirty,
                )
            } else {
                true
            }
        } else {
            // Release — end any active wheel gesture.
            // If no gesture was active (e.g.
            // Standalone + outside-press fell
            // through), this is a no-op and the
            // release should also fall through.
            end_color_picker_gesture(color_picker_state)
        };
        if consumed {
            return;
        }
    }
    match button {
        MouseButton::Middle => {
            if state == ElementState::Pressed {
                *drag_state = DragState::Panning;
            } else {
                *drag_state = DragState::None;
            }
        }
        MouseButton::Left => {
            // In reparent or connect mode, left-click (release) is consumed as
            // a "choose target" gesture and never transitions to Pending/drag.
            if matches!(app_mode, AppMode::Reparent { .. }) {
                if state == ElementState::Released {
                    handle_reparent_target_click(
                        cursor_pos,
                        app_mode,
                        hovered_node,
                        document,
                        mindmap_tree,
                        app_scene,
                        renderer,
                    );
                    // Session 7A: mode-exit via target
                    // click — clear any stale click so
                    // the first post-mode click can't
                    // be paired into a double-click.
                    *last_click = None;
                }
                // Pressed: swallow — do not transition drag state
            } else if matches!(app_mode, AppMode::Connect { .. }) {
                if state == ElementState::Released {
                    handle_connect_target_click(
                        cursor_pos,
                        app_mode,
                        hovered_node,
                        document,
                        mindmap_tree,
                        app_scene,
                        renderer,
                    );
                    *last_click = None;
                }
                // Pressed: swallow
            } else if state == ElementState::Pressed {
                // Hit test to determine if clicking on a node
                let canvas_pos = renderer.screen_to_canvas(
                    cursor_pos.0 as f32,
                    cursor_pos.1 as f32,
                );
                let hit_node = mindmap_tree.as_mut().and_then(|tree| {
                    hit_test(canvas_pos, tree)
                });

                // Double-click detection. If this press within the
                // double-click window matches the previous one (same
                // hit target, within time + distance), dispatch:
                //  - Double-click on a node → open the text editor.
                //  - Double-click on a portal marker → pan the camera
                //    to the OTHER endpoint of the portal-mode edge.
                //  - Double-click on empty space (and no edge
                //    selected) → create a new orphan and edit it.
                //
                // Guard: if the editor is already open on the same
                // hit target, DO NOT re-open it — that would
                // silently discard the in-progress buffer. Let the
                // press fall through; the corresponding release
                // will be swallowed as click-inside.
                let now = now_ms();
                // Resolve the "what was hit" used by double-click
                // detection. Node hits beat portal hits (a node
                // under a portal marker is the more common target);
                // no hit at all is `Empty` — still meaningful for
                // empty-canvas double-click.
                let portal_hit = if hit_node.is_none() {
                    renderer.hit_test_portal(canvas_pos)
                } else {
                    None
                };
                let click_hit: ClickHit = match (&hit_node, &portal_hit) {
                    (Some(id), _) => ClickHit::Node(id.clone()),
                    (None, Some((key, ep))) => ClickHit::PortalMarker {
                        edge: key.clone(),
                        endpoint: ep.clone(),
                    },
                    (None, None) => ClickHit::Empty,
                };
                let already_editing_same_target = text_edit_state
                    .node_id()
                    .map(|id| hit_node.as_deref() == Some(id))
                    .unwrap_or(false);
                let is_dblclick = !already_editing_same_target
                    && last_click
                        .as_ref()
                        .map(|prev| is_double_click(prev, now, cursor_pos, &click_hit))
                        .unwrap_or(false);
                if is_dblclick {
                    *last_click = None;
                    match &click_hit {
                        ClickHit::Node(node_id) => {
                            if let Some(doc) = document.as_mut() {
                                let nid = node_id.clone();
                                doc.selection = SelectionState::Single(nid.clone());
                                // rebuild_all first so the selection
                                // highlight color regions are
                                // applied to the tree. open_text_edit's
                                // subsequent apply_text_edit_to_tree
                                // only touches the Text field of the
                                // target node's GlyphArea (via
                                // DeltaGlyphArea's selective field
                                // application) so the highlight
                                // regions survive untouched.
                                rebuild_all(doc, mindmap_tree, app_scene, renderer);
                                open_text_edit(
                                    &nid,
                                    false,
                                    doc,
                                    text_edit_state,
                                    mindmap_tree,
                                    app_scene,
                                    renderer,
                                );
                            }
                            return;
                        }
                        ClickHit::PortalMarker { edge, endpoint } => {
                            // Portal double-click: pan the camera to
                            // the node "on the other side" of the
                            // portal-mode edge. The hit endpoint is
                            // the node this marker sits above; the
                            // opposite endpoint is the navigation
                            // target.
                            let other_id = if *endpoint == edge.from_id {
                                edge.to_id.clone()
                            } else {
                                edge.from_id.clone()
                            };
                            if let Some(doc) = document.as_ref() {
                                if let Some(node) = doc.mindmap.nodes.get(&other_id) {
                                    let target = glam::Vec2::new(
                                        node.position.x as f32
                                            + node.size.width as f32 * 0.5,
                                        node.position.y as f32
                                            + node.size.height as f32 * 0.5,
                                    );
                                    renderer.set_camera_center(target);
                                }
                            }
                            if let Some(doc) = document.as_mut() {
                                // Keep the edge selected after the
                                // jump so the user can cmd+. to
                                // jump back (via undo on the
                                // camera) or edit the portal
                                // in-place via the console.
                                doc.selection = SelectionState::Edge(
                                    crate::application::document::EdgeRef::new(
                                        &edge.from_id,
                                        &edge.to_id,
                                        &edge.edge_type,
                                    ),
                                );
                                rebuild_all(doc, mindmap_tree, app_scene, renderer);
                            }
                            return;
                        }
                        ClickHit::Empty => {
                            // Empty space: only create an orphan if
                            // no edge was selected (otherwise the
                            // user was probably aiming at the
                            // selected edge).
                            let allow_create = document
                                .as_ref()
                                .map(|d| !matches!(d.selection, SelectionState::Edge(_)))
                                .unwrap_or(false);
                            if allow_create {
                                if let Some(doc) = document.as_mut() {
                                    let new_id = doc.create_orphan_and_select(canvas_pos);
                                    rebuild_all(doc, mindmap_tree, app_scene, renderer);
                                    open_text_edit(
                                        &new_id,
                                        true,
                                        doc,
                                        text_edit_state,
                                        mindmap_tree,
                                        app_scene,
                                        renderer,
                                    );
                                }
                                return;
                            }
                        }
                    }
                }
                *last_click = Some(LastClick {
                    time: now,
                    screen_pos: cursor_pos,
                    hit: click_hit,
                });

                // If an edge is currently selected, check
                // whether the cursor is over one of its
                // grab-handles. This check has precedence
                // over the node hit at threshold-cross
                // time — see the `Pending` → drag
                // transition below. Returns `None` if no
                // edge is selected, nothing is in range,
                // or the hit test infrastructure isn't
                // ready yet.
                let hit_edge_handle = match document.as_ref() {
                    Some(doc) => match &doc.selection {
                        SelectionState::Edge(er) => {
                            let tol = EDGE_HANDLE_HIT_TOLERANCE_PX
                                * renderer.canvas_per_pixel();
                            doc.hit_test_edge_handle(canvas_pos, er, tol)
                                .map(|(kind, _pos)| (er.clone(), kind))
                        }
                        _ => None,
                    },
                    None => None,
                };
                // Portal-label drag capture. Takes precedence
                // over `hit_node` at threshold-cross time so
                // pressing a marker and dragging slides the label
                // along its owning node's border rather than
                // moving the node itself. Captured regardless of
                // current selection — grabbing a marker is a
                // valid first action, not just a follow-up to a
                // prior click.
                let hit_portal_label = match &portal_hit {
                    Some((key, endpoint)) if hit_node.is_none() => {
                        Some((key.clone(), endpoint.clone()))
                    }
                    _ => None,
                };
                *drag_state = DragState::Pending {
                    start_pos: cursor_pos,
                    hit_node,
                    hit_edge_handle,
                    hit_portal_label,
                };
            } else {
                // Released
                match std::mem::replace(drag_state, DragState::None) {
                    DragState::Pending { hit_node, .. } => {
                        // Session 7A: if the node text
                        // editor is open, the release
                        // decides whether to commit or
                        // swallow. If the release lands
                        // inside the edited node's AABB,
                        // keep editing (no commit, no
                        // selection change). Otherwise
                        // commit and fall through.
                        if text_edit_state.is_open() {
                            let release_canvas = renderer.screen_to_canvas(
                                cursor_pos.0 as f32,
                                cursor_pos.1 as f32,
                            );
                            let inside = text_edit_state
                                .node_id()
                                .zip(mindmap_tree.as_ref())
                                .map(|(id, tree)| {
                                    crate::application::document::point_in_node_aabb(
                                        release_canvas, id, tree,
                                    )
                                })
                                .unwrap_or(false);
                            if inside {
                                // Click-inside: keep
                                // editing. Do NOT fall
                                // through to handle_click
                                // (that would change the
                                // selection). Also do
                                // not transition drag
                                // state — the release
                                // is fully consumed.
                                return;
                            }
                            // Click-outside: commit the
                            // edit first, then fall
                            // through to the regular
                            // click path so the new
                            // selection lands.
                            if let Some(doc) = document.as_mut() {
                                close_text_edit(
                                    true,
                                    doc,
                                    text_edit_state,
                                    mindmap_tree,
                                    app_scene,
                                    renderer,
                                );
                            }
                        }
                        // Session 6D: if an edge is selected and
                        // the cursor hits its label, open the
                        // inline label editor instead of
                        // processing a regular click. Takes
                        // precedence over node / edge selection.
                        let mut entered_label_edit = false;
                        if hit_node.is_none() {
                            // First, a read-only check to see
                            // whether we should even call the
                            // editor (hits the selected edge's
                            // label AABB). Split from the
                            // `open_label_edit` call so the
                            // mutable borrow of `document`
                            // doesn't conflict with the
                            // immutable read.
                            let label_edit_target: Option<crate::application::document::EdgeRef> =
                                if let Some(doc) = document.as_ref() {
                                    if let SelectionState::Edge(er) = &doc.selection {
                                        let canvas_pos = renderer.screen_to_canvas(
                                            cursor_pos.0 as f32,
                                            cursor_pos.1 as f32,
                                        );
                                        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
                                            &er.from_id,
                                            &er.to_id,
                                            &er.edge_type,
                                        );
                                        if renderer.hit_test_edge_label(canvas_pos, &edge_key) {
                                            Some(er.clone())
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };
                            if let Some(er_clone) = label_edit_target {
                                if let Some(doc) = document.as_mut() {
                                    open_label_edit(
                                        &er_clone,
                                        doc,
                                        label_edit_state,
                                        app_scene,
                                        renderer,
                                    );
                                    entered_label_edit = true;
                                }
                            }
                        }
                        if !entered_label_edit {
                            handle_click(
                                hit_node,
                                cursor_pos,
                                modifiers.shift_key(),
                                document,
                                mindmap_tree,
                                app_scene,
                                renderer,
                            );
                        }
                    }
                    DragState::MovingNode { node_ids, total_delta, pending_delta, individual } => {
                        // Flush any remaining pending delta to the tree before drop.
                        // This always runs regardless of the throttle — on release
                        // we want the final position committed in full, even if
                        // the throttle was mid-stretch skipping intermediate drains.
                        if pending_delta != Vec2::ZERO {
                            if let Some(tree) = mindmap_tree.as_mut() {
                                for nid in &node_ids {
                                    apply_drag_delta(tree, nid, pending_delta.x, pending_delta.y, !individual);
                                }
                            }
                        }
                        // Drop: sync to model, full rebuild, push undo
                        if let Some(doc) = document.as_mut() {
                            let dx = total_delta.x as f64;
                            let dy = total_delta.y as f64;
                            let undo_data = doc.apply_move_multiple(&node_ids, dx, dy, individual);
                            doc.undo_stack.push(UndoAction::MoveNodes {
                                original_positions: undo_data,
                            });
                            doc.dirty = true;

                            // Full rebuild from model
                            rebuild_all(doc, mindmap_tree, app_scene, renderer);
                        }
                        // Drag ended — reset the throttle so the next drag
                        // starts at n = 1 without inheriting any residual
                        // throttling from this one.
                        mutation_throttle.reset();
                    }
                    DragState::DraggingEdgeHandle { edge_ref, handle, original, start_handle_pos, total_delta, pending_delta: _ } => {
                        // The drain loop has been writing
                        // each new edge state directly
                        // into the model. Before release,
                        // flush one last write using the
                        // full `total_delta` (independent
                        // of any throttled pending drain)
                        // so the final committed state
                        // matches the cursor position
                        // exactly. Reaching this branch
                        // means the drag threshold was
                        // crossed, so push an EditEdge
                        // undo with the pre-drag snapshot
                        // unconditionally.
                        if let Some(doc) = document.as_mut() {
                            apply_edge_handle_drag(
                                doc,
                                &edge_ref,
                                handle,
                                start_handle_pos,
                                total_delta,
                            );
                            if let Some(idx) = doc.edge_index(&edge_ref) {
                                doc.undo_stack.push(UndoAction::EditEdge {
                                    index: idx,
                                    before: original,
                                });
                                doc.dirty = true;
                            }
                            rebuild_all(doc, mindmap_tree, app_scene, renderer);
                        }
                        mutation_throttle.reset();
                    }
                    DragState::DraggingPortalLabel { edge_ref, original, .. } => {
                        // Per-frame CursorMoved already mutated the
                        // edge. Commit with a single EditEdge undo
                        // carrying the pre-drag snapshot, matching
                        // the DraggingEdgeHandle release path. The
                        // no-op check only compares the two fields
                        // this drag can touch (`portal_from` /
                        // `portal_to`) — whole-edge `PartialEq`
                        // would also read `control_points`, whose
                        // derived float equality is fragile under
                        // NaN and unrelated to the drag outcome.
                        if let Some(doc) = document.as_mut() {
                            if let Some(idx) = doc.edge_index(&edge_ref) {
                                let current = &doc.mindmap.edges[idx];
                                if current.portal_from != original.portal_from
                                    || current.portal_to != original.portal_to
                                {
                                    doc.undo_stack.push(UndoAction::EditEdge {
                                        index: idx,
                                        before: original,
                                    });
                                    doc.dirty = true;
                                }
                            }
                            rebuild_all(doc, mindmap_tree, app_scene, renderer);
                        }
                        mutation_throttle.reset();
                    }
                    DragState::SelectingRect { start_canvas, current_canvas } => {
                        // Finalize: select all nodes in the rectangle
                        renderer.clear_overlay_buffers();
                        if let (Some(doc), Some(tree)) = (document.as_mut(), mindmap_tree.as_ref()) {
                            let hits = rect_select(start_canvas, current_canvas, tree);
                            doc.selection = match hits.len() {
                                0 => SelectionState::None,
                                1 => SelectionState::Single(hits.into_iter().next().unwrap()),
                                _ => SelectionState::Multi(hits),
                            };
                            rebuild_all(doc, mindmap_tree, app_scene, renderer);
                        }
                    }
                    DragState::Panning | DragState::None => {}
                }
            }
        }
        _ => {}
    }
}
