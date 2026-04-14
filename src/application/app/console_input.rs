//! Console line-editor key handling. The console is a shell-style
//! line editor (Tab completion, Up/Down history, Enter to execute,
//! Esc to close). All the per-keystroke dispatch + the
//! recompute/nav/accept/execute helpers + the on-disk history
//! persistence live here so the `Application::run` event loop in
//! the parent module stays focused on the loop itself.

use winit::keyboard::Key;

use crate::application::console::commands::Command;
use crate::application::console::completion::complete as complete_console;
use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{
    ConsoleContext, ConsoleEffects, ConsoleLine, ConsoleState, ExecResult, MAX_HISTORY,
};
use crate::application::document::MindMapDocument;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;

use super::color_picker_flow::{
    close_color_picker_standalone, open_color_picker_contextual,
    open_color_picker_standalone,
};
use super::{open_label_edit, rebuild_all, LabelEditState};

// --- Console line-editor key names ---------------------------------
//
// `keybinds::normalize_key_name` lowercases the winit key identifier,
// so every console-handled key matches the lowercase forms here. Kept
// local to `app.rs` because this is the only module that dispatches
// on them.
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ESCAPE: &str = "escape";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ENTER: &str = "enter";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_TAB: &str = "tab";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ARROW_UP: &str = "arrowup";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_UP: &str = "up";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ARROW_DOWN: &str = "arrowdown";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_DOWN: &str = "down";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ARROW_LEFT: &str = "arrowleft";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_LEFT: &str = "left";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ARROW_RIGHT: &str = "arrowright";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_RIGHT: &str = "right";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_HOME: &str = "home";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_END: &str = "end";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_BACKSPACE: &str = "backspace";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_DELETE: &str = "delete";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_SPACE: &str = "space";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_CTRL_A: &str = "a";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_CTRL_C: &str = "c";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_CTRL_E: &str = "e";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_CTRL_U: &str = "u";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_CTRL_W: &str = "w";

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
pub(super) fn handle_console_key(
    key_name: &Option<String>,
    logical_key: &Key,
    ctrl_pressed: bool,
    console_state: &mut ConsoleState,
    console_history: &mut Vec<String>,
    label_edit_state: &mut LabelEditState,
    color_picker_state: &mut crate::application::color_picker::ColorPickerState,
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

/// Re-run the completion engine against the current input and
/// cursor, populating `completions` and defaulting `completion_idx`
/// to the bottom row (row closest to the prompt — which is what
/// Down-then-Tab muscle memory expects to land on first).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn recompute_console_completions(
    console_state: &mut ConsoleState,
    document: Option<&MindMapDocument>,
) {
    use baumhard::util::grapheme_chad::find_byte_index_of_grapheme;
    let Some(doc) = document else { return };
    if let ConsoleState::Open {
        input,
        cursor,
        completions,
        completion_idx,
        ..
    } = console_state
    {
        let byte_cursor = find_byte_index_of_grapheme(input, *cursor).unwrap_or(input.len());
        let ctx = ConsoleContext::from_document(doc);
        let new = complete_console(input, byte_cursor, &ctx);
        *completions = new
            .into_iter()
            .map(|c| crate::application::console::completion::Completion {
                text: c.text,
                display: c.display,
                hint: c.hint,
            })
            .collect();
        // Default highlight: the first row. Matches the terminal /
        // IDE convention where the top candidate is "most likely".
        // Users Down-arrow toward the prompt when they want a
        // different row.
        *completion_idx = if completions.is_empty() { None } else { Some(0) };
    }
}

/// Move the completion highlight by `step` (-1 for Up, +1 for Down).
/// Returns `true` if a popup was present and the move was consumed;
/// `false` when there's no popup, letting the caller fall through
/// to history navigation.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn nav_popup(console_state: &mut ConsoleState, step: i32) -> bool {
    if let ConsoleState::Open { completions, completion_idx, .. } = console_state {
        if completions.is_empty() {
            return false;
        }
        let len = completions.len() as i32;
        let cur = completion_idx.map(|i| i as i32).unwrap_or(-1);
        let next = ((cur + step).rem_euclid(len)) as usize;
        *completion_idx = Some(next);
        return true;
    }
    false
}

/// Replace the current token (or kv-value slot) under the cursor
/// with the highlighted completion's `text`, advancing the cursor
/// past the replacement.
///
/// Trailing-space rule:
/// - positional / command-name: append a space (next token starts fresh)
/// - kv-key (text ends in `=`): no space (value follows immediately)
/// - kv-value: no space (user may still be typing a quoted value,
///   or wants to type an adjacent kv pair)
///
/// No-op if no popup is present.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn accept_console_completion(console_state: &mut ConsoleState) {
    use baumhard::util::grapheme_chad::{count_grapheme_clusters, find_byte_index_of_grapheme};
    use unicode_segmentation::UnicodeSegmentation;
    let ConsoleState::Open {
        input,
        cursor,
        completions,
        completion_idx,
        ..
    } = console_state
    else {
        return;
    };
    if completions.is_empty() {
        return;
    }
    let idx = completion_idx.unwrap_or(completions.len() - 1);
    let Some(cand) = completions.get(idx).cloned() else {
        return;
    };

    // Find the start of the token under the cursor: walk back from
    // the cursor position past non-whitespace grapheme clusters,
    // treating `key=value` as one token (so a kv-value completion
    // replaces only the value portion).
    let cursor_byte = find_byte_index_of_grapheme(input, *cursor).unwrap_or(input.len());
    let before: Vec<&str> = input[..cursor_byte].graphemes(true).collect();
    let mut start_g = before.len();
    while start_g > 0 && !before[start_g - 1].chars().all(|c| c.is_whitespace()) {
        start_g -= 1;
    }
    // If the token contains an `=`, and we're completing a kv-value,
    // the replacement starts *after* the `=`.
    let token: String = before[start_g..].concat();
    let is_kv_value_replace = matches!(token.find('='), Some(pos) if pos > 0);
    let replace_from = if is_kv_value_replace {
        let eq_pos = token.find('=').expect("guarded by is_kv_value_replace");
        let graph_before_eq = token[..eq_pos].graphemes(true).count();
        start_g + graph_before_eq + 1
    } else {
        start_g
    };

    // Delete graphemes from replace_from..cursor, then insert the
    // candidate text at replace_from.
    let replace_from_byte =
        find_byte_index_of_grapheme(input, replace_from).unwrap_or(input.len());
    input.replace_range(replace_from_byte..cursor_byte, &cand.text);
    *cursor = replace_from + count_grapheme_clusters(&cand.text);

    // Trailing space rule: append only when the completion closes a
    // positional / command-name / kv-key (i.e. the next logical
    // thing is a *new* token). A kv-value replacement never gets a
    // trailing space — the user may still be typing a quoted value
    // or an adjacent kv pair directly. A kv-key replacement (text
    // ending in `=`) also gets no space — the value comes next.
    let wants_trailing_space = !is_kv_value_replace && !cand.text.ends_with('=');
    if wants_trailing_space {
        let cursor_byte_after =
            find_byte_index_of_grapheme(input, *cursor).unwrap_or(input.len());
        let next_is_ws = input[cursor_byte_after..]
            .chars()
            .next()
            .map(|c| c.is_whitespace())
            .unwrap_or(true);
        if !next_is_ws {
            input.insert_str(cursor_byte_after, " ");
            *cursor += 1;
        } else if cursor_byte_after == input.len() {
            input.push(' ');
            *cursor += 1;
        }
    }
}

/// Parse and execute a console line. Drains deferred modal handoffs
/// (`open_label_edit`, `open_color_picker`), custom mutation apply
/// requests (`run_mutation`, needs tree access), binding overlay
/// updates (`bind_mutation` / `unbind_mutation`, need
/// `ResolvedKeybinds` access), and alias writes (`set_alias`).
/// Appends the result to the scrollback; rebuilds the scene on any
/// document mutation.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn execute_console_line(
    line: &str,
    console_state: &mut ConsoleState,
    label_edit_state: &mut LabelEditState,
    color_picker_state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    if line.trim().is_empty() {
        return;
    }
    let (cmd, args) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => return,
        ParseResult::Unknown(ref head) => {
            push_scrollback_error(
                console_state,
                format!("unknown command: {}", head),
            );
            return;
        }
    };
    let cmd: &'static Command = cmd;
    let mut effects = ConsoleEffects::new(doc);
    let result = (cmd.execute)(&Args::new(&args), &mut effects);
    let label_edit_req = effects.open_label_edit.take();
    let color_picker_req = effects.open_color_picker.take();
    let color_picker_standalone_req = effects.open_color_picker_standalone;
    let color_picker_close_req = effects.close_color_picker;
    let close_after = effects.close_console;
    let replace_doc = effects.replace_document.take();

    // Emit the command's result lines into the scrollback.
    match result {
        ExecResult::Ok(s) => {
            if !s.is_empty() {
                push_scrollback_output(console_state, s);
            }
        }
        ExecResult::Err(s) => push_scrollback_error(console_state, s),
        ExecResult::Lines(lines) => {
            for l in lines {
                push_scrollback_output(console_state, l);
            }
        }
    }

    // Wholesale document swap from `open` / `new`. Drop the cached
    // tree so `rebuild_all` rebuilds it fresh against the new map,
    // and clear any open modal-editor state so stale references
    // into the old document can't outlive the swap.
    if let Some(new_doc) = replace_doc {
        *doc = new_doc;
        *mindmap_tree = None;
        *label_edit_state = LabelEditState::Closed;
        *color_picker_state =
            crate::application::color_picker::ColorPickerState::Closed;
    }

    // Any successful command may have mutated the doc; rebuild.
    scene_cache.clear();
    rebuild_all(doc, mindmap_tree, app_scene, renderer);

    if let Some(er) = label_edit_req {
        open_label_edit(&er, doc, label_edit_state, app_scene, renderer);
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if let Some(target) = color_picker_req {
        open_color_picker_contextual(target, doc, color_picker_state, app_scene, renderer);
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if color_picker_standalone_req {
        open_color_picker_standalone(doc, color_picker_state, app_scene, renderer);
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if color_picker_close_req {
        close_color_picker_standalone(
            color_picker_state,
            doc,
            mindmap_tree,
            app_scene,
            renderer,
        );
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if close_after {
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn push_scrollback_output(state: &mut ConsoleState, text: String) {
    if let ConsoleState::Open { scrollback, .. } = state {
        scrollback.push(ConsoleLine::Output(text));
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn push_scrollback_error(state: &mut ConsoleState, text: String) {
    if let ConsoleState::Open { scrollback, .. } = state {
        scrollback.push(ConsoleLine::Error(text));
    }
}

/// Persist the document to its bound `file_path`, clear the dirty
/// flag, and surface the outcome — to the console scrollback when
/// open, and always to the log. Used by the `Ctrl+S` keybind. When
/// no path is bound, surfaces a hint pointing the user at `save
/// <path>` from the console; the dirty flag is left untouched.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn save_document_to_bound_path(
    doc: &mut MindMapDocument,
    console_state: &mut ConsoleState,
) {
    let path = match doc.file_path.clone() {
        Some(p) => p,
        None => {
            let msg = "no file path bound; use `save <path>` to choose one".to_string();
            log::warn!("{}", msg);
            push_scrollback_error(console_state, msg);
            return;
        }
    };
    match baumhard::mindmap::loader::save_to_file(
        std::path::Path::new(&path),
        &doc.mindmap,
    ) {
        Ok(()) => {
            doc.dirty = false;
            let msg = format!("saved to {}", path);
            log::info!("{}", msg);
            push_scrollback_output(console_state, msg);
        }
        Err(e) => {
            log::error!("{}", e);
            push_scrollback_error(console_state, e);
        }
    }
}

/// Build the console overlay geometry from the current state and
/// push it to the renderer. Called whenever the console opens, the
/// input changes, or scrollback / completions update.
#[cfg(not(target_arch = "wasm32"))]
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

/// Load persisted console history from `$XDG_STATE_HOME/mandala/history`
/// (or `$HOME/.local/state/mandala/history`). Returns an empty vec
/// on any failure — history is best-effort and must never take the
/// app down.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn load_console_history() -> Vec<String> {
    let path = match console_history_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<String> = contents
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();
    if out.len() > MAX_HISTORY {
        let drop = out.len() - MAX_HISTORY;
        out.drain(..drop);
    }
    out
}

/// Write the current history to disk. Best-effort — logs and moves
/// on if the directory can't be created or the file can't be written.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn save_console_history(history: &[String]) {
    let path = match console_history_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!("console history: create dir {}: {}", parent.display(), e);
            return;
        }
    }
    let start = history.len().saturating_sub(MAX_HISTORY);
    let body: String = history[start..].join("\n") + "\n";
    if let Err(e) = std::fs::write(&path, body) {
        log::warn!("console history: write {}: {}", path.display(), e);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn console_history_path() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        if !xdg.is_empty() {
            let mut p = PathBuf::from(xdg);
            p.push("mandala");
            p.push("history");
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            let mut p = PathBuf::from(home);
            p.push(".local");
            p.push("state");
            p.push("mandala");
            p.push("history");
            return Some(p);
        }
    }
    None
}

