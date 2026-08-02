// SPDX-License-Identifier: MPL-2.0

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
    count_grapheme_clusters, delete_front_unicode, delete_grapheme_at, insert_str_at_grapheme_counted,
    prev_word_boundary_ws,
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

/// Direction + magnitude for a scrollback navigation step. Mapped
/// from the `Action` set in `dispatch::map_scroll_action`. The unit
/// "page" matches `MAX_CONSOLE_SCROLLBACK_ROWS` so PgUp/PgDn move
/// exactly one visible-window worth of lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScrollDirection {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Home,
    End,
}

/// Adjust `ConsoleState::Open.scroll_offset` per `direction`,
/// clamping against the maximum reachable offset (= scrollback
/// length minus the visible window size). `Home` jumps to the
/// oldest reachable line; `End` pins the bottom (offset = 0).
pub(super) fn adjust_scroll(state: &mut ConsoleState, direction: ScrollDirection) {
    use crate::application::renderer::MAX_CONSOLE_SCROLLBACK_ROWS;
    if let ConsoleState::Open {
        scrollback,
        scroll_offset,
        ..
    } = state
    {
        let max = scrollback.len().saturating_sub(MAX_CONSOLE_SCROLLBACK_ROWS);
        let new = match direction {
            ScrollDirection::LineUp => scroll_offset.saturating_add(1).min(max),
            ScrollDirection::LineDown => scroll_offset.saturating_sub(1),
            ScrollDirection::PageUp => scroll_offset.saturating_add(MAX_CONSOLE_SCROLLBACK_ROWS).min(max),
            ScrollDirection::PageDown => scroll_offset.saturating_sub(MAX_CONSOLE_SCROLLBACK_ROWS),
            ScrollDirection::Home => max,
            ScrollDirection::End => 0,
        };
        *scroll_offset = new;
    }
}

/// Adjust the scroll offset by an integer line delta returned from
/// the mousewheel-step accumulator. Positive delta scrolls up
/// (older lines into view), negative scrolls down. Same clamp as
/// [`adjust_scroll`] — clamped to `[0, scrollback.len() -
/// MAX_CONSOLE_SCROLLBACK_ROWS]`.
pub(super) fn scroll_by_lines(state: &mut ConsoleState, delta: i32) {
    use crate::application::renderer::MAX_CONSOLE_SCROLLBACK_ROWS;
    if let ConsoleState::Open {
        scrollback,
        scroll_offset,
        ..
    } = state
    {
        let max = scrollback.len().saturating_sub(MAX_CONSOLE_SCROLLBACK_ROWS);
        let signed = (*scroll_offset as i32).saturating_add(delta);
        let clamped = signed.max(0) as usize;
        *scroll_offset = clamped.min(max);
    }
}

/// Drain a wheel delta into integer line steps, carrying the
/// fractional remainder in `accum` for the next call. Native
/// mousewheel events arrive as fixed pixel amounts (or per-platform
/// line counts) that are rarely a clean multiple of one line; the
/// accumulator keeps slow scrolls (sub-line per tick) from getting
/// rounded to zero forever.
///
/// Pure function over `&mut f32` — extracted from the winit event
/// closure so it can be unit-tested directly. Tests cover the
/// fractional-carry, negative-delta, and large-delta paths.
///
/// `dy` is the wheel delta in lines (the caller's responsibility to
/// divide pixel deltas by a per-platform line height before
/// calling). Returns the integer line steps to apply this tick;
/// `accum` retains the leftover fractional residue.
pub(super) fn accumulate_wheel_lines(accum: &mut f32, dy: f32) -> i32 {
    if !dy.is_finite() {
        // Defensive: a bogus event must not poison the
        // accumulator. Drop the tick and reset.
        *accum = 0.0;
        return 0;
    }
    *accum += dy;
    let lines = accum.trunc() as i32;
    if lines != 0 {
        *accum -= lines as f32;
    }
    lines
}

pub(super) fn clear_line(state: &mut ConsoleState) -> EditOutcome {
    let ConsoleState::Open {
        input,
        cursor,
        history_idx,
        ..
    } = state
    else {
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
    let ConsoleState::Open { input, cursor, .. } = state else {
        return EditOutcome::Unchanged;
    };
    let end_g = *cursor;
    if end_g == 0 {
        return EditOutcome::Unchanged;
    }
    // Whitespace-delimited, not alphanumeric-run: `Ctrl+W` kills a
    // whole shell token, so `key=value` goes in one stroke.
    let start_g = prev_word_boundary_ws(input, end_g);
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
    let ConsoleState::Open {
        input,
        cursor,
        history_idx,
        ..
    } = state
    else {
        return EditOutcome::Unchanged;
    };
    let filtered: String = text.chars().filter(|ch| !ch.is_control()).collect();
    if !filtered.is_empty() {
        *cursor += insert_str_at_grapheme_counted(input, *cursor, &filtered);
        *history_idx = None;
        EditOutcome::InputChanged
    } else {
        EditOutcome::Unchanged
    }
}

/// Walk history backward (toward older entries). Caller is
/// responsible for trying popup navigation first.
pub(super) fn history_walk_back(state: &mut ConsoleState) -> EditOutcome {
    let ConsoleState::Open {
        input,
        cursor,
        history,
        history_idx,
        ..
    } = state
    else {
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
    let ConsoleState::Open {
        input,
        cursor,
        history,
        history_idx,
        ..
    } = state
    else {
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
    let ConsoleState::Open {
        completions,
        completion_idx,
        ..
    } = state
    else {
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
