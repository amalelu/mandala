//! Completion-popup helpers: recompute against the current input,
//! navigate the popup row by row, and accept the highlighted
//! candidate into the input buffer.

use crate::application::console::completion::complete as complete_console;
use crate::application::console::{ConsoleContext, ConsoleState};
use crate::application::document::MindMapDocument;

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

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;
