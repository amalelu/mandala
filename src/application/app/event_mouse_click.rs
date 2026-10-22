// SPDX-License-Identifier: MPL-2.0

//! Mouse-input dispatch. Left/middle/right + Pressed/Released routed
//! through selection, double-click, drag start/end, and the
//! console / color-picker steals.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::platform::input::{ElementState, MouseButton};

use super::click::handle_click;
use super::color_picker_flow::{end_color_picker_gesture, handle_color_picker_click, PickerClick};
use super::console_input::save_console_history;
use super::drain_frame::end_rect_select_gesture;
use super::input_context::InputHandlerContext;
use super::modal_editor::commit_modal_editors_on_release;
use super::scene_rebuild::{rebuild_after_selection_change, rebuild_all};
use super::throttled_interaction::release::commit_if_resolved;
use super::throttled_interaction::ThrottledDrag;
use super::{is_double_click, now_ms, DragState, InteractionMode, LastClick, HANDLE_HIT_TOLERANCE_PX};
use crate::application::console::ConsoleState;
use crate::application::document::{rect_select, SelectionState};
use crate::application::keybinds::Action;

/// What a middle-button event does to [`DragState`], with the
/// bound Action and the renderer factored out.
///
/// Pure so both halves of the middle button are pinnable at
/// `DragState` level — the dispatcher below takes an
/// [`InputHandlerContext`], i.e. a live wgpu device, which
/// TEST_CONVENTIONS §T8 keeps out of the harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MiddleButtonRoute {
    /// The drag state is idle — run whatever the user bound to
    /// `MouseGesture::MiddleClick`.
    Dispatch,
    /// End the pan the middle press armed: the state goes to `None`.
    Clear,
    /// Another gesture owns the drag state. Leave it exactly as it
    /// is, and do nothing else.
    Keep,
}

/// Route one middle-button event against the drag state it finds.
///
/// The guard on the press half is the same one
/// [`handle_right_button`] applies, and for the same reason its
/// comment gives: a press that clobbers a gesture already in flight
/// destroys it. Middle-click's unconditional overwrite was the
/// posture that comment named as the one it rejected — this is that
/// posture removed. Pre-fix, a middle press mid-drag replaced
/// `DragState::Throttled(..)` (via `Action::PanCanvas` →
/// `DragState::Panning`) and the release forced `DragState::None`,
/// so the abandoned drag's `commit_on_release_core` never ran: the
/// tree kept the dragged offsets until the next model rebuild
/// snapped them back, with no undo entry to recover from.
///
/// The release half needs a guard for the same reason, but it is
/// **not the same guard** — it is wider by one state, and the width
/// is a known hole rather than a design. `Panning` is what a middle
/// press can arm (`Action::PanCanvas` is the only Action that writes
/// a drag state with no prior state; `Action::FastResizeStart` reads
/// `PendingRight` and bails otherwise), but it is not *only* that:
/// the `LeftDrag` threshold cross and any keyboard or macro
/// `pan_canvas` arm the same variant, and `Panning` carries nothing
/// that says which. So a middle release during a *left*-drag pan
/// still writes `DragState::None` and ends the pan while the left
/// button is down. Pre-existing, and left standing here on purpose:
/// closing it means giving `Panning` its origin the way the resize
/// gestures carry `started_with_right`, and once it has one the same
/// question has to be answered for the left release (which ends a
/// middle-started pan just as readily) and for a keyboard-armed pan,
/// which has no button at all — i.e. whether `pan_canvas` on a key
/// is momentary or modal. That is a decision about the Action's
/// semantics, not a guard. Nothing is at risk while it stands:
/// `Panning` owes the model no write and no undo entry, which is
/// what separates it from the class above.
///
/// `None` clears trivially — a middle press bound to a non-drag
/// Action leaves the state alone and its release has nothing to end.
fn route_middle_button(state: ElementState, drag_state: &DragState) -> MiddleButtonRoute {
    match state {
        ElementState::Pressed => match drag_state {
            DragState::None => MiddleButtonRoute::Dispatch,
            _ => MiddleButtonRoute::Keep,
        },
        ElementState::Released => match drag_state {
            DragState::Panning | DragState::None => MiddleButtonRoute::Clear,
            _ => MiddleButtonRoute::Keep,
        },
    }
}

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
        MouseButton::Middle => match route_middle_button(state, ctx.drag_state) {
            MiddleButtonRoute::Dispatch => {
                // Middle-click press on an idle drag state: lookup what's
                // bound to MiddleClick (default `PanCanvas`). The dispatch
                // arm sets `DragState::Panning`, which the `Clear` route
                // below takes back down on release.
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
            }
            MiddleButtonRoute::Clear => {
                *ctx.drag_state = DragState::None;
            }
            MiddleButtonRoute::Keep => {
                log::debug!(
                    "middle-button {:?} ignored (another gesture owns the drag state); \
                     state stays put",
                    state
                );
            }
        },
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
                // Portal-label drag capture: pressing a marker
                // and dragging slides the label along its owning
                // node's border. Captured regardless of current
                // selection — grabbing a marker is a valid first
                // action, not just a follow-up to a prior click.
                //
                // The comment here used to say this takes
                // precedence over `hit_node` at threshold-cross
                // time. It does not, and never has:
                // `portal_label_drag_capture` stores the hit only
                // when the click chain left `hit_node` empty, so
                // a node under a marker wins the press on the
                // drag path exactly as it does on the click path.
                let hit_portal_label =
                    super::portal_label_drag_capture(&portal_icon_hit, hit_node.as_deref());
                // Reuse the press-time edge-label hit captured
                // earlier so the threshold-cross transition can
                // promote to `DraggingEdgeLabel`. Both label hits
                // are already node-gated by the click chain; what
                // the promotion order in `event_cursor_moved.rs`
                // resolves is the three hits above that the chain
                // does not gate — the edge handle and the two
                // resize handles, each captured on a mode or a
                // selection rather than on `hit_node`, and each
                // ranked ahead of it. That is also where a click
                // and a drag on one press can name different
                // targets; CONCEPTS `DragState` has the shape.
                //
                // Don't clobber a gesture that nothing else will
                // finish — see `DragState::would_abandon_gesture` for
                // which two states those are and why the other three
                // are not. Symmetric with the right-press guard in
                // `handle_right_button` (`if !matches!(.., None)
                // { return }`), which is broader because a
                // right-press has no re-arming role to preserve.
                // Pre-fix, a left-press during a `PendingRight`
                // would silently overwrite the right-button state,
                // the user's intended RightClick / FastResizeStart
                // would never fire, and the put-back arm in the
                // left-release match (`other @
                // DragState::PendingRight => …`) was unreachable in
                // Default mode. C3 from the 9-agent review. The
                // `Throttled` half joined it with #37 item 5: a
                // right-started fast-resize is in flight while the
                // left button is free, so a left press reached it
                // and dropped its release-commit.
                if ctx.drag_state.would_abandon_gesture() {
                    log::debug!("left-button press ignored (uncommitted gesture in flight); state stays put");
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
                                    &mut super::dispatch::cross_dispatch::rebuild_ctx!(ctx, doc),
                                );
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if !entered_label_select {
                            handle_click(hit_node, hit_section_idx, ctx);
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
                    // there for why only the button that started the
                    // gesture does.
                    DragState::Throttled(drag) => {
                        finalize_or_put_back(MouseButton::Left, drag, ctx);
                    }
                    DragState::SelectingRect {
                        start_canvas,
                        current_canvas,
                    } => {
                        // Finalize: select all nodes in the rectangle.
                        // Ending the gesture first, and above the
                        // document / tree checks, so the `rebuild_all`
                        // below cannot paint the preview one last
                        // time on its way out. The drain would end it
                        // anyway on the next frame — that is where the
                        // invariant lives, not here — but a frame late.
                        end_rect_select_gesture(ctx.document, ctx.renderer);
                        if let Some(doc) = ctx.document.as_mut() {
                            // Re-hit-test rather than reuse the
                            // preview: the last `CursorMoved` may have
                            // moved `current_canvas` after the final
                            // drain, and the release commits where the
                            // pointer *is*.
                            if let Some(tree) = ctx.mindmap_tree.as_ref() {
                                let hits = rect_select(start_canvas, current_canvas, tree);
                                doc.selection = SelectionState::from_ids(hits);
                            }
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
                    // before any drag has fired. In Default mode
                    // `DragState::would_abandon_gesture` short-circuits
                    // the left-press path when `PendingRight` is
                    // active, so the path through here from a
                    // Default-mode press is unreachable.
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
        // log + ignore. This is the broadest of the three press
        // guards: a right-press arms `PendingRight` and nothing
        // else, so there is no re-arming case to preserve the way
        // the left button has one. Middle-click used to be the
        // counter-example this comment named — it overwrote any
        // state unconditionally — and [`route_middle_button`]'s
        // *press* half now applies exactly this guard. Its release
        // half is wider by `Panning`, for the reason recorded there:
        // that variant does not say which button armed it.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::moving_node::MovingNodeInteraction;
    use std::collections::HashSet;

    /// A left-started move-node drag, mid-gesture: pending state
    /// buffered, `commit_on_release_core` not yet run.
    fn moving_node_drag() -> DragState {
        DragState::throttled(ThrottledDrag::MovingNode(MovingNodeInteraction::new(
            vec!["n0".to_string()],
            false,
            HashSet::new(),
        )))
    }

    /// The reported symptom: a middle-press part-way through a drag
    /// destroyed it. `Action::PanCanvas` overwrote
    /// `DragState::Throttled(..)` with `DragState::Panning`, the
    /// release then forced `DragState::None`, and the drag's
    /// `commit_on_release_core` — the only place its model write and
    /// its undo entry happen — never ran.
    ///
    /// Fails on the input below when either half of
    /// [`route_middle_button`]'s guard is removed: pre-fix the press
    /// answered `Dispatch` for every state and the release answered
    /// `Clear` for every state.
    #[test]
    fn test_middle_click_mid_drag_leaves_the_throttled_drag_intact() {
        let drag = moving_node_drag();
        assert_eq!(
            route_middle_button(ElementState::Pressed, &drag),
            MiddleButtonRoute::Keep,
            "a middle press mid-drag must not reach the MiddleClick binding: \
             dispatching PanCanvas is what overwrote the drag"
        );
        assert_eq!(
            route_middle_button(ElementState::Released, &drag),
            MiddleButtonRoute::Keep,
            "a middle release mid-drag must not force None: the drag's \
             release-commit has not run, so clearing it drops the model \
             write and its undo entry"
        );
    }

    /// The other half of the same guard — without these rows the fix
    /// could be "answer `Keep` always", which would leave the middle
    /// button unable to pan at all.
    ///
    /// Fails when the press arm stops matching `DragState::None`, or
    /// when the release arm stops matching `Panning`: a middle-drag
    /// pan would then never end and the canvas would keep panning
    /// after the button came up.
    #[test]
    fn test_middle_click_from_idle_still_arms_and_ends_a_pan() {
        assert_eq!(
            route_middle_button(ElementState::Pressed, &DragState::None),
            MiddleButtonRoute::Dispatch,
            "an idle drag state is what a middle press is for"
        );
        assert_eq!(
            route_middle_button(ElementState::Released, &DragState::Panning),
            MiddleButtonRoute::Clear,
            "the release must end the pan its own press armed"
        );
        assert_eq!(
            route_middle_button(ElementState::Released, &DragState::None),
            MiddleButtonRoute::Clear,
            "a middle press bound to a non-drag Action leaves None behind; \
             its release has nothing to end"
        );
    }

    /// A middle press must not cancel the two unthrottled gestures
    /// either. Neither loses model state, but both are live gestures
    /// the user is performing, and the pre-fix arm ended them.
    #[test]
    fn test_middle_click_does_not_end_a_pan_or_a_rect_select_it_did_not_start() {
        for drag in [
            DragState::SelectingRect {
                start_canvas: glam::Vec2::ZERO,
                current_canvas: glam::Vec2::new(10.0, 10.0),
            },
            DragState::Pending(Box::new(super::super::PendingPress {
                start_pos: (0.0, 0.0),
                hit_node: None,
                hit_section_idx: None,
                hit_edge_handle: None,
                hit_portal_label: None,
                hit_edge_label: None,
                hit_section_resize_handle: None,
                hit_node_resize_handle: None,
            })),
        ] {
            assert_eq!(
                route_middle_button(ElementState::Pressed, &drag),
                MiddleButtonRoute::Keep,
                "middle press must not clobber {:?}",
                std::mem::discriminant(&drag)
            );
            assert_eq!(
                route_middle_button(ElementState::Released, &drag),
                MiddleButtonRoute::Keep,
                "middle release must not end {:?}",
                std::mem::discriminant(&drag)
            );
        }
    }
}
