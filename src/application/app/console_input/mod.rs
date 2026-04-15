//! Console line-editor: per-keystroke dispatch, completion-popup
//! helpers, command execution, overlay rebuild, and history
//! persistence. Split across five leaf modules:
//!
//! - [`keys`] — lowercase key-name constants the dispatcher matches.
//! - [`dispatch`] — `handle_console_key`: the keystroke router.
//! - [`completion`] — recompute / nav / accept for the popup.
//! - [`exec`] — `execute_console_line` + Ctrl+S save.
//! - [`history`] — load / save of the on-disk history file.
//!
//! `mod.rs` itself owns `rebuild_console_overlay` (shared by
//! dispatch + exec) and the tiny scrollback push helpers both use.

#![cfg(not(target_arch = "wasm32"))]

mod completion;
mod dispatch;
mod exec;
mod history;
mod keys;

pub(super) use dispatch::handle_console_key;
pub(super) use exec::save_document_to_bound_path;
pub(super) use history::{load_console_history, save_console_history};

use crate::application::console::{ConsoleLine, ConsoleState};
use crate::application::document::MindMapDocument;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;

pub(super) fn push_scrollback_output(state: &mut ConsoleState, text: String) {
    if let ConsoleState::Open { scrollback, .. } = state {
        scrollback.push(ConsoleLine::Output(text));
    }
}

pub(super) fn push_scrollback_error(state: &mut ConsoleState, text: String) {
    if let ConsoleState::Open { scrollback, .. } = state {
        scrollback.push(ConsoleLine::Error(text));
    }
}

/// Build the console overlay geometry from the current state and
/// push it to the renderer. Called whenever the console opens, the
/// input changes, or scrollback / completions update.
pub(super) fn rebuild_console_overlay(
    console_state: &ConsoleState,
    _document: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    keybinds: &ResolvedKeybinds,
) {
    use crate::application::renderer::{
        ConsoleOverlayCompletion, ConsoleOverlayGeometry, ConsoleOverlayLine,
        ConsoleOverlayLineKind,
    };
    let (input, cursor, scrollback, completions, selected_completion) = match console_state {
        ConsoleState::Closed => {
            renderer.rebuild_console_overlay_buffers(app_scene, None);
            return;
        }
        ConsoleState::Open {
            input,
            cursor,
            scrollback,
            completions,
            completion_idx,
            ..
        } => (input, *cursor, scrollback, completions, *completion_idx),
    };
    let scrollback_lines: Vec<ConsoleOverlayLine> = scrollback
        .iter()
        .map(|l| match l {
            ConsoleLine::Input(t) => ConsoleOverlayLine {
                text: t.clone(),
                kind: ConsoleOverlayLineKind::Input,
            },
            ConsoleLine::Output(t) => ConsoleOverlayLine {
                text: t.clone(),
                kind: ConsoleOverlayLineKind::Output,
            },
            ConsoleLine::Error(t) => ConsoleOverlayLine {
                text: t.clone(),
                kind: ConsoleOverlayLineKind::Error,
            },
        })
        .collect();
    let completion_geo: Vec<ConsoleOverlayCompletion> = completions
        .iter()
        .map(|c| ConsoleOverlayCompletion {
            text: c.text.clone(),
            hint: c.hint.clone(),
        })
        .collect();
    let geometry = ConsoleOverlayGeometry {
        input: input.clone(),
        cursor_grapheme: cursor,
        scrollback: scrollback_lines,
        completions: completion_geo,
        selected_completion,
        font_family: keybinds.console_font.clone(),
        font_size: keybinds.console_font_size,
    };
    renderer.rebuild_console_overlay_buffers(app_scene, Some(&geometry));
}
