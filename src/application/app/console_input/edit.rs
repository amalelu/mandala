//! Pure per-action edit helpers for the console line editor. Each
//! takes `&mut ConsoleState` and returns whether the input text
//! changed, so the dispatcher knows to recompute completions.
//!
//! Extracted from `dispatch.rs` so the line-edit primitives are
//! testable without a `Renderer` or `AppScene`. The dispatcher is
//! still the single owner of the rebuild side-effect; these helpers
//! never touch the renderer.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::util::grapheme_chad::{
    count_grapheme_clusters, delete_front_unicode, delete_grapheme_at,
    find_byte_index_of_grapheme, insert_str_at_grapheme,
};

use crate::application::console::ConsoleState;

/// Outcome of a pure edit. The dispatcher always rebuilds the
/// overlay; only `InputChanged` triggers a completion recompute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EditOutcome {
    Unchanged,
    InputChanged,
}

impl EditOutcome {
    pub(super) fn input_changed(self) -> bool {
        matches!(self, EditOutcome::InputChanged)
    }
}

pub(super) fn clear_line(state: &mut ConsoleState) -> EditOutcome {
    let ConsoleState::Open { input, cursor, history_idx, .. } = state else {
        return EditOutcome::Unchanged;
    };
    if input.is_empty() && *cursor == 0 && history_idx.is_none() {
        return EditOutcome::Unchanged;
    }
    input.clear();
    *cursor = 0;
    *history_idx = None;
    EditOutcome::InputChanged
}

pub(super) fn jump_to_start(state: &mut ConsoleState) -> EditOutcome {
    if let ConsoleState::Open { cursor, .. } = state {
        *cursor = 0;
    }
    EditOutcome::Unchanged
}

pub(super) fn jump_to_end(state: &mut ConsoleState) -> EditOutcome {
    if let ConsoleState::Open { cursor, input, .. } = state {
        *cursor = count_grapheme_clusters(input);
    }
    EditOutcome::Unchanged
}

pub(super) fn kill_to_start(state: &mut ConsoleState) -> EditOutcome {
    let ConsoleState::Open { input, cursor, .. } = state else {
        return EditOutcome::Unchanged;
    };
    if *cursor == 0 {
        return EditOutcome::Unchanged;
    }
    delete_front_unicode(input, *cursor);
    *cursor = 0;
    EditOutcome::InputChanged
}

pub(super) fn kill_word(state: &mut ConsoleState) -> EditOutcome {
    use unicode_segmentation::UnicodeSegmentation;
    let ConsoleState::Open { input, cursor, .. } = state else {
        return EditOutcome::Unchanged;
    };
    let end_g = *cursor;
    if end_g == 0 {
        return EditOutcome::Unchanged;
    }
    let prefix_bytes = find_byte_index_of_grapheme(input, end_g).unwrap_or(input.len());
    let clusters: Vec<&str> = input[..prefix_bytes].graphemes(true).collect();
    let mut start_g = clusters.len();
    while start_g > 0 && clusters[start_g - 1].chars().all(|c| c.is_whitespace()) {
        start_g -= 1;
    }
    while start_g > 0 && !clusters[start_g - 1].chars().all(|c| c.is_whitespace()) {
        start_g -= 1;
    }
    if start_g == end_g {
        return EditOutcome::Unchanged;
    }
    for _ in 0..(end_g - start_g) {
        delete_grapheme_at(input, start_g);
    }
    *cursor = start_g;
    EditOutcome::InputChanged
}

pub(super) fn cursor_left(state: &mut ConsoleState) -> EditOutcome {
    if let ConsoleState::Open { cursor, .. } = state {
        if *cursor > 0 {
            *cursor -= 1;
        }
    }
    EditOutcome::Unchanged
}

pub(super) fn cursor_right(state: &mut ConsoleState) -> EditOutcome {
    if let ConsoleState::Open { cursor, input, .. } = state {
        let max = count_grapheme_clusters(input);
        if *cursor < max {
            *cursor += 1;
        }
    }
    EditOutcome::Unchanged
}

pub(super) fn cursor_home(state: &mut ConsoleState) -> EditOutcome {
    if let ConsoleState::Open { cursor, .. } = state {
        *cursor = 0;
    }
    EditOutcome::Unchanged
}

pub(super) fn cursor_end(state: &mut ConsoleState) -> EditOutcome {
    if let ConsoleState::Open { cursor, input, .. } = state {
        *cursor = count_grapheme_clusters(input);
    }
    EditOutcome::Unchanged
}

pub(super) fn delete_back(state: &mut ConsoleState) -> EditOutcome {
    let ConsoleState::Open { input, cursor, .. } = state else {
        return EditOutcome::Unchanged;
    };
    if *cursor == 0 {
        return EditOutcome::Unchanged;
    }
    *cursor -= 1;
    delete_grapheme_at(input, *cursor);
    EditOutcome::InputChanged
}

pub(super) fn delete_forward(state: &mut ConsoleState) -> EditOutcome {
    let ConsoleState::Open { input, cursor, .. } = state else {
        return EditOutcome::Unchanged;
    };
    if *cursor >= count_grapheme_clusters(input) {
        return EditOutcome::Unchanged;
    }
    delete_grapheme_at(input, *cursor);
    EditOutcome::InputChanged
}

pub(super) fn insert_space(state: &mut ConsoleState) -> EditOutcome {
    insert_text(state, " ")
}

/// Insert `text` at the cursor as a single edit. Skips control
/// characters (winit can deliver Tab / Enter as Character — those
/// have named-action bindings and shouldn't show up as literal
/// characters).
pub(super) fn insert_text(state: &mut ConsoleState, text: &str) -> EditOutcome {
    let ConsoleState::Open { input, cursor, history_idx, .. } = state else {
        return EditOutcome::Unchanged;
    };
    let mut changed = false;
    for ch in text.chars() {
        if ch.is_control() {
            continue;
        }
        let mut buf = [0u8; 4];
        let encoded = ch.encode_utf8(&mut buf);
        insert_str_at_grapheme(input, *cursor, encoded);
        *cursor += 1;
        changed = true;
    }
    if changed {
        *history_idx = None;
        EditOutcome::InputChanged
    } else {
        EditOutcome::Unchanged
    }
}

/// Walk history backward (toward older entries). Caller is
/// responsible for trying popup navigation first.
pub(super) fn history_walk_back(state: &mut ConsoleState) -> EditOutcome {
    let ConsoleState::Open { input, cursor, history, history_idx, .. } = state else {
        return EditOutcome::Unchanged;
    };
    if history.is_empty() {
        return EditOutcome::Unchanged;
    }
    let next = match history_idx {
        None => history.len() - 1,
        Some(0) => 0,
        Some(i) => *i - 1,
    };
    *history_idx = Some(next);
    *input = history[next].clone();
    *cursor = count_grapheme_clusters(input);
    EditOutcome::InputChanged
}

/// Walk history forward (toward newer entries; past the newest
/// resets to a fresh empty line). Caller is responsible for trying
/// popup navigation first.
pub(super) fn history_walk_forward(state: &mut ConsoleState) -> EditOutcome {
    let ConsoleState::Open { input, cursor, history, history_idx, .. } = state else {
        return EditOutcome::Unchanged;
    };
    match history_idx {
        Some(i) if *i + 1 < history.len() => {
            let next = *i + 1;
            *history_idx = Some(next);
            *input = history[next].clone();
            *cursor = count_grapheme_clusters(input);
            EditOutcome::InputChanged
        }
        Some(_) => {
            *history_idx = None;
            input.clear();
            *cursor = 0;
            EditOutcome::InputChanged
        }
        None => EditOutcome::Unchanged,
    }
}

/// Dismiss an open completion popup without closing the console.
/// Returns `true` if a popup was present and was cleared.
pub(super) fn dismiss_popup(state: &mut ConsoleState) -> bool {
    let ConsoleState::Open { completions, completion_idx, .. } = state else {
        return false;
    };
    if completions.is_empty() {
        return false;
    }
    completions.clear();
    *completion_idx = None;
    true
}

#[cfg(test)]
mod tests;
