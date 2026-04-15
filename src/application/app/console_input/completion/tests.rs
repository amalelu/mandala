//! Acceptance tests for [`accept_console_completion`] — the
//! console-popup completion helper that distinguishes positional
//! completions (append trailing space) from kv-key / kv-value
//! completions (no trailing space).

use super::*;

use crate::application::console::completion::Completion;
use crate::application::console::ConsoleState;

fn open_state(input: &str, cursor: usize, candidates: &[&str]) -> ConsoleState {
    ConsoleState::Open {
        input: input.to_string(),
        cursor,
        history: Vec::new(),
        history_idx: None,
        scrollback: Vec::new(),
        completions: candidates
            .iter()
            .map(|c| Completion {
                text: c.to_string(),
                display: c.to_string(),
                hint: None,
            })
            .collect(),
        completion_idx: if candidates.is_empty() { None } else { Some(0) },
    }
}

/// Accepting a command-name completion replaces the partial
/// prefix and appends a trailing space so the user can type
/// the next token immediately.
#[test]
fn test_accept_completion_positional_appends_space() {
    let mut state = open_state("co", 2, &["color"]);
    accept_console_completion(&mut state);
    if let ConsoleState::Open { input, cursor, .. } = state {
        assert_eq!(input, "color ");
        assert_eq!(cursor, 6);
    } else {
        panic!("state closed");
    }
}

/// Accepting a kv-key completion (text ends in `=`) adds no
/// trailing space — the value comes next.
#[test]
fn test_accept_completion_kv_key_no_trailing_space() {
    let mut state = open_state("color b", 7, &["bg="]);
    accept_console_completion(&mut state);
    if let ConsoleState::Open { input, cursor, .. } = state {
        assert_eq!(input, "color bg=");
        assert_eq!(cursor, 9);
    } else {
        panic!("state closed");
    }
}

/// Accepting a kv-value completion replaces only the value slot
/// (not the key=) and adds no trailing space.
#[test]
fn test_accept_completion_kv_value_replaces_only_value_slot() {
    let mut state = open_state("color bg=ac", 11, &["accent"]);
    accept_console_completion(&mut state);
    if let ConsoleState::Open { input, cursor, .. } = state {
        assert_eq!(input, "color bg=accent");
        assert_eq!(cursor, 15);
    } else {
        panic!("state closed");
    }
}

/// Accepting a kv-value with no partial typed (cursor right
/// after `=`) inserts at the value slot and keeps the cursor
/// after the value — no trailing space.
#[test]
fn test_accept_completion_kv_value_empty_partial() {
    let mut state = open_state("color bg=", 9, &["accent"]);
    accept_console_completion(&mut state);
    if let ConsoleState::Open { input, cursor, .. } = state {
        assert_eq!(input, "color bg=accent");
        assert_eq!(cursor, 15);
    } else {
        panic!("state closed");
    }
}

/// Accepting when the popup is empty is a no-op.
#[test]
fn test_accept_completion_no_popup_is_noop() {
    let mut state = open_state("color bg=", 9, &[]);
    accept_console_completion(&mut state);
    if let ConsoleState::Open { input, cursor, .. } = state {
        assert_eq!(input, "color bg=");
        assert_eq!(cursor, 9);
    } else {
        panic!("state closed");
    }
}
