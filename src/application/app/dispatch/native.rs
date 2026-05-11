// SPDX-License-Identifier: MPL-2.0

//! `dispatch_action` — the single entry point that runs `Action`
//! bodies on native. Mouse handlers and the keyboard handler funnel
//! through here. WASM has its own dispatch path today; the
//! convergence track is documented in `WASM_CONVERGENCE.md`.
//! Adding a new behaviour
//! is variant + default + arm, in that order; never inline a body in
//! a handler.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;

use crate::application::document::{EdgeRef, SelectionState, UndoAction};
use crate::application::keybinds::Action;

use super::super::click::rebuild_all_with_mode;
use super::super::color_picker_flow::{close_color_picker_standalone, open_color_picker_standalone};
use super::super::console_input::{
    rebuild_console_overlay, save_console_history, save_document_to_bound_path,
};
use super::super::input_context::InputHandlerContext;
use super::super::label_edit::{open_label_edit, open_portal_text_edit};
use super::super::scene_rebuild::rebuild_all;
use super::super::text_edit::open_text_edit;
use super::super::{ClickHit, DragState, InteractionMode};
use super::apply_keybind_custom_mutation;
use crate::application::console::ConsoleState;

/// Per-event payload that mouse-driven Actions need but keyboard
/// dispatch doesn't. Populated by mouse handlers right before they
/// call `dispatch_action`; `None` for keyboard / macro callers.
#[derive(Debug, Clone)]
pub struct DispatchHit {
    /// What the click landed on. The `DoubleClickActivate` arm routes
    /// on this.
    pub click_hit: ClickHit,
    /// Canvas-space cursor position at the gesture's trigger time.
    /// Used by orphan-creation / open-editor arms.
    pub canvas_pos: Vec2,
}

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
///   - the mouse-mixed branch of `DoubleClickActivate` and the
///     mouse-with-hit branch of `CreateOrphanNodeAndEdit`, both
///     of which need `DispatchHit::canvas_pos` (a payload
///     `dispatch_compatible` doesn't carry). The keyboard /
///     no-hit branch of `CreateOrphanNodeAndEdit` is handled
///     in `dispatch_compatible` (uses `cursor_pos`).
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
        super::action_core::dispatch_compatible(&action, &mut core)
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
                    host: core.host,
                    scene_cache: core.scene_cache,
                    interaction_mode: core.interaction_mode,
                };
                let _ = super::cross_dispatch::apply_enter_node_edit(
                    clean,
                    &mut rc,
                    core.text_edit_state,
                );
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
                    host: core.host,
                    scene_cache: core.scene_cache,
                    interaction_mode: core.interaction_mode,
                };
                let _ = super::cross_dispatch::apply_enter_section_edit(
                    false,
                    &mut rc,
                    core.text_edit_state,
                );
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
            let _clean = matches!(action, Action::EditSelectionClean);
            if let Some(doc) = ctx.document.as_mut() {
                match doc.selection.clone() {
                    SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
                        let er = s.edge_ref();
                        open_portal_text_edit(
                            &er,
                            &s.endpoint_node_id,
                            doc,
                            ctx.portal_text_edit_state,
                            ctx.app_scene,
                            ctx.renderer,
                        );
                    }
                    SelectionState::EdgeLabel(s) => {
                        open_label_edit(
                            &s.edge_ref,
                            doc,
                            ctx.label_edit_state,
                            ctx.app_scene,
                            ctx.renderer,
                        );
                    }
                    _ => {}
                }
            }
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
            // Routes by what the press hit. The mouse handler populates
            // `hit` before calling here; without it we have no target
            // and silently no-op (the gesture was bound but fired from
            // a non-mouse source like a macro that didn't carry hit
            // context).
            let Some(h) = hit else {
                log::debug!("DoubleClickActivate: no DispatchHit; skipping");
                return DispatchOutcome::Handled;
            };
            match &h.click_hit {
                ClickHit::Node(node_id, section_idx) => {
                    if let Some(doc) = ctx.document.as_mut() {
                        let nid = node_id.clone();
                        // Preserve the section identity so the
                        // editor opens on the section the user
                        // pointed at. Pre-fix this collapsed to
                        // `SelectionState::Single` unconditionally,
                        // and `open_text_edit` then defaulted to
                        // `section_idx = 0` — so a double-click on
                        // section[1] opened the editor on
                        // section[0].
                        doc.selection = match section_idx {
                            Some(idx) => SelectionState::Section(
                                crate::application::document::SectionSel {
                                    node_id: nid.clone(),
                                    section_idx: *idx,
                                },
                            ),
                            None => SelectionState::Single(nid.clone()),
                        };
                        rebuild_all(
                            doc,
                            ctx.interaction_mode,
                            ctx.mindmap_tree,
                            ctx.app_scene,
                            ctx.renderer,
                            ctx.scene_cache,
                        );
                        open_text_edit(
                            &nid,
                            false,
                            doc,
                            ctx.text_edit_state,
                            ctx.mindmap_tree,
                            ctx.app_scene,
                            ctx.renderer,
                        );
                    }
                }
                ClickHit::PortalMarker { edge, endpoint } | ClickHit::PortalText { edge, endpoint } => {
                    // Pan to the partner endpoint of the portal-mode
                    // edge — the node "on the other side."
                    let other_id = if *endpoint == edge.from_id {
                        edge.to_id.clone()
                    } else {
                        edge.from_id.clone()
                    };
                    if let Some(doc) = ctx.document.as_ref() {
                        if let Some(node) = doc.mindmap.nodes.get(&other_id) {
                            ctx.renderer.set_camera_center(node.center_vec2());
                        }
                    }
                    if let Some(doc) = ctx.document.as_mut() {
                        doc.selection = SelectionState::Edge(crate::application::document::EdgeRef::new(
                            &edge.from_id,
                            &edge.to_id,
                            &edge.edge_type,
                        ));
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
                ClickHit::EdgeLabel(edge_key) => {
                    if let Some(doc) = ctx.document.as_mut() {
                        let er = crate::application::document::EdgeRef::new(
                            edge_key.from_id.as_str(),
                            edge_key.to_id.as_str(),
                            edge_key.edge_type.as_str(),
                        );
                        let prev = doc.selection.clone();
                        doc.selection = SelectionState::EdgeLabel(
                            crate::application::document::EdgeLabelSel::new(er.clone()),
                        );
                        super::super::scene_rebuild::rebuild_after_selection_change(
                            &prev,
                            doc,
                            ctx.interaction_mode,
                            ctx.mindmap_tree,
                            ctx.app_scene,
                            ctx.renderer,
                            ctx.scene_cache,
                        );
                        open_label_edit(&er, doc, ctx.label_edit_state, ctx.app_scene, ctx.renderer);
                    }
                }
                ClickHit::Empty => {
                    // Empty-canvas double-click: only fire
                    // CreateOrphanNodeAndEdit if the user has explicitly
                    // bound it (any binding counts as opt-in). Ships
                    // unbound by default — empty-canvas double-click
                    // is a no-op out of the box per user request.
                    let edge_selected = ctx
                        .document
                        .as_ref()
                        .map(|d| matches!(d.selection, SelectionState::Edge(_)))
                        .unwrap_or(false);
                    if !edge_selected && ctx.keybinds.has_any_binding_for(Action::CreateOrphanNodeAndEdit) {
                        dispatch_create_orphan_and_edit(ctx, h);
                    }
                }
            }
            DispatchOutcome::Handled
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
            // currently-selected edge / portal-endpoint.
            if let Some(doc) = ctx.document.as_mut() {
                match doc.selection.clone() {
                    SelectionState::EdgeLabel(s) => {
                        open_label_edit(
                            &s.edge_ref,
                            doc,
                            ctx.label_edit_state,
                            ctx.app_scene,
                            ctx.renderer,
                        );
                    }
                    SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
                        let er = s.edge_ref();
                        open_portal_text_edit(
                            &er,
                            &s.endpoint_node_id,
                            doc,
                            ctx.portal_text_edit_state,
                            ctx.app_scene,
                            ctx.renderer,
                        );
                    }
                    _ => {
                        log::debug!(
                            "LabelEditOnSelection: selection is not an edge / portal endpoint; no-op"
                        );
                    }
                }
            }
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
        Action::LabelEditCancel => {
            if let Some(doc) = ctx.document.as_mut() {
                if ctx.portal_text_edit_state.is_open() {
                    super::super::label_edit::close_portal_text_edit(
                        false,
                        doc,
                        ctx.interaction_mode,
                        ctx.portal_text_edit_state,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                } else if ctx.label_edit_state.is_open() {
                    super::super::label_edit::close_label_edit(
                        false,
                        doc,
                        ctx.interaction_mode,
                        ctx.label_edit_state,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::LabelEditCommit => {
            if let Some(doc) = ctx.document.as_mut() {
                if ctx.portal_text_edit_state.is_open() {
                    super::super::label_edit::close_portal_text_edit(
                        true,
                        doc,
                        ctx.interaction_mode,
                        ctx.portal_text_edit_state,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                } else if ctx.label_edit_state.is_open() {
                    super::super::label_edit::close_label_edit(
                        true,
                        doc,
                        ctx.interaction_mode,
                        ctx.label_edit_state,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }

        // ── LabelEdit cursor primitives ───────────────────────
        Action::LabelEditCursorLeft
        | Action::LabelEditCursorRight
        | Action::LabelEditCursorHome
        | Action::LabelEditCursorEnd
        | Action::LabelEditDeleteBack
        | Action::LabelEditDeleteForward => {
            apply_label_edit_action(action, ctx.label_edit_state);
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
                    ctx.label_edit_state,
                    ctx.portal_text_edit_state,
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

/// Apply a LabelEdit cursor / delete primitive to a generic
/// `(buffer, cursor)` pair. Both `LabelEditState` and
/// `PortalTextEditState` share the same single-line semantics; this
/// helper is generic over the carrier so the dispatch arms can fan
/// out into either modal. Returns `true` when state changed.
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

/// Convenience wrapper for the dispatch-table call site that takes
/// the LabelEditState carrier directly.
pub(in crate::application::app) fn apply_label_edit_action(
    action: Action,
    state: &mut super::super::label_edit::LabelEditState,
) -> bool {
    use super::super::label_edit::LabelEditState;
    let LabelEditState::Open {
        buffer,
        cursor_grapheme_pos,
        ..
    } = state
    else {
        return false;
    };
    apply_label_edit_action_to_buffer(action, buffer, cursor_grapheme_pos)
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
        // matches pre-Track-B behaviour where the warn arm left
        // `any_ran` unchanged.
        let Some(doc) = self.ctx.document.as_mut() else {
            log::warn!("macro step ConsoleLine: no document loaded; skipping '{}'", line,);
            return false;
        };
        crate::application::app::console_input::exec::execute_console_line(
            line,
            self.ctx.console_state,
            self.ctx.label_edit_state,
            self.ctx.portal_text_edit_state,
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

/// Resolve a custom-mutation key binding and apply it through the same
/// path the click-trigger handler at `click.rs:35-64` uses: animation-
/// aware (`start_animation` when `timing.duration_ms > 0`), and always
/// invoking `apply_document_actions`. Returns `true` when a mutation
/// was found and applied.
///
/// Phase-7 fix: the previous keyboard-side fall-through at
/// `event_keyboard.rs:528-553` skipped both `apply_document_actions`
/// and the timing envelope, so document-action and animated mutations
/// silently mis-fired when triggered from a key. This helper unifies
/// the two paths through `apply_keybind_custom_mutation`.
pub(in crate::application::app) fn dispatch_custom_mutation_for_key(
    ctx: &mut InputHandlerContext<'_>,
    key_name: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> bool {
    let id = match ctx.keybinds.custom_mutation_for(key_name, ctrl, shift, alt) {
        Some(s) => s.to_string(),
        None => return false,
    };
    let Some(doc) = ctx.document.as_mut() else {
        return false;
    };
    let SelectionState::Single(nid) = doc.selection.clone() else {
        return false;
    };
    let Some(cm) = doc.mutation_registry.get(&id).cloned() else {
        return false;
    };
    let now = super::super::now_ms() as u64;
    let applied = apply_keybind_custom_mutation(doc, ctx.mindmap_tree, ctx.scene_cache, &cm, &nid, now);
    if applied {
        rebuild_all(
            doc,
            ctx.interaction_mode,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        );
    }
    applied
}

/// Inline helper for the empty-canvas orphan-and-edit gesture so
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
/// In each case the state resets to `None` so the cursor doesn't
/// re-fire the threshold-cross.
fn apply_fast_resize_start(ctx: &mut InputHandlerContext<'_>, hit: Option<&DispatchHit>) {
    use baumhard::mindmap::scene_builder::infer_resize_anchor;
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
        *ctx.drag_state = DragState::Throttled(ThrottledDrag::SectionResize(
            SectionResizeInteraction::new(
                node_id, section_idx, side, start_offset, start_size,
                // Fast-resize gesture (`PendingRight` promotion) — the
                // right-button release path may finalize this drag.
                true,
            ),
        ));
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
        *ctx.drag_state = DragState::Throttled(ThrottledDrag::NodeResize(
            NodeResizeInteraction::new(node_id, side, start_position, start_size, true),
        ));
    }
}

/// `DoubleClickActivate` and `CreateOrphanNodeAndEdit` share one
/// implementation.
fn dispatch_create_orphan_and_edit(ctx: &mut InputHandlerContext<'_>, hit: &DispatchHit) {
    if let Some(doc) = ctx.document.as_mut() {
        let new_id = doc.create_orphan_and_select(hit.canvas_pos);
        rebuild_all(
            doc,
            ctx.interaction_mode,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        );
        open_text_edit(
            &new_id,
            true,
            doc,
            ctx.text_edit_state,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
        );
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
