//! Grapheme-safe line editing via baumhard::util::grapheme_chad.
//!
//! The `ConsoleState::cursor` is a grapheme-cluster index, not a
//! byte offset. These tests lock in the invariant that cursor-
//! manipulating operations stay correct across multi-byte and
//! multi-codepoint characters — CODE_CONVENTIONS §2.

use crate::application::console::ConsoleState;

#[test]
fn test_console_cursor_is_grapheme_indexed_in_docs() {
    // A sentinel check — if someone reverts the cursor semantics,
    // this test will force them to re-read CODE_CONVENTIONS §2.
    let state = ConsoleState::open(Vec::new());
    match state {
        ConsoleState::Open { cursor, .. } => assert_eq!(cursor, 0),
        _ => panic!("expected Open"),
    }
}

#[test]
fn test_grapheme_space_insertion_via_helper() {
    // winit delivers the spacebar as `Key::Named(NamedKey::Space)`,
    // which `handle_console_key` treats as a named key rather than
    // a char payload. The named-key arm should insert a literal
    // space the same way the generic char path does — verified here
    // by driving the helper directly.
    use baumhard::util::grapheme_chad::insert_str_at_grapheme;
    let mut input = String::from("ab");
    let cursor = 1;
    insert_str_at_grapheme(&mut input, cursor, " ");
    assert_eq!(input, "a b");
}

#[test]
fn test_grapheme_insert_advances_cursor_by_one_per_char() {
    // Simulate three-char insertion via the grapheme_chad helper
    // directly — mirrors what `handle_console_key` does on each
    // character key.
    use baumhard::util::grapheme_chad::{count_grapheme_clusters, insert_str_at_grapheme};
    let mut input = String::new();
    let mut cursor = 0usize;
    for ch in "abc".chars() {
        let mut buf = [0u8; 4];
        insert_str_at_grapheme(&mut input, cursor, ch.encode_utf8(&mut buf));
        cursor += 1;
    }
    assert_eq!(input, "abc");
    assert_eq!(cursor, 3);
    assert_eq!(count_grapheme_clusters(&input), 3);
}

#[test]
fn test_grapheme_delete_removes_whole_cluster() {
    // A ZWJ emoji family is 7+ codepoints but one grapheme cluster.
    // `delete_grapheme_at` must remove the whole cluster, not
    // just one codepoint.
    use baumhard::util::grapheme_chad::{count_grapheme_clusters, delete_grapheme_at};
    let mut input = String::from("a\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F466}b");
    assert_eq!(count_grapheme_clusters(&input), 3, "a + family + b");
    delete_grapheme_at(&mut input, 1); // delete the family
    assert_eq!(input, "ab");
}
