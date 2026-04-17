//! Unit tests for the pure edit helpers. No renderer / scene
//! involvement — every test constructs a `ConsoleState::Open` with
//! known input, calls a helper, and asserts on the resulting fields.

use super::*;
use crate::application::console::ConsoleState;

fn open_with(input: &str, cursor: usize) -> ConsoleState {
    ConsoleState::Open {
        input: input.to_string(),
        cursor,
        history: Vec::new(),
        history_idx: None,
        scrollback: Vec::new(),
        completions: Vec::new(),
        completion_idx: None,
    }
}

fn open_with_history(input: &str, cursor: usize, history: Vec<String>) -> ConsoleState {
    ConsoleState::Open {
        input: input.to_string(),
        cursor,
        history,
        history_idx: None,
        scrollback: Vec::new(),
        completions: Vec::new(),
        completion_idx: None,
    }
}

fn input_of(state: &ConsoleState) -> &str {
    match state {
        ConsoleState::Open { input, .. } => input,
        _ => panic!("expected open"),
    }
}

fn cursor_of(state: &ConsoleState) -> usize {
    match state {
        ConsoleState::Open { cursor, .. } => *cursor,
        _ => panic!("expected open"),
    }
}

fn history_idx_of(state: &ConsoleState) -> Option<usize> {
    match state {
        ConsoleState::Open { history_idx, .. } => *history_idx,
        _ => panic!("expected open"),
    }
}

#[test]
fn closed_state_is_a_noop() {
    let mut s = ConsoleState::Closed;
    assert_eq!(clear_line(&mut s), EditOutcome::Unchanged);
    assert_eq!(jump_to_start(&mut s), EditOutcome::Unchanged);
    assert_eq!(insert_text(&mut s, "hello"), EditOutcome::Unchanged);
    assert!(matches!(s, ConsoleState::Closed));
}

#[test]
fn clear_line_clears_input_and_cursor() {
    let mut s = open_with("hello", 3);
    assert_eq!(clear_line(&mut s), EditOutcome::InputChanged);
    assert_eq!(input_of(&s), "");
    assert_eq!(cursor_of(&s), 0);
}

#[test]
fn clear_line_on_empty_input_is_noop() {
    let mut s = open_with("", 0);
    assert_eq!(clear_line(&mut s), EditOutcome::Unchanged);
}

#[test]
fn jump_to_start_moves_cursor_to_zero() {
    let mut s = open_with("hello", 3);
    assert_eq!(jump_to_start(&mut s), EditOutcome::Unchanged);
    assert_eq!(cursor_of(&s), 0);
}

#[test]
fn jump_to_end_moves_cursor_to_grapheme_count() {
    let mut s = open_with("hello", 0);
    assert_eq!(jump_to_end(&mut s), EditOutcome::Unchanged);
    assert_eq!(cursor_of(&s), 5);
}

#[test]
fn kill_to_start_deletes_prefix() {
    let mut s = open_with("hello world", 6);
    assert_eq!(kill_to_start(&mut s), EditOutcome::InputChanged);
    assert_eq!(input_of(&s), "world");
    assert_eq!(cursor_of(&s), 0);
}

#[test]
fn kill_to_start_at_zero_is_noop() {
    let mut s = open_with("hello", 0);
    assert_eq!(kill_to_start(&mut s), EditOutcome::Unchanged);
    assert_eq!(input_of(&s), "hello");
}

#[test]
fn kill_word_deletes_trailing_word_and_whitespace() {
    let mut s = open_with("foo bar", 7);
    assert_eq!(kill_word(&mut s), EditOutcome::InputChanged);
    assert_eq!(input_of(&s), "foo ");
    assert_eq!(cursor_of(&s), 4);
}

#[test]
fn kill_word_with_trailing_space_kills_word_before_space() {
    let mut s = open_with("foo bar ", 8);
    assert_eq!(kill_word(&mut s), EditOutcome::InputChanged);
    assert_eq!(input_of(&s), "foo ");
}

#[test]
fn kill_word_at_zero_is_noop() {
    let mut s = open_with("foo", 0);
    assert_eq!(kill_word(&mut s), EditOutcome::Unchanged);
}

#[test]
fn cursor_left_decrements_until_zero() {
    let mut s = open_with("hi", 2);
    cursor_left(&mut s);
    assert_eq!(cursor_of(&s), 1);
    cursor_left(&mut s);
    assert_eq!(cursor_of(&s), 0);
    cursor_left(&mut s);
    assert_eq!(cursor_of(&s), 0);
}

#[test]
fn cursor_right_increments_until_end() {
    let mut s = open_with("hi", 0);
    cursor_right(&mut s);
    assert_eq!(cursor_of(&s), 1);
    cursor_right(&mut s);
    assert_eq!(cursor_of(&s), 2);
    cursor_right(&mut s);
    assert_eq!(cursor_of(&s), 2);
}

#[test]
fn delete_back_removes_grapheme_before_cursor() {
    let mut s = open_with("abc", 2);
    assert_eq!(delete_back(&mut s), EditOutcome::InputChanged);
    assert_eq!(input_of(&s), "ac");
    assert_eq!(cursor_of(&s), 1);
}

#[test]
fn delete_back_at_zero_is_noop() {
    let mut s = open_with("abc", 0);
    assert_eq!(delete_back(&mut s), EditOutcome::Unchanged);
    assert_eq!(input_of(&s), "abc");
}

#[test]
fn delete_forward_removes_grapheme_at_cursor() {
    let mut s = open_with("abc", 1);
    assert_eq!(delete_forward(&mut s), EditOutcome::InputChanged);
    assert_eq!(input_of(&s), "ac");
    assert_eq!(cursor_of(&s), 1);
}

#[test]
fn delete_forward_at_end_is_noop() {
    let mut s = open_with("abc", 3);
    assert_eq!(delete_forward(&mut s), EditOutcome::Unchanged);
    assert_eq!(input_of(&s), "abc");
}

#[test]
fn insert_space_inserts_a_space_character() {
    let mut s = open_with("ab", 1);
    assert_eq!(insert_space(&mut s), EditOutcome::InputChanged);
    assert_eq!(input_of(&s), "a b");
    assert_eq!(cursor_of(&s), 2);
}

#[test]
fn insert_text_skips_control_chars() {
    let mut s = open_with("", 0);
    assert_eq!(insert_text(&mut s, "a\tb"), EditOutcome::InputChanged);
    assert_eq!(input_of(&s), "ab");
    assert_eq!(cursor_of(&s), 2);
}

#[test]
fn insert_text_only_control_chars_is_unchanged() {
    let mut s = open_with("", 0);
    assert_eq!(insert_text(&mut s, "\t\r\n"), EditOutcome::Unchanged);
    assert_eq!(input_of(&s), "");
}

#[test]
fn insert_text_clears_history_idx() {
    let mut s = open_with_history("", 0, vec!["prev".into()]);
    if let ConsoleState::Open { history_idx, .. } = &mut s {
        *history_idx = Some(0);
    }
    insert_text(&mut s, "x");
    assert_eq!(history_idx_of(&s), None);
}

#[test]
fn history_walk_back_with_empty_history_is_noop() {
    let mut s = open_with("", 0);
    assert_eq!(history_walk_back(&mut s), EditOutcome::Unchanged);
    assert_eq!(history_idx_of(&s), None);
}

#[test]
fn history_walk_back_seeds_to_newest() {
    let mut s = open_with_history("", 0, vec!["a".into(), "b".into(), "c".into()]);
    assert_eq!(history_walk_back(&mut s), EditOutcome::InputChanged);
    assert_eq!(input_of(&s), "c");
    assert_eq!(history_idx_of(&s), Some(2));
}

#[test]
fn history_walk_back_steps_to_older_entries() {
    let mut s = open_with_history("", 0, vec!["a".into(), "b".into(), "c".into()]);
    history_walk_back(&mut s);
    history_walk_back(&mut s);
    assert_eq!(input_of(&s), "b");
    assert_eq!(history_idx_of(&s), Some(1));
}

#[test]
fn history_walk_back_clamps_at_oldest() {
    let mut s = open_with_history("", 0, vec!["a".into(), "b".into()]);
    history_walk_back(&mut s);
    history_walk_back(&mut s);
    history_walk_back(&mut s);
    assert_eq!(input_of(&s), "a");
    assert_eq!(history_idx_of(&s), Some(0));
}

#[test]
fn history_walk_forward_steps_to_newer_then_resets() {
    let mut s = open_with_history("", 0, vec!["a".into(), "b".into()]);
    history_walk_back(&mut s);
    history_walk_back(&mut s);
    assert_eq!(history_idx_of(&s), Some(0));
    history_walk_forward(&mut s);
    assert_eq!(input_of(&s), "b");
    assert_eq!(history_idx_of(&s), Some(1));
    // Past the newest entry resets to a fresh empty line.
    assert_eq!(history_walk_forward(&mut s), EditOutcome::InputChanged);
    assert_eq!(input_of(&s), "");
    assert_eq!(history_idx_of(&s), None);
}

#[test]
fn history_walk_forward_with_no_idx_is_noop() {
    let mut s = open_with_history("", 0, vec!["a".into()]);
    assert_eq!(history_walk_forward(&mut s), EditOutcome::Unchanged);
}

#[test]
fn dismiss_popup_returns_false_when_empty() {
    let mut s = open_with("", 0);
    assert!(!dismiss_popup(&mut s));
}

#[test]
fn dismiss_popup_clears_completions_when_present() {
    use crate::application::console::completion::Completion;
    let mut s = open_with("", 0);
    if let ConsoleState::Open { completions, completion_idx, .. } = &mut s {
        completions.push(Completion {
            text: "help".into(),
            display: "help".into(),
            hint: None,
        });
        *completion_idx = Some(0);
    }
    assert!(dismiss_popup(&mut s));
    if let ConsoleState::Open { completions, completion_idx, .. } = &s {
        assert!(completions.is_empty());
        assert_eq!(*completion_idx, None);
    }
}

#[test]
fn grapheme_aware_delete_back_handles_multibyte() {
    // "héllo" — the 'é' is a single grapheme but multiple bytes.
    let mut s = open_with("héllo", 2);
    delete_back(&mut s);
    assert_eq!(input_of(&s), "hllo");
    assert_eq!(cursor_of(&s), 1);
}

#[test]
fn grapheme_aware_jump_to_end_with_multibyte() {
    let mut s = open_with("héllo", 0);
    jump_to_end(&mut s);
    // 5 graphemes regardless of byte length.
    assert_eq!(cursor_of(&s), 5);
}
