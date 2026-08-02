// SPDX-License-Identifier: MPL-2.0

//! Mouse-input dispatch. Left/middle/right + Pressed/Released routed
//! through selection, double-click, drag start/end, and the
//! console / color-picker steals.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::platform::input::{ElementState, MouseButton};

use super::click::handle_click;
use super::color_picker_flow::{end_color_picker_gesture, handle_color_picker_click, PickerClick};
use super::console_input::save_console_history;
use super::input_context::InputHandlerContext;
use super::modal_editor::commit_modal_editors_on_release;
use super::scene_rebuild::{rebuild_after_selection_change, rebuild_all};
use super::throttled_interaction::release::commit_if_resolved;
use super::throttled_interaction::ThrottledDrag;
use super::{is_double_click, now_ms, DragState, InteractionMode, LastClick, HANDLE_HIT_TOLERANCE_PX};
use crate::application::console::ConsoleState;
use crate::application::document::{rect_select, SelectionState};
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
            let route = if let Some(doc) = ctx.document.as_mut() {
                handle_color_picker_click(
                    cursor_pos_val,
                    button,
                    ctx.color_picker_state,
                    doc,
                    ctx.picker_hover,
                )
            } else {
                PickerClick::Consumed
            };
            match route {
                PickerClick::Consumed => true,
                PickerClick::FallThrough => false,
                // The ࿕ commit button and the contextual
                // outside-click cancel are user-named effects, so
                // the router hands back an Action and the §3
                // funnel runs it — same shape as the text editor's
                // click-outside `Action::TextEditCommit` below.
                PickerClick::Dispatch(action) => {
                    let _ = super::dispatch::dispatch_action(action, ctx, None);
                    true
                }
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
                // drag state below — mirrors today's behavior where
                // any drag's release goes to None regardless of which
                // gesture started it.
                let name = crate::application::keybinds::MouseGesture::MiddleClick.key_name();
                // Modifier-fallback: Ctrl+MiddleClick matches the bare
                // MiddleClick binding when no exact-modifier match
                // exists. Preserves pre-branch modifier-agnostic
                // behavior for mouse gestures.
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
                let parts = super::compute_click_hit(canvas_pos, ctx.mindmap_tree.as_mut(), ctx.app_scene);
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
                // two editor states are mutually exclusive by
                // construction (the event-keyboard dispatch steals on
                // whichever is open first), so one match suffices.
                // Without this guard for the single-line editor, a
                // double-click while editing would call
                // `open_single_line_edit` a second time, which
                // re-seeds the buffer from the committed model
                // value and silently destroys the in-progress edit.
                let single_line_match = ctx
                    .single_line_edit_state
                    .target()
                    .map(|t| t.matches_press_hit(edge_label_hit.as_ref(), portal_text_hit.as_ref()))
                    .unwrap_or(false);
                let already_editing_same_target = super::already_editing_same_target(
                    ctx.text_edit_state.node_id(),
                    hit_node.as_deref(),
                    single_line_match,
                );
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
                *ctx.drag_state = DragState::Pending(Box::new(super::PendingPress {
                    start_pos: cursor_pos_val,
                    hit_node,
                    hit_section_idx,
                    hit_edge_handle,
                    hit_portal_label,
                    hit_edge_label: edge_label_hit,
                    hit_section_resize_handle,
                    hit_node_resize_handle,
                }));
            } else {
                match std::mem::replace(ctx.drag_state, DragState::None) {
                    DragState::Pending(press) => {
                        let super::PendingPress {
                            hit_node,
                            hit_section_idx,
                            hit_edge_label,
                            ..
                        } = *press;
                        // If an inline text editor is open, the
                        // release decides whether to commit or
                        // swallow. A release inside the element
                        // under edit keeps editing (no commit, no
                        // selection change, no drag-state
                        // transition — the release is fully
                        // consumed); a release anywhere else
                        // commits through the funnel and falls
                        // through to the regular click path so the
                        // new selection lands. Without the
                        // click-outside half, the only way to close
                        // an editor would be Esc / Enter, and
                        // clicking elsewhere would feel
                        // unresponsive.
                        if commit_modal_editors_on_release(ctx) {
                            return;
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
                    // Seven throttled drag variants, one release
                    // shape: flush whatever the throttle left
                    // pending, commit to the model with its undo
                    // entry, clear the scene cache if the per-frame
                    // drains left stale samples, then run the
                    // canvas decree the commit hands back. The
                    // gesture-specific half lives in each
                    // interaction's `commit_on_release_core`, so an
                    // eighth variant does not grow this ladder.
                    //
                    // Whether *this* button finalizes at all is
                    // `resolve_release`'s call, not this arm's — see
                    // there for why the left button always does.
                    DragState::Throttled(drag) => {
                        finalize_or_put_back(MouseButton::Left, drag, ctx);
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
///    finalize through the interaction's own
///    `commit_on_release_core`, the same body the left-button
///    release path runs. Single-source commit shape regardless of
///    which button started the gesture.
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
        if ctx.text_edit_state.is_open() || ctx.single_line_edit_state.is_open() {
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
            // Threshold-cross promoted `PendingRight` to a
            // Throttled variant — finalize through the same
            // `commit_on_release` the left-button release runs, and
            // through the same resolver, which is what keeps the
            // right button from ending a left-button drag.
            DragState::Throttled(drag) => {
                finalize_or_put_back(MouseButton::Right, drag, ctx);
            }
            // Any other state on right-release: put it back. Right-
            // button release shouldn't terminate a panning gesture
            // or a rubber-band selection either.
            other => {
                *ctx.drag_state = other;
            }
        }
    }
}

/// Run `released` against an in-flight throttled drag: either commit
/// the gesture, or put the drag state back because this button did
/// not start it.
///
/// The caller has already `mem::replace`d the drag state with `None`,
/// so the put-back branch has to restore it — that restore and the
/// commit are the only two outcomes, and which one applies is
/// [`commit_if_resolved`]'s renderer-free decision.
///
/// Two arms, and both of them are the part that needs the input
/// layer: running the canvas decree (`&mut Renderer`) and restoring
/// `DragState`. Everything before that — the button gate and the
/// model write itself — is [`commit_if_resolved`], which the
/// lifecycle harness drives directly.
#[cfg(not(target_arch = "wasm32"))]
fn finalize_or_put_back(
    released: MouseButton,
    mut drag: Box<ThrottledDrag>,
    ctx: &mut InputHandlerContext<'_>,
) {
    // Bound for readability; matching the call in place compiles
    // just as well. The `ReleaseCommit` is moved into
    // `commit_if_resolved` and dropped there, and the
    // `Option<ReleaseRefresh>` that comes back is `Copy` and borrows
    // nothing, so no borrow of `ctx` survives into the arms.
    let refresh = commit_if_resolved(released, &mut drag, ctx.release_commit());
    match refresh {
        Some(refresh) => refresh.execute(ctx.drain_context()),
        None => {
            log::debug!(
                "{:?}-button release on a drag it did not start; the originating \
                 button's release will finalize",
                released
            );
            *ctx.drag_state = DragState::Throttled(drag);
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
    // active node's AABB before exiting (overflowing sections count
    // as inside). Same refresh-then-contain body the text editor's
    // click-outside-commit gate runs — the AABB refresh is the step
    // that must not be forgotten, so it lives in one place.
    let release_canvas = ctx
        .renderer
        .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
    let inside =
        super::text_edit::point_inside_node_fresh_aabb(&active_node, ctx.mindmap_tree, release_canvas);
    if !inside {
        let _ = super::dispatch::dispatch_action(Action::ExitMode, ctx, None);
    }
}
