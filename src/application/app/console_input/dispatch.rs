// SPDX-License-Identifier: MPL-2.0

//! Console keystroke dispatcher. Routes each key event through the
//! contextual keybind resolver (`InputContext::Console`) and
//! delegates state mutations to the pure helpers in `edit.rs`.
//! Character input that matches no action is inserted at the cursor
//! as literal text.

use crate::application::platform::input::Key;

use crate::application::console::{ConsoleLine, ConsoleState, MAX_HISTORY};
use crate::application::document::MindMapDocument;
use crate::application::keybinds::{Action, InputContext, ResolvedKeybinds};
use crate::application::renderer::Renderer;

use super::completion::{accept_console_completion, nav_popup, recompute_console_completions};
use super::edit::{self, EditOutcome};
use super::exec::execute_console_line;
use super::history::save_console_history;
use super::rebuild_console_overlay;

/// Handle a keystroke while the console is open. Resolves the key
/// through `action_for_context(InputContext::Console, ...)` and
/// dispatches on the resulting `Action`. Pure state mutations live
/// in `super::edit`; this function owns the heavy-lifting cases
/// (close / submit / overlay rebuild) and routes the rest to the
/// helpers.
///
/// Cursor arithmetic is **grapheme-indexed** via
/// `baumhard::util::grapheme_chad` so ZWJ emoji and combining marks
/// are treated as atomic cursor cells — see `edit.rs`.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_console_key(
    key_name: &Option<String>,
    logical_key: &Key,
    ctx: &mut super::super::input_context::InputHandlerContext<'_>,
) {
    let Some(name) = key_name.as_deref() else {
        return;
    };

    let action = ctx.keybinds.action_for_context(
        InputContext::Console,
        name,
        ctx.modifiers.control_key(),
        ctx.modifiers.shift_key(),
        ctx.modifiers.alt_key(),
    );

    // Funneled path: every `Action::Console*` variant routes
    // through `dispatch_action`, which delegates back to the
    // [`dispatch_console_action`] fan-out below. Macros can fire
    // any Console variant (e.g. `Action::ConsoleScrollPageUp` from
    // a User-tier macro stepping through the scrollback) — same
    // funnel `OpenConsole` already used. CODE_CONVENTIONS §3.
    if let Some(a) = action {
        let _ = super::super::dispatch::dispatch_action(a, ctx, None);
        return;
    }

    // Carve-out (§3 "Modal steals own the literal `winit::Key`
    // payload — character insertion, IME sequences"): no Action
    // matched but the key produced a character, so insert it.
    let Key::Character(c) = logical_key else {
        return;
    };
    let outcome = edit::insert_text(ctx.console_state, c.as_str());
    after_state_change(
        outcome,
        ctx.console_state,
        ctx.document,
        ctx.app_scene,
        ctx.renderer,
        ctx.keybinds,
    );
}

/// Dispatch fan-out for every `Action::Console*` variant. Called
/// from `dispatch::dispatch_action`'s single Console* arm so
/// macros AND keystrokes reach the same body. Each branch calls
/// the matching `edit::*` helper (or `submit_line` /
/// `nav_popup` / `accept_console_completion` for the multi-line
/// cases) and runs `after_state_change` for the post-edit
/// completion-recompute + overlay rebuild.
///
/// `pub(in crate::application::app)` so the dispatch arm can call
/// it; `edit::*` helpers stay `pub(super)` to console_input —
/// outside callers reach them only through this one seam.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn dispatch_console_action(
    action: &Action,
    ctx: &mut super::super::input_context::InputHandlerContext<'_>,
) {
    match action {
        // Two-tier Close: dismiss popup first, then close the modal
        // and persist history.
        Action::ConsoleClose => {
            if edit::dismiss_popup(ctx.console_state) {
                after_state_change(
                    EditOutcome::Unchanged,
                    ctx.console_state,
                    ctx.document,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.keybinds,
                );
            } else {
                save_console_history(ctx.console_history);
                *ctx.console_state = ConsoleState::Closed;
                ctx.renderer.rebuild_console_overlay_buffers(ctx.app_scene, None);
            }
        }
        // Submit executes the line — own flow because it needs
        // scene/tree/renderer + macro registry to run commands.
        Action::ConsoleSubmit => {
            submit_line(ctx);
        }
        // History navigation first tries the completion popup,
        // then falls back to walking command history.
        Action::ConsoleHistoryUp => {
            let outcome = if nav_popup(ctx.console_state, -1) {
                EditOutcome::Unchanged
            } else {
                edit::history_walk_back(ctx.console_state)
            };
            after_state_change(
                outcome,
                ctx.console_state,
                ctx.document,
                ctx.app_scene,
                ctx.renderer,
                ctx.keybinds,
            );
        }
        Action::ConsoleHistoryDown => {
            let outcome = if nav_popup(ctx.console_state, 1) {
                EditOutcome::Unchanged
            } else {
                edit::history_walk_forward(ctx.console_state)
            };
            after_state_change(
                outcome,
                ctx.console_state,
                ctx.document,
                ctx.app_scene,
                ctx.renderer,
                ctx.keybinds,
            );
        }
        // Tab accepts the highlighted completion; recompute the
        // popup against the new cursor.
        Action::ConsoleTabComplete => {
            accept_console_completion(ctx.console_state);
            after_state_change(
                EditOutcome::InputChanged,
                ctx.console_state,
                ctx.document,
                ctx.app_scene,
                ctx.renderer,
                ctx.keybinds,
            );
        }
        // Scrollback navigation (Shift+Up/Down + PgUp/PgDn +
        // Shift+Home/End). `Unchanged` outcome — scrollback is
        // a view position, not an input mutation.
        Action::ConsoleScrollUp
        | Action::ConsoleScrollDown
        | Action::ConsoleScrollPageUp
        | Action::ConsoleScrollPageDown
        | Action::ConsoleScrollHome
        | Action::ConsoleScrollEnd => {
            let direction = match action {
                Action::ConsoleScrollUp => edit::ScrollDirection::LineUp,
                Action::ConsoleScrollDown => edit::ScrollDirection::LineDown,
                Action::ConsoleScrollPageUp => edit::ScrollDirection::PageUp,
                Action::ConsoleScrollPageDown => edit::ScrollDirection::PageDown,
                Action::ConsoleScrollHome => edit::ScrollDirection::Home,
                Action::ConsoleScrollEnd => edit::ScrollDirection::End,
                // Safe fallback per CODE_CONVENTIONS §9 — `Action`
                // is `#[non_exhaustive]`, so a future variant added
                // to the outer or-pattern without an inner arm
                // would otherwise panic in an interactive path.
                // Match the precedent established in
                // `dispatch_action_core::dispatch_compatible`.
                _ => {
                    log::error!(
                        "dispatch_console_action: scroll fan-out missed inner-match: {:?}",
                        action,
                    );
                    return;
                }
            };
            edit::adjust_scroll(ctx.console_state, direction);
            after_state_change(
                EditOutcome::Unchanged,
                ctx.console_state,
                ctx.document,
                ctx.app_scene,
                ctx.renderer,
                ctx.keybinds,
            );
        }
        // Pure edits: cursor / delete / kill / clear-line / insert-
        // space. Each helper returns an `EditOutcome` that
        // `after_state_change` consumes.
        Action::ConsoleClearLine
        | Action::ConsoleJumpStart
        | Action::ConsoleJumpEnd
        | Action::ConsoleKillToStart
        | Action::ConsoleKillWord
        | Action::ConsoleCursorLeft
        | Action::ConsoleCursorRight
        | Action::ConsoleCursorHome
        | Action::ConsoleCursorEnd
        | Action::ConsoleDeleteBack
        | Action::ConsoleDeleteForward
        | Action::ConsoleInsertSpace => {
            let outcome = match action {
                Action::ConsoleClearLine => edit::clear_line(ctx.console_state),
                Action::ConsoleJumpStart => edit::jump_to_start(ctx.console_state),
                Action::ConsoleJumpEnd => edit::jump_to_end(ctx.console_state),
                Action::ConsoleKillToStart => edit::kill_to_start(ctx.console_state),
                Action::ConsoleKillWord => edit::kill_word(ctx.console_state),
                Action::ConsoleCursorLeft => edit::cursor_left(ctx.console_state),
                Action::ConsoleCursorRight => edit::cursor_right(ctx.console_state),
                Action::ConsoleCursorHome => edit::cursor_home(ctx.console_state),
                Action::ConsoleCursorEnd => edit::cursor_end(ctx.console_state),
                Action::ConsoleDeleteBack => edit::delete_back(ctx.console_state),
                Action::ConsoleDeleteForward => edit::delete_forward(ctx.console_state),
                Action::ConsoleInsertSpace => edit::insert_space(ctx.console_state),
                // Safe fallback per CODE_CONVENTIONS §9 — same
                // forward-compat reasoning as the scroll cluster
                // above.
                _ => {
                    log::error!(
                        "dispatch_console_action: pure-edit fan-out missed inner-match: {:?}",
                        action,
                    );
                    return;
                }
            };
            after_state_change(
                outcome,
                ctx.console_state,
                ctx.document,
                ctx.app_scene,
                ctx.renderer,
                ctx.keybinds,
            );
        }
        // No-op: `Action` is `#[non_exhaustive]` so a future variant
        // added inadvertently to a Console* binding context would
        // land here. Log per CODE_CONVENTIONS §9 (interactive paths
        // fail-safe).
        _ => log::error!("dispatch_console_action: unrecognized action {:?}", action,),
    }
}

/// Apply the side effects of an edit: recompute completions if the
/// input changed, then rebuild the overlay so the next frame
/// reflects the new state. `pub(in crate::application::app)` so
/// the funneled `Action::Console*` dispatch arms in `dispatch.rs`
/// reach the same post-edit bookkeeping.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn after_state_change(
    outcome: EditOutcome,
    console_state: &mut ConsoleState,
    document: &Option<MindMapDocument>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    keybinds: &ResolvedKeybinds,
) {
    if outcome.input_changed() {
        recompute_console_completions(console_state, document.as_ref());
        // Typing should put the user back at the bottom of the
        // scrollback so the next command's output is visible. Same
        // contract as `push_scrollback_*`.
        if let ConsoleState::Open { scroll_offset, .. } = console_state {
            *scroll_offset = 0;
        }
    }
    if let Some(doc) = document.as_ref() {
        rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
    }
}

use crate::application::app::input_context::InputHandlerContext;

/// Take the current input line, append to history + scrollback,
/// execute via `execute_console_line`, and rebuild the overlay.
/// `pub(in crate::application::app)` so the funneled
/// `Action::ConsoleSubmit` dispatch arm in `dispatch.rs` reaches it.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn submit_line(ctx: &mut InputHandlerContext<'_>) {
    // Local re-bindings preserve the original
    // SubmitLineContext-destructure shape. Without these, `match
    // ctx.console_state` doesn't trigger match-ergonomics on the
    // through-field place expression and inner field bindings come
    // out by value instead of by `&mut`.
    let console_state: &mut ConsoleState = &mut *ctx.console_state;
    let line = match console_state {
        ConsoleState::Open { input, .. } => std::mem::take(input),
        ConsoleState::Closed => return,
    };
    if let ConsoleState::Open {
        cursor,
        history_idx,
        scrollback,
        completions,
        completion_idx,
        history,
        scroll_offset,
        ..
    } = console_state
    {
        *cursor = 0;
        *history_idx = None;
        completions.clear();
        *completion_idx = None;
        scrollback.push(ConsoleLine::Input(format!("> {}", line)));
        // Submitting a command should always pin the view to the
        // bottom — same contract `push_scrollback_*` honors. The
        // input echo above bypassed those helpers, so do the
        // reset here to keep the documented `scroll_offset`
        // contract intact regardless of whether the command
        // produces any output.
        *scroll_offset = 0;
        if !line.trim().is_empty() && history.last().map(|s| s.as_str()) != Some(line.as_str()) {
            history.push(line.clone());
            if history.len() > MAX_HISTORY {
                let drop = history.len() - MAX_HISTORY;
                history.drain(..drop);
            }
            ctx.console_history.push(line.clone());
            if ctx.console_history.len() > MAX_HISTORY {
                let drop = ctx.console_history.len() - MAX_HISTORY;
                ctx.console_history.drain(..drop);
            }
        }
        if let Some(doc) = ctx.document.as_mut() {
            execute_console_line(
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
        }
    }
    if let Some(doc) = ctx.document.as_ref() {
        rebuild_console_overlay(ctx.console_state, doc, ctx.app_scene, ctx.renderer, ctx.keybinds);
    }
}
