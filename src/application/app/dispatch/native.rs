// SPDX-License-Identifier: MPL-2.0

//! `dispatch_action` — the single entry point that runs `Action`
//! bodies on native. Mouse handlers and the keyboard handler funnel
//! through here. WASM has its own dispatch path today; the
//! convergence track is documented in `WASM_CONVERGENCE.md`.
//! Adding a new behavior
//! is variant + default + arm, in that order; never inline a body in
//! a handler.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::document::{EdgeRef, SelectionState, UndoAction};
use crate::application::keybinds::Action;

use super::super::click::rebuild_all_with_mode;
use super::super::color_picker_flow::{
    apply_picker_nudge, cancel_color_picker, close_color_picker_standalone, commit_color_picker,
    commit_color_picker_to_selection, open_color_picker_standalone, picker_decline_reason, picker_op_for,
    PickerOp,
};
use super::super::console_input::{
    rebuild_console_overlay, save_console_history, save_document_to_bound_path,
};
use super::super::input_context::InputHandlerContext;
use super::super::scene_rebuild::rebuild_all;
use super::super::single_line_edit::{
    close_single_line_edit, open_single_line_edit, resolve_single_line_target, SingleLineEditTarget,
};
use super::super::{DragState, InteractionMode};
use super::apply_keybind_custom_mutation;
use crate::application::console::ConsoleState;

// `DispatchHit` lives in `cross_dispatch::pointer` — both targets'
// mouse handlers populate one now, so the payload is cross-platform.
// Re-exported through `dispatch/mod.rs` so the `super::dispatch::
// DispatchHit` import shape at the call sites is unchanged.
pub(in crate::application::app) use super::cross_dispatch::DispatchHit;

// `DispatchOutcome` lives in `cross_dispatch`; the dispatch arms
// here name it via `super::DispatchOutcome` (re-exported in
// `dispatch/mod.rs`).
use super::DispatchOutcome;

/// Quote a free-form string (typically a filesystem path) so the
/// console parser sees it as a single token. Wraps with `"..."`
/// unconditionally and escapes both `\` (→ `\\`) and `"` (→ `\"`)
/// so Windows-style paths and embedded quotes round-trip cleanly
/// through `parser::tokenize`'s quoted-string handling. Order
/// matters: backslash MUST be escaped before quote, otherwise a
/// path ending in `\` produces an unterminated quoted token.
/// Used by the parametric filesystem Action arms.
fn quote_console_arg(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 2);
    escaped.push('"');
    for ch in s.chars() {
        if ch == '\\' || ch == '"' {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('"');
    escaped
}

/// What [`Action::PanCanvas`] does with the drag state it finds.
///
/// Pure so the arm is pinnable at `DragState` level — the arm itself
/// takes an `InputHandlerContext`, i.e. a live wgpu device, which
/// TEST_CONVENTIONS §T8 keeps out of the harness. Same shape, and
/// for the same reason, as `event_mouse_click::route_middle_button`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanCanvasRoute {
    /// Nothing is in flight that a pan would destroy — enter
    /// `DragState::Panning` for the duration of the gesture.
    Arm,
    /// Another gesture owns the drag state and owes the model a
    /// commit. Leave it exactly as it is, and do nothing else.
    Refuse,
}

/// Route one `Action::PanCanvas` dispatch against the drag state it
/// finds.
///
/// The guard belongs on the Action rather than on any one of the
/// gestures that reach it (CODE_CONVENTIONS §3), because `PanCanvas`
/// has three entry points and only one of them was ever guarded:
/// `MouseGesture::MiddleClick` (routed through
/// `event_mouse_click::route_middle_button`),
/// `MouseGesture::LeftDrag` (which only runs from
/// `DragState::Pending`), and **any keyboard binding or macro step
/// naming `pan_canvas`** — `keybinds::config` declares it
/// user-bindable, `event_keyboard` dispatches with no drag-state
/// check, and `SourceTier::allows_action` does not gate it, so every
/// tier reaches it. Pre-fix that third route overwrote
/// `DragState::Throttled(..)` with `Panning` exactly the way the
/// middle button used to: the abandoned drag's
/// `commit_on_release_core` never ran, so the tree kept the dragged
/// offsets until the next model rebuild snapped them back, with no
/// undo entry to recover from. Same class as #37 item 5, one
/// dispatch surface over.
///
/// `Pending` is not in the refused class and must not be: the
/// `LeftDrag` threshold cross dispatches `PanCanvas` *from*
/// `Pending`, so refusing there would leave the left button unable
/// to pan at all.
fn route_pan_canvas(drag_state: &DragState) -> PanCanvasRoute {
    if drag_state.would_abandon_gesture() {
        PanCanvasRoute::Refuse
    } else {
        PanCanvasRoute::Arm
    }
}

/// End a rubber band that is live when a target-picker mode
/// (`Reparent` / `Connect`) takes the pointer.
///
/// The two modes swallow both halves of the left button —
/// `handle_mouse_input` consumes the release as "choose target" and
/// never reaches the `mem::replace` that ends a `SelectingRect`, and
/// `handle_cursor_moved` returns before the drag-state ladder — so a
/// band entered mid-gesture would otherwise sit frozen on the canvas
/// for the whole picker session, and its covered set would be painted
/// by every hover rebuild the mode runs.
///
/// Only `SelectingRect` is ended. The other states a picker mode can
/// interrupt are not this function's call: `Throttled(..)` owes the
/// model a commit that its own release still performs, and dropping
/// it here would be the silent loss #37 item 5 is about.
///
/// The shell around [`take_rubber_band_for_target_picker`], which is
/// the whole of it except dropping the overlay rectangle —
/// `&mut Renderer` is a live wgpu device, which TEST_CONVENTIONS §T8
/// keeps out of the harness.
fn end_target_picker_rubber_band(
    drag_state: &mut DragState,
    document: &mut Option<crate::application::document::MindMapDocument>,
    renderer: &mut crate::application::renderer::Renderer,
) {
    if take_rubber_band_for_target_picker(drag_state, document) {
        renderer.clear_overlay_buffers();
    }
}

/// The renderer-free half of [`end_target_picker_rubber_band`]:
/// drop the drag state and the covered set it authorized, and report
/// whether there was one.
///
/// The per-frame drain would end the covered set on its own next
/// frame — that is where the invariant lives, not here — but a mode
/// entry rebuilds immediately, and that rebuild would paint the
/// abandoned set once on its way out.
fn take_rubber_band_for_target_picker(
    drag_state: &mut DragState,
    document: &mut Option<crate::application::document::MindMapDocument>,
) -> bool {
    if !matches!(drag_state, DragState::SelectingRect { .. }) {
        return false;
    }
    *drag_state = DragState::None;
    document
        .as_mut()
        .and_then(crate::application::document::MindMapDocument::take_rect_select_preview);
    true
}

/// Run an `Action` against the live application context. The body of
/// every Document-level action lives here; handlers (`event_keyboard`,
/// `event_mouse_click`, the macro runtime via `dispatch_macro`)
/// construct an `InputHandlerContext` and call this.
///
/// `hit` carries mouse-event-only payload (what the click hit, where
/// the cursor was in canvas space). Keyboard / macro callers pass
/// `None`; mouse callers populate it before invoking the dispatcher.
///
/// **Two-stage dispatch.** Every call routes through the cross-
/// platform [`super::action_core::dispatch_compatible`] first.
/// On `Handled`, this returns immediately — that path covers every
/// Compatible-classified Action plus the cross-platform slice of
/// mixed-branch arms (`ExitMode`'s `last_click` clear and
///   Resize-mode reset,
/// `EditSelection*`-Single open). The native match below runs only
/// when the cross-platform dispatcher returns `Unhandled`, which
/// means one of:
///   - a NativeOnly Action whose body needs `NativeContextExt` fields
///     (console / picker / interaction_mode / drag — see
///     `Action::wasm_compatibility`'s NativeOnly classification),
///   - a mixed-branch arm's native residual (`ExitMode`'s mode
///     reset + rebuild; `EditSelection*` on EdgeLabel / Portal
///     selections),
///   - the edge-label branch of `DoubleClickActivate`. Not a
///     payload question — `dispatch_compatible` takes the same
///     `Option<&DispatchHit>` this function does, and both targets
///     populate it from their mouse handlers. What the branch needs
///     is `single_line_edit_state`, a `NativeContextExt` field the
///     browser has no counterpart for; the arm below is that one
///     step and nothing else. `CreateOrphanNodeAndEdit` no longer
///     appears here at all: `dispatch_create_orphan_and_edit` is
///     gone and the mouse path reaches
///     `apply_create_orphan_node_and_edit` through
///     `DoubleClickRoute::CreateOrphanAndEdit`,
///   - a `DoubleClickActivate` dispatched with no `DispatchHit` at
///     all (a macro, say). That is a soft-skip: nothing ran. The arm
///     below finds no target, does nothing, and returns `Unhandled`
///     itself, so this function — which is what the *native* macro
///     loop reads — reports "did not run" for it. That is a behavior
///     change on native: the arm used to return `Handled`
///     unconditionally, so a native macro step
///     `[Action(DoubleClickActivate)]` bumped `any_ran` for a step
///     that touched nothing, and stopped falling through to the
///     custom-mutation tier. WASM was fixed first; this is the same
///     fix on the other side of the seam, so the two targets now
///     answer identically for identical input.
///
/// `WASM_CONVERGENCE.md` Track C records the architecture; calling
/// `dispatch_compatible` from this fn is the seam.
pub(in crate::application::app) fn dispatch_action(
    action: Action,
    ctx: &mut InputHandlerContext<'_>,
    hit: Option<&DispatchHit>,
) -> DispatchOutcome {
    // Cross-platform stage. Bounded scope so `_` (the unused
    // `NativeContextExt` view returned by `split_borrow`) drops
    // before the outer match re-borrows `ctx`.
    let cross_outcome = {
        // `_` (not `_ext`) — the extension view is constructed by
        // `split_borrow` because it returns the pair, but the
        // cross-platform dispatcher takes only `core`. The native-
        // only arms below re-borrow from `ctx` directly after this
        // scope drops.
        let (mut core, _) = ctx.split_borrow();
        super::action_core::dispatch_compatible(&action, &mut core, hit)
    };
    if matches!(cross_outcome, DispatchOutcome::Handled) {
        return cross_outcome;
    }
    match action {
        Action::OpenConsole => {
            if ctx.console_state.is_open() {
                save_console_history(ctx.console_history);
                *ctx.console_state = ConsoleState::Closed;
                ctx.renderer.rebuild_console_overlay_buffers(ctx.app_scene, None);
            } else {
                *ctx.console_state = ConsoleState::open(ctx.console_history.clone());
                if let Some(doc) = ctx.document.as_ref() {
                    rebuild_console_overlay(
                        ctx.console_state,
                        doc,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.keybinds,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        // ── Console keystroke Actions (NativeOnly) ──────────────
        // Every `Console*` variant funnels here; the fan-out body
        // lives in `console_input::dispatch::dispatch_console_action`
        // alongside the `edit::*` private helpers it reaches.
        // Or-pattern groups all 22 variants under one arm — they
        // share the delegation. CODE_CONVENTIONS §3: the funnel
        // requires each user-named effect to be an `Action` and
        // reach `dispatch_action`; it does not require one match
        // arm per variant.
        Action::ConsoleClose
        | Action::ConsoleSubmit
        | Action::ConsoleTabComplete
        | Action::ConsoleHistoryUp
        | Action::ConsoleHistoryDown
        | Action::ConsoleCursorLeft
        | Action::ConsoleCursorRight
        | Action::ConsoleCursorHome
        | Action::ConsoleCursorEnd
        | Action::ConsoleDeleteBack
        | Action::ConsoleDeleteForward
        | Action::ConsoleInsertSpace
        | Action::ConsoleClearLine
        | Action::ConsoleJumpStart
        | Action::ConsoleJumpEnd
        | Action::ConsoleKillToStart
        | Action::ConsoleKillWord
        | Action::ConsoleScrollUp
        | Action::ConsoleScrollDown
        | Action::ConsoleScrollPageUp
        | Action::ConsoleScrollPageDown
        | Action::ConsoleScrollEnd
        | Action::ConsoleScrollHome => {
            // Every Console* body edits the open modal. With the
            // console closed there is nothing for one to act on (the
            // keyboard only reaches here while it is open), so a macro
            // step or an IPC `act.action` sending one reports that
            // nothing ran, as the hitless `DoubleClickActivate` does,
            // rather than claiming a keystroke into a modal that is
            // not there.
            if !ctx.console_state.is_open() {
                return DispatchOutcome::Unhandled;
            }
            super::super::console_input::dispatch_console_action(&action, ctx);
            DispatchOutcome::Handled
        }
        Action::ExitMode => {
            // Cross-platform slice (mode reset + rebuild on Resize)
            // already ran in `dispatch_compatible`; that branch
            // returns `Handled` and we never reach here. We arrive
            // only when mode was target-picker (Reparent / Connect)
            // — those depend on `hovered_node` from `NativeContextExt`
            // and the `rebuild_all_with_mode` overlay path (orange /
            // green highlights), so the residual stays native.
            if ctx.interaction_mode.is_target_picker() {
                *ctx.interaction_mode = InteractionMode::Default;
                *ctx.hovered_node = None;
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
            DispatchOutcome::Handled
        }
        Action::EnterResizeMode => {
            // The body is cross-platform (mode flip + scene rebuild)
            // and lives in `apply_enter_resize_mode`, but the Action
            // is `wasm = NativeOnly` until WASM gains a resize
            // gesture pipeline (no `DragState`, no handle hit-test
            // on `run_wasm/event_mouse_click.rs` today). On native,
            // the helper does the resolve + flip + rebuild.
            let (mut core, _ext) = ctx.split_borrow();
            super::action_core::with_doc_rebuild(&mut core, super::cross_dispatch::apply_enter_resize_mode);
            DispatchOutcome::Handled
        }
        Action::FastResizeStart => {
            // Fast-resize gesture start. Reads the press-time hit
            // off `ctx.drag_state` (PendingRight, set by the
            // right-button press in `event_mouse_click.rs`) and the
            // threshold-cross cursor position from
            // `hit.canvas_pos`. Computes a corner anchor via
            // `infer_resize_anchor` and transitions
            // `PendingRight → Throttled(NodeResize | SectionResize)`
            // with the chosen `ResizeHandleSide`. Drag drains and
            // release commit follow the existing left-button
            // resize plumbing (commit goes through `set_node_aabb`
            // / `set_section_aabb` on right-button release).
            apply_fast_resize_start(ctx, hit);
            DispatchOutcome::Handled
        }
        Action::EnterNodeEdit | Action::EnterNodeEditClean => {
            // NodeEdit-mode entry. Single-section nodes
            // short-circuit to the editor; multi-section nodes
            // stop at NodeEdit and the user selects a section.
            // `wasm = NativeOnly` because the editor depends on
            // `TextEditState` (modal-stealer cascade is native).
            let clean = matches!(action, Action::EnterNodeEditClean);
            let (mut core, _ext) = ctx.split_borrow();
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::RebuildContext {
                    document: doc,
                    mindmap_tree: core.mindmap_tree,
                    app_scene: core.app_scene,
                    renderer: core.renderer,
                    scene_cache: core.scene_cache,
                    interaction_mode: core.interaction_mode,
                };
                let _ = super::cross_dispatch::apply_enter_node_edit(clean, &mut rc, core.text_edit_state);
            }
            DispatchOutcome::Handled
        }
        Action::EnterSectionEdit => {
            // SectionEdit (open text editor) on the active section
            // while in NodeEdit. Same NativeOnly story as
            // EnterNodeEdit — the editor is native-only.
            let (mut core, _ext) = ctx.split_borrow();
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::RebuildContext {
                    document: doc,
                    mindmap_tree: core.mindmap_tree,
                    app_scene: core.app_scene,
                    renderer: core.renderer,
                    scene_cache: core.scene_cache,
                    interaction_mode: core.interaction_mode,
                };
                let _ = super::cross_dispatch::apply_enter_section_edit(false, &mut rc, core.text_edit_state);
            }
            DispatchOutcome::Handled
        }
        Action::EnterReparentMode => {
            // `sel` is collected out of the borrow rather than held
            // across the block: `end_target_picker_rubber_band` takes
            // `&mut Option<MindMapDocument>`, and the rebuild below
            // re-resolves the document afterwards.
            let sel: Vec<String> = ctx
                .document
                .as_ref()
                .map(|doc| {
                    doc.selection
                        .selected_ids()
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            if !sel.is_empty() {
                *ctx.interaction_mode = InteractionMode::Reparent { sources: sel };
                *ctx.hovered_node = None;
                *ctx.last_click = None;
                end_target_picker_rubber_band(ctx.drag_state, ctx.document, ctx.renderer);
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
            DispatchOutcome::Handled
        }
        Action::EnterConnectMode => {
            let source = match ctx.document.as_ref().map(|doc| &doc.selection) {
                Some(SelectionState::Single(source)) => Some(source.clone()),
                _ => None,
            };
            if let Some(source) = source {
                *ctx.interaction_mode = InteractionMode::Connect { source };
                *ctx.hovered_node = None;
                *ctx.last_click = None;
                end_target_picker_rubber_band(ctx.drag_state, ctx.document, ctx.renderer);
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
            DispatchOutcome::Handled
        }
        Action::ReparentToTarget(ref target) => {
            // Mode-exit + mutation: extract sources from
            // `interaction_mode` atomically with the reset to
            // `Default`. A stale fire outside Reparent mode silently
            // no-ops; the `mem::replace` guards against re-entry
            // leaving the mode half-reset on early return.
            let sources = match std::mem::replace(ctx.interaction_mode, InteractionMode::Default) {
                InteractionMode::Reparent { sources } => sources,
                _ => {
                    return DispatchOutcome::Handled;
                }
            };
            *ctx.hovered_node = None;
            if let Some(doc) = ctx.document.as_mut() {
                // `target` of `Some(id)` reparents under that node;
                // `None` promotes sources to root (empty-canvas click).
                let undo_data = doc.apply_reparent(&sources, target.as_deref());
                if !undo_data.entries.is_empty() {
                    doc.undo_stack.push(UndoAction::ReparentNodes {
                        entries: undo_data.entries,
                        old_edges: undo_data.old_edges,
                    });
                    doc.dirty = true;
                }
                // Full rebuild regardless: tree structure changed
                // (or even if no-op, mode-exit must clear orange/
                // green highlights).
                rebuild_all(
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::ConnectToTarget(ref target) => {
            // Mirror `ReparentToTarget`'s mode-exit pattern. Source
            // comes from `InteractionMode::Connect { source }`;
            // stale-fire outside Connect mode silently no-ops.
            // `target = None` is empty-canvas mode-exit (no edge to
            // create); the arm still runs the rebuild so orange /
            // green highlights clear.
            let source = match std::mem::replace(ctx.interaction_mode, InteractionMode::Default) {
                InteractionMode::Connect { source } => source,
                _ => {
                    return DispatchOutcome::Handled;
                }
            };
            *ctx.hovered_node = None;
            if let Some(doc) = ctx.document.as_mut() {
                if let Some(target_id) = target.as_deref() {
                    if let Some(idx) = doc.create_cross_link_edge(&source, target_id) {
                        doc.undo_stack.push(UndoAction::CreateEdge { index: idx });
                        // Snap selection to the new edge so the
                        // user gets immediate visual confirmation
                        // and can Delete or style it next.
                        doc.selection = SelectionState::Edge(EdgeRef::new(
                            source.clone(),
                            target_id.to_string(),
                            "cross_link",
                        ));
                        doc.dirty = true;
                    }
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
            DispatchOutcome::Handled
        }
        Action::EditSelection | Action::EditSelectionClean => {
            // The Single branch is owned by the cross-platform
            // dispatcher (`dispatch_compatible`'s mixed-branch slice).
            // This arm runs ONLY when that returned `Unhandled`,
            // which means selection was non-Single — so we go
            // straight to the EdgeLabel / Portal native-only
            // branches without re-checking Single.
            //
            // `clean` is threaded into the single-line editors so
            // `EditSelectionClean` keeps its empty-buffer contract
            // on edge-label / portal selections, not just on nodes.
            let clean = matches!(action, Action::EditSelectionClean);
            open_editor_for_edge_selection(clean, ctx);
            DispatchOutcome::Handled
        }
        Action::SaveDocument => match ctx.document.as_mut().map(save_document_to_bound_path) {
            Some(true) => DispatchOutcome::Handled,
            Some(false) | None => DispatchOutcome::Unhandled,
        },

        // ── Mouse-gesture Actions ──────────────────────────────
        Action::DoubleClickActivate => {
            // The cross-platform stage above ran the whole gesture
            // except one branch: an edge-label double-click commits
            // the selection there and hands back
            // `DoubleClickResidual::OpenEdgeLabelEditor`, which
            // surfaces here as `Unhandled`. The single-line editor is
            // the only piece that needs `NativeContextExt` state, so
            // it is the only piece left in this arm.
            //
            // `edge_label_target` is the same `EdgeKey` -> `EdgeRef`
            // conversion the route resolver used, so the editor
            // cannot open on a different edge than the one the
            // selection just committed to.
            let target = hit.and_then(|h| super::edge_label_target(&h.click_hit));
            match (target, ctx.document.as_mut()) {
                (Some(edge_ref), Some(doc)) => {
                    // Double-click on an edge label edits the existing
                    // text — not clean.
                    open_single_line_edit(
                        SingleLineEditTarget::EdgeLabel { edge_ref },
                        false,
                        doc,
                        ctx.single_line_edit_state,
                        ctx.app_scene,
                        ctx.renderer,
                    );
                    DispatchOutcome::Handled
                }
                // No edge-label target means no `DispatchHit` at all:
                // the cross-platform stage only hands this arm an
                // `Unhandled` for the edge-label residual (which
                // always has one) or for the hitless soft-skip. So
                // this is the soft-skip — nothing ran here and
                // nothing ran above — and it is now reported as such
                // on native too. It previously returned `Handled`,
                // which bumped the *native* macro loop's `any_ran`
                // for a step that did nothing, the mirror image of
                // the WASM misreport fixed in `c023ff9`. Both targets
                // now report the same thing for the same input.
                (None, _) => DispatchOutcome::Unhandled,
                // A label hit with no document loaded cannot reach
                // here — `apply_double_click_activate` returns `Done`
                // (and so `Handled`) when there is no document — but
                // "nothing ran" is the honest answer if it ever does.
                (Some(_), None) => DispatchOutcome::Unhandled,
            }
        }
        Action::PanCanvas => match route_pan_canvas(ctx.drag_state) {
            // Continuous gesture: enter pan mode for the duration of
            // the press. Both release paths that can end it — the
            // left button's `DragState::Panning | DragState::None`
            // arm and `route_middle_button`'s `Clear` — reset
            // `drag_state` to `None`, so this arm only needs to
            // handle the press side.
            PanCanvasRoute::Arm => {
                *ctx.drag_state = DragState::Panning;
                DispatchOutcome::Handled
            }
            // Nothing ran, and the caller — the native macro loop
            // reads this for `any_ran` — is told so.
            PanCanvasRoute::Refuse => {
                log::debug!("dispatch: PanCanvas ignored (uncommitted gesture in flight); state stays put");
                DispatchOutcome::Unhandled
            }
        },
        // ── Console-verb Actions ───────────────────────────────
        Action::OpenColorPicker => {
            // Mirror `color picker on`: open the standalone palette.
            if let Some(doc) = ctx.document.as_mut() {
                open_color_picker_standalone(
                    doc,
                    ctx.color_picker_state,
                    ctx.interaction_mode,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::CloseColorPicker => {
            // Mirror `color picker off`.
            if let Some(doc) = ctx.document.as_mut() {
                close_color_picker_standalone(
                    ctx.color_picker_state,
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::LabelEditOnSelection => {
            // Mirror `label edit`: open the inline editor on the
            // currently-selected edge / portal-endpoint, seeded
            // with the existing text (the console verb has no
            // "clean" spelling).
            open_editor_for_edge_selection(false, ctx);
            DispatchOutcome::Handled
        }

        // ── Modal commit / cancel ────────────────────────────
        // §3 funnel: modal handlers used to call `close_*` helpers
        // inline. Commit/cancel are user-named effects (Esc /
        // Enter / click-outside), NOT the §3 carve-out for literal
        // Key character insertion — so they belong in the funnel.
        // `TextEditCommit` / `TextEditCancel` are Compatible and
        // handled by `dispatch_compatible` (cross-platform); they
        // never reach this match. `LabelEdit*` are NativeOnly
        // (label_edit + portal_text_edit modules are cfg-gated to
        // native) — handled here. Both reuse the same Action
        // variants since the editors are mutually exclusive by
        // selection-state construction (a node selection opens the
        // text editor; an edge-label selection opens the label
        // editor; a portal-text selection opens the portal-text
        // editor — never two at once). Order is observationally
        // equivalent because at most one is_open(); checking
        // portal-text first picks the more specific selection.
        Action::LabelEditCancel | Action::LabelEditCommit => {
            if let Some(doc) = ctx.document.as_mut() {
                close_single_line_edit(
                    matches!(action, Action::LabelEditCommit),
                    ctx.single_line_edit_state,
                    &mut super::cross_dispatch::rebuild_ctx!(ctx, doc),
                );
            }
            DispatchOutcome::Handled
        }

        // ── LabelEdit cursor primitives ───────────────────────
        //
        // Three declared behavior changes live on this arm, all of
        // them consequences of routing the funnel through the same
        // `handle_input_core` a keystroke takes instead of mutating
        // the buffer directly. Macros, the console and IPC are the
        // callers that reach it without a keystroke.
        //
        // 1. The preview is refreshed after the caret moves; before,
        //    it wrote state and painted nothing.
        // 2. It reaches the portal caption at all; before, it wrote
        //    to the edge-label state only.
        // 3. It meets `still_editable` first — so one of these
        //    actions arriving while the portal caption editor is
        //    open on an edge that has since been deleted or left
        //    portal mode now closes the editor and **discards the
        //    buffer uncommitted**, where before the buffer survived
        //    until Enter or a click outside. That guard existed on
        //    `main` only on the keystroke path; unifying the entry
        //    points widened its reach here. It is the consistent
        //    behavior — one editor, one guard — and it is pinned by
        //    `single_line_edit::tests::oracle::
        //    test_oracle_funnel_action_on_an_invalidated_portal_caption_discards_the_buffer`
        //    so it stays a decision rather than a side effect.
        //
        // The `if let Some(doc)` gate also makes these actions
        // no-ops with no document loaded, where before they still
        // mutated the buffer. Unobservable — an open editor implies
        // a document — noted so the audit is complete.
        Action::LabelEditCursorLeft
        | Action::LabelEditCursorRight
        | Action::LabelEditCursorHome
        | Action::LabelEditCursorEnd
        | Action::LabelEditDeleteBack
        | Action::LabelEditDeleteForward => {
            if let Some(doc) = ctx.document.as_mut() {
                super::super::single_line_edit::apply_single_line_edit_action(
                    action,
                    ctx.single_line_edit_state,
                    &mut super::cross_dispatch::rebuild_ctx!(ctx, doc),
                );
            }
            DispatchOutcome::Handled
        }

        // ── Filesystem variants (NativeOnly) ────────────────────
        // Dispatch arms route through `execute_console_line` so the
        // existing `replace_document` / `dirty` / `file_path`
        // plumbing on `ConsoleEffects` is reused. The whole module
        // is already `cfg(not(target_arch = "wasm32"))`, so no
        // additional cfg gate is needed.
        Action::OpenDocument(ref path)
        | Action::SaveDocumentAs(ref path)
        | Action::NewDocumentAt(ref path) => {
            let verb = match action {
                Action::OpenDocument(_) => "open",
                Action::SaveDocumentAs(_) => "save",
                Action::NewDocumentAt(_) => "new",
                _ => {
                    log::error!(
                        "dispatch: fs-variant fan-out missed inner-match variant: {:?}",
                        action,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            let line = format!("{} {}", verb, quote_console_arg(path));
            // The Action applied only if its line did: a missing file
            // or an unwritable path is `Unhandled`, so a caller that
            // reads the outcome (a macro's `any_ran`, an IPC reply)
            // is not told the map was opened or saved when it was not.
            match crate::application::app::console_input::exec::execute_console_line(ctx, &line) {
                Some(run) => {
                    crate::application::app::console_input::exec::log_console_line(&line, &run.outcome);
                    if run.outcome.applied() {
                        DispatchOutcome::Handled
                    } else {
                        DispatchOutcome::Unhandled
                    }
                }
                None => {
                    log::warn!("dispatch: {}: no document loaded; skipping '{}'", verb, line);
                    DispatchOutcome::Unhandled
                }
            }
        }

        // ── Color-picker modal Actions (NativeOnly) ─────────────
        // `PickerCancel` / `PickerCommit` / the six `PickerNudge*`.
        // These used to run entirely inside the picker's own key
        // handler, which made `MacroStep::Action { PickerCommit }`
        // a silent no-op while `TextEditCommit` from a macro
        // worked — the third modal was the odd one out. Same
        // rationale as the `LabelEdit*` arms above: commit /
        // cancel / nudge are user-named effects, not the §3
        // carve-out for literal Key payloads.
        //
        // The guard is `picker_op_for`, the picker module's single
        // source of truth for "the picker owns this Action". The
        // keyboard pre-filter (`event_keyboard.rs`) and the click
        // router (`color_picker_flow::click`) resolve through the
        // same fn, so nothing can be routed toward this funnel
        // without an arm here to receive it, and a ninth `Picker*`
        // variant is live on all three surfaces the moment it is
        // added there.
        ref a if picker_op_for(a).is_some() => {
            let Some(op) = picker_op_for(a) else {
                // The guard just proved `Some`; a mismatch means
                // `picker_op_for` is not a pure function any more.
                // Fail safe per CODE_CONVENTIONS §9.
                log::error!("dispatch: picker arm guard/body disagreement on {:?}", a);
                return DispatchOutcome::Unhandled;
            };
            dispatch_picker_op(op, ctx)
        }

        // Console / Picker / LabelEdit / TextEdit modal-context actions
        // not handled above (e.g. cancel/commit) are dispatched by their
        // respective modal handlers. Falling through to `Unhandled`
        // lets the keyboard handler's contextual resolution own them.
        _ => {
            log::debug!("dispatch: {:?} not handled at Document context", action);
            DispatchOutcome::Unhandled
        }
    }
}

/// Open the single-line editor the current selection names: the
/// edge-label editor for an `EdgeLabel` selection, the portal-text
/// editor for `PortalLabel` / `PortalText`. Any other selection
/// logs and no-ops.
///
/// One body for two arms. `Action::EditSelection` /
/// `EditSelectionClean` reach it as the native residual after
/// `dispatch_compatible` declines the node-scoped selections;
/// `Action::LabelEditOnSelection` (the `label edit` console verb's
/// Action mirror) reaches it directly. The two used to carry
/// byte-identical match bodies, and the `EditSelectionClean` half
/// computed a `clean` flag it then discarded — so the "empty
/// buffer" contract held on nodes and silently didn't on edge
/// labels and portal endpoints.
///
/// A selection whose target evaporated between the selection and
/// the dispatch leaves the editor closed; the `open_*` helpers own
/// that `log::warn!` themselves, so there is nothing for callers
/// here to branch on.
fn open_editor_for_edge_selection(clean: bool, ctx: &mut InputHandlerContext<'_>) {
    let Some(doc) = ctx.document.as_mut() else {
        return;
    };
    let Some(target) = resolve_single_line_target(&doc.selection) else {
        log::debug!("dispatch: edit-selection: selection is not an edge label / portal endpoint; no-op");
        return;
    };
    open_single_line_edit(
        target,
        clean,
        doc,
        ctx.single_line_edit_state,
        ctx.app_scene,
        ctx.renderer,
    );
}

/// Run a [`PickerOp`] against the live picker. Body of the
/// `dispatch_action` picker arm, lifted out so the arm stays a
/// two-liner and the mode branches read in one place.
///
/// Returns `Unhandled` whenever nothing ran — see
/// [`picker_decline_reason`] for the three cases and why each one
/// must not report `Handled`. `Handled` means the op reached its
/// effect, so a nudge that the picker state rejected also reports
/// `Unhandled` rather than claiming success.
fn dispatch_picker_op(op: PickerOp, ctx: &mut InputHandlerContext<'_>) -> DispatchOutcome {
    let standalone = ctx.color_picker_state.is_standalone();
    if let Some(reason) = picker_decline_reason(
        op,
        ctx.color_picker_state.is_open(),
        standalone,
        ctx.document.is_some(),
    ) {
        log::debug!("dispatch: picker action {:?} declined: {:?}", op, reason);
        return DispatchOutcome::Unhandled;
    }
    // `picker_decline_reason` just proved the document is present.
    let Some(doc) = ctx.document.as_mut() else {
        log::error!("dispatch: picker arm: decline check and document borrow disagree");
        return DispatchOutcome::Unhandled;
    };
    match op {
        PickerOp::Cancel => cancel_color_picker(
            ctx.color_picker_state,
            doc,
            ctx.interaction_mode,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        ),
        PickerOp::Commit => {
            if standalone {
                // Standalone: fan the wheel color across the
                // document selection and stay open.
                commit_color_picker_to_selection(
                    ctx.color_picker_state,
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            } else {
                // Contextual: write the bound handle and close.
                commit_color_picker(
                    ctx.color_picker_state,
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
        }
        PickerOp::Nudge(nudge) => {
            // Renderer-free: the preview stamp marks
            // `picker_hover.dirty` and the per-frame drain rebuilds.
            // The helper's `false` means the picker state rejected
            // the nudge, so the op did not take effect — report
            // that rather than a blanket `Handled`.
            if !apply_picker_nudge(nudge, ctx.color_picker_state, doc, ctx.picker_hover) {
                log::debug!(
                    "dispatch: picker nudge {:?} did not apply; reporting Unhandled",
                    nudge
                );
                return DispatchOutcome::Unhandled;
            }
        }
    }
    DispatchOutcome::Handled
}

/// Apply a `LabelEdit*` cursor / delete primitive to a bare
/// `(buffer, cursor)` pair.
///
/// Generic over the carrier so it serves both the single-line
/// editor and any other buffer with grapheme-cursor semantics.
/// Returns `true` when state changed.
pub(in crate::application::app) fn apply_label_edit_action_to_buffer(
    action: Action,
    buffer: &mut String,
    cursor: &mut usize,
) -> bool {
    use super::super::text_edit::{delete_at_cursor, delete_before_cursor};
    use baumhard::util::grapheme_chad;
    let before = *cursor;
    let len_before = buffer.len();
    match action {
        Action::LabelEditCursorLeft => {
            if *cursor > 0 {
                *cursor -= 1;
            }
        }
        Action::LabelEditCursorRight => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor += 1;
            }
        }
        Action::LabelEditCursorHome => {
            *cursor = 0;
        }
        Action::LabelEditCursorEnd => {
            *cursor = grapheme_chad::count_grapheme_clusters(buffer);
        }
        Action::LabelEditDeleteBack => {
            if *cursor > 0 {
                *cursor = delete_before_cursor(buffer, *cursor);
            }
        }
        Action::LabelEditDeleteForward if *cursor < grapheme_chad::count_grapheme_clusters(buffer) => {
            *cursor = delete_at_cursor(buffer, *cursor);
        }
        // At the end of the buffer delete-forward is a no-op, and it
        // says so in its own arm rather than falling through to the
        // catch-all, so an arm added above `_` later cannot receive it.
        // The guard above is clippy's `collapsible_match` form (the
        // nested `if` the siblings use trips it on the last arm only).
        Action::LabelEditDeleteForward => {}
        _ => {}
    }
    *cursor != before || buffer.len() != len_before
}

// `sibling_id` lifted to `dispatch/cross_dispatch/selection/mod.rs`
// so the WASM dispatcher can reach the same fold-aware navigation
// logic.

/// Run a macro by id against the current `InputHandlerContext`.
/// Iterates the macro's steps in order, forwarding each through the
/// matching dispatch surface:
/// - `MacroStep::Action` → `dispatch_action`
/// - `MacroStep::CustomMutation` → `apply_keybind_custom_mutation`
///   (selection-fallback target resolution)
/// - `MacroStep::ConsoleLine` → `console_input::execute_console_line`
///
/// Steps are run sequentially; a step that fails (e.g. an unbound
/// custom-mutation id, or an Action that returns Unhandled) logs and
/// the next step still runs. This matches "best-effort macro" — if a
/// later step depends on an earlier one, the macro author can split
/// it into two macros.
///
/// Returns `true` if any step ran successfully.
pub(in crate::application::app) fn dispatch_macro(macro_id: &str, ctx: &mut InputHandlerContext<'_>) -> bool {
    // Body lifted to `dispatch_macro_core` (cross-platform); this
    // shim wraps `ctx` in a `NativeMacroDispatchTarget` so the
    // native dispatch funnel calls the same step loop the WASM
    // dispatcher uses. The privilege gate is single-sourced there.
    let mut target = NativeMacroDispatchTarget { ctx };
    super::macro_core::dispatch_macro(macro_id, &mut target)
}

/// Run an anonymous step sequence through the same gated step loop
/// [`dispatch_macro`] runs a registered macro through, as a macro
/// from tier `source`. `label` names the sequence in diagnostics.
pub(in crate::application::app) fn run_macro_steps(
    label: &str,
    steps: &[crate::application::macros::MacroStep],
    source: crate::application::source_tier::SourceTier,
    ctx: &mut InputHandlerContext<'_>,
) -> bool {
    let mut target = NativeMacroDispatchTarget { ctx };
    super::macro_core::run_macro_steps(label, steps, source, &mut target)
}

/// Native impl of [`super::macro_core::MacroDispatchTarget`].
/// Wraps `&mut InputHandlerContext` and forwards each operation to
/// the existing native helpers (`dispatch_action`,
/// `apply_keybind_custom_mutation`, `execute_console_line`).
struct NativeMacroDispatchTarget<'a, 'b> {
    ctx: &'a mut InputHandlerContext<'b>,
}

impl<'a, 'b> super::macro_core::MacroDispatchTarget for NativeMacroDispatchTarget<'a, 'b> {
    fn registry(&self) -> &crate::application::macros::MacroRegistry {
        self.ctx.macros
    }

    fn dispatch_action(&mut self, action: Action) -> DispatchOutcome {
        dispatch_action(action, self.ctx, None)
    }

    fn apply_custom_mutation(&mut self, id: &str, node_id: &str) -> bool {
        // Lookup mutation, apply via the existing
        // `apply_keybind_custom_mutation` helper, rebuild scene if
        // applied. Mirrors the `MacroStep::CustomMutation` body
        // pre-Commit-3 (lines 1067-1094 of the prior dispatch.rs).
        let cm = self
            .ctx
            .document
            .as_ref()
            .and_then(|d| d.mutation_registry.get(id).cloned());
        let Some(cm) = cm else {
            log::warn!("macros: CustomMutation step: unknown custom-mutation id '{}'", id);
            return false;
        };
        let Some(doc) = self.ctx.document.as_mut() else {
            return false;
        };
        let now = super::super::now_ms() as u64;
        if apply_keybind_custom_mutation(
            doc,
            self.ctx.mindmap_tree,
            self.ctx.scene_cache,
            &cm,
            node_id,
            now,
        ) {
            rebuild_all(
                doc,
                self.ctx.interaction_mode,
                self.ctx.mindmap_tree,
                self.ctx.app_scene,
                self.ctx.renderer,
                self.ctx.scene_cache,
            );
            true
        } else {
            false
        }
    }

    fn execute_console_line(&mut self, line: &str) -> bool {
        // Every verb runs against a document, so a macro fired before
        // one loads soft-skips. A line that ran but failed (an unknown
        // verb, a verb error) did not apply either. Neither bumps the
        // macro's `any_ran`, so a keystroke whose macro did nothing
        // still falls through to the custom-mutation tier.
        match crate::application::app::console_input::exec::execute_console_line(self.ctx, line) {
            Some(run) => {
                crate::application::app::console_input::exec::log_console_line(line, &run.outcome);
                run.outcome.applied()
            }
            None => {
                log::warn!(
                    "macros: ConsoleLine step: no document loaded; skipping '{}'",
                    line
                );
                false
            }
        }
    }

    fn current_selection_node_id(&self) -> Option<String> {
        self.ctx.document.as_ref().and_then(|d| {
            if let SelectionState::Single(nid) = &d.selection {
                Some(nid.clone())
            } else {
                None
            }
        })
    }

    fn has_node(&self, node_id: &str) -> bool {
        self.ctx
            .document
            .as_ref()
            .map(|d| d.mindmap.nodes.contains_key(node_id))
            .unwrap_or(false)
    }
}

/// Fast-resize gesture start (`Action::FastResizeStart`).
///
/// Threshold-cross arm in `event_cursor_moved.rs` dispatches this
/// when a `DragState::PendingRight` press has moved past the drag
/// threshold. The threshold-cross arm packs the **press-time**
/// canvas position into `hit.canvas_pos` (not the threshold-cross
/// position) so anchor inference fires from where the user pressed
/// — plan §6.3: "Quadrant determined at press time, not
/// continuously". This helper reads that press-time canvas pos,
/// reads the press-time hit off `PendingRight`, computes the
/// corner anchor via `infer_resize_anchor`, and transitions
/// `PendingRight → Throttled(NodeResize | SectionResize)` so the
/// existing per-frame drain + right-button release commit handles
/// the rest.
///
/// No-op on:
/// - hit was empty (right-press on empty canvas)
/// - section's `size` is `None` (fill-parent — can't resize)
/// - node / section vanished between press and threshold (e.g.
///   the user deleted via console while right-button was held)
///
/// In each case the state resets to `None` so the cursor doesn't
/// re-fire the threshold-cross.
fn apply_fast_resize_start(ctx: &mut InputHandlerContext<'_>, hit: Option<&DispatchHit>) {
    use baumhard::mindmap::tree_builder::infer_resize_anchor;
    use glam::Vec2;

    use super::super::throttled_interaction::{
        NodeResizeInteraction, SectionResizeInteraction, ThrottledDrag,
    };

    let Some(h) = hit else {
        log::debug!("dispatch: FastResizeStart: no DispatchHit; skipping");
        return;
    };
    // Snapshot press-time hit out of PendingRight. If the state
    // doesn't match, the threshold-cross caller already moved on
    // (race with another gesture) — log + bail without mutating.
    let (hit_node, hit_section_idx) = match ctx.drag_state {
        DragState::PendingRight {
            hit_node,
            hit_section_idx,
            ..
        } => (hit_node.clone(), *hit_section_idx),
        _ => {
            log::debug!("dispatch: FastResizeStart: drag_state isn't PendingRight; skipping");
            return;
        }
    };
    let Some(node_id) = hit_node else {
        // Empty-canvas right-press — no fast-resize target.
        log::debug!("dispatch: FastResizeStart: press landed on empty canvas; skipping");
        *ctx.drag_state = DragState::None;
        return;
    };

    let Some(doc) = ctx.document.as_mut() else {
        log::debug!("dispatch: FastResizeStart: no document; skipping");
        *ctx.drag_state = DragState::None;
        return;
    };

    // Two paths: section target (multi-section node hit) or node
    // target (whole-node hit, including single-section nodes).
    if let Some(section_idx) = hit_section_idx {
        let Some(node) = doc.mindmap.nodes.get(&node_id) else {
            log::debug!(
                "dispatch: FastResizeStart: node '{}' not found; skipping",
                node_id
            );
            *ctx.drag_state = DragState::None;
            return;
        };
        let Some(section) = node.sections.get(section_idx) else {
            log::debug!(
                "dispatch: FastResizeStart: section[{}] not found on '{}'; skipping",
                section_idx,
                node_id
            );
            *ctx.drag_state = DragState::None;
            return;
        };
        let Some(start_size) = section.size else {
            // fill-parent section — no AABB to anchor against.
            log::info!(
                "dispatch: FastResizeStart: section[{}] of '{}' is fill-parent (size=None); cannot fast-resize",
                section_idx,
                node_id
            );
            *ctx.drag_state = DragState::None;
            return;
        };
        let start_offset = section.offset;
        let aabb_pos = Vec2::new(
            (node.position.x + start_offset.x) as f32,
            (node.position.y + start_offset.y) as f32,
        );
        let aabb_size = Vec2::new(start_size.width as f32, start_size.height as f32);
        let side = infer_resize_anchor(h.canvas_pos, aabb_pos, aabb_size);
        ctx.scene_cache.clear();
        *ctx.drag_state = DragState::throttled(ThrottledDrag::SectionResize(SectionResizeInteraction::new(
            node_id,
            section_idx,
            side,
            start_offset,
            start_size,
            // Fast-resize gesture (`PendingRight` promotion) — the
            // right-button release path may finalize this drag.
            true,
        )));
    } else {
        let Some(node) = doc.mindmap.nodes.get(&node_id) else {
            log::debug!(
                "dispatch: FastResizeStart: node '{}' not found; skipping",
                node_id
            );
            *ctx.drag_state = DragState::None;
            return;
        };
        let start_position = node.position;
        let start_size = node.size;
        let aabb_pos = Vec2::new(start_position.x as f32, start_position.y as f32);
        let aabb_size = Vec2::new(start_size.width as f32, start_size.height as f32);
        let side = infer_resize_anchor(h.canvas_pos, aabb_pos, aabb_size);
        ctx.scene_cache.clear();
        *ctx.drag_state = DragState::throttled(ThrottledDrag::NodeResize(NodeResizeInteraction::new(
            node_id,
            side,
            start_position,
            start_size,
            true,
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::moving_node::MovingNodeInteraction;
    use crate::application::app::throttled_interaction::node_resize::NodeResizeInteraction;
    use crate::application::app::throttled_interaction::ThrottledDrag;
    use baumhard::mindmap::model::{Position, Size};
    use baumhard::mindmap::tree_builder::ResizeHandleSide;
    use std::collections::HashSet;

    // Most dispatch arms touch the renderer (wgpu) which is forbidden
    // in tests per `TEST_CONVENTIONS.md §T8`. Arms whose bodies factor
    // cleanly into a pure helper are tested through that helper —
    // `route_pan_canvas` below is the first; the rest of the funnel is
    // exercised manually via `./run.sh` and through end-to-end
    // integration on top of the keybind tests in `keybinds/tests.rs`
    // (which exercise the resolver, not the dispatch bodies).

    /// A right-started fast-resize, mid-gesture: the model write and
    /// the undo entry it owes happen only in
    /// `commit_on_release_core`.
    fn fast_resize_drag() -> DragState {
        DragState::throttled(ThrottledDrag::NodeResize(NodeResizeInteraction::new(
            "n0".to_string(),
            ResizeHandleSide::SE,
            Position { x: 0.0, y: 0.0 },
            Size {
                width: 100.0,
                height: 50.0,
            },
            true,
        )))
    }

    /// A left-started move-node drag, mid-gesture.
    fn moving_node_drag() -> DragState {
        DragState::throttled(ThrottledDrag::MovingNode(MovingNodeInteraction::new(
            vec!["n0".to_string()],
            false,
            HashSet::new(),
        )))
    }

    /// **A `pan_canvas` dispatch mid-drag must not destroy the drag.**
    /// The middle-button route was guarded at the route in #37 item 5;
    /// this is the same silent loss reached through the *Action*,
    /// which every keyboard binding and every macro tier can name.
    ///
    /// Fails on the pre-fix arm, which wrote `DragState::Panning`
    /// unconditionally — i.e. on `fn route_pan_canvas(_) ->
    /// PanCanvasRoute { PanCanvasRoute::Arm }`.
    #[test]
    fn test_pan_canvas_mid_drag_leaves_the_throttled_drag_intact() {
        assert_eq!(
            route_pan_canvas(&fast_resize_drag()),
            PanCanvasRoute::Refuse,
            "a keyboard-bound pan must not replace a right-started fast-resize: \
             its release-commit has not run, so the model write and the undo \
             entry go with it"
        );
        assert_eq!(
            route_pan_canvas(&moving_node_drag()),
            PanCanvasRoute::Refuse,
            "nor any other throttled drag"
        );
        assert_eq!(
            route_pan_canvas(&DragState::PendingRight {
                start_pos: (0.0, 0.0),
                start_canvas: glam::Vec2::ZERO,
                hit_node: None,
                hit_section_idx: None,
            }),
            PanCanvasRoute::Refuse,
            "nor a right press whose RightClick / FastResizeStart has not fired"
        );
    }

    /// The other half of the same guard — without these rows the fix
    /// could be "answer `Refuse` always", which would leave the canvas
    /// unable to pan at all.
    ///
    /// `Pending` is the load-bearing row: the `LeftDrag` threshold
    /// cross dispatches `PanCanvas` *from* `Pending`, so a guard as
    /// broad as the right-button press's `!matches!(.., None)` would
    /// break the default left-drag pan.
    #[test]
    fn test_pan_canvas_still_arms_from_every_state_that_owes_the_model_nothing() {
        for drag in [
            DragState::None,
            DragState::Panning,
            DragState::SelectingRect {
                start_canvas: glam::Vec2::ZERO,
                current_canvas: glam::Vec2::new(10.0, 10.0),
            },
            DragState::Pending(Box::new(crate::application::app::PendingPress {
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
                route_pan_canvas(&drag),
                PanCanvasRoute::Arm,
                "PanCanvas must still arm from {:?}",
                std::mem::discriminant(&drag)
            );
        }
    }

    /// **A target-picker mode takes the pointer, so it has to take
    /// the rubber band with it.** `Reparent` / `Connect` swallow both
    /// halves of the left button and `handle_cursor_moved` returns
    /// before the drag-state ladder, so a band live at mode entry
    /// would sit frozen for the whole picker session — and its
    /// covered set would be painted by every hover rebuild the mode
    /// runs, because `highlight_entries_for` is what all of them
    /// read.
    ///
    /// Fails on the pre-fix shape, which is this function not
    /// existing: the mode entries wrote `interaction_mode` and
    /// rebuilt, and touched neither the drag state nor the set.
    #[test]
    fn test_entering_a_target_picker_mode_ends_a_live_rubber_band() {
        let mut doc = crate::application::document::tests_common::load_test_doc();
        doc.selection = SelectionState::Single("the-real-selection".to_string());
        doc.set_rect_select_preview(vec!["stale-preview".to_string()]);
        let mut document = Some(doc);
        let mut drag_state = DragState::SelectingRect {
            start_canvas: glam::Vec2::ZERO,
            current_canvas: glam::Vec2::new(10.0, 10.0),
        };

        assert!(take_rubber_band_for_target_picker(&mut drag_state, &mut document));

        assert!(
            matches!(drag_state, DragState::None),
            "the gesture the mode interrupted must not stay live"
        );
        let ids: Vec<&str> = crate::application::app::scene_rebuild::highlight_entries_for(
            document.as_ref().expect("fixture document"),
        )
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
        assert_eq!(
            ids,
            vec!["the-real-selection"],
            "the mode's own hover rebuilds must not paint the abandoned set"
        );
    }

    /// The complement: every other state is left alone. `Throttled`
    /// is the row that matters — it owes the model a write and an
    /// undo entry that only its own release performs, so ending it
    /// here would be the silent loss this issue opened with.
    #[test]
    fn test_entering_a_target_picker_mode_leaves_every_other_gesture_alone() {
        for mut drag in [
            fast_resize_drag(),
            moving_node_drag(),
            DragState::Panning,
            DragState::None,
        ] {
            let before = std::mem::discriminant(&drag);
            let mut document = None;
            assert!(!take_rubber_band_for_target_picker(&mut drag, &mut document));
            assert_eq!(
                std::mem::discriminant(&drag),
                before,
                "a picker mode must not end {:?}",
                before
            );
        }
    }

    #[test]
    fn test_quote_console_arg_wraps_plain_path_in_double_quotes() {
        assert_eq!(super::quote_console_arg("/tmp/x.json"), "\"/tmp/x.json\"");
    }

    #[test]
    fn test_quote_console_arg_handles_paths_with_spaces() {
        // Embedded whitespace is the whole reason quoting exists —
        // the tokenizer would otherwise split the path into multiple
        // positionals.
        assert_eq!(
            super::quote_console_arg("/tmp/some dir/x.json"),
            "\"/tmp/some dir/x.json\"",
        );
    }

    #[test]
    fn test_quote_console_arg_escapes_embedded_double_quotes() {
        // A literal `"` inside the path becomes `\"` so the
        // tokenizer doesn't terminate the quoted token early.
        assert_eq!(
            super::quote_console_arg(r#"/tmp/he said "hi"/x.json"#),
            r#""/tmp/he said \"hi\"/x.json""#,
        );
    }

    #[test]
    fn test_quote_console_arg_escapes_backslashes_for_windows_paths() {
        // Windows path: every `\` becomes `\\` so the tokenizer
        // doesn't consume the next char as part of an escape, and
        // a path ending in `\` doesn't unterminate the quote.
        assert_eq!(
            super::quote_console_arg(r"C:\Users\foo\map.json"),
            r#""C:\\Users\\foo\\map.json""#,
        );
    }

    #[test]
    fn test_quote_console_arg_handles_path_ending_in_backslash() {
        // Pre-fix this would produce `"C:\\foo\"` — an unterminated
        // quoted token. With the backslash escape it produces
        // `"C:\\foo\\"` which round-trips cleanly.
        assert_eq!(super::quote_console_arg(r"C:\foo\"), r#""C:\\foo\\""#);
    }
}
