//! Console keystroke dispatcher. Routes each key event through the
//! contextual keybind resolver (`InputContext::Console`) and
//! delegates state mutations to the pure helpers in `edit.rs`.
//! Character input that matches no action is inserted at the cursor
//! as literal text.

use winit::keyboard::Key;

use crate::application::color_picker::ColorPickerState;
use crate::application::console::{ConsoleLine, ConsoleState, MAX_HISTORY};
use crate::application::document::MindMapDocument;
use crate::application::keybinds::{Action, InputContext, ResolvedKeybinds};
use crate::application::renderer::Renderer;

use super::super::LabelEditState;
use super::completion::{accept_console_completion, nav_popup, recompute_console_completions};
use super::edit::{self, EditOutcome};
use super::super::PortalTextEditState;
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
    ctrl_pressed: bool,
    shift_pressed: bool,
    alt_pressed: bool,
    console_state: &mut ConsoleState,
    console_history: &mut Vec<String>,
    label_edit_state: &mut LabelEditState,
    portal_text_edit_state: &mut PortalTextEditState,
    color_picker_state: &mut ColorPickerState,
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    keybinds: &mut ResolvedKeybinds,
) {
    let Some(name) = key_name.as_deref() else {
        return;
    };

    let action = keybinds.action_for_context(
        InputContext::Console, name, ctrl_pressed, shift_pressed, alt_pressed,
    );

    // Two-tier Close: dismiss popup first, then close.
    if let Some(Action::ConsoleClose) = action {
        if edit::dismiss_popup(console_state) {
            after_state_change(
                EditOutcome::Unchanged, console_state, document, app_scene, renderer, keybinds,
            );
        } else {
            save_console_history(console_history);
            *console_state = ConsoleState::Closed;
            renderer.rebuild_console_overlay_buffers(app_scene, None);
        }
        return;
    }

    // Submit executes the line; keeps its own flow because it needs
    // scene/tree/renderer access to run commands.
    if let Some(Action::ConsoleSubmit) = action {
        submit_line(SubmitLineContext {
            console_state,
            console_history,
            label_edit_state,
            portal_text_edit_state,
            color_picker_state,
            document,
            mindmap_tree,
            app_scene,
            renderer,
            scene_cache,
            keybinds,
        });
        return;
    }

    // History navigation first tries the completion popup, then
    // walks the command history.
    if let Some(Action::ConsoleHistoryUp) = action {
        let outcome = if nav_popup(console_state, -1) {
            EditOutcome::Unchanged
        } else {
            edit::history_walk_back(console_state)
        };
        after_state_change(outcome, console_state, document, app_scene, renderer, keybinds);
        return;
    }
    if let Some(Action::ConsoleHistoryDown) = action {
        let outcome = if nav_popup(console_state, 1) {
            EditOutcome::Unchanged
        } else {
            edit::history_walk_forward(console_state)
        };
        after_state_change(outcome, console_state, document, app_scene, renderer, keybinds);
        return;
    }

    // Tab accepts the highlighted completion, then recomputes so
    // the popup either narrows or clears against the new cursor.
    if let Some(Action::ConsoleTabComplete) = action {
        accept_console_completion(console_state);
        after_state_change(
            EditOutcome::InputChanged, console_state, document, app_scene, renderer, keybinds,
        );
        return;
    }

    // All remaining Console actions are pure edits.
    let edit_outcome = match action {
        Some(Action::ConsoleClearLine) => edit::clear_line(console_state),
        Some(Action::ConsoleJumpStart) => edit::jump_to_start(console_state),
        Some(Action::ConsoleJumpEnd) => edit::jump_to_end(console_state),
        Some(Action::ConsoleKillToStart) => edit::kill_to_start(console_state),
        Some(Action::ConsoleKillWord) => edit::kill_word(console_state),
        Some(Action::ConsoleCursorLeft) => edit::cursor_left(console_state),
        Some(Action::ConsoleCursorRight) => edit::cursor_right(console_state),
        Some(Action::ConsoleCursorHome) => edit::cursor_home(console_state),
        Some(Action::ConsoleCursorEnd) => edit::cursor_end(console_state),
        Some(Action::ConsoleDeleteBack) => edit::delete_back(console_state),
        Some(Action::ConsoleDeleteForward) => edit::delete_forward(console_state),
        Some(Action::ConsoleInsertSpace) => edit::insert_space(console_state),
        _ => match logical_key {
            // No console action matched. If the key produced a
            // character, insert it as literal text.
            Key::Character(c) => edit::insert_text(console_state, c.as_str()),
            _ => return,
        },
    };

    after_state_change(edit_outcome, console_state, document, app_scene, renderer, keybinds);
}

/// Apply the side-effects of an edit: recompute completions if the
/// input changed, then rebuild the overlay so the next frame
/// reflects the new state.
#[cfg(not(target_arch = "wasm32"))]
fn after_state_change(
    outcome: EditOutcome,
    console_state: &mut ConsoleState,
    document: &Option<MindMapDocument>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    keybinds: &ResolvedKeybinds,
) {
    if outcome.input_changed() {
        recompute_console_completions(console_state, document.as_ref());
    }
    if let Some(doc) = document.as_ref() {
        rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
    }
}

/// Console-submit context — the narrow view the line executor needs
/// into app state. Mirrors the `InputHandlerContext` shape but
/// scoped to console submission, and kept inside this module because
/// no code outside the submit path constructs it.
#[cfg(not(target_arch = "wasm32"))]
struct SubmitLineContext<'a> {
    console_state: &'a mut ConsoleState,
    console_history: &'a mut Vec<String>,
    label_edit_state: &'a mut LabelEditState,
    portal_text_edit_state: &'a mut PortalTextEditState,
    color_picker_state: &'a mut ColorPickerState,
    document: &'a mut Option<MindMapDocument>,
    mindmap_tree: &'a mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &'a mut crate::application::scene_host::AppScene,
    renderer: &'a mut Renderer,
    scene_cache: &'a mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    keybinds: &'a ResolvedKeybinds,
}

/// Take the current input line, append to history + scrollback,
/// execute via `execute_console_line`, and rebuild the overlay.
#[cfg(not(target_arch = "wasm32"))]
fn submit_line(ctx: SubmitLineContext<'_>) {
    let SubmitLineContext {
        console_state,
        console_history,
        label_edit_state,
        portal_text_edit_state,
        color_picker_state,
        document,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
        keybinds,
    } = ctx;
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
        ..
    } = console_state
    {
        *cursor = 0;
        *history_idx = None;
        completions.clear();
        *completion_idx = None;
        scrollback.push(ConsoleLine::Input(format!("> {}", line)));
        if !line.trim().is_empty()
            && history.last().map(|s| s.as_str()) != Some(line.as_str())
        {
            history.push(line.clone());
            if history.len() > MAX_HISTORY {
                let drop = history.len() - MAX_HISTORY;
                history.drain(..drop);
            }
            console_history.push(line.clone());
            if console_history.len() > MAX_HISTORY {
                let drop = console_history.len() - MAX_HISTORY;
                console_history.drain(..drop);
            }
        }
        if let Some(doc) = document.as_mut() {
            execute_console_line(
                &line,
                console_state,
                label_edit_state,
                portal_text_edit_state,
                color_picker_state,
                doc,
                mindmap_tree,
                app_scene,
                renderer,
                scene_cache,
            );
        }
    }
    if let Some(doc) = document.as_ref() {
        rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
    }
}
