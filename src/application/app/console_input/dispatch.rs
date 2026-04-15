//! Console keystroke dispatcher. Routes each key event through the
//! Ctrl-chord path (Ctrl+A / C / E / U / W) or the named-key /
//! character path (Enter / Tab / arrows / Home / End / Backspace /
//! Delete / Space / printable chars). The per-branch editing logic
//! is inline here because every branch reaches into the same
//! `ConsoleState::Open` fields and extracting one arm at a time
//! would just replace one match with another.

use winit::keyboard::Key;

use crate::application::color_picker::ColorPickerState;
use crate::application::console::{ConsoleLine, ConsoleState, MAX_HISTORY};
use crate::application::document::MindMapDocument;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;

use super::super::LabelEditState;
use super::completion::{accept_console_completion, nav_popup, recompute_console_completions};
use super::exec::execute_console_line;
use super::history::save_console_history;
use super::keys::*;
use super::rebuild_console_overlay;

/// Handle a keystroke while the console is open. The console is a
/// shell-style line editor: char input inserts at the cursor, Tab
/// cycles completions, Up/Down walks history, Enter parses +
/// executes the buffered line, and Escape closes. Regular hotkeys
/// are suppressed — this runs entirely outside the keybinds
/// resolver.
///
/// Cursor arithmetic throughout this function is **grapheme-indexed**,
/// not byte-indexed, to satisfy CODE_CONVENTIONS §2. All mutations
/// route through `baumhard::util::grapheme_chad` so ZWJ emoji and
/// combining marks are treated as atomic cursor cells.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_console_key(
    key_name: &Option<String>,
    logical_key: &Key,
    ctrl_pressed: bool,
    console_state: &mut ConsoleState,
    console_history: &mut Vec<String>,
    label_edit_state: &mut LabelEditState,
    color_picker_state: &mut ColorPickerState,
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    keybinds: &mut ResolvedKeybinds,
) {
    use baumhard::util::grapheme_chad::{
        count_grapheme_clusters, delete_front_unicode, delete_grapheme_at,
        find_byte_index_of_grapheme, insert_str_at_grapheme,
    };

    let name = match key_name.as_deref() {
        Some(n) => n,
        None => return,
    };
    // Ctrl-chords take priority over named / character handling so
    // Ctrl+C / Ctrl+A / etc. don't get swallowed by the `_` branch.
    if ctrl_pressed {
        match name {
            CONSOLE_KEY_CTRL_C => {
                // Clear input without closing — same as the shell
                // muscle-memory: Ctrl+C abandons the current line.
                if let ConsoleState::Open { input, cursor, history_idx, .. } = console_state {
                    input.clear();
                    *cursor = 0;
                    *history_idx = None;
                }
                recompute_console_completions(console_state, document.as_ref());
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
                return;
            }
            CONSOLE_KEY_CTRL_A => {
                if let ConsoleState::Open { cursor, .. } = console_state {
                    *cursor = 0;
                }
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
                return;
            }
            CONSOLE_KEY_CTRL_E => {
                if let ConsoleState::Open { cursor, input, .. } = console_state {
                    *cursor = count_grapheme_clusters(input);
                }
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
                return;
            }
            CONSOLE_KEY_CTRL_U => {
                // Kill to start of line — drop the first `cursor`
                // grapheme clusters via `delete_front_unicode`.
                if let ConsoleState::Open { input, cursor, .. } = console_state {
                    delete_front_unicode(input, *cursor);
                    *cursor = 0;
                }
                recompute_console_completions(console_state, document.as_ref());
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
                return;
            }
            CONSOLE_KEY_CTRL_W => {
                // Kill word before cursor (whitespace-separated).
                // Walk back through grapheme clusters, first skipping
                // trailing whitespace, then the word — everything is
                // kept grapheme-indexed.
                if let ConsoleState::Open { input, cursor, .. } = console_state {
                    let end_g = *cursor;
                    // Collect graphemes up to `end_g` so we can walk
                    // them backwards without re-parsing.
                    use unicode_segmentation::UnicodeSegmentation;
                    let prefix_bytes = find_byte_index_of_grapheme(input, end_g)
                        .unwrap_or(input.len());
                    let clusters: Vec<&str> = input[..prefix_bytes].graphemes(true).collect();
                    let mut start_g = clusters.len();
                    while start_g > 0
                        && clusters[start_g - 1].chars().all(|c| c.is_whitespace())
                    {
                        start_g -= 1;
                    }
                    while start_g > 0
                        && !clusters[start_g - 1].chars().all(|c| c.is_whitespace())
                    {
                        start_g -= 1;
                    }
                    for _ in 0..(end_g - start_g) {
                        delete_grapheme_at(input, start_g);
                    }
                    *cursor = start_g;
                }
                recompute_console_completions(console_state, document.as_ref());
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
                return;
            }
            _ => {}
        }
    }
    match name {
        CONSOLE_KEY_ESCAPE => {
            // Two-tier Esc: if the completion popup is open, first
            // press dismisses it; second press (with no popup)
            // closes the console entirely. Matches the
            // "temporary-UI-first" muscle memory from vim and
            // browser address bars.
            let had_popup = matches!(
                console_state,
                ConsoleState::Open { completions, .. } if !completions.is_empty()
            );
            if had_popup {
                if let ConsoleState::Open { completions, completion_idx, .. } = console_state {
                    completions.clear();
                    *completion_idx = None;
                }
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
            } else {
                save_console_history(console_history);
                *console_state = ConsoleState::Closed;
                renderer.rebuild_console_overlay_buffers(app_scene, None);
            }
        }
        CONSOLE_KEY_ENTER => {
            // Snapshot input, reset state, then parse + execute.
            // Append the executed line to persistent history + the
            // in-state history copy, then re-rebuild the overlay.
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
                if !line.trim().is_empty() {
                    // Dedup against the most recent history entry —
                    // the shell convention that repeated commands
                    // don't clutter the stack.
                    if history.last().map(|s| s.as_str()) != Some(line.as_str()) {
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
                }
                if let Some(doc) = document.as_mut() {
                    execute_console_line(
                        &line,
                        console_state,
                        label_edit_state,
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
        CONSOLE_KEY_TAB => {
            // Tab accepts the highlighted completion (or index 0 if
            // somehow no row is highlighted). The popup is live —
            // it's already populated by the per-keystroke recompute
            // that the character-input arms run below, so Tab has
            // no "first-press compute" branch anymore.
            accept_console_completion(console_state);
            recompute_console_completions(console_state, document.as_ref());
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_ARROW_UP | CONSOLE_KEY_UP => {
            // If the popup is open, Up moves the selection toward
            // the top of the list; otherwise it walks history
            // backwards.
            let moved_in_popup = nav_popup(console_state, -1);
            if !moved_in_popup {
                if let ConsoleState::Open { input, cursor, history, history_idx, .. } = console_state {
                    if !history.is_empty() {
                        let next = match history_idx {
                            None => history.len() - 1,
                            Some(0) => 0,
                            Some(i) => *i - 1,
                        };
                        *history_idx = Some(next);
                        *input = history[next].clone();
                        *cursor = count_grapheme_clusters(input);
                    }
                }
                recompute_console_completions(console_state, document.as_ref());
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_ARROW_DOWN | CONSOLE_KEY_DOWN => {
            // Down moves the selection toward the prompt (the row
            // closest to the input line). Same branch logic as Up.
            let moved_in_popup = nav_popup(console_state, 1);
            if !moved_in_popup {
                if let ConsoleState::Open { input, cursor, history, history_idx, .. } = console_state {
                    match history_idx {
                        Some(i) if *i + 1 < history.len() => {
                            let next = *i + 1;
                            *history_idx = Some(next);
                            *input = history[next].clone();
                            *cursor = count_grapheme_clusters(input);
                        }
                        Some(_) => {
                            *history_idx = None;
                            input.clear();
                            *cursor = 0;
                        }
                        None => {}
                    }
                }
                recompute_console_completions(console_state, document.as_ref());
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_ARROW_LEFT | CONSOLE_KEY_LEFT => {
            if let ConsoleState::Open { cursor, .. } = console_state {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_ARROW_RIGHT | CONSOLE_KEY_RIGHT => {
            if let ConsoleState::Open { cursor, input, .. } = console_state {
                let max = count_grapheme_clusters(input);
                if *cursor < max {
                    *cursor += 1;
                }
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_HOME => {
            if let ConsoleState::Open { cursor, .. } = console_state {
                *cursor = 0;
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_END => {
            if let ConsoleState::Open { cursor, input, .. } = console_state {
                *cursor = count_grapheme_clusters(input);
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_BACKSPACE => {
            if let ConsoleState::Open { input, cursor, .. } = console_state {
                if *cursor > 0 {
                    *cursor -= 1;
                    delete_grapheme_at(input, *cursor);
                }
            }
            recompute_console_completions(console_state, document.as_ref());
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_DELETE => {
            if let ConsoleState::Open { input, cursor, .. } = console_state {
                if *cursor < count_grapheme_clusters(input) {
                    delete_grapheme_at(input, *cursor);
                }
            }
            recompute_console_completions(console_state, document.as_ref());
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_SPACE => {
            // winit delivers the spacebar as `Key::Named(NamedKey::Space)`
            // rather than a `Key::Character(" ")`, so the `_` arm below
            // (which only handles `Key::Character`) would drop it. Insert
            // a literal space here instead.
            if let ConsoleState::Open { input, cursor, history_idx, .. } = console_state {
                insert_str_at_grapheme(input, *cursor, " ");
                *cursor += 1;
                *history_idx = None;
            }
            recompute_console_completions(console_state, document.as_ref());
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        _ => {
            // Character input: insert at cursor, one grapheme at a
            // time. Filter control chars — dead keys / IME can
            // occasionally ship control payloads via
            // `Key::Character` and those must not land in the input
            // buffer as literal glyphs. Inserts go through
            // `insert_str_at_grapheme` so the cursor stays a
            // grapheme index.
            if let Key::Character(c) = logical_key {
                if let ConsoleState::Open {
                    input, cursor, history_idx, ..
                } = console_state
                {
                    for ch in c.as_str().chars() {
                        if ch.is_control() {
                            continue;
                        }
                        let mut buf = [0u8; 4];
                        let encoded = ch.encode_utf8(&mut buf);
                        insert_str_at_grapheme(input, *cursor, encoded);
                        *cursor += 1;
                    }
                    *history_idx = None;
                }
                recompute_console_completions(console_state, document.as_ref());
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
            }
        }
    }
}
