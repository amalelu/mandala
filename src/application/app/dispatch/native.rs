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
            if let Some(doc) = ctx.document.as_ref() {
                let sel: Vec<String> = doc
                    .selection
                    .selected_ids()
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                if !sel.is_empty() {
                    *ctx.interaction_mode = InteractionMode::Reparent { sources: sel };
                    *ctx.hovered_node = None;
                    *ctx.last_click = None;
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
            if let Some(doc) = ctx.document.as_ref() {
                if let SelectionState::Single(source) = &doc.selection {
                    *ctx.interaction_mode = InteractionMode::Connect {
                        source: source.clone(),
                    };
                    *ctx.hovered_node = None;
                    *ctx.last_click = None;
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
        Action::SaveDocument => {
            if let Some(doc) = ctx.document.as_mut() {
                save_document_to_bound_path(doc, ctx.console_state);
            }
            DispatchOutcome::Handled
        }

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
        Action::PanCanvas => {
            // Continuous gesture: enter pan mode for the duration of
            // the press. The mouse-release handler unconditionally
            // resets `drag_state` to `None`, so this arm only needs
            // to handle the press side.
            *ctx.drag_state = DragState::Panning;
            DispatchOutcome::Handled
        }
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
                    doc,
                    ctx.interaction_mode,
                    ctx.single_line_edit_state,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
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
                    log::error!("fs-variant fan-out missed inner-match variant: {:?}", action,);
                    return DispatchOutcome::Handled;
                }
            };
            let line = format!("{} {}", verb, quote_console_arg(path));
            if let Some(doc) = ctx.document.as_mut() {
                crate::application::app::console_input::exec::execute_console_line(
                    &line,
                    ctx.console_state,
                    ctx.single_line_edit_state,
                    ctx.color_picker_state,
                    ctx.text_edit_state,
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                    ctx.macros,
                );
            } else {
                log::warn!("{}: no document loaded; skipping '{}'", verb, line);
            }
            DispatchOutcome::Handled
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
                log::error!("picker arm guard/body disagreement on {:?}", a);
                return DispatchOutcome::Unhandled;
            };
            dispatch_picker_op(op, ctx)
        }

        // Console / Picker / LabelEdit / TextEdit modal-context actions
        // not handled above (e.g. cancel/commit) are dispatched by their
        // respective modal handlers. Falling through to `Unhandled`
        // lets the keyboard handler's contextual resolution own them.
        _ => {
            log::debug!("dispatch_action: {:?} not handled at Document context", action);
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
        log::debug!(
            "open_editor_for_edge_selection: selection is not an edge label / portal endpoint; no-op"
        );
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
        log::debug!("picker action {:?} declined: {:?}", op, reason);
        return DispatchOutcome::Unhandled;
    }
    // `picker_decline_reason` just proved the document is present.
    let Some(doc) = ctx.document.as_mut() else {
        log::error!("picker arm: decline check and document borrow disagree");
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
                log::debug!("picker nudge {:?} did not apply; reporting Unhandled", nudge);
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
        Action::LabelEditDeleteForward => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor = delete_at_cursor(buffer, *cursor);
            }
        }
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
            log::warn!("macro step: unknown custom-mutation id '{}'", id);
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
        // `execute_console_line` requires a loaded document (takes
        // `&mut MindMapDocument`, not `Option`). Macros fired before
        // any document is loaded silently skip and return false so
        // the macro's `any_ran` doesn't bump on the no-op path —
        // matches pre-Track-B behavior where the warn arm left
        // `any_ran` unchanged.
        let Some(doc) = self.ctx.document.as_mut() else {
            log::warn!("macro step ConsoleLine: no document loaded; skipping '{}'", line,);
            return false;
        };
        crate::application::app::console_input::exec::execute_console_line(
            line,
            self.ctx.console_state,
            self.ctx.single_line_edit_state,
            self.ctx.color_picker_state,
            self.ctx.text_edit_state,
            doc,
            self.ctx.interaction_mode,
            self.ctx.mindmap_tree,
            self.ctx.app_scene,
            self.ctx.renderer,
            self.ctx.scene_cache,
            self.ctx.macros,
        );
        true
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
        log::debug!("FastResizeStart: no DispatchHit; skipping");
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
            log::debug!("FastResizeStart: drag_state isn't PendingRight; skipping");
            return;
        }
    };
    let Some(node_id) = hit_node else {
        // Empty-canvas right-press — no fast-resize target.
        log::debug!("FastResizeStart: press landed on empty canvas; skipping");
        *ctx.drag_state = DragState::None;
        return;
    };

    let Some(doc) = ctx.document.as_mut() else {
        log::debug!("FastResizeStart: no document; skipping");
        *ctx.drag_state = DragState::None;
        return;
    };

    // Two paths: section target (multi-section node hit) or node
    // target (whole-node hit, including single-section nodes).
    if let Some(section_idx) = hit_section_idx {
        let Some(node) = doc.mindmap.nodes.get(&node_id) else {
            log::debug!("FastResizeStart: node '{}' not found; skipping", node_id);
            *ctx.drag_state = DragState::None;
            return;
        };
        let Some(section) = node.sections.get(section_idx) else {
            log::debug!(
                "FastResizeStart: section[{}] not found on '{}'; skipping",
                section_idx,
                node_id
            );
            *ctx.drag_state = DragState::None;
            return;
        };
        let Some(start_size) = section.size else {
            // fill-parent section — no AABB to anchor against.
            log::info!(
                "FastResizeStart: section[{}] of '{}' is fill-parent (size=None); cannot fast-resize",
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
            log::debug!("FastResizeStart: node '{}' not found; skipping", node_id);
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
    // Most dispatch arms touch the renderer (wgpu) which is forbidden
    // in tests per `TEST_CONVENTIONS.md §T8`. Per-arm pure helpers
    // would be tested here; for now the whole funnel is exercised
    // manually via `./run.sh` and through end-to-end integration on
    // top of the keybind tests in `keybinds/tests.rs` (which exercise
    // the resolver, not the dispatch bodies).
    //
    // When adding new arms whose bodies factor cleanly into a pure
    // helper, add the helper test here.
    #[test]
    fn dispatch_action_module_compiles() {
        // Smoke test: the module's public surface is reachable.
        // Replaced by per-arm tests in later phases.
    }

    #[test]
    fn quote_console_arg_wraps_plain_path_in_double_quotes() {
        assert_eq!(super::quote_console_arg("/tmp/x.json"), "\"/tmp/x.json\"");
    }

    #[test]
    fn quote_console_arg_handles_paths_with_spaces() {
        // Embedded whitespace is the whole reason quoting exists —
        // the tokenizer would otherwise split the path into multiple
        // positionals.
        assert_eq!(
            super::quote_console_arg("/tmp/some dir/x.json"),
            "\"/tmp/some dir/x.json\"",
        );
    }

    #[test]
    fn quote_console_arg_escapes_embedded_double_quotes() {
        // A literal `"` inside the path becomes `\"` so the
        // tokenizer doesn't terminate the quoted token early.
        assert_eq!(
            super::quote_console_arg(r#"/tmp/he said "hi"/x.json"#),
            r#""/tmp/he said \"hi\"/x.json""#,
        );
    }

    #[test]
    fn quote_console_arg_escapes_backslashes_for_windows_paths() {
        // Windows path: every `\` becomes `\\` so the tokenizer
        // doesn't consume the next char as part of an escape, and
        // a path ending in `\` doesn't unterminate the quote.
        assert_eq!(
            super::quote_console_arg(r"C:\Users\foo\map.json"),
            r#""C:\\Users\\foo\\map.json""#,
        );
    }

    #[test]
    fn quote_console_arg_handles_path_ending_in_backslash() {
        // Pre-fix this would produce `"C:\\foo\"` — an unterminated
        // quoted token. With the backslash escape it produces
        // `"C:\\foo\\"` which round-trips cleanly.
        assert_eq!(super::quote_console_arg(r"C:\foo\"), r#""C:\\foo\\""#);
    }
}
