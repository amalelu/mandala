// SPDX-License-Identifier: MPL-2.0

//! Mouse-input dispatch. Left/middle/right + Pressed/Released routed
//! through selection, double-click, drag start/end, and the
//! console / color-picker steals.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;

use crate::application::platform::input::{ElementState, MouseButton};

use super::click::handle_click;
use super::color_picker_flow::{end_color_picker_gesture, handle_color_picker_click};
use super::console_input::save_console_history;
use super::edge_drag::apply_edge_handle_drag;
use super::input_context::InputHandlerContext;
use super::portal_label_drag::apply_portal_label_drag;
use super::scene_rebuild::{rebuild_after_selection_change, rebuild_all, rebuild_scene_only};
use super::throttled_interaction::ThrottledDrag;
use super::{is_double_click, now_ms, DragState, InteractionMode, LastClick, HANDLE_HIT_TOLERANCE_PX};
use crate::application::console::ConsoleState;
use crate::application::document::{apply_drag_delta, rect_select, SelectionState, UndoAction};
use crate::application::keybinds::Action;

/// Dispatch a `WindowEvent::MouseInput`. Persistent state arrives
/// via [`InputHandlerContext`].
pub(super) fn handle_mouse_input(
    state: ElementState,
    button: MouseButton,
    ctx: &mut InputHandlerContext<'_>,
) {
    let cursor_pos_val = *ctx.cursor_pos;
    // The console swallows mouse clicks as a close
    // gesture. Clicking anywhere while open dismisses
    // the console without running a command, mirroring
    // Escape.
    if ctx.console_state.is_open() && state == ElementState::Pressed {
        save_console_history(ctx.console_history);
        *ctx.console_state = ConsoleState::Closed;
        ctx.renderer.rebuild_console_overlay_buffers(ctx.app_scene, None);
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
    if ctx.color_picker_state.is_open() && matches!(button, MouseButton::Left | MouseButton::Right) {
        let consumed = if state == ElementState::Pressed {
            if let Some(doc) = ctx.document.as_mut() {
                handle_color_picker_click(
                    cursor_pos_val,
                    button,
                    ctx.color_picker_state,
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                    ctx.picker_hover,
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
            end_color_picker_gesture(ctx.color_picker_state)
        };
        if consumed {
            return;
        }
    }
    match button {
        MouseButton::Middle => {
            if state == ElementState::Pressed {
                // Middle-click press: lookup what's bound to MiddleClick
                // (default `PanCanvas`). The dispatch arm sets
                // `DragState::Panning`. Release unconditionally resets
                // drag state below — mirrors today's behaviour where
                // any drag's release goes to None regardless of which
                // gesture started it.
                let name = crate::application::keybinds::MouseGesture::MiddleClick.key_name();
                // Modifier-fallback: Ctrl+MiddleClick matches the bare
                // MiddleClick binding when no exact-modifier match
                // exists. Preserves pre-branch modifier-agnostic
                // behaviour for mouse gestures.
                let action = ctx.keybinds.action_for_gesture(
                    name,
                    ctx.modifiers.control_key(),
                    ctx.modifiers.shift_key(),
                    ctx.modifiers.alt_key(),
                );
                if let Some(a) = action {
                    let _ = super::dispatch::dispatch_action(a, ctx, None);
                }
            } else {
                *ctx.drag_state = DragState::None;
            }
        }
        MouseButton::Left => {
            // In reparent or connect mode, left-click (release) is consumed as
            // a "choose target" gesture and never transitions to Pending/drag.
            // Hit-test inline so the dispatch arm receives a resolved target id;
            // the arms read the source(s) from `ctx.interaction_mode` directly.
            if matches!(ctx.interaction_mode, InteractionMode::Reparent { .. }) {
                if state == ElementState::Released {
                    let target: Option<String> = ctx.mindmap_tree.as_mut().and_then(|tree| {
                        let canvas_pos = ctx
                            .renderer
                            .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
                        crate::application::document::hit_test(canvas_pos, tree)
                    });
                    let _ = super::dispatch::dispatch_action(Action::ReparentToTarget(target), ctx, None);
                    // Mode-exit via target click — clear any stale
                    // click so the first post-mode click can't be
                    // paired into a double-click. Stays here per the
                    // §3 carve-out: pre-funnel state-machine
                    // bookkeeping, not user-named effect.
                    *ctx.last_click = None;
                }
                // Pressed: swallow — do not transition drag state
            } else if matches!(ctx.interaction_mode, InteractionMode::Connect { .. }) {
                if state == ElementState::Released {
                    let target: Option<String> = ctx.mindmap_tree.as_mut().and_then(|tree| {
                        let canvas_pos = ctx
                            .renderer
                            .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
                        crate::application::document::hit_test(canvas_pos, tree)
                    });
                    // `target = None` (empty-canvas) and
                    // `target = Some(id)` both flow through the
                    // funnel; the arm body owns the mode-exit
                    // rebuild on either branch. Symmetric with
                    // `Action::ReparentToTarget` (also takes
                    // `Option<String>`).
                    let _ = super::dispatch::dispatch_action(Action::ConnectToTarget(target), ctx, None);
                    // Mode-exit via target click — clear any stale
                    // click so the first post-mode click can't be
                    // paired into a double-click. Stays here per
                    // the §3 carve-out: pre-funnel state-machine
                    // bookkeeping, not user-named effect.
                    *ctx.last_click = None;
                }
                // Pressed: swallow
            } else if state == ElementState::Pressed {
                // Hit test to determine if clicking on a node
                let canvas_pos = ctx
                    .renderer
                    .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);

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
                let parts = super::compute_click_hit(canvas_pos, ctx.mindmap_tree.as_mut(), ctx.renderer);
                let super::ClickHitParts {
                    click_hit,
                    hit_node,
                    hit_section_idx,
                    portal_text_hit,
                    portal_icon_hit,
                    edge_label_hit,
                } = parts;
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
                    let node_match = ctx
                        .text_edit_state
                        .node_id()
                        .map(|id| hit_node.as_deref() == Some(id))
                        .unwrap_or(false);
                    let edge_label_match = ctx
                        .label_edit_state
                        .edited_edge_ref()
                        .zip(edge_label_hit.as_ref())
                        .map(|(er, hit)| {
                            hit.from_id == er.from_id.as_str()
                                && hit.to_id == er.to_id.as_str()
                                && hit.edge_type == er.edge_type.as_str()
                        })
                        .unwrap_or(false);
                    let portal_text_match = ctx
                        .portal_text_edit_state
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
                    && ctx
                        .last_click
                        .as_ref()
                        .map(|prev| is_double_click(prev, now, cursor_pos_val, &click_hit))
                        .unwrap_or(false);
                if is_dblclick {
                    *ctx.last_click = None;
                    // Look up which Action (if any) the user has bound
                    // to `DoubleClick`. Default is `DoubleClickActivate`
                    // which routes by `ClickHit`; `Empty` only fires
                    // `CreateOrphanNodeAndEdit` when the user has
                    // explicitly bound that Action somewhere
                    // (off-by-default per user request).
                    let dblclick_name = crate::application::keybinds::MouseGesture::DoubleClick.key_name();
                    // Modifier-fallback so Shift+DoubleClick still
                    // activates the bare DoubleClick binding when no
                    // explicit Shift+DoubleClick binding exists.
                    let action = ctx.keybinds.action_for_gesture(
                        dblclick_name,
                        ctx.modifiers.control_key(),
                        ctx.modifiers.shift_key(),
                        ctx.modifiers.alt_key(),
                    );
                    if let Some(a) = action {
                        let dispatch_hit = super::dispatch::DispatchHit {
                            click_hit: click_hit.clone(),
                            canvas_pos,
                        };
                        let _ = super::dispatch::dispatch_action(a, ctx, Some(&dispatch_hit));
                        return;
                    }
                    // No Action bound to DoubleClick: silently no-op.
                    // (The double-click consumed `ctx.last_click`; we don't
                    // fall through to the single-click selection path.)
                    return;
                }
                *ctx.last_click = Some(LastClick {
                    time: now,
                    screen_pos: cursor_pos_val,
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
                let hit_edge_handle = match ctx.document.as_ref() {
                    Some(doc) => match &doc.selection {
                        SelectionState::Edge(er) => {
                            let tol = HANDLE_HIT_TOLERANCE_PX * ctx.renderer.canvas_per_pixel();
                            doc.hit_test_edge_handle(canvas_pos, er, tol)
                                .map(|(kind, _pos)| (er.clone(), kind))
                        }
                        _ => None,
                    },
                    None => None,
                };
                // Section resize handle press capture — only fires
                // when the active mode is `Resize { Section { .. } }`.
                // Fill-parent sections emit no handles regardless;
                // `hit_test_section_resize_handle` filters them out
                // internally.
                let hit_section_resize_handle = match (
                    ctx.document.as_ref(),
                    ctx.interaction_mode.resize_handle_section(),
                ) {
                    (Some(doc), Some((node_id, section_idx))) => {
                        let tol = HANDLE_HIT_TOLERANCE_PX * ctx.renderer.canvas_per_pixel();
                        crate::application::document::hit_test_section_resize_handle(
                            &doc.mindmap,
                            canvas_pos,
                            node_id,
                            section_idx,
                            tol,
                        )
                        .map(|side| (node_id.to_string(), section_idx, side))
                    }
                    _ => None,
                };
                // Node resize handle press capture — only fires when
                // the active mode is `Resize { Node(_) }`.
                let hit_node_resize_handle =
                    match (ctx.document.as_ref(), ctx.interaction_mode.resize_handle_node()) {
                        (Some(doc), Some(node_id)) => {
                            let tol = HANDLE_HIT_TOLERANCE_PX * ctx.renderer.canvas_per_pixel();
                            crate::application::document::hit_test_node_resize_handle(
                                &doc.mindmap,
                                canvas_pos,
                                node_id,
                                tol,
                            )
                            .map(|side| (node_id.to_string(), side))
                        }
                        _ => None,
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
                    Some((key, endpoint)) if hit_node.is_none() => Some((key.clone(), endpoint.clone())),
                    _ => None,
                };
                // Reuse the press-time edge-label hit captured
                // earlier so the threshold-cross transition can
                // promote to `DraggingEdgeLabel`. Priority
                // ordering in `event_cursor_moved.rs` still
                // gives portal-label / edge-handle drag higher
                // precedence when multiple hits overlap.
                //
                // Don't clobber a right-button gesture in flight.
                // Symmetric with the right-press guard in
                // `handle_right_button` (`if !matches!(.., None)
                // { return }`). Pre-fix, a left-press during a
                // `PendingRight` would silently overwrite the
                // right-button state, the user's intended
                // RightClick / FastResizeStart would never fire,
                // and the put-back arm in the left-release match
                // (`other @ DragState::PendingRight => …`) was
                // unreachable in Default mode. C3 from the
                // 9-agent review.
                if matches!(*ctx.drag_state, DragState::PendingRight { .. }) {
                    log::debug!(
                        "left-button press ignored (right-button gesture in flight); state stays put"
                    );
                    return;
                }
                *ctx.drag_state = DragState::Pending {
                    start_pos: cursor_pos_val,
                    hit_node,
                    hit_section_idx,
                    hit_edge_handle,
                    hit_portal_label,
                    hit_edge_label: edge_label_hit,
                    hit_section_resize_handle,
                    hit_node_resize_handle,
                };
            } else {
                match std::mem::replace(ctx.drag_state, DragState::None) {
                    DragState::Pending {
                        hit_node,
                        hit_section_idx,
                        hit_edge_label,
                        ..
                    } => {
                        // If the node text editor is open, the
                        // release decides whether to commit or
                        // swallow. If the release lands inside the
                        // edited node's AABB, keep editing (no
                        // commit, no selection change). Otherwise
                        // commit and fall through.
                        if ctx.text_edit_state.is_open() {
                            let release_canvas = ctx
                                .renderer
                                .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
                            // Refresh the subtree-AABB cache before the
                            // overflow-aware containment check —
                            // `point_in_node_aabb` reads
                            // `subtree_aabb()` which returns `None`
                            // when the cache is dirty (post-mutation
                            // / post-tree-rebuild). A `None` falls
                            // back to the container-only path,
                            // regressing the multi-section overflow
                            // gesture this branch was added to fix.
                            // `ensure_subtree_aabbs` is O(1) on a
                            // clean cache and O(arena) on the first
                            // call after a mutation; either way it's
                            // cheap relative to the click handler.
                            if let Some(tree) = ctx.mindmap_tree.as_mut() {
                                tree.tree.ensure_subtree_aabbs();
                            }
                            let inside = ctx
                                .text_edit_state
                                .node_id()
                                .zip(ctx.mindmap_tree.as_ref())
                                .map(|(id, tree)| {
                                    crate::application::document::point_in_node_aabb(release_canvas, id, tree)
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
                            // Click-outside: commit the edit through
                            // the funnel (`Action::TextEditCommit`),
                            // then fall through to the regular click
                            // path so the new selection lands.
                            let _ = super::dispatch::dispatch_action(Action::TextEditCommit, ctx, None);
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
                        if ctx.label_edit_state.is_open() {
                            let release_canvas = ctx
                                .renderer
                                .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
                            let edited = ctx.label_edit_state.edited_edge_ref().cloned();
                            let stays_on_edited_label = edited
                                .as_ref()
                                .and_then(|er| {
                                    ctx.renderer.hit_test_any_edge_label(release_canvas).map(|hit| {
                                        hit.from_id == er.from_id.as_str()
                                            && hit.to_id == er.to_id.as_str()
                                            && hit.edge_type == er.edge_type.as_str()
                                    })
                                })
                                .unwrap_or(false);
                            if stays_on_edited_label {
                                return;
                            }
                            // Click-outside the edited label: commit
                            // through the funnel (`LabelEditCommit`).
                            let _ = super::dispatch::dispatch_action(Action::LabelEditCommit, ctx, None);
                        }
                        // Portal-text editor uses the portal-text
                        // hitbox instead of the edge-label hitbox,
                        // and matches `(edge_key, endpoint)` rather
                        // than just the edge key — clicking the
                        // *other* endpoint of the same portal edge
                        // commits this side and then routes the
                        // click as a fresh selection on the new
                        // endpoint.
                        if ctx.portal_text_edit_state.is_open() {
                            let release_canvas = ctx
                                .renderer
                                .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
                            let edited = ctx
                                .portal_text_edit_state
                                .edited_endpoint()
                                .map(|(er, ep)| (er.clone(), ep.to_string()));
                            let stays_on_edited_text = edited
                                .as_ref()
                                .and_then(|(er, ep)| {
                                    ctx.renderer.hit_test_portal_text(release_canvas).map(
                                        |(hit_key, hit_ep)| {
                                            hit_key.from_id == er.from_id.as_str()
                                                && hit_key.to_id == er.to_id.as_str()
                                                && hit_key.edge_type == er.edge_type.as_str()
                                                && hit_ep == *ep
                                        },
                                    )
                                })
                                .unwrap_or(false);
                            if stays_on_edited_text {
                                return;
                            }
                            // Click-outside the edited portal-text:
                            // commit through the funnel. The dispatch
                            // arm picks `portal_text_edit_state` over
                            // `label_edit_state` since portal is
                            // checked first.
                            let _ = super::dispatch::dispatch_action(Action::LabelEditCommit, ctx, None);
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
                        let edge_label_target: Option<crate::application::document::EdgeRef> = hit_edge_label
                            .map(|k| {
                                crate::application::document::EdgeRef::new(
                                    k.from_id.as_str(),
                                    k.to_id.as_str(),
                                    k.edge_type.as_str(),
                                )
                            });
                        // NodeEdit-mode outside-click: clicking
                        // outside the active node's overflow-aware
                        // AABB exits NodeEdit back to Default
                        // BEFORE any selection routing fires, so
                        // every selection branch below (edge-label,
                        // node, empty-canvas) lands in Default mode.
                        // Pre-fix this only ran for the node-hit
                        // arm — clicking an edge label or portal
                        // from inside NodeEdit left the user in
                        // an orphan "NodeEdit + EdgeLabel selection"
                        // state.
                        maybe_exit_node_edit_on_outside_click(ctx, cursor_pos_val, hit_node.as_deref());
                        let entered_label_select = if let Some(er) = edge_label_target {
                            if let Some(doc) = ctx.document.as_mut() {
                                let prev = doc.selection.clone();
                                doc.selection = SelectionState::EdgeLabel(
                                    crate::application::document::EdgeLabelSel::new(er),
                                );
                                rebuild_after_selection_change(
                                    &prev,
                                    doc,
                                    ctx.interaction_mode,
                                    ctx.mindmap_tree,
                                    ctx.app_scene,
                                    ctx.renderer,
                                    ctx.scene_cache,
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
                                hit_section_idx,
                                cursor_pos_val,
                                ctx.modifiers.shift_key(),
                                ctx.document,
                                ctx.interaction_mode,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                    }
                    DragState::Throttled(ThrottledDrag::MovingNode(i)) => {
                        // Flush any remaining pending delta to the tree before drop.
                        // This always runs regardless of the throttle — on release
                        // we want the final position committed in full, even if
                        // the throttle was mid-stretch skipping intermediate drains.
                        let had_pending = i.pending_delta != Vec2::ZERO;
                        if had_pending {
                            if let Some(tree) = ctx.mindmap_tree.as_mut() {
                                for nid in &i.node_ids {
                                    apply_drag_delta(
                                        tree,
                                        nid,
                                        i.pending_delta.x,
                                        i.pending_delta.y,
                                        !i.individual,
                                    );
                                }
                            }
                        }
                        // Drop: sync to model, full rebuild, push undo
                        if let Some(doc) = ctx.document.as_mut() {
                            let dx = i.total_delta.x as f64;
                            let dy = i.total_delta.y as f64;
                            let undo_data = doc.apply_move_multiple(&i.node_ids, dx, dy, i.individual);
                            doc.undo_stack.push(UndoAction::MoveNodes {
                                original_positions: undo_data,
                            });
                            doc.dirty = true;

                            // Under rapid drag the throttle can skip the last
                            // drain or two, leaving `pending_delta` stranded
                            // in the accumulator. The flush above syncs the
                            // tree and `apply_move_multiple` above syncs the
                            // model, but the `SceneConnectionCache`'s
                            // `pre_clip_positions` still reflect the
                            // second-to-last drain's `offsets` (short of the
                            // committed position by `pending_delta`). The
                            // subsequent `rebuild_all` → `rebuild_scene_only`
                            // runs with empty offsets, so the cache's fast
                            // path returns those stale samples and the edges
                            // appear glued to the node's pre-flush position
                            // until the next cache-invalidating event
                            // (mutation or zoom). Clearing here forces a
                            // resample from the now-authoritative model.
                            if had_pending {
                                ctx.scene_cache.clear();
                            }

                            // Full rebuild from model
                            rebuild_all(
                                doc,
                                ctx.interaction_mode,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                    }
                    DragState::Throttled(ThrottledDrag::MovingSection(i)) => {
                        // Single setter call on release; AABB
                        // overflow rejection logs and falls
                        // through to `rebuild_all`, which
                        // rebuilds the tree from the unchanged
                        // model and snaps the section back.
                        if let Some(doc) = ctx.document.as_mut() {
                            let new_x = i.start_offset.0 + i.total_delta.x as f64;
                            let new_y = i.start_offset.1 + i.total_delta.y as f64;
                            match doc.set_section_offset(&i.node_id, i.section_idx, new_x, new_y) {
                                Ok(true) => {}
                                Ok(false) => {
                                    log::debug!(
                                        "section drag committed no-op offset on '{}' section[{}]",
                                        i.node_id,
                                        i.section_idx
                                    );
                                }
                                Err(msg) => {
                                    log::info!("section drag release rejected: {} (snapping back)", msg);
                                }
                            }
                            // Unconditional clear so the
                            // rebuild_all path resamples from
                            // the authoritative model — the
                            // per-frame drain mutated the tree
                            // (and therefore stale scene-cache
                            // samples) regardless of which
                            // arm above ran.
                            ctx.scene_cache.clear();
                            rebuild_all(
                                doc,
                                ctx.interaction_mode,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                    }
                    DragState::Throttled(ThrottledDrag::NodeResize(i)) => {
                        finalize_node_resize_release(&i, "node resize", ctx);
                    }
                    DragState::Throttled(ThrottledDrag::SectionResize(i)) => {
                        finalize_section_resize_release(&i, "section resize", ctx);
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
                        if let Some(doc) = ctx.document.as_mut() {
                            apply_edge_handle_drag(doc, &edge_ref, handle, start_handle_pos, total_delta);
                            // Crossing the drag threshold guarantees a
                            // state change, so commit unconditionally.
                            doc.commit_throttled_edge_drag(&edge_ref, original, |_, _| true);
                            rebuild_all(
                                doc,
                                ctx.interaction_mode,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
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
                        if let (Some(doc), Some(cursor)) = (ctx.document.as_mut(), pending_cursor) {
                            apply_portal_label_drag(doc, &edge_ref, &endpoint_node_id, cursor);
                        }
                        // Commit with a single EditEdge undo
                        // carrying the pre-drag snapshot, matching
                        // the EdgeHandle release path. The no-op
                        // check compares only the two fields this
                        // drag touches (`portal_from` /
                        // `portal_to`) — whole-edge `PartialEq`
                        // would fold in float-fragile
                        // `control_points`.
                        if let Some(doc) = ctx.document.as_mut() {
                            doc.commit_throttled_edge_drag(&edge_ref, original, |c, o| {
                                c.portal_from != o.portal_from || c.portal_to != o.portal_to
                            });
                            rebuild_all(
                                doc,
                                ctx.interaction_mode,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
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
                        if let (Some(doc), Some(cursor)) = (ctx.document.as_mut(), pending_cursor) {
                            super::edge_label_drag::apply_edge_label_drag(doc, &edge_ref, cursor);
                        }
                        // Commit with a single `EditEdge` carrying
                        // the pre-drag snapshot, skipping the undo
                        // entry if nothing actually moved.
                        if let Some(doc) = ctx.document.as_mut() {
                            doc.commit_throttled_edge_drag(&edge_ref, original, |c, o| {
                                c.label_config != o.label_config
                            });
                            // Scene-only rebuild: every per-frame
                            // drain already used `rebuild_scene_only`
                            // because node trees are untouched by a
                            // label move; the release commit is
                            // the same story.
                            rebuild_scene_only(
                                doc,
                                ctx.interaction_mode,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                    }
                    DragState::SelectingRect {
                        start_canvas,
                        current_canvas,
                    } => {
                        // Finalize: select all nodes in the rectangle
                        ctx.renderer.clear_overlay_buffers();
                        if let (Some(doc), Some(tree)) = (ctx.document.as_mut(), ctx.mindmap_tree.as_ref()) {
                            let hits = rect_select(start_canvas, current_canvas, tree);
                            doc.selection = SelectionState::from_ids(hits);
                            rebuild_all(
                                doc,
                                ctx.interaction_mode,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                    }
                    DragState::Panning | DragState::None => {}
                    // Left-button release while a right-button gesture
                    // is pending: the `mem::replace` above already
                    // swapped in `None`, so put the original state
                    // back so the right-button release path can act
                    // on it. Reachable in Reparent / Connect modes
                    // (where the left-press path swallows the press
                    // without setting `Pending`) and at startup
                    // before any drag has fired. In Default mode the
                    // left-press gate at line 361 short-circuits when
                    // `PendingRight` is active, so the path through
                    // here from a Default-mode press is unreachable.
                    other @ DragState::PendingRight { .. } => {
                        *ctx.drag_state = other;
                    }
                }
            }
        }
        MouseButton::Right => {
            handle_right_button(state, cursor_pos_val, ctx);
        }
        _ => {}
    }
}

/// Right-button press / release handler — fast-resize gesture
/// substrate (`SECTIONS_BORDERS_RESIZE_PLAN.md` §6.3).
///
/// Press: stash the press-time hit (body of any node / section,
/// no edge-handle / portal-label / resize-handle precedence — the
/// gesture is "grab a corner from anywhere on this body") into
/// `DragState::PendingRight`. Skips when an active drag is in
/// flight to avoid clobbering it; logs and falls through.
///
/// Release: two cases:
/// 1. `PendingRight` (no movement past threshold) — fire the bound
///    `MouseGesture::RightClick` action lookup. Default-bound to
///    nothing; users opt in. State resets to `None`.
/// 2. `Throttled(NodeResize | SectionResize)` (threshold-cross
///    promoted to fast-resize via `Action::FastResizeStart`) —
///    finalize via [`finalize_node_resize_release`] /
///    [`finalize_section_resize_release`], the same helpers the
///    left-button release path uses. Single-source commit shape
///    regardless of which button started the gesture.
fn handle_right_button(state: ElementState, cursor_pos_val: (f64, f64), ctx: &mut InputHandlerContext<'_>) {
    if state == ElementState::Pressed {
        // Mode + modal guards: don't arm a fast-resize gesture
        // when the user's intent is unambiguously elsewhere.
        // Architecture-review findings I3 + I4 + I5:
        //
        // - **Reparent / Connect modes** consume left-click as
        //   "pick target" — accepting right-presses here would
        //   strand `PendingRight` invisibly behind the picker
        //   chrome, and a release-without-movement would fire
        //   whatever `RightClick` action the user happens to
        //   have bound, into the wrong context.
        // - **Text editors** (label / portal-text / section-text)
        //   are modal — the left-button path already commits-
        //   outside-click before any resize logic runs. Right-
        //   button has no equivalent commit funnel; better to
        //   block until one exists than to fast-resize a
        //   different node while the editor stays open with a
        //   half-edited buffer.
        // - **Resize mode** with handles visible on node X: the
        //   user's intent is "I'm resizing X". A Ctrl+RightDrag
        //   on node Y would resize Y while X's handles stay
        //   drawn — visible chrome disagreeing with the active
        //   gesture. Block to preserve the mode's meaning.
        if ctx.interaction_mode.is_target_picker() {
            log::debug!("right-button press ignored (target-picker mode active)");
            return;
        }
        if ctx.text_edit_state.is_open()
            || ctx.label_edit_state.is_open()
            || ctx.portal_text_edit_state.is_open()
        {
            log::debug!("right-button press ignored (modal text editor open)");
            return;
        }
        if matches!(*ctx.interaction_mode, super::InteractionMode::Resize { .. }) {
            log::debug!("right-button press ignored (Resize mode active; use the visible handles)");
            return;
        }

        // Body-only hit-test; no edge-handle / portal-label / resize-
        // handle hits — the fast-resize gesture deliberately bypasses
        // those because it's a corner-anchored resize from anywhere
        // on the body. Resize-handle hits would compete with the
        // press-time corner inference; portal/edge-label hits would
        // promote the gesture to label-drag and never reach FastResize.
        let canvas_pos = ctx
            .renderer
            .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
        let (hit_node, hit_section_idx) = match ctx.mindmap_tree.as_mut() {
            Some(tree) => match crate::application::document::hit_test_target(canvas_pos, tree) {
                Some(crate::application::document::HitTarget::Section { node_id, section_idx }) => {
                    (Some(node_id), Some(section_idx))
                }
                Some(crate::application::document::HitTarget::NodeContainer { node_id }) => {
                    (Some(node_id), None)
                }
                None => (None, None),
            },
            None => (None, None),
        };
        // Don't clobber an active drag. If state is already
        // Pending / PendingRight / Throttled / Panning / SelectingRect,
        // log + ignore. Mirror's middle-click's posture (which
        // unconditionally overwrites) is intentionally not chosen
        // here — fast-resize is a meaningful gesture; clobbering an
        // in-flight resize with a stray right-press would be visible.
        if !matches!(*ctx.drag_state, DragState::None) {
            log::debug!("right-button press ignored (drag already in flight); state stays put");
            return;
        }
        *ctx.drag_state = DragState::PendingRight {
            start_pos: cursor_pos_val,
            start_canvas: canvas_pos,
            hit_node,
            hit_section_idx,
        };
    } else {
        match std::mem::replace(ctx.drag_state, DragState::None) {
            DragState::PendingRight { .. } => {
                // No movement past threshold — fire the bound
                // RightClick action (default-unbound). The action
                // lookup uses `action_for_gesture` so a user can
                // bind `Ctrl+RightClick` separately from bare
                // `RightClick`, with the standard modifier-fallback.
                let name = crate::application::keybinds::MouseGesture::RightClick.key_name();
                let action = ctx.keybinds.action_for_gesture(
                    name,
                    ctx.modifiers.control_key(),
                    ctx.modifiers.shift_key(),
                    ctx.modifiers.alt_key(),
                );
                if let Some(a) = action {
                    let _ = super::dispatch::dispatch_action(a, ctx, None);
                }
            }
            // Threshold-cross promoted `PendingRight` to one of
            // the resize Throttled variants — finalize via the
            // shared helpers. Gate on `started_with_right` so an
            // accidental right-click during a left-button-driven
            // handle drag doesn't terminate the resize: the
            // left-button release is the rightful finalizer.
            DragState::Throttled(ThrottledDrag::NodeResize(i)) if i.started_with_right => {
                finalize_node_resize_release(&i, "fast-resize node", ctx);
            }
            DragState::Throttled(ThrottledDrag::SectionResize(i)) if i.started_with_right => {
                finalize_section_resize_release(&i, "fast-resize section", ctx);
            }
            // Left-button-driven Throttled resize that survived
            // a stray right-release — restore the state and let
            // the eventual left-button release finalize it.
            other @ DragState::Throttled(ThrottledDrag::NodeResize(_))
            | other @ DragState::Throttled(ThrottledDrag::SectionResize(_)) => {
                log::debug!(
                    "right-release on left-button resize ignored; left-button release \
                     will finalize"
                );
                *ctx.drag_state = other;
            }
            // Any other state on right-release: put it back. Right-
            // button release shouldn't terminate a left-button
            // drag, a panning gesture, a rubber-band selection,
            // or any of the non-resize Throttled variants.
            other => {
                *ctx.drag_state = other;
            }
        }
    }
}

/// Outside-click NodeEdit-exit helper. When the active mode is
/// `InteractionMode::NodeEdit { node_id }` and the release lands
/// outside `node_id`'s overflow-aware AABB, dispatch
/// `Action::ExitMode` to flip back to `Default`. This runs before
/// the regular `handle_click` so the click that lands outside the
/// active node registers in Default mode (whole-node Single).
///
/// "Outside" is determined by `point_in_node_aabb`, which is
/// shape-aware and counts overflowing-section territory as
/// inside — same rule the text-editor's click-outside-commit
/// uses. Inside-AABB clicks (including hits on overflowing
/// sections) keep NodeEdit active.
///
/// `hit_node` is the click hit's owning node id (`None` for empty
/// canvas). `cursor_pos_val` is screen-space; we project to canvas
/// inside.
/// Finalize a `Throttled(NodeResize)` drag: write the resolved
/// `(position, size)` through `set_node_aabb` (atomic, single
/// `EditNodeAabb` undo entry), clear the scene cache, rebuild
/// the scene from the authoritative model.
///
/// `gesture_label` distinguishes the log-line origin
/// ("node resize" for handle-driven left-button drags vs
/// "fast-resize node" for right-button corner-anchored drags) —
/// users grepping logs for "rejected" can tell the two apart.
/// Rejection (NaN, non-positive size, astronomical) logs and
/// falls through to `rebuild_all` from the unchanged model so
/// the node snaps back to its pre-drag AABB.
///
/// Single-source for both the left-release and right-release
/// finalization paths — pre-fix, the two arms held byte-near
/// duplicates of this body. CODE_CONVENTIONS §5: "If a function
/// is needed in two or more places, the answer is never to copy
/// it, but to use a single function called in two or more
/// places." C6 of the 9-agent review.
#[cfg(not(target_arch = "wasm32"))]
fn finalize_node_resize_release(
    interaction: &super::throttled_interaction::NodeResizeInteraction,
    gesture_label: &str,
    ctx: &mut InputHandlerContext<'_>,
) {
    let Some(doc) = ctx.document.as_mut() else {
        return;
    };
    let (new_position, new_size) = interaction.resolve(interaction.total_delta);
    match doc.set_node_aabb(&interaction.node_id, new_position, new_size) {
        Ok(true) => {}
        Ok(false) => {
            log::debug!(
                "{} release committed no-op on '{}'",
                gesture_label,
                interaction.node_id
            );
        }
        Err(msg) => {
            log::info!("{} release rejected: {} (snapping back)", gesture_label, msg);
        }
    }
    ctx.scene_cache.clear();
    rebuild_all(
        doc,
        ctx.interaction_mode,
        ctx.mindmap_tree,
        ctx.app_scene,
        ctx.renderer,
        ctx.scene_cache,
    );
}

/// Finalize a `Throttled(SectionResize)` drag — see
/// [`finalize_node_resize_release`] for the shape rationale.
/// Routes through `set_section_aabb` which validates the
/// post-mutation `(offset, size)` against the parent in one
/// step, so a W-grow gesture (shrink offset, grow width) passes
/// the right-edge guard the two-step `set_section_size` +
/// `set_section_offset` path rejected (intermediate state had
/// new size at old offset, overflowing). Pushes an
/// `EditNodeStyle` undo entry via the section's parent node
/// (sections share their owning node's style undo envelope —
/// see `mutate_section_with_style_undo` in `nodes/`).
#[cfg(not(target_arch = "wasm32"))]
fn finalize_section_resize_release(
    interaction: &super::throttled_interaction::SectionResizeInteraction,
    gesture_label: &str,
    ctx: &mut InputHandlerContext<'_>,
) {
    let Some(doc) = ctx.document.as_mut() else {
        return;
    };
    let (new_offset, new_size) = interaction.resolve(interaction.total_delta);
    match doc.set_section_aabb(
        &interaction.node_id,
        interaction.section_idx,
        new_offset,
        new_size,
    ) {
        Ok(true) => {}
        Ok(false) => {
            log::debug!(
                "{} release committed no-op on '{}' section[{}]",
                gesture_label,
                interaction.node_id,
                interaction.section_idx
            );
        }
        Err(msg) => {
            log::info!("{} release rejected: {} (snapping back)", gesture_label, msg);
        }
    }
    ctx.scene_cache.clear();
    rebuild_all(
        doc,
        ctx.interaction_mode,
        ctx.mindmap_tree,
        ctx.app_scene,
        ctx.renderer,
        ctx.scene_cache,
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn maybe_exit_node_edit_on_outside_click(
    ctx: &mut InputHandlerContext<'_>,
    cursor_pos_val: (f64, f64),
    hit_node: Option<&str>,
) {
    let active_node = match &*ctx.interaction_mode {
        super::InteractionMode::NodeEdit { node_id } => node_id.clone(),
        _ => return,
    };
    // Fast path: the click hit a different node than the active
    // one. This catches sibling-click cleanly without the AABB
    // computation.
    if let Some(hit) = hit_node {
        if hit != active_node {
            let _ = super::dispatch::dispatch_action(Action::ExitMode, ctx, None);
            return;
        }
        // Same-node hit: stay in NodeEdit.
        return;
    }
    // Empty-canvas hit: confirm the cursor is actually outside the
    // active node's AABB before exiting (overflowing sections
    // count as inside). `ensure_subtree_aabbs` is needed because
    // post-mutation AABB caches go dirty; same shape as the
    // text-editor's click-outside-commit gate.
    let release_canvas = ctx
        .renderer
        .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
    if let Some(tree) = ctx.mindmap_tree.as_mut() {
        tree.tree.ensure_subtree_aabbs();
    }
    let inside = ctx
        .mindmap_tree
        .as_ref()
        .map(|tree| crate::application::document::point_in_node_aabb(release_canvas, &active_node, tree))
        .unwrap_or(false);
    if !inside {
        let _ = super::dispatch::dispatch_action(Action::ExitMode, ctx, None);
    }
}
