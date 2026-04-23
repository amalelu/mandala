//! Mouse-input event handler extracted from the native event loop in
//! [`super::run_native`]. Routes left / middle / right button press
//! and release through selection, click, double-click detection, drag
//! start, and drag end (MovingNode, DraggingEdgeHandle, SelectingRect).

#![cfg(not(target_arch = "wasm32"))]

use super::*;
use super::input_context::InputHandlerContext;
use super::throttled_interaction::ThrottledDrag;

/// Dispatch a `WindowEvent::MouseInput` event. Event payload
/// (`state`, `button`) stays as direct arguments; all persistent
/// app state arrives through [`InputHandlerContext`] instead of the
/// twenty-ref tuple this function used to carry. See that type's
/// header for the rationale.
pub(super) fn handle_mouse_input(
    state: ElementState,
    button: MouseButton,
    ctx: InputHandlerContext<'_>,
) {
    let InputHandlerContext {
        document,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
        drag_state,
        app_mode,
        console_state,
        console_history,
        label_edit_state,
        portal_text_edit_state,
        text_edit_state,
        color_picker_state,
        last_click,
        hovered_node,
        cursor_pos,
        modifiers,
        picker_hover,
        ..
    } = ctx;
    let cursor_pos = *cursor_pos;
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
                    scene_cache,
                    picker_hover,
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
                        scene_cache,
                    );
                    // Mode-exit via target click — clear any stale
                    // click so the first post-mode click can't be
                    // paired into a double-click.
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
                        scene_cache,
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
                // under a portal marker is the more common target).
                // Portal sub-parts are resolved in priority order:
                // text first, then icon — the two AABBs don't
                // overlap in practice but the ordering keeps the
                // routing deterministic if geometry ever places
                // them adjacent.
                let portal_text_hit = if hit_node.is_none() {
                    renderer.hit_test_portal_text(canvas_pos)
                } else {
                    None
                };
                let portal_icon_hit =
                    if hit_node.is_none() && portal_text_hit.is_none() {
                        renderer.hit_test_portal(canvas_pos)
                    } else {
                        None
                    };
                // Edge-label hit only when no node / portal sub-part
                // has claimed the click. Edge labels sit along the
                // connection path; placing them behind the portal
                // check keeps the portal's "floating over a node"
                // behaviour correct even if a label happens to
                // overlap.
                let edge_label_hit = if hit_node.is_none()
                    && portal_text_hit.is_none()
                    && portal_icon_hit.is_none()
                {
                    renderer.hit_test_any_edge_label(canvas_pos)
                } else {
                    None
                };
                let click_hit: ClickHit = if let Some(id) = &hit_node {
                    ClickHit::Node(id.clone())
                } else if let Some((key, ep)) = &portal_text_hit {
                    ClickHit::PortalText {
                        edge: key.clone(),
                        endpoint: ep.clone(),
                    }
                } else if let Some((key, ep)) = &portal_icon_hit {
                    ClickHit::PortalMarker {
                        edge: key.clone(),
                        endpoint: ep.clone(),
                    }
                } else if let Some(key) = &edge_label_hit {
                    ClickHit::EdgeLabel(key.clone())
                } else {
                    ClickHit::Empty
                };
                // Suppress the double-click → open-editor gesture when
                // an editor is already open on the click's target. The
                // three editor states are mutually exclusive by
                // construction (the event-keyboard dispatch steals on
                // whichever is open first), so one match suffices.
                // Without this guard for the label / portal-text
                // editors, a double-click while editing would call
                // `open_label_edit` / `open_portal_text_edit` a second
                // time, which re-seeds the buffer from the committed
                // model value and silently destroys the in-progress
                // edit.
                let already_editing_same_target = {
                    let node_match = text_edit_state
                        .node_id()
                        .map(|id| hit_node.as_deref() == Some(id))
                        .unwrap_or(false);
                    let edge_label_match = label_edit_state
                        .edited_edge_ref()
                        .zip(edge_label_hit.as_ref())
                        .map(|(er, hit)| {
                            hit.from_id == er.from_id.as_str()
                                && hit.to_id == er.to_id.as_str()
                                && hit.edge_type == er.edge_type.as_str()
                        })
                        .unwrap_or(false);
                    let portal_text_match = portal_text_edit_state
                        .edited_endpoint()
                        .zip(portal_text_hit.as_ref())
                        .map(|((er, ep), (hit_key, hit_ep))| {
                            hit_key.from_id == er.from_id.as_str()
                                && hit_key.to_id == er.to_id.as_str()
                                && hit_key.edge_type == er.edge_type.as_str()
                                && hit_ep.as_str() == ep
                        })
                        .unwrap_or(false);
                    node_match || edge_label_match || portal_text_match
                };
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
                                rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
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
                        ClickHit::PortalMarker { edge, endpoint }
                        | ClickHit::PortalText { edge, endpoint } => {
                            // Portal double-click: pan the camera to
                            // the node "on the other side" of the
                            // portal-mode edge. Works identically
                            // for an icon or text double-click —
                            // both share the same endpoint identity
                            // and the same "jump to partner" intent.
                            // The hit endpoint is the node this
                            // marker sits above; the opposite
                            // endpoint is the navigation target.
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
                                rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
                            }
                            return;
                        }
                        ClickHit::EdgeLabel(edge_key) => {
                            // Double-click on an edge label opens
                            // the inline label editor — the "click
                            // to select, dbl-click to edit" idiom
                            // the `Node` variant already follows.
                            // Commit the EdgeLabel selection first
                            // so the editor opens against the
                            // authoritative selection state.
                            if let Some(doc) = document.as_mut() {
                                let er = crate::application::document::EdgeRef::new(
                                    edge_key.from_id.as_str(),
                                    edge_key.to_id.as_str(),
                                    edge_key.edge_type.as_str(),
                                );
                                let prev = doc.selection.clone();
                                doc.selection = SelectionState::EdgeLabel(
                                    crate::application::document::EdgeLabelSel::new(er.clone()),
                                );
                                // Selection-change rebuild picks the
                                // right granularity — scene-only when
                                // both prev and new are edge-adjacent,
                                // full rebuild when transitioning from
                                // a node selection so the old node
                                // highlight clears. `open_label_edit`
                                // below will trigger any further
                                // buffer updates it needs.
                                rebuild_after_selection_change(
                                    &prev,
                                    doc,
                                    mindmap_tree,
                                    app_scene,
                                    renderer,
                                    scene_cache,
                                );
                                open_label_edit(
                                    &er,
                                    doc,
                                    label_edit_state,
                                    app_scene,
                                    renderer,
                                );
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
                                    rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
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
                // Portal **icon** drag captures the `border_t`
                // slide gesture — dragging the text sub-part
                // isn't a supported interaction. Only populate
                // this when the icon-side hit was present.
                let hit_portal_label = match &portal_icon_hit {
                    Some((key, endpoint)) if hit_node.is_none() => {
                        Some((key.clone(), endpoint.clone()))
                    }
                    _ => None,
                };
                // Reuse the press-time edge-label hit captured
                // earlier so the threshold-cross transition can
                // promote to `DraggingEdgeLabel`. Priority
                // ordering in `event_cursor_moved.rs` still
                // gives portal-label / edge-handle drag higher
                // precedence when multiple hits overlap.
                *drag_state = DragState::Pending {
                    start_pos: cursor_pos,
                    hit_node,
                    hit_edge_handle,
                    hit_portal_label,
                    hit_edge_label: edge_label_hit,
                };
            } else {
                // Released
                match std::mem::replace(drag_state, DragState::None) {
                    DragState::Pending { hit_node, hit_edge_label, .. } => {
                        // If the node text editor is open, the
                        // release decides whether to commit or
                        // swallow. If the release lands inside the
                        // edited node's AABB, keep editing (no
                        // commit, no selection change). Otherwise
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
                                    scene_cache,
                                );
                            }
                        }
                        // Same shape for the inline edge-label
                        // editor: a release that doesn't hit the
                        // edge currently being edited commits the
                        // buffer; a release that lands back on
                        // the same edge label keeps the editor
                        // open. Without this branch, the only way
                        // to close the editor was Esc / Enter,
                        // and clicking elsewhere felt unresponsive.
                        // Mirrors the node text editor's behaviour
                        // so the same muscle memory transfers.
                        if label_edit_state.is_open() {
                            let release_canvas = renderer.screen_to_canvas(
                                cursor_pos.0 as f32,
                                cursor_pos.1 as f32,
                            );
                            let edited = label_edit_state.edited_edge_ref().cloned();
                            let stays_on_edited_label = edited
                                .as_ref()
                                .and_then(|er| {
                                    renderer
                                        .hit_test_any_edge_label(release_canvas)
                                        .map(|hit| {
                                            hit.from_id == er.from_id.as_str()
                                                && hit.to_id == er.to_id.as_str()
                                                && hit.edge_type == er.edge_type.as_str()
                                        })
                                })
                                .unwrap_or(false);
                            if stays_on_edited_label {
                                return;
                            }
                            if let Some(doc) = document.as_mut() {
                                close_label_edit(
                                    true,
                                    doc,
                                    label_edit_state,
                                    mindmap_tree,
                                    app_scene,
                                    renderer,
                                    scene_cache,
                                );
                            }
                        }
                        // Portal-text editor uses the portal-text
                        // hitbox instead of the edge-label hitbox,
                        // and matches `(edge_key, endpoint)` rather
                        // than just the edge key — clicking the
                        // *other* endpoint of the same portal edge
                        // commits this side and then routes the
                        // click as a fresh selection on the new
                        // endpoint.
                        if portal_text_edit_state.is_open() {
                            let release_canvas = renderer.screen_to_canvas(
                                cursor_pos.0 as f32,
                                cursor_pos.1 as f32,
                            );
                            let edited = portal_text_edit_state
                                .edited_endpoint()
                                .map(|(er, ep)| (er.clone(), ep.to_string()));
                            let stays_on_edited_text = edited
                                .as_ref()
                                .and_then(|(er, ep)| {
                                    renderer
                                        .hit_test_portal_text(release_canvas)
                                        .map(|(hit_key, hit_ep)| {
                                            hit_key.from_id
                                                == er.from_id.as_str()
                                                && hit_key.to_id
                                                    == er.to_id.as_str()
                                                && hit_key.edge_type
                                                    == er.edge_type.as_str()
                                                && hit_ep == *ep
                                        })
                                })
                                .unwrap_or(false);
                            if stays_on_edited_text {
                                return;
                            }
                            if let Some(doc) = document.as_mut() {
                                close_portal_text_edit(
                                    true,
                                    doc,
                                    portal_text_edit_state,
                                    mindmap_tree,
                                    app_scene,
                                    renderer,
                                    scene_cache,
                                );
                            }
                        }
                        // Edge-label single click: route to the
                        // `EdgeLabel` selection rather than opening
                        // the editor. Matches the "click to select,
                        // dbl-click to edit" idiom the node /
                        // portal-label variants already follow —
                        // the dbl-click branch above handles the
                        // editor-open case.
                        //
                        // Consume the `hit_edge_label` captured at
                        // press time (with its full priority chain:
                        // node > portal_text > portal_icon >
                        // edge_label > edge_body). Re-hit-testing
                        // at release would ignore that chain — a
                        // press that landed on a portal icon but
                        // drifted a few pixels onto an overlapping
                        // edge label before release would mis-
                        // route to `EdgeLabel` instead of the
                        // portal's sub-threshold single-click.
                        let edge_label_target: Option<crate::application::document::EdgeRef> =
                            hit_edge_label.map(|k| {
                                crate::application::document::EdgeRef::new(
                                    k.from_id.as_str(),
                                    k.to_id.as_str(),
                                    k.edge_type.as_str(),
                                )
                            });
                        let entered_label_select =
                            if let Some(er) = edge_label_target {
                                if let Some(doc) = document.as_mut() {
                                    let prev = doc.selection.clone();
                                    doc.selection = SelectionState::EdgeLabel(
                                        crate::application::document::EdgeLabelSel::new(
                                            er,
                                        ),
                                    );
                                    rebuild_after_selection_change(
                                        &prev,
                                        doc,
                                        mindmap_tree,
                                        app_scene,
                                        renderer,
                                        scene_cache,
                                    );
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                        if !entered_label_select {
                            handle_click(
                                hit_node,
                                cursor_pos,
                                modifiers.shift_key(),
                                document,
                                mindmap_tree,
                                app_scene,
                                renderer,
                                scene_cache,
                            );
                        }
                    }
                    DragState::Throttled(ThrottledDrag::MovingNode(i)) => {
                        // Flush any remaining pending delta to the tree before drop.
                        // This always runs regardless of the throttle — on release
                        // we want the final position committed in full, even if
                        // the throttle was mid-stretch skipping intermediate drains.
                        if i.pending_delta != Vec2::ZERO {
                            if let Some(tree) = mindmap_tree.as_mut() {
                                for nid in &i.node_ids {
                                    apply_drag_delta(tree, nid, i.pending_delta.x, i.pending_delta.y, !i.individual);
                                }
                            }
                        }
                        // Drop: sync to model, full rebuild, push undo
                        if let Some(doc) = document.as_mut() {
                            let dx = i.total_delta.x as f64;
                            let dy = i.total_delta.y as f64;
                            let undo_data = doc.apply_move_multiple(&i.node_ids, dx, dy, i.individual);
                            doc.undo_stack.push(UndoAction::MoveNodes {
                                original_positions: undo_data,
                            });
                            doc.dirty = true;

                            // Full rebuild from model
                            rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
                        }
                    }
                    DragState::Throttled(ThrottledDrag::EdgeHandle(i)) => {
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
                        let super::throttled_interaction::EdgeHandleInteraction {
                            edge_ref,
                            handle,
                            original,
                            start_handle_pos,
                            total_delta,
                            ..
                        } = i;
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
                            rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
                        }
                    }
                    DragState::Throttled(ThrottledDrag::PortalLabel(i)) => {
                        // Flush the final cursor if one is buffered.
                        // When `pending_cursor` is `None` the last
                        // drain already consumed it and no flush is
                        // needed; when `Some`, the throttle skipped
                        // that cursor and release must commit the
                        // user's actual drop position rather than
                        // wherever the prior drain happened to land.
                        // Bypasses the throttle — there is no "next
                        // frame" after release.
                        let super::throttled_interaction::PortalLabelInteraction {
                            edge_ref,
                            endpoint_node_id,
                            original,
                            pending_cursor,
                            ..
                        } = i;
                        if let (Some(doc), Some(cursor)) =
                            (document.as_mut(), pending_cursor)
                        {
                            apply_portal_label_drag(
                                doc,
                                &edge_ref,
                                &endpoint_node_id,
                                cursor,
                            );
                        }
                        // Commit with a single EditEdge undo
                        // carrying the pre-drag snapshot, matching
                        // the EdgeHandle release path. The no-op
                        // check compares only the two fields this
                        // drag touches (`portal_from` /
                        // `portal_to`) — whole-edge `PartialEq`
                        // would fold in float-fragile
                        // `control_points`.
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
                            rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
                        }
                    }
                    DragState::Throttled(ThrottledDrag::EdgeLabel(i)) => {
                        // Flush the final cursor if one is buffered.
                        // See the portal release arm above for the
                        // rationale — `None` means the last drain
                        // already caught it, `Some` means the
                        // throttle skipped the final CursorMoved.
                        let super::throttled_interaction::EdgeLabelInteraction {
                            edge_ref,
                            original,
                            pending_cursor,
                            ..
                        } = i;
                        if let (Some(doc), Some(cursor)) =
                            (document.as_mut(), pending_cursor)
                        {
                            super::edge_label_drag::apply_edge_label_drag(
                                doc,
                                &edge_ref,
                                cursor,
                            );
                        }
                        // Commit with a single `EditEdge` carrying
                        // the pre-drag snapshot, skipping the undo
                        // entry if nothing actually moved.
                        if let Some(doc) = document.as_mut() {
                            if let Some(idx) = doc.edge_index(&edge_ref) {
                                let current = &doc.mindmap.edges[idx];
                                if current.label_config != original.label_config {
                                    doc.undo_stack.push(UndoAction::EditEdge {
                                        index: idx,
                                        before: original,
                                    });
                                    doc.dirty = true;
                                }
                            }
                            // Scene-only rebuild: every per-frame
                            // drain already used `rebuild_scene_only`
                            // because node trees are untouched by a
                            // label move; the release commit is
                            // the same story.
                            rebuild_scene_only(doc, app_scene, renderer, scene_cache);
                        }
                    }
                    DragState::SelectingRect { start_canvas, current_canvas } => {
                        // Finalize: select all nodes in the rectangle
                        renderer.clear_overlay_buffers();
                        if let (Some(doc), Some(tree)) = (document.as_mut(), mindmap_tree.as_ref()) {
                            let hits = rect_select(start_canvas, current_canvas, tree);
                            doc.selection = SelectionState::from_ids(hits);
                            rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
                        }
                    }
                    DragState::Panning | DragState::None => {}
                }
            }
        }
        _ => {}
    }
}
