// SPDX-License-Identifier: MPL-2.0

//! Cursor-move dispatch. Owns drag-state transitions (Pending →
//! Panning / SelectingRect / Throttled(...) — where the throttled
//! variants are MovingNode, MovingSection, SectionResize,
//! NodeResize, EdgeHandle, PortalLabel, EdgeLabel), Reparent /
//! Connect hover highlights, and the button-cursor swap for
//! trigger-bearing nodes.

#![cfg(not(target_arch = "wasm32"))]

use winit::window::Window;

use crate::application::platform::window::{CursorIcon, PhysicalPosition};

use super::click::rebuild_all_with_mode;
use super::color_picker_flow::handle_color_picker_mouse_move;
use super::input_context::InputHandlerContext;
use super::scene_rebuild::{rebuild_after_selection_change, rebuild_all};
use super::throttled_interaction::{
    DragInput, EdgeHandleInteraction, EdgeLabelInteraction, MovingNodeInteraction, MovingSectionInteraction,
    NodeResizeInteraction, PortalLabelInteraction, SectionResizeInteraction, ThrottledDrag,
};
use super::DragState;
use crate::application::common::RenderDecree;
use crate::application::document::{hit_test, SelectionState};

pub(super) fn handle_cursor_moved(
    position: PhysicalPosition<f64>,
    window: &Window,
    ctx: &mut InputHandlerContext<'_>,
) {
    let prev_pos = *ctx.cursor_pos;
    *ctx.cursor_pos = (position.x, position.y);
    let cursor_pos_val = *ctx.cursor_pos;

    // Glyph-wheel color picker hover preview. Routes
    // mouse-over to the picker hit-test, updates the
    // current HSV in place, and lives-previews the
    // change on the affected edge/portal.
    //
    // Guard on `DragState::None`: if a canvas-side
    // drag is already in flight, do not route the
    // move to the picker.
    if ctx.color_picker_state.is_open() && matches!(*ctx.drag_state, DragState::None) {
        let consumed = if let Some(doc) = ctx.document.as_mut() {
            handle_color_picker_mouse_move(cursor_pos_val, ctx.color_picker_state, doc, ctx.picker_hover)
        } else {
            true
        };
        if consumed {
            return;
        }
    }

    // Reparent or Connect mode: hit-test under cursor to update the hover
    // target highlight. Skip the regular drag-state handling.
    if ctx.interaction_mode.is_target_picker() {
        let new_hover = ctx.mindmap_tree.as_mut().and_then(|tree| {
            let canvas_pos = ctx
                .renderer
                .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
            hit_test(canvas_pos, tree)
        });
        if new_hover != *ctx.hovered_node {
            *ctx.hovered_node = new_hover;
            if let Some(doc) = ctx.document.as_ref() {
                rebuild_all_with_mode(
                    doc,
                    ctx.interaction_mode,
                    ctx.hovered_node.as_deref(),
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
        }
        return;
    }

    // Cursor icon update — three branches:
    //   1. Throttled resize drag (handle drag in Resize mode OR
    //      fast-resize via `Action::FastResizeStart`): show a
    //      direction-appropriate resize cursor based on the
    //      `ResizeHandleSide`.
    //   2. Idle (DragState::None): hand cursor over button-like
    //      nodes, default elsewhere.
    //   3. Other drags (Pending / PendingRight / Panning /
    //      SelectingRect / non-resize Throttled): cursor stays
    //      as-is from the gesture's start.
    //
    // The desired cursor goes through `set_cursor_if_changed` so
    // a redundant `set_cursor` call (same icon as last frame) is
    // a Rust-side branch instead of a winit FFI hop. winit dedupes
    // on macOS / X11 but NOT on Windows (`LoadCursorW` +
    // `SetCursor` + mutex lock per call) or Wayland (calls into
    // the pointer manager every time), so the application-side
    // gate matters on those targets. `cursor_icon_last` lives on
    // `InitState` — see its doc-comment for the rationale.
    let desired = match ctx.drag_state {
        DragState::Throttled(drag) => match &**drag {
            ThrottledDrag::NodeResize(i) => Some(cursor_icon_for_resize_side(i.side)),
            ThrottledDrag::SectionResize(i) => Some(cursor_icon_for_resize_side(i.side)),
            // Every other throttled gesture keeps whatever cursor
            // the gesture started with.
            _ => None,
        },
        DragState::None => {
            let over_button = match (ctx.document.as_ref(), ctx.mindmap_tree.as_mut()) {
                (Some(doc), Some(tree)) => {
                    let canvas_pos = ctx
                        .renderer
                        .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
                    hit_test(canvas_pos, tree)
                        .and_then(|id| doc.mindmap.nodes.get(&id))
                        .map(|n| !n.trigger_bindings.is_empty())
                        .unwrap_or(false)
                }
                _ => false,
            };
            *ctx.cursor_is_hand = over_button;
            Some(if over_button {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            })
        }
        _ => None,
    };
    if let Some(icon) = desired {
        if icon != *ctx.cursor_icon_last {
            window.set_cursor(icon);
            *ctx.cursor_icon_last = icon;
        }
    }

    match ctx.drag_state {
        DragState::Panning => {
            let dx = cursor_pos_val.0 - prev_pos.0;
            let dy = cursor_pos_val.1 - prev_pos.1;
            ctx.renderer
                .process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
        }
        DragState::Throttled(drag) => {
            // One sample, both forms; the gesture's own pending
            // discipline decides which half survives — the five
            // delta drags sum, the two label drags keep the last
            // cursor and discard the rest. Per-frame mutation and
            // rebuild happen in AboutToWait behind
            // `ThrottledInteraction::drive`'s adaptive gate.
            let input = drag_input(ctx.renderer, prev_pos, cursor_pos_val);
            drag.as_dyn_mut().accumulate(input);
        }
        DragState::Pending(press) => {
            let super::PendingPress {
                start_pos,
                hit_node,
                hit_section_idx,
                hit_edge_handle,
                hit_portal_label,
                hit_edge_label,
                hit_section_resize_handle,
                hit_node_resize_handle,
            } = &mut **press;
            let dist_x = cursor_pos_val.0 - start_pos.0;
            let dist_y = cursor_pos_val.1 - start_pos.1;
            if dist_x * dist_x + dist_y * dist_y > super::DRAG_THRESHOLD_SQ_PX {
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
                // Specific-gesture arms (edge-label / portal-label /
                // edge-handle / node-resize-handle / section-resize-
                // handle) each abort with `return` on validation miss
                // rather than fall through to less-specific arms.
                // The user pressed a handle; if the handle's target
                // is gone (deleted mid-press, mutated through the
                // console, etc.), aborting the promotion is the
                // honest UX — falling through to MovingNode would
                // silently pivot the gesture from resize to move.
                if let Some(edge_key) = hit_edge_label.take() {
                    if let Some(doc) = ctx.document.as_mut() {
                        let edge_ref = crate::application::document::EdgeRef::new(
                            &edge_key.from_id,
                            &edge_key.to_id,
                            &edge_key.edge_type,
                        );
                        if let Some(original) =
                            doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).cloned()
                        {
                            // Capture `prev` BEFORE the assignment —
                            // post-write capture would always read
                            // the new EdgeLabel selection back, so
                            // `rebuild_after_selection_change` would
                            // see prev == new and pick scene-only
                            // even from a `Single(node)` start. The
                            // node's tree-text highlight would then
                            // stay painted cyan through the drag.
                            let prev = doc.selection.clone();
                            doc.selection = SelectionState::EdgeLabel(
                                crate::application::document::EdgeLabelSel::new(edge_ref.clone()),
                            );
                            ctx.scene_cache.clear();
                            *ctx.drag_state = DragState::throttled(ThrottledDrag::EdgeLabel(
                                EdgeLabelInteraction::new(edge_ref, original),
                            ));
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
                                ctx.interaction_mode,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                            return;
                        }
                    }
                    // EdgeLabel hit consumed, validation missed
                    // (edge deleted mid-press / `as_mut` failed) —
                    // abort rather than fall through to MovingNode.
                    return;
                }
                if let Some((edge_key, endpoint)) = hit_portal_label.take() {
                    if let Some(doc) = ctx.document.as_mut() {
                        let edge_ref = crate::application::document::EdgeRef::new(
                            &edge_key.from_id,
                            &edge_key.to_id,
                            &edge_key.edge_type,
                        );
                        let original = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).cloned();
                        if let Some(original) = original {
                            doc.selection =
                                SelectionState::PortalLabel(crate::application::document::PortalLabelSel {
                                    edge_key,
                                    endpoint_node_id: endpoint.clone(),
                                });
                            ctx.scene_cache.clear();
                            *ctx.drag_state = DragState::throttled(ThrottledDrag::PortalLabel(
                                PortalLabelInteraction::new(edge_ref, endpoint, original),
                            ));
                            rebuild_all(
                                doc,
                                ctx.interaction_mode,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                            return;
                        }
                    }
                    // PortalLabel hit consumed, validation missed
                    // — abort rather than fall through.
                    return;
                }
                if let Some((edge_ref, handle_kind)) = hit_edge_handle.take() {
                    // Grab the pre-edit snapshot + start
                    // position so the drain loop can do
                    // absolute-positioning math.
                    if let Some(doc) = ctx.document.as_mut() {
                        if let Some(original) =
                            doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).cloned()
                        {
                            let canvas_pos = ctx
                                .renderer
                                .screen_to_canvas(start_pos.0 as f32, start_pos.1 as f32);
                            let start_handle_pos = doc
                                .hit_test_edge_handle(canvas_pos, &edge_ref, f32::INFINITY)
                                .map(|(_, p)| p)
                                .unwrap_or(canvas_pos);
                            ctx.scene_cache.clear();
                            *ctx.drag_state = DragState::throttled(ThrottledDrag::EdgeHandle(
                                EdgeHandleInteraction::new(edge_ref, handle_kind, original, start_handle_pos),
                            ));
                            return;
                        }
                    }
                    // EdgeHandle hit consumed, validation missed
                    // — abort rather than fall through.
                    return;
                }
                if let Some((node_id, side)) = hit_node_resize_handle.take() {
                    // Snapshot the node's pre-drag (position, size)
                    // so the drain math + release commit derive
                    // from a stable base. Skip if the node vanished
                    // between press and threshold (deletion mid-
                    // press) or had its size go non-finite /
                    // non-positive (selection-gating means we
                    // shouldn't see this; defensive against
                    // model mutations through the console mid-
                    // press).
                    if let Some(doc) = ctx.document.as_mut() {
                        if let Some(node) = doc.mindmap.nodes.get(&node_id) {
                            if node.size.width.is_finite()
                                && node.size.height.is_finite()
                                && node.size.width > 0.0
                                && node.size.height > 0.0
                            {
                                let start_position = node.position;
                                let start_size = node.size;
                                // Same demote-on-press as the
                                // whole-node move arm — a
                                // `MultiSection`/`Section` selection
                                // on the resized node would otherwise
                                // leave mid-drag picker hints reading
                                // a section state while the user
                                // bodily resizes the parent.
                                if let Some(new_sel) =
                                    selection_after_node_drag_press(&doc.selection, &node_id)
                                {
                                    doc.selection = new_sel;
                                    rebuild_selection_highlight(
                                        doc,
                                        ctx.interaction_mode,
                                        ctx.mindmap_tree,
                                        ctx.renderer,
                                    );
                                }
                                ctx.scene_cache.clear();
                                *ctx.drag_state = DragState::throttled(ThrottledDrag::NodeResize(
                                    NodeResizeInteraction::new(
                                        node_id,
                                        side,
                                        start_position,
                                        start_size,
                                        // Left-button handle drag — `right_release`
                                        // finalize must not fire on this.
                                        false,
                                    ),
                                ));
                                return;
                            }
                        }
                    }
                    // NodeResize handle consumed, validation missed
                    // (node deleted / non-finite size) — abort
                    // rather than fall through to MovingNode on
                    // the same `hit_node`.
                    return;
                }
                if let Some((node_id, section_idx, side)) = hit_section_resize_handle.take() {
                    // Snapshot the section's pre-drag offset/size
                    // so the drain math + release commit derive
                    // from a stable base. Skip the gesture if the
                    // section vanished between press and threshold
                    // (deletion mid-drag) or its size went `None`
                    // (selection-gating means we shouldn't see this
                    // in practice, but the per-frame dispatch
                    // shouldn't crash on a model the user mutated
                    // through the console mid-press).
                    if let Some(doc) = ctx.document.as_mut() {
                        if let Some(node) = doc.mindmap.nodes.get(&node_id) {
                            if let Some(section) = node.sections.get(section_idx) {
                                if let Some(start_size) = section.size {
                                    let start_offset = section.offset;
                                    // Same demote-on-press as the
                                    // section-move arm — a
                                    // `MultiSection` containing the
                                    // resized section demotes to
                                    // `Section(node, idx)` so the
                                    // mid-drag picker hint matches
                                    // the in-flight gesture.
                                    if let Some(new_sel) = selection_after_section_drag_press(
                                        &doc.selection,
                                        &node_id,
                                        section_idx,
                                    ) {
                                        doc.selection = new_sel;
                                        rebuild_selection_highlight(
                                            doc,
                                            ctx.interaction_mode,
                                            ctx.mindmap_tree,
                                            ctx.renderer,
                                        );
                                    }
                                    ctx.scene_cache.clear();
                                    *ctx.drag_state = DragState::throttled(ThrottledDrag::SectionResize(
                                        SectionResizeInteraction::new(
                                            node_id,
                                            section_idx,
                                            side,
                                            start_offset,
                                            start_size,
                                            // Left-button handle drag.
                                            false,
                                        ),
                                    ));
                                    return;
                                }
                            }
                        }
                    }
                    // SectionResize handle consumed, validation
                    // missed — abort rather than fall through to
                    // MovingNode/MovingSection on the same press.
                    return;
                }
                if let Some(node_id) = hit_node.take() {
                    // Snapshot before demote — shift+drag
                    // harvest reads from this so the demote
                    // below doesn't shrink the multi-set scope
                    // to one.
                    let pre_demote_ids: Vec<String> = ctx
                        .document
                        .as_ref()
                        .map(|d| d.selection.dedup_owning_node_ids())
                        .unwrap_or_default();

                    // Multi-section + non-shift hits drag only the
                    // pressed section's offset; everything else
                    // (single-section, shift-multi-select) falls
                    // through to whole-node drag, mirroring
                    // `hit_test_target`'s single-section fold.
                    if let Some((section_idx, ox, oy)) = resolve_section_drag_target(
                        ctx.document.as_ref(),
                        ctx.interaction_mode,
                        &node_id,
                        *hit_section_idx,
                        ctx.modifiers.shift_key(),
                    ) {
                        if let Some(doc) = ctx.document.as_mut() {
                            if let Some(new_sel) =
                                selection_after_section_drag_press(&doc.selection, &node_id, section_idx)
                            {
                                doc.selection = new_sel;
                                rebuild_selection_highlight(
                                    doc,
                                    ctx.interaction_mode,
                                    ctx.mindmap_tree,
                                    ctx.renderer,
                                );
                            }
                        }
                        ctx.scene_cache.clear();
                        *ctx.drag_state = DragState::throttled(ThrottledDrag::MovingSection(
                            MovingSectionInteraction::new(node_id, section_idx, (ox, oy)),
                        ));
                        return;
                    }
                    // Whole-node drag fall-through: ensure the
                    // dragged node is selected as a *node*, not a
                    // section sub-selection. A Section selection
                    // on the dragged node satisfies `is_selected`
                    // but leaves the picker hint and per-section
                    // verbs reading "Section[K]" while the user
                    // bodily moves the parent — incoherent. Demote
                    // a same-node Section selection to Single
                    // here so mid-drag UX matches the gesture
                    // (release rebuild lands the same coherent
                    // shape).
                    if let Some(doc) = ctx.document.as_mut() {
                        if let Some(new_sel) = selection_after_node_drag_press(&doc.selection, &node_id) {
                            doc.selection = new_sel;
                            rebuild_selection_highlight(
                                doc,
                                ctx.interaction_mode,
                                ctx.mindmap_tree,
                                ctx.renderer,
                            );
                        }
                    }
                    // Shift+drag: move all selected nodes together.
                    // Read from the pre-demote snapshot — the
                    // demote above may have just narrowed the
                    // doc selection to `Single(node_id)`, which
                    // would silently drop every other node from
                    // a `Multi` / `MultiSection` set out of the
                    // drag scope.
                    let node_ids = if ctx.modifiers.shift_key() {
                        let mut ids = pre_demote_ids;
                        if !ids.contains(&node_id) {
                            ids.push(node_id);
                        }
                        ids
                    } else {
                        vec![node_id]
                    };
                    // Start each drag with a clean scene cache so
                    // the keyed-edge rebuild picks up the moving
                    // node's edges from scratch.
                    ctx.scene_cache.clear();
                    let individual = ctx.modifiers.alt_key();
                    let mut descendant_ids = std::collections::HashSet::new();
                    if !individual {
                        if let Some(doc) = ctx.document.as_ref() {
                            for nid in &node_ids {
                                descendant_ids.extend(doc.mindmap.all_descendants(nid));
                            }
                        }
                    }
                    *ctx.drag_state = DragState::throttled(ThrottledDrag::MovingNode(
                        MovingNodeInteraction::new(node_ids, individual, descendant_ids),
                    ));
                } else if ctx.modifiers.shift_key() {
                    // Shift+drag on empty space: rubber-band selection
                    let start_canvas = ctx
                        .renderer
                        .screen_to_canvas(start_pos.0 as f32, start_pos.1 as f32);
                    let current_canvas = ctx
                        .renderer
                        .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
                    *ctx.drag_state = DragState::SelectingRect {
                        start_canvas,
                        current_canvas,
                    };
                } else {
                    // LeftDrag-on-empty. Honor the user's binding:
                    // whatever Action is bound to `LeftDrag`
                    // (default `PanCanvas`) goes through the §3
                    // funnel, so rebinding the gesture rebinds the
                    // behavior and a macro-visible Action isn't
                    // shadowed by a second copy of its body here.
                    // Same shape as the `RightDrag` threshold-cross
                    // arm below and the `MiddleClick` press arm in
                    // `event_mouse_click.rs`. If the user unbound
                    // `LeftDrag`, nothing fires.
                    // `action_for_gesture` falls back to unmodified
                    // when no exact-modifier binding exists, so
                    // Ctrl+LeftDrag-on-empty pans like a bare
                    // LeftDrag-on-empty did pre-branch.
                    let action = ctx.keybinds.action_for_gesture(
                        crate::application::keybinds::MouseGesture::LeftDrag.key_name(),
                        ctx.modifiers.control_key(),
                        ctx.modifiers.shift_key(),
                        ctx.modifiers.alt_key(),
                    );
                    if let Some(a) = action {
                        // Copy the press position out of
                        // `DragState::Pending` before dispatch:
                        // `start_pos` borrows `*ctx.drag_state`, and
                        // the arm bodies take `ctx` mutably (and may
                        // replace the drag state outright).
                        let press_pos = *start_pos;
                        let canvas_pos = ctx
                            .renderer
                            .screen_to_canvas(press_pos.0 as f32, press_pos.1 as f32);
                        // This branch is the empty-canvas leg of the
                        // Pending fan-out (no node / handle / label
                        // hit), so the hit is `Empty` by
                        // construction. Carrying it lets
                        // hit-consuming Actions bound to `LeftDrag`
                        // work here the same way they do off
                        // `RightDrag`.
                        let dispatch_hit = super::dispatch::DispatchHit {
                            click_hit: super::ClickHit::Empty,
                            canvas_pos,
                        };
                        super::dispatch::dispatch_action(a, ctx, Some(&dispatch_hit));
                        // If the arm didn't take ownership of the
                        // gesture the state is still `Pending`, and
                        // every following cursor move would re-cross
                        // the threshold and re-fire the same Action.
                        // Clear it so a rebound one-shot Action fires
                        // exactly once per press — the same guard the
                        // `RightDrag` threshold-cross arm below
                        // applies to `PendingRight`. Skipped entirely
                        // when nothing is bound, so an unbound
                        // `LeftDrag` still leaves `Pending` intact for
                        // the release handler's click path.
                        if matches!(*ctx.drag_state, DragState::Pending { .. }) {
                            *ctx.drag_state = DragState::None;
                        }
                    }
                    // Per-frame continuous-gesture state stays inline
                    // (CODE_CONVENTIONS §3): the funnel owns the
                    // discrete entry (`Action::PanCanvas` →
                    // `DragState::Panning`) and this emits the
                    // threshold-crossing frame's delta so the pan
                    // doesn't visibly lag one cursor-move behind the
                    // gesture.
                    if emits_first_pan_delta(ctx.drag_state) {
                        let dx = cursor_pos_val.0 - prev_pos.0;
                        let dy = cursor_pos_val.1 - prev_pos.1;
                        ctx.renderer
                            .process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                    }
                }
            }
        }
        DragState::SelectingRect { current_canvas, .. } => {
            *current_canvas = ctx
                .renderer
                .screen_to_canvas(cursor_pos_val.0 as f32, cursor_pos_val.1 as f32);
        }
        DragState::PendingRight {
            start_pos,
            start_canvas,
            hit_node,
            hit_section_idx,
        } => {
            // Threshold-cross arm for the right-button fast-resize
            // gesture (`SECTIONS_BORDERS_RESIZE_PLAN.md` §6.3).
            // Same `DRAG_THRESHOLD_SQ_PX` as the left-button arm
            // above. The DispatchHit carries the **press-time**
            // canvas position and hit (not the threshold-cross
            // values) so anchor inference fires from "where the
            // user pressed", not "where the cursor is now". Plan
            // §6.3: "Quadrant determined at press time, not
            // continuously".
            let dist_x = cursor_pos_val.0 - start_pos.0;
            let dist_y = cursor_pos_val.1 - start_pos.1;
            if dist_x * dist_x + dist_y * dist_y <= super::DRAG_THRESHOLD_SQ_PX {
                return;
            }
            // Look up the bound action via `action_for_gesture` so a
            // user can rebind `RightDrag` away from `FastResizeStart`
            // (or onto bare `RightDrag` without the Ctrl modifier).
            let name = crate::application::keybinds::MouseGesture::RightDrag.key_name();
            let action = ctx.keybinds.action_for_gesture(
                name,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
            );
            if let Some(a) = action {
                // Snapshot press-time data BEFORE dispatch (which
                // may consume PendingRight). Cloning the String is
                // cheap and the alternative — reading post-dispatch
                // — would race with the arm's state mutation.
                let click_hit = match (hit_node.clone(), *hit_section_idx) {
                    (Some(id), Some(idx)) => super::ClickHit::Node(id, Some(idx)),
                    (Some(id), None) => super::ClickHit::Node(id, None),
                    (None, _) => super::ClickHit::Empty,
                };
                let dispatch_hit = super::dispatch::DispatchHit {
                    click_hit,
                    canvas_pos: *start_canvas,
                };
                super::dispatch::dispatch_action(a, ctx, Some(&dispatch_hit));
            }
            // After dispatch the state should be `Throttled(...)` if
            // the arm took ownership of the gesture. If the arm
            // didn't run (no binding) or couldn't resolve a target,
            // reset to None so subsequent cursor moves don't re-fire
            // the threshold-cross.
            if matches!(*ctx.drag_state, DragState::PendingRight { .. }) {
                *ctx.drag_state = DragState::None;
            }
        }
        DragState::None => {}
    }
}

/// Project one screen-space cursor move into the canvas-space
/// [`DragInput`] every throttled drag consumes.
///
/// Both ends go through `screen_to_canvas` because the camera
/// transform (zoom + pan) lives in the renderer, and converting at
/// both ends is the only honest way to derive a delta that survives
/// an interleaved camera pan. The absolute half is the same
/// projection the delta's right-hand term already needed, so the
/// two forms cost one conversion pair between them.
fn drag_input(
    renderer: &crate::application::renderer::Renderer,
    prev: (f64, f64),
    curr: (f64, f64),
) -> DragInput {
    DragInput::between(
        renderer.screen_to_canvas(prev.0 as f32, prev.1 as f32),
        renderer.screen_to_canvas(curr.0 as f32, curr.1 as f32),
    )
}

/// Whether the `LeftDrag`-on-empty threshold-crossing frame should
/// also emit its camera-pan delta, read *after* the bound Action
/// has been dispatched.
///
/// True exactly when the dispatch left us in `DragState::Panning`
/// — i.e. the Action bound to `LeftDrag` was `PanCanvas` (or a
/// future Action that also enters pan mode). Before the funnel fix
/// this arm hardcoded `action == Some(PanCanvas)` and set
/// `DragState::Panning` inline, so an Action rebound onto
/// `LeftDrag` fired nothing at all. Now the funnel decides and this
/// predicate reads the decision back, which also means an Action
/// that starts some *other* gesture doesn't get a free camera
/// nudge on its first frame.
fn emits_first_pan_delta(drag_state: &DragState) -> bool {
    matches!(drag_state, DragState::Panning)
}

/// Map a `ResizeHandleSide` to the matching winit `CursorIcon`
/// for the corresponding 8-handle resize cursor. Diagonal corners
/// map to `NwseResize` / `NeswResize`; edge midpoints map to the
/// vertical / horizontal resize cursors. Used by both
/// handle-driven Resize-mode drags and right-button fast-resize
/// gestures (`SECTIONS_BORDERS_RESIZE_PLAN.md` §6.5).
fn cursor_icon_for_resize_side(side: baumhard::mindmap::tree_builder::ResizeHandleSide) -> CursorIcon {
    use baumhard::mindmap::tree_builder::ResizeHandleSide as S;
    match side {
        // Diagonal corners — NW/SE share \ axis, NE/SW share / axis.
        S::NW | S::SE => CursorIcon::NwseResize,
        S::NE | S::SW => CursorIcon::NeswResize,
        // Edge midpoints.
        S::N | S::S => CursorIcon::NsResize,
        S::E | S::W => CursorIcon::EwResize,
    }
}

/// Decide whether a press on `node_id` with `hit_section_idx` and
/// the given shift modifier should promote to section drag rather
/// than the default whole-node drag. Returns `Some((idx, ox, oy))`
/// when the section's offset can be captured for the drag
/// `start_offset`; `None` when the press should fall through to
/// the existing whole-node path.
///
/// Three gates, applied in order:
/// 1. **Shift** — reserved for multi-node selection; shift+drag on
///    a section falls through to whole-node drag.
/// 2. **Multi-section node** — `hit_test_target`'s single-section
///    fold means single-section nodes never produce a section hit
///    in the first place, but the redundant check here is a
///    cheap defense against a future drift.
/// 3. **`InteractionMode::NodeEdit { matching_id }`** — outside
///    NodeEdit, drags on a section's area move the whole node
///    (consistent with click-on-section folding to `Single` per
///    `click_resolves_to_section`). Plan §4.1.
pub(super) fn resolve_section_drag_target(
    doc: Option<&crate::application::document::MindMapDocument>,
    interaction_mode: &super::InteractionMode,
    node_id: &str,
    hit_section_idx: Option<usize>,
    shift: bool,
) -> Option<(usize, f64, f64)> {
    if shift {
        return None;
    }
    if !interaction_mode.click_resolves_to_section(node_id) {
        return None;
    }
    let idx = hit_section_idx?;
    let node = doc?.mindmap.nodes.get(node_id)?;
    if node.sections.len() <= 1 {
        return None;
    }
    node.sections.get(idx).map(|s| (idx, s.offset.x, s.offset.y))
}

/// Decide what the selection should become when a section drag
/// promotes from `Pending` to `Throttled(MovingSection)`. Returns
/// `Some(new_sel)` when the selection needs to be rewritten;
/// `None` when the press lands exactly on the already-selected
/// `Section(node_id, section_idx)` and no rewrite is needed.
///
/// **Demote discipline.** The gesture is "move this section",
/// so the selection narrows to a single `Section`. A pre-existing
/// `MultiSection` set IS demoted — mirroring the whole-node arm
/// (`selection_after_node_drag_press`) which collapses the same
/// multi-section state to `Single(node_id)`. Without the demote
/// the picker hint + per-section verbs would read out a
/// multi-section state mid-drag while the user bodily moves a
/// single section.
pub(super) fn selection_after_section_drag_press(
    prev: &SelectionState,
    node_id: &str,
    section_idx: usize,
) -> Option<SelectionState> {
    let target = crate::application::document::SectionSel {
        node_id: node_id.to_string(),
        section_idx,
    };
    if matches!(prev, SelectionState::Section(s) if s == &target) {
        return None;
    }
    Some(SelectionState::Section(target))
}

/// Decide what the selection should become when a whole-node
/// drag promotes from `Pending` to `Throttled(MovingNode)`.
/// Returns `Some(new_sel)` when the selection needs rewriting;
/// `None` to keep the existing selection (the dragged node is
/// already part of a `Multi(ids)` / `Single(node_id)` set).
///
/// `Section` / `MultiSection` selections that touch the dragged
/// node demote to `Single(node_id)` — the gesture is "move this
/// node", not "operate on these sections".
pub(super) fn selection_after_node_drag_press(
    prev: &SelectionState,
    node_id: &str,
) -> Option<SelectionState> {
    let needs_demote = match prev {
        SelectionState::Section(s) => s.node_id == node_id,
        SelectionState::MultiSection(secs) => secs.iter().any(|s| s.node_id == node_id),
        SelectionState::SectionRange { sel, .. } => sel.node_id == node_id,
        _ => false,
    };
    if needs_demote || !prev.is_selected(node_id) {
        Some(SelectionState::Single(node_id.to_string()))
    } else {
        None
    }
}

/// Rebuild the tree's selection highlight from the current
/// `doc.selection`. Shared by the section-drag and whole-node
/// drag promotion arms — both set selection then need the tree
/// + renderer buffers refreshed to reflect the new highlight.
fn rebuild_selection_highlight(
    doc: &mut crate::application::document::MindMapDocument,
    interaction_mode: &super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut crate::application::renderer::Renderer,
) {
    if let Some(tree) = mindmap_tree.as_mut() {
        // Every overlay, in the one correct order — see
        // `build_overlaid_tree`. Highlights route through the
        // canonical `selection_highlight_entries` helper: Section /
        // MultiSection narrow to the selected sections, whole-node
        // selections paint every section. `interaction_mode` is
        // threaded in for the `NodeEdit` dim; without it a section
        // drag mid-edit snapped every other node's text back to
        // full opacity while its border stayed dimmed.
        let new_tree = super::scene_rebuild::build_overlaid_tree(
            doc,
            interaction_mode,
            super::scene_rebuild::selection_highlight_entries(&doc.selection),
        );
        renderer.rebuild_buffers_from_tree(&new_tree.tree);
        *tree = new_tree;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cursor_icon_for_resize_side, emits_first_pan_delta, resolve_section_drag_target,
        selection_after_node_drag_press, selection_after_section_drag_press, DragState,
    };
    use crate::application::app::InteractionMode;
    use crate::application::document::tests_common::{load_test_doc, pinned_two_section_node};
    use crate::application::document::{SectionSel, SelectionState};
    use crate::application::platform::window::CursorIcon;
    use baumhard::mindmap::tree_builder::ResizeHandleSide;

    /// The `LeftDrag`-on-empty arm dispatches whatever Action the
    /// user bound, then emits the threshold frame's pan delta only
    /// if that dispatch entered pan mode. `Panning` is the sole
    /// state that qualifies — a rebound Action that promotes the
    /// press to a rect-select or a throttled drag must not also
    /// move the camera.
    #[test]
    fn test_emits_first_pan_delta_only_in_panning_state() {
        assert!(emits_first_pan_delta(&DragState::Panning));
        assert!(!emits_first_pan_delta(&DragState::None));
        assert!(!emits_first_pan_delta(&DragState::SelectingRect {
            start_canvas: glam::Vec2::ZERO,
            current_canvas: glam::Vec2::ZERO,
        }));
    }

    /// Regression pin for the funnel gap: the arm resolves the
    /// `LeftDrag` binding with **no filter**, so rebinding the
    /// gesture to a non-`PanCanvas` Action yields that Action.
    /// The pre-fix arm compared the lookup against
    /// `Some(Action::PanCanvas)` and dropped everything else on
    /// the floor — this asserts the comparison is gone and the
    /// gesture name / modifier plumbing the arm uses actually
    /// reaches the rebound entry.
    #[test]
    fn test_leftdrag_gesture_lookup_honors_a_non_pan_rebinding() {
        use crate::application::keybinds::{Action, KeybindConfig, MouseGesture};
        let mut cfg = KeybindConfig::default();
        cfg.pan_canvas.retain(|b| b != "LeftDrag");
        cfg.zoom_fit.push("LeftDrag".into());
        let resolved = cfg.resolve();
        assert_eq!(
            resolved.action_for_gesture(MouseGesture::LeftDrag.key_name(), false, false, false),
            Some(Action::ZoomFit),
            "a rebound LeftDrag must resolve to the user's Action, \
             not be filtered down to PanCanvas"
        );
    }

    /// Pure 8→4 mapping: every `ResizeHandleSide` lands on the
    /// matching winit `CursorIcon` for direction-appropriate
    /// resize feedback. Pinned per-side so a future refactor that
    /// (e.g.) swaps NW and SW silently leaves users with the
    /// wrong cursor on every diagonal grab. Test agent flagged
    /// this as the most important coverage gap in Batch 4.
    #[test]
    fn cursor_icon_for_resize_side_pin_per_side() {
        // Diagonals share an axis: NW/SE = `\` = NwseResize.
        //                          NE/SW = `/` = NeswResize.
        assert_eq!(
            cursor_icon_for_resize_side(ResizeHandleSide::NW),
            CursorIcon::NwseResize
        );
        assert_eq!(
            cursor_icon_for_resize_side(ResizeHandleSide::SE),
            CursorIcon::NwseResize
        );
        assert_eq!(
            cursor_icon_for_resize_side(ResizeHandleSide::NE),
            CursorIcon::NeswResize
        );
        assert_eq!(
            cursor_icon_for_resize_side(ResizeHandleSide::SW),
            CursorIcon::NeswResize
        );
        // Edge midpoints.
        assert_eq!(
            cursor_icon_for_resize_side(ResizeHandleSide::N),
            CursorIcon::NsResize
        );
        assert_eq!(
            cursor_icon_for_resize_side(ResizeHandleSide::S),
            CursorIcon::NsResize
        );
        assert_eq!(
            cursor_icon_for_resize_side(ResizeHandleSide::E),
            CursorIcon::EwResize
        );
        assert_eq!(
            cursor_icon_for_resize_side(ResizeHandleSide::W),
            CursorIcon::EwResize
        );
    }

    /// Helper: NodeEdit mode targeting `node_id` — the mode that
    /// licenses section-drag promotion.
    fn node_edit_for(node_id: &str) -> InteractionMode {
        InteractionMode::NodeEdit {
            node_id: node_id.to_string(),
        }
    }

    /// Multi-section node + non-shift + valid section_idx + NodeEdit
    /// mode → drag the section. Pins the threshold-cross promotion
    /// gate.
    #[test]
    fn test_resolve_section_drag_target_multi_section_non_shift_returns_some() {
        let (doc, id) = pinned_two_section_node();
        let mode = node_edit_for(&id);
        let result = resolve_section_drag_target(Some(&doc), &mode, &id, Some(1), false);
        assert!(
            result.is_some(),
            "multi-section + non-shift + NodeEdit must promote"
        );
        let (idx, _, _) = result.unwrap();
        assert_eq!(idx, 1);
    }

    /// `section_idx=0` on a multi-section node also drags — the
    /// gate is `sections.len() > 1`, not `idx > 0`.
    #[test]
    fn test_resolve_section_drag_target_section_zero_on_multi_section_returns_some() {
        let (doc, id) = pinned_two_section_node();
        let mode = node_edit_for(&id);
        let result = resolve_section_drag_target(Some(&doc), &mode, &id, Some(0), false);
        assert!(result.is_some(), "section_idx=0 on multi-section must promote");
    }

    /// **NEW: Default mode never promotes.** Plan §4.1 — outside
    /// NodeEdit, drag-on-section behaves identically to drag-on-
    /// node-body (whole-node drag). Same Multi-section node, same
    /// hit, but mode is `Default` instead of `NodeEdit`.
    #[test]
    fn test_resolve_section_drag_target_default_mode_returns_none() {
        let (doc, id) = pinned_two_section_node();
        let result = resolve_section_drag_target(Some(&doc), &InteractionMode::Default, &id, Some(1), false);
        assert!(result.is_none(), "Default mode must NOT promote section drag");
    }

    /// **NEW: NodeEdit on a different node never promotes.** A user
    /// in `NodeEdit { "0" }` who drags on a section of `"1"` gets
    /// whole-node drag, not section-drag.
    #[test]
    fn test_resolve_section_drag_target_node_edit_mismatch_returns_none() {
        let (doc, id) = pinned_two_section_node();
        let mode = node_edit_for("some-other-node-id");
        let result = resolve_section_drag_target(Some(&doc), &mode, &id, Some(1), false);
        assert!(
            result.is_none(),
            "NodeEdit on a different node must NOT promote section drag on this one"
        );
    }

    /// Single-section node falls to whole-node drag — mirrors
    /// `hit_test_target`'s single-section fold to `NodeContainer`.
    #[test]
    fn test_resolve_section_drag_target_single_section_returns_none() {
        let mut doc = load_test_doc();
        let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
        if let Some(n) = doc.mindmap.nodes.get_mut(&nid) {
            n.sections.truncate(1);
        }
        let mode = node_edit_for(&nid);
        let result = resolve_section_drag_target(Some(&doc), &mode, &nid, Some(0), false);
        assert!(result.is_none(), "single-section node must NOT promote");
    }

    /// Shift+drag-on-section falls to whole-node drag (multi-select
    /// discipline). Pins the shift gate; mode-gate is moot here.
    #[test]
    fn test_resolve_section_drag_target_shift_returns_none() {
        let (doc, id) = pinned_two_section_node();
        let mode = node_edit_for(&id);
        let result = resolve_section_drag_target(Some(&doc), &mode, &id, Some(1), true);
        assert!(result.is_none(), "shift+drag must fall to whole-node");
    }

    /// Out-of-range section index → fall-through (no panic, no
    /// mis-promotion).
    #[test]
    fn test_resolve_section_drag_target_out_of_range_returns_none() {
        let (doc, id) = pinned_two_section_node();
        let mode = node_edit_for(&id);
        let result = resolve_section_drag_target(Some(&doc), &mode, &id, Some(99), false);
        assert!(result.is_none());
    }

    /// `None` document or `None` hit_section_idx → fall-through.
    #[test]
    fn test_resolve_section_drag_target_no_doc_or_idx_returns_none() {
        assert!(resolve_section_drag_target(None, &node_edit_for("0"), "0", Some(0), false).is_none());
        let (doc, id) = pinned_two_section_node();
        let mode = node_edit_for(&id);
        assert!(resolve_section_drag_target(Some(&doc), &mode, &id, None, false).is_none());
    }

    // ── Selection-after-press helpers ────────────────────────────

    /// Pressing on the already-selected `Section(node, idx)` does
    /// not rewrite the selection — pins the no-op early-out so a
    /// section drag started on its own selection doesn't trigger
    /// a redundant tree highlight rebuild.
    #[test]
    fn test_section_drag_press_on_already_selected_section_returns_none() {
        let prev = SelectionState::Section(SectionSel::new("a", 1));
        assert!(selection_after_section_drag_press(&prev, "a", 1).is_none());
    }

    /// **Demote-on-press for MultiSection.** Pressing on a section
    /// that lives inside a `MultiSection` set demotes the entire
    /// set down to a single `Section`. Pins the parallel of the
    /// whole-node-arm demote (a multi-section selection on the
    /// dragged node demotes to `Single(node_id)`).
    #[test]
    fn test_section_drag_press_demotes_multisection_to_section() {
        let prev = SelectionState::MultiSection(vec![
            SectionSel::new("a", 0),
            SectionSel::new("a", 1),
            SectionSel::new("b", 0),
        ]);
        let new = selection_after_section_drag_press(&prev, "a", 1).expect("rewrite");
        match new {
            SelectionState::Section(s) => assert_eq!(s, SectionSel::new("a", 1)),
            other => panic!("expected Section, got {:?}", other),
        }
    }

    /// `Section(node, k)` press on a different `(node, j)` pair
    /// rewrites the selection to the new pair — narrows from one
    /// section to another when the user clicks a sibling section.
    #[test]
    fn test_section_drag_press_rewrites_when_different_section() {
        let prev = SelectionState::Section(SectionSel::new("a", 0));
        let new = selection_after_section_drag_press(&prev, "a", 1).expect("rewrite");
        assert!(matches!(
            new,
            SelectionState::Section(s) if s == SectionSel::new("a", 1)
        ));
    }

    /// Whole-node press on a `Single(node)` matching the dragged
    /// id is a no-op — the dragged node is already the selected
    /// node, no rewrite + no highlight churn needed.
    #[test]
    fn test_node_drag_press_on_single_same_node_returns_none() {
        let prev = SelectionState::Single("a".into());
        assert!(selection_after_node_drag_press(&prev, "a").is_none());
    }

    /// Whole-node press on a `Multi(ids)` containing the dragged
    /// node is a no-op — the multi-set is preserved so the
    /// shift+drag harvest below sees every selected node.
    #[test]
    fn test_node_drag_press_on_multi_containing_node_returns_none() {
        let prev = SelectionState::Multi(vec!["a".into(), "b".into()]);
        assert!(selection_after_node_drag_press(&prev, "a").is_none());
    }

    /// Whole-node press on a `Section(node)` for the same node
    /// demotes to `Single(node)` — the gesture is to move the
    /// parent node, not operate on the section.
    #[test]
    fn test_node_drag_press_demotes_section_to_single() {
        let prev = SelectionState::Section(SectionSel::new("a", 1));
        let new = selection_after_node_drag_press(&prev, "a").expect("rewrite");
        assert!(matches!(new, SelectionState::Single(id) if id == "a"));
    }

    /// Whole-node press on a `SectionRange` whose owning node
    /// matches demotes to `Single(node)` — same shape as the
    /// `Section` arm. Pins the missed-arm fix from the N4-C.a
    /// review.
    #[test]
    fn test_node_drag_press_demotes_section_range_to_single() {
        let prev = SelectionState::SectionRange {
            sel: SectionSel::new("a", 1),
            range: (3, 7),
        };
        let new = selection_after_node_drag_press(&prev, "a").expect("rewrite");
        assert!(matches!(new, SelectionState::Single(id) if id == "a"));
    }

    /// Whole-node press on a `MultiSection` containing the dragged
    /// node demotes to `Single(node)`. Pins the parallel of the
    /// section-drag arm's demote.
    #[test]
    fn test_node_drag_press_demotes_multisection_to_single() {
        let prev = SelectionState::MultiSection(vec![SectionSel::new("a", 0), SectionSel::new("b", 0)]);
        let new = selection_after_node_drag_press(&prev, "a").expect("rewrite");
        assert!(matches!(new, SelectionState::Single(id) if id == "a"));
    }

    /// Whole-node press on a selection that doesn't include the
    /// dragged node rewrites to `Single(node)` — the user clicked
    /// to grab a fresh node, the prior selection is reset.
    #[test]
    fn test_node_drag_press_replaces_when_node_not_selected() {
        let prev = SelectionState::Single("b".into());
        let new = selection_after_node_drag_press(&prev, "a").expect("rewrite");
        assert!(matches!(new, SelectionState::Single(id) if id == "a"));
    }

    /// **Pre-demote snapshot for shift+drag harvest.** A
    /// `MultiSection` set's `dedup_owning_node_ids()` is what the
    /// shift+drag harvest reads — pin that this snapshot
    /// preserves every owning node when a single section's drag
    /// would otherwise demote the set down to one. Without this
    /// pre-demote capture the demote runs *before* the harvest
    /// and the drag scope shrinks to one node.
    #[test]
    fn test_multisection_pre_demote_snapshot_preserves_all_nodes() {
        let prev = SelectionState::MultiSection(vec![
            SectionSel::new("a", 0),
            SectionSel::new("b", 1),
            SectionSel::new("c", 0),
        ]);
        // What the call site captures BEFORE the demote runs.
        let captured = prev.dedup_owning_node_ids();
        // Now simulate the demote (whole-node press path).
        let after_demote = selection_after_node_drag_press(&prev, "a")
            .map(|s| s.dedup_owning_node_ids())
            .unwrap_or_else(|| prev.dedup_owning_node_ids());
        // The captured snapshot has every node; the post-demote
        // selection has only the dragged node. The shift+drag
        // arm reads the captured snapshot, not the post-demote.
        assert_eq!(captured, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(after_demote, vec!["a".to_string()]);
    }
}
