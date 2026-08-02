// SPDX-License-Identifier: MPL-2.0

//! Grapheme-safe line editing via baumhard::util::grapheme_chad.
//!
//! The `ConsoleState::cursor` is a grapheme-cluster index, not a
//! byte offset. These tests lock in the invariant that cursor-
//! manipulating operations stay correct across multi-byte and
//! multi-codepoint characters — CODE_CONVENTIONS §2.

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
fn test_grapheme_insert_advances_cursor_by_cluster_delta() {
    // Simulate a single console payload insertion via the counted
    // grapheme_chad helper directly.
    use baumhard::util::grapheme_chad::{count_grapheme_clusters, insert_str_at_grapheme_counted};
    let mut input = String::new();
    let mut cursor = 0usize;
    cursor += insert_str_at_grapheme_counted(&mut input, cursor, "abc");
    assert_eq!(input, "abc");
    assert_eq!(cursor, 3);
    assert_eq!(count_grapheme_clusters(&input), 3);
}

#[test]
fn test_grapheme_combining_mark_insert_does_not_overadvance_cursor() {
    use baumhard::util::grapheme_chad::{count_grapheme_clusters, insert_str_at_grapheme_counted};
    let mut input = String::from("e");
    let mut cursor = 1usize;
    cursor += insert_str_at_grapheme_counted(&mut input, cursor, "\u{0301}");
    assert_eq!(input, "e\u{0301}");
    assert_eq!(cursor, count_grapheme_clusters(&input));
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
