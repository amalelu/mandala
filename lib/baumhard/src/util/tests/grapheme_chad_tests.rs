// SPDX-License-Identifier: MPL-2.0

use lazy_static::lazy_static;

use crate::util::grapheme_chad::{
    count_grapheme_clusters, count_number_lines, delete_back_unicode, delete_front_unicode,
    delete_grapheme_at, find_byte_index_of_grapheme, find_nth_line_grapheme_range,
    first_non_whitespace_grapheme, grapheme_display_width, insert_new_lines, insert_spaces,
    insert_str_at_grapheme, insert_str_at_grapheme_counted, join_graphemes, line_bounds_at,
    prev_word_boundary_ws, push_spaces, replace_graphemes_until_newline, scalar_display_width,
    slice_to_newline, split_graphemes_owned, split_off_graphemes, take_graphemes, token_start_ws,
    truncate_to_display_width, word_left, word_right, LineReplacement,
};

lazy_static! {
    pub static ref SPLIT_GRAPHEMES_TEST: Vec<(&'static str, usize, &'static str, &'static str)> = vec![
        ("abcd🍕1234", 4, "abcd", "🍕1234"),
        ("abcd🍕1234", 5, "abcd🍕", "1234"),
        ("abcd🙏🏻1234", 5, "abcd🙏🏻", "1234"),
        ("abcd🙏🏻1234", 4, "abcd", "🙏🏻1234"),
        ("🙏🏻🙏🏻🙏🏻🙏🏻1234", 4, "🙏🏻🙏🏻🙏🏻🙏🏻", "1234"),
        (
            "🙏🏻🙏🏻🙏🏻🙏🏻🍕🍕🍕🍕",
            3,
            "🙏🏻🙏🏻🙏🏻",
            "🙏🏻🍕🍕🍕🍕"
        ),
        ("🏳️‍🌈👩‍👧‍👦👯‍♂️👰‍♂️👨‍🚀", 3, "🏳️‍🌈👩‍👧‍👦👯‍♂️", "👰‍♂️👨‍🚀"),
        ("", 2, "", ""),
    ];
    pub static ref REMOVE_PREFIX_TESTS: Vec<(&'static str, usize, &'static str)> = vec![
        // n = 0 must be a no-op, never eating a leading grapheme — regardless of
        // whether that grapheme is ASCII, a multi-scalar emoji, or a ZWJ cluster.
        ("abcd", 0, "abcd"),
        ("🙏🏻abcd", 0, "🙏🏻abcd"),
        ("👨‍👩‍👧abcd", 0, "👨‍👩‍👧abcd"),
        ("", 0, ""),
        ("abcd🍕1234", 3, "d🍕1234"),
        ("\n\nabcd🍕1234", 3, "bcd🍕1234"),
        ("\n\n bcd🍕1234", 3, "bcd🍕1234"),
        ("abcd🍕1234", 4, "🍕1234"),
        ("abcd🍕1234", 5, "1234"),
        ("abcd🍕1234", 6, "234"),
        ("abcd🍕1234", 7, "34"),
        ("abcd🍕1234", 8, "4"),
        ("abcd🍕1234", 9, ""),
        ("abcd🍕1234", 10, ""),
        ("🙏🏻🙏🏻🙏🏻", 1, "🙏🏻🙏🏻"),
        ("🙏🏻🙏🏻🙏🏻", 2, "🙏🏻"),
        ("🍕🍕🍕🍕🍕🍕🍕🍕🍕🍕", 2, "🍕🍕🍕🍕🍕🍕🍕🍕"),
    ];
    pub static ref TRUNCATE_TESTS: Vec<(&'static str, usize, &'static str)> = vec![
        // n = 0 is a no-op on the tail too — the symmetric partner of the
        // delete_front_unicode(_, 0) no-op above.
        ("abcd", 0, "abcd"),
        ("abcd🙏🏻", 0, "abcd🙏🏻"),
        ("", 0, ""),
        ("abcd🍕1234", 3, "abcd🍕1"),
        ("abcd🍕1234", 4, "abcd🍕"),
        ("abcd🍕1234", 5, "abcd"),
        ("abcd🍕1234", 6, "abc"),
        ("abcd🍕1234", 7, "ab"),
        ("abcd🍕1234", 8, "a"),
        ("abcd🍕1234", 9, ""),
        ("abcd🍕1234", 10, ""),
        ("🙏🏻🙏🏻🙏🏻", 1, "🙏🏻🙏🏻"),
        ("🙏🏻🙏🏻🙏🏻", 2, "🙏🏻"),
        ("🍕🍕🍕🍕🍕🍕🍕🍕🍕🍕", 2, "🍕🍕🍕🍕🍕🍕🍕🍕"),
        ("🍕🍕🍕🍕🍕🍕🍕🍕🍕🍕\n", 2, "🍕🍕🍕🍕🍕🍕🍕🍕🍕"),
        (
            "🍕🍕🍕🍕🍕🍕🍕🍕🍕🍕\n\n\n\n\n\n\n\n\n\n\n",
            12,
            "🍕🍕🍕🍕🍕🍕🍕🍕🍕"
        ),
    ];
    pub static ref COUNT_LINES_TEST: Vec<(&'static str, usize)> = vec![
        ("abcd🍕1234", 1),
        ("abcd\n", 2),
        ("abcd\n\n", 3),
        ("\n\n", 3),
        ("", 1),
        ("\n", 2),
        ("\n\n\n", 4),
        ("\nho\nhi\nhello", 4),
        ("\n🙏🏻\n🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻\n🙏🏻\n🙏🏻\n\n\n\n\n\n\n\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n\n\n\n\n\n🙏🏻🙏🏻🙏🏻\n\n\n\n", 24),
        ("\nho\nhi\nhello🙏🏻🙏🏻🙏🏻\n\n\n🙏🏻\n\n\n🙏🏻\n\n\n🙏🏻\n\n\n\n\n🙏🏻\n\n\n\n🙏🏻\n\n", 24),
    ];

    pub static ref NTH_LINE_GRAPHEME_INDICES_TEST: Vec<(&'static str, usize, Option<(usize, usize)>)> = vec![
        ("\n", 0, Some((0, 0))),
        ("", 0, None),
        ("a", 0, Some((0,1))),
        ("a\n", 1, None),
        ("a\n", 0, Some((0,1))),
        ("", 1, None),
        ("Hello\nxxxxxxxxxxqqqqqqqqqqxxxxxxxxxxqqqqqqqqqq\n", 1, Some((6, 46))),
        ("\n🙏🏻\n🙏🏻\n", 2, Some((3, 4))),
        ("\n🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n", 2, Some((3, 8))),
        ("\n🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n", 2, Some((4, 9))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 0, Some((0, 10))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 1, Some((11, 21))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 2, Some((22, 32))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 1, Some((11, 11))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\na\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 1, Some((11, 12))),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 2, Some((12, 22))),
        // UAX #29 fuses `\r\n` into one cluster, so a `g == "\n"`
        // test walks straight past a Windows line ending and folds
        // two lines into one. `GlyphMatrix::place_in` sizes its
        // buffer with `count_number_lines`, which counts `\n`
        // *characters*, so the two have to agree on CRLF or every
        // row after a CRLF-bearing one lands a line too far down.
        ("AA\r\nBB", 0, Some((0, 2))),
        ("AA\r\nBB", 1, Some((3, 5))),
        ("AA\r\nBB", 2, None),
        ("\r\n", 0, Some((0, 0))),
        ("\r\n", 1, None),
        ("\r\nx", 1, Some((1, 2))),
        ("a\r\n\r\nb", 1, Some((2, 2))),
        ("a\r\n\r\nb", 2, Some((3, 4))),
        // A lone CR is not a line terminator under UAX #29 and must
        // not split the line here either.
        ("a\rb", 0, Some((0, 3))),
    ];

    pub static ref REPLACE_GRAPHEMES_UNTIL_NEWLINE_TEST: Vec<(&'static str, usize, &'static str, &'static str)> = vec![
        ("abc", 0, "abc", "abc"),
        ("abd", 0, "abc", "abd"),
        ("abd", 0, "", "abd"),
        ("abd", 0, "abcd", "abdd"),
        ("🙏🏻🙏🏻🙏🏻", 0, "", "🙏🏻🙏🏻🙏🏻"),
        ("🙏🏻🙏🏻🙏🏻", 0, "\n", "🙏🏻🙏🏻🙏🏻\n"),
        ("🙏🏻", 0, "abcd", "🙏🏻bcd"),
        ("a", 0, "🙏🏻bcd", "abcd"),
        ("aaaaaaaaaaaaaa", 0, "🙏🏻bcd12🙏🏻3456789", "aaaaaaaaaaaaaa"),
        ("aaaaaaaaaaaaaa", 0, "🙏🏻bcd12🙏🏻\n3456789", "aaaaaaaaaaaaaa\n3456789"),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 0,
            "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
            "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"),
        ("12345", 10, "01234\n678\n", "01234\n678\n12345"),
        ("12345", 10, "01234\n678\n\n", "01234\n678\n12345\n"),
        ("12345", 10, "01234\n678\nabcde\n", "01234\n678\n12345\n"),
        ("12345", 10, "01234\n678\n     \n", "01234\n678\n12345\n"),
        ("123", 10, "01234\n678\n     \n", "01234\n678\n123  \n"),
        ("123", 10, "01234\n678\nabcde\n", "01234\n678\n123de\n"),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 11, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n", "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"),
        ("@@@@@@@@@@", 63,
            "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
            "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"),
        // CRLF in the *target*. Under UAX #29 `\r\n` is one cluster
        // and the whole of it is the line terminator, so the line
        // tail this helper is allowed to overwrite stops **before**
        // the CR. Cutting at the raw `\n` byte instead splices the CR
        // away (a Windows line ending silently becomes a Unix one)
        // and under-reports `growth` by one, which slides every
        // downstream `ColorFontRegions` span one cluster left.
        ("WXYZ", 0, "AA\r\nBB", "WXYZ\r\nBB"),
        ("XY", 0, "AA\r\nBB", "XY\r\nBB"),
        ("X", 0, "AA\r\nBB", "XA\r\nBB"),
        // Writing *at* the terminator's own cluster index: the tail
        // is empty, so the source is inserted ahead of the CRLF.
        ("Z", 1, "A\r\nB", "AZ\r\nB"),
        // CRLF at the very start — line 0 is empty.
        ("XY", 0, "\r\nB", "XY\r\nB"),
        ("Q", 0, "\r\n", "Q\r\n"),
        // CRLF at the very end, and two CRLFs in a row.
        ("Q", 0, "A\r\n", "Q\r\n"),
        ("ab", 0, "A\r\n\r\nB", "ab\r\n\r\nB"),
        // A CRLF terminator next to clusters that are themselves
        // multi-scalar: a skin-toned emoji and a regional-indicator
        // pair. A byte-counting tail would land inside either.
        ("🙏🏻🙏🏻", 0, "🙏🏻\r\n🙏🏻", "🙏🏻🙏🏻\r\n🙏🏻"),
        ("xy", 0, "🇩🇰\r\n🇸🇪", "xy\r\n🇸🇪"),
        ("xy", 0, "e\u{301}\r\nx", "xy\r\nx"),
        // A lone CR is NOT a terminator, so it stays inside the line
        // and is overwritable like any other cluster.
        ("Z", 0, "AA\rBB", "ZA\rBB"),
        ("ZZZZZZ", 0, "AA\rBB", "ZZZZZZ"),
    ];

    pub static ref COUNT_GRAPHEMES_TEST: Vec<(&'static str, usize)> = vec![
        ("abcde", 5),
        ("🙏🏻", 1),
        ("abcd🙏🏻", 5),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 5),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 10),

    ];
    pub static ref COMPLEX_NEWLINE_STRING: &'static str =
    "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻";

    pub static ref INDEX_OF_GRAPHEME_TEST: Vec<(&'static str, usize, Option<usize>)> = vec![
        ("abcde", 4, Some(4)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 4, Some(4*8)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 10, Some(10*8)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 11, Some(10*8 + 1)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 20, Some(10*8 + 10)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 21, None),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 10, Some(10*8)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 21, Some(10*8 + 11)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 31, Some(20*8 + 11)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 41, Some(20*8 + 21)),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n", 42, None),
        (COMPLEX_NEWLINE_STRING.clone(), 10, Some(10*8)),
        (COMPLEX_NEWLINE_STRING.clone(), 11, Some(10*8 + 1)),
        (COMPLEX_NEWLINE_STRING.clone(), 20, Some(10*8 + 10)),
        (COMPLEX_NEWLINE_STRING.clone(), 21, Some(10*8 + 11)),
        (COMPLEX_NEWLINE_STRING.clone(), 31, Some(20*8 + 11)),
        (COMPLEX_NEWLINE_STRING.clone(), 41, Some(20*8 + 21)),
        (COMPLEX_NEWLINE_STRING.clone(), 41, Some(20*8 + 21)),
        (COMPLEX_NEWLINE_STRING.clone(), 52, Some(30*8 + 22)),
        (COMPLEX_NEWLINE_STRING.clone(), 62, Some(30*8 + 32)),
        (COMPLEX_NEWLINE_STRING.clone(), 63, Some(30*8 + 33)),
        (COMPLEX_NEWLINE_STRING.clone(), 83, Some(40*8 + 43)),
        (COMPLEX_NEWLINE_STRING.clone(), 84, Some(40*8 + 44)),
        (COMPLEX_NEWLINE_STRING.clone(), 94, Some(40*8 + 54)),
        (COMPLEX_NEWLINE_STRING.clone(), 104, Some(50*8 + 54)),
        (COMPLEX_NEWLINE_STRING.clone(), 105, Some(50*8 + 55)),
        (COMPLEX_NEWLINE_STRING.clone(), 115, Some(50*8 + 65)),
        (COMPLEX_NEWLINE_STRING.clone(), 124, Some(59*8 + 65)),
        // This is a little confusing, there's 60 "hands emojis" but the last one doesn't count here
        // Since we can only get a byte index to its beginning, its bytes aren't included in the count
    ];
    pub static ref SLICE_TO_NEWLINE_TEST: Vec<(&'static str, usize, &'static str)> = vec![
        ("abcde\n", 0, "abcde"),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 0, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"),
        ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 5*8+1, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"),
        (COMPLEX_NEWLINE_STRING.clone(),
            0, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@"),
        (COMPLEX_NEWLINE_STRING.clone(),
            find_byte_index_of_grapheme(&COMPLEX_NEWLINE_STRING, 21).unwrap(), "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@"),
        // A CRLF terminator: the CR is part of the terminator cluster
        // under UAX #29, so it falls outside the line — the same rule
        // `line_bounds_at` and both nth-line finders apply. Stopping
        // at the raw `\n` byte would hand the caller a slice ending
        // mid-cluster.
        ("abcde\r\n", 0, "abcde"),
        ("abcde\r\nfg", 0, "abcde"),
        ("\r\nfg", 0, ""),
        // Starting *on* the terminator's own first byte yields an
        // empty tail rather than a one-byte `"\r"`.
        ("abcde\r\nfg", 5, ""),
        // A lone CR is not a terminator and stays in the line.
        ("abc\rde", 0, "abc\rde"),
        // Degenerate: a byte index that lands on the LF of a CRLF is
        // mid-cluster and outside the contract, but it must not
        // produce an inverted slice and panic.
        ("abcde\r\nfg", 6, ""),
    ];
}

#[test]
pub fn test_slice_to_newline() {
    do_slice_to_newline();
}

pub fn do_slice_to_newline() {
    for (subject, index, expected) in SLICE_TO_NEWLINE_TEST.clone() {
        let result = slice_to_newline(subject, index);
        assert_eq!(result, expected);
    }
}

#[test]
pub fn test_split_graphemes() {
    do_split_graphemes();
}

pub fn do_split_graphemes() {
    for (original, split_at, remainder, new) in SPLIT_GRAPHEMES_TEST.clone() {
        let mut actual_remainder = original.to_string();
        let actual_new = split_off_graphemes(&mut actual_remainder, split_at);
        assert_eq!(actual_remainder, remainder);
        assert_eq!(actual_new, new);
    }
}

#[test]
pub fn test_find_byte_index_of_grapheme() {
    do_find_byte_index_of_grapheme();
}

pub fn do_find_byte_index_of_grapheme() {
    for (string, graph_index, expected_byte_index) in INDEX_OF_GRAPHEME_TEST.clone() {
        if expected_byte_index.is_some() && expected_byte_index.clone().unwrap() >= string.len() {
            panic!("Expected byte index is out of bounds!! This is a test error.");
        }
        let result = find_byte_index_of_grapheme(string, graph_index);
        assert_eq!(result, expected_byte_index);
    }
}

#[test]
pub fn test_replace_graphemes_until_newline() {
    do_replace_graphemes_until_newline();
}

pub fn do_replace_graphemes_until_newline() {
    for (source, idx, target, expected) in REPLACE_GRAPHEMES_UNTIL_NEWLINE_TEST.clone() {
        let mut result = target.to_string();
        let before = count_grapheme_clusters(&result);
        let outcome = replace_graphemes_until_newline(&mut result, idx, &source);
        assert_eq!(result, expected);

        // The four fields of `LineReplacement` are a contract, not
        // decoration: `growth` must be the buffer's real cluster
        // growth (that is what the region shift rides on), `at` must
        // be where the write actually landed — the caller's index,
        // clamped to the end of the buffer when it points past it —
        // `written_end` must bound the span the source occupies in the
        // post-edit buffer, and `added_lines` must be the count of
        // newlines the source contributed.
        let after = count_grapheme_clusters(&result);
        assert_eq!(
            outcome.at,
            idx.min(before),
            "`at` must report the clamped write position for source {:?} into {:?}",
            source,
            target
        );
        assert_eq!(
            outcome.growth,
            after as isize - before as isize,
            "`growth` must equal the real cluster growth for source {:?} into {:?}",
            source,
            target
        );
        assert_eq!(
            outcome.added_lines,
            source.matches('\n').count(),
            "`added_lines` must count the source's newlines for source {:?}",
            source
        );

        // `written_end` is the half of the span contract a caller
        // pins its `ColorFontRegion` to, so it must be in bounds and
        // must actually bracket the bytes the splice wrote. The write
        // landed at `b_index`; the source occupies
        // `[b_index, b_index + source.len())` afterwards.
        let b_index = find_byte_index_of_grapheme(target, idx).unwrap_or(target.len());
        assert!(
            outcome.at <= outcome.written_end && outcome.written_end <= after,
            "the written span {}..{} must be non-inverted and inside the {}-cluster buffer for source {:?} into {:?}",
            outcome.at,
            outcome.written_end,
            after,
            source,
            target
        );
        let span_start_byte = find_byte_index_of_grapheme(&result, outcome.at).unwrap_or(result.len());
        let span_end_byte = find_byte_index_of_grapheme(&result, outcome.written_end).unwrap_or(result.len());
        assert!(
            span_start_byte >= b_index && span_end_byte >= b_index + source.len(),
            "the written span must cover the spliced bytes {}..{} for source {:?} into {:?}, got bytes {}..{}",
            b_index,
            b_index + source.len(),
            source,
            target,
            span_start_byte,
            span_end_byte
        );
        if span_start_byte == b_index && span_end_byte == b_index + source.len() {
            // Neither seam fused, so the span must slice back to the
            // source exactly — the property `component.length()`
            // arithmetic only ever got right by luck.
            assert_eq!(
                &result[span_start_byte..span_end_byte],
                source,
                "a non-fusing span must slice back to its source for {:?} into {:?}",
                source,
                target
            );
        }
        assert_eq!(
            count_number_lines(&result),
            count_number_lines(target) + outcome.added_lines,
            "the target must gain exactly `added_lines` lines for source {:?} into {:?}",
            source,
            target
        );
    }

    // A single-line source into a single-line target: the plain case
    // the old `Option<(usize, usize)>` return could describe.
    let mut buf = String::from("abc");
    assert_eq!(
        replace_graphemes_until_newline(&mut buf, 1, "XYZ"),
        LineReplacement {
            at: 1,
            // The 2-cluster tail "bc" became the 3-cluster "XYZ".
            growth: 1,
            // Nothing fused, so the span is the source's own three
            // clusters, sitting where the write landed.
            written_end: 4,
            added_lines: 0
        }
    );
    assert_eq!(buf, "aXYZ");

    // Replacement that fits inside the line tail reports no growth
    // and preserves the surplus.
    let mut buf = String::from("abcdef");
    assert_eq!(
        replace_graphemes_until_newline(&mut buf, 1, "XY"),
        LineReplacement {
            at: 1,
            growth: 0,
            written_end: 3,
            added_lines: 0
        }
    );
    assert_eq!(buf, "aXYdef");

    // The contract fix: a multi-line source reflows the target, and
    // `added_lines` is how the caller learns it. The old doc claimed
    // the source was cut at its first `\n` — it never was, and there
    // was no way for a line-addressed caller to notice.
    let mut buf = String::from("abc\ntail");
    let outcome = replace_graphemes_until_newline(&mut buf, 0, "1\n2\n3");
    assert_eq!(buf, "1\n2\n3\ntail");
    assert_eq!(outcome.added_lines, 2, "two newlines came from the source");
    assert_eq!(outcome.growth, 2, "5 clusters replaced 3");
    assert_eq!(count_number_lines(&buf), 4);

    // The bound is on the *target* only: the target's own `\n` is
    // never consumed, so the tail after it survives untouched.
    let mut buf = String::from("ab\nkeep");
    replace_graphemes_until_newline(&mut buf, 0, "longer");
    assert_eq!(buf, "longer\nkeep");

    // Adversarial sources. A skin-toned emoji and a ZWJ family are
    // one cluster each, so a two-emoji source grows a 1-cluster line
    // by exactly 1 — a codepoint-counting implementation would say 5.
    let mut buf = String::from("x");
    let outcome = replace_graphemes_until_newline(&mut buf, 0, "🙏🏻👨‍👩‍👧");
    assert_eq!(buf, "🙏🏻👨‍👩‍👧");
    assert_eq!(outcome.growth, 1);
    assert_eq!(outcome.added_lines, 0);

    // A CRLF in the source is one cluster but still one added line —
    // `added_lines` counts `\n` characters, matching what
    // `count_number_lines` sees. Every line-addressing helper has to
    // agree with that or a caller mixing the two drifts: see the CRLF
    // rows in `NTH_LINE_GRAPHEME_INDICES_TEST`.
    let mut buf = String::from("ab");
    let outcome = replace_graphemes_until_newline(&mut buf, 0, "a\r\nb");
    assert_eq!(buf, "a\r\nb");
    assert_eq!(outcome.added_lines, 1);
    assert_eq!(count_number_lines(&buf), 2);
    assert_eq!(find_nth_line_grapheme_range(&buf, 1), Some((2, 3)));

    // Empty source into an empty target: nothing moves, nothing grows.
    let mut buf = String::new();
    assert_eq!(
        replace_graphemes_until_newline(&mut buf, 0, ""),
        LineReplacement::default()
    );
    assert_eq!(buf, "");

    // `g_index` past the end appends rather than panicking, and the
    // reported `at` clamps to where the write actually landed. Echoing
    // the caller's out-of-range index would make
    // `shift_regions_after(at, growth)` skip every region between the
    // real end of the buffer and it.
    let mut buf = String::from("ab");
    let outcome = replace_graphemes_until_newline(&mut buf, 99, "cd");
    assert_eq!(buf, "abcd");
    assert_eq!(outcome.at, 2);
    assert_eq!(outcome.growth, 2);

    // In range, `at` is the caller's index untouched — including the
    // exact-end index, which is the natural "append here" spelling.
    let mut buf = String::from("ab");
    let outcome = replace_graphemes_until_newline(&mut buf, 2, "cd");
    assert_eq!(buf, "abcd");
    assert_eq!(outcome.at, 2);
    let mut buf = String::from("ab");
    let outcome = replace_graphemes_until_newline(&mut buf, 1, "cd");
    assert_eq!(buf, "acd");
    assert_eq!(outcome.at, 1);

    // The CRLF terminator, spelled out. `"AA\r\nBB"` is five clusters
    // — A, A, "\r\n", B, B — so line 0 is the two-cluster "AA" and
    // the write must stop before the CR. Cutting at the `\n` byte
    // instead eats the CR (five clusters become "WXYZ\nBB", seven)
    // *and* reports `growth` as 4 - 3 = 1 rather than the true 2,
    // sliding every downstream region one cluster left.
    let mut buf = String::from("AA\r\nBB");
    let outcome = replace_graphemes_until_newline(&mut buf, 0, "WXYZ");
    assert_eq!(buf, "WXYZ\r\nBB", "the CRLF terminator must survive intact");
    assert_eq!(outcome.growth, 2, "growth is the whole-buffer cluster delta");
    assert_eq!(outcome.added_lines, 0);
    assert_eq!(count_number_lines(&buf), 2);
    assert_eq!(find_nth_line_grapheme_range(&buf, 0), Some((0, 4)));
    assert_eq!(find_nth_line_grapheme_range(&buf, 1), Some((5, 7)));

    // The same edit through `line_bounds_at`: every line helper in
    // this module must agree on where line 0 of a CRLF buffer ends,
    // and `replace_graphemes_until_newline` is one of them.
    let subject = "AA\r\nBB";
    assert_eq!(line_bounds_at(subject, 0), (0, 2));
    assert_eq!(
        slice_to_newline(subject, 0),
        "AA",
        "the writable line tail stops before the CR, not before the LF"
    );

    // `growth` is a *measured* delta, not `source_clusters -
    // tail_clusters` arithmetic: UAX #29 segmentation is not
    // compositional across a splice seam, so a source whose first
    // scalar extends the preceding cluster fuses with it.
    let mut buf = String::from("\n");
    let outcome = replace_graphemes_until_newline(&mut buf, 0, "\r");
    assert_eq!(buf, "\r\n");
    assert_eq!(
        outcome.growth, 0,
        "the inserted CR fused with the following LF into one cluster"
    );
    assert_eq!(
        (outcome.at, outcome.written_end),
        (0, 1),
        "the source contributed the base scalar of the fused CRLF cell, so it owns it"
    );

    let mut buf = String::from("abc");
    let outcome = replace_graphemes_until_newline(&mut buf, 1, "\u{301}abc");
    assert_eq!(buf, "a\u{301}abc");
    assert_eq!(
        outcome.growth, 1,
        "the leading combining mark joined the 'a' before it: 3 clusters -> 4"
    );

    // Fusion can make the buffer *shrink*, which is why `growth` is
    // signed. Replacing the one-cluster tail "b" with a lone
    // combining mark leaves a single cluster where there were two.
    let mut buf = String::from("ab");
    let outcome = replace_graphemes_until_newline(&mut buf, 1, "\u{301}");
    assert_eq!(buf, "a\u{301}");
    assert_eq!(count_grapheme_clusters(&buf), 1);
    assert_eq!(outcome.growth, -1, "growth must be able to report a shrink");
    assert_eq!(
        (outcome.at, outcome.written_end),
        (1, 1),
        "a source that only extends the cell before it owns no cell, so its span is empty"
    );

    // The mirror of the case above at the *other* seam: the source
    // keeps a cell that the text after it extends, because the base
    // scalar of that cell is the source's own. A lone regional
    // indicator left in the target pairs up with the one the source
    // ends on, so two cells become one and the source owns the
    // survivor.
    let mut buf = String::from("ab\u{1F1F4}");
    let outcome = replace_graphemes_until_newline(&mut buf, 1, "\u{1F1F3}");
    assert_eq!(buf, "a\u{1F1F3}\u{1F1F4}");
    assert_eq!(
        count_grapheme_clusters(&buf),
        2,
        "the two indicators formed one flag"
    );
    assert_eq!(outcome.growth, -1);
    assert_eq!(
        (outcome.at, outcome.written_end),
        (1, 2),
        "the trailing indicator joined the source's cell; the source still owns it"
    );

    // The span is the answer arithmetic gets wrong. `"\u{301}"` is one
    // cluster in isolation, so `at + count_grapheme_clusters(source)`
    // says 1..2 in a buffer that is one cluster long — a region one
    // past the end of the text it styles. That is the shape of the
    // defect `GlyphMatrix::place_in` used to emit on every row.
    let mut buf = String::from("ab");
    let outcome = replace_graphemes_until_newline(&mut buf, 1, "\u{301}");
    assert!(
        outcome.written_end <= count_grapheme_clusters(&buf),
        "a measured span can never point past the buffer"
    );
    assert_ne!(
        outcome.written_end,
        outcome.at + count_grapheme_clusters("\u{301}"),
        "the arithmetic answer is the wrong one here; that is why the span is measured"
    );
}

#[test]
pub fn test_count_grapheme_clusters() {
    do_count_grapheme_clusters();
}

pub fn do_count_grapheme_clusters() {
    for (string, num_clusters) in COUNT_GRAPHEMES_TEST.clone() {
        let result = count_grapheme_clusters(string);
        assert_eq!(result, num_clusters);
    }
}

#[test]
pub fn test_find_nth_line_grapheme_indices() {
    do_find_nth_line_grapheme_indices();
}

pub fn do_find_nth_line_grapheme_indices() {
    for (str, n, idx) in NTH_LINE_GRAPHEME_INDICES_TEST.clone() {
        let result = find_nth_line_grapheme_range(str, n);
        assert_eq!(result, idx);
    }
}

#[test]
pub fn test_remove_prefix_unicode() {
    do_remove_prefix_unicode();
}

pub fn do_remove_prefix_unicode() {
    for (original, n, expected) in REMOVE_PREFIX_TESTS.clone() {
        let mut our_og = original.to_string();
        delete_front_unicode(&mut our_og, n);
        assert_eq!(our_og, expected);
    }
}

#[test]
pub fn test_insert_new_lines() {
    do_insert_new_lines();
}

pub fn do_insert_new_lines() {
    let mut my_string = String::new();
    insert_new_lines(&mut my_string, 4);
    let mut count = count_number_lines(&my_string);
    assert_eq!(count, 5);
    insert_new_lines(&mut my_string, 5);
    count = count_number_lines(&my_string);
    assert_eq!(count, 10);
    insert_new_lines(&mut my_string, 5000);
    count = count_number_lines(&my_string);
    assert_eq!(count, 5010);
}
#[test]
pub fn test_push_spaces() {
    do_push_spaces();
}

pub fn do_push_spaces() {
    let mut my_string = String::new();
    push_spaces(&mut my_string, 5);
    assert_eq!(my_string.len(), count_grapheme_clusters(&my_string));
    assert_eq!(my_string.len(), 5);
    push_spaces(&mut my_string, 15);
    assert_eq!(my_string.len(), count_grapheme_clusters(&my_string));
    assert_eq!(my_string.len(), 20);
    push_spaces(&mut my_string, 0);
    assert_eq!(my_string.len(), 20);
}

#[test]
pub fn test_count_number_of_lines() {
    do_count_grapheme_clusters();
}

pub fn do_count_number_of_lines() {
    for (str, num_lines) in COUNT_LINES_TEST.clone() {
        let result = count_number_lines(str);
        assert_eq!(result, num_lines);
    }
}

#[test]
pub fn test_truncate_unicode() {
    do_truncate_unicode();
}

pub fn do_truncate_unicode() {
    for (original, n, expected) in TRUNCATE_TESTS.clone() {
        let mut our_og = original.to_string();
        delete_back_unicode(&mut our_og, n);
        assert_eq!(our_og, expected);
    }
}

#[test]
pub fn test_insert_str_at_grapheme() {
    do_insert_str_at_grapheme();
}

pub fn do_insert_str_at_grapheme() {
    // ASCII insert in the middle.
    let mut s = String::from("abcd");
    insert_str_at_grapheme(&mut s, 2, "X");
    assert_eq!(s, "abXcd");

    // Insert past the end appends.
    let mut s = String::from("abc");
    insert_str_at_grapheme(&mut s, 99, "Z");
    assert_eq!(s, "abcZ");

    // Multi-byte char doesn't split a grapheme.
    let mut s = String::from("café"); // 4 graphemes; 'é' is 2 bytes
    insert_str_at_grapheme(&mut s, 4, "!");
    assert_eq!(s, "café!");
    let mut s = String::from("café");
    insert_str_at_grapheme(&mut s, 3, "!");
    assert_eq!(s, "caf!é");

    // Emoji ZWJ cluster is treated as one unit.
    let mut s = String::from("ab🧑‍🚀cd");
    insert_str_at_grapheme(&mut s, 3, "Z");
    assert_eq!(s, "ab🧑‍🚀Zcd");
}

#[test]
pub fn test_insert_str_at_grapheme_counted() {
    do_insert_str_at_grapheme_counted();
}

pub fn do_insert_str_at_grapheme_counted() {
    let mut s = String::new();
    let delta = insert_str_at_grapheme_counted(&mut s, 0, "abc");
    assert_eq!(s, "abc");
    assert_eq!(delta, 3);

    let mut s = String::new();
    let delta = insert_str_at_grapheme_counted(&mut s, 0, "e\u{0301}");
    assert_eq!(s, "e\u{0301}");
    assert_eq!(delta, 1);

    let mut s = String::from("e");
    let delta = insert_str_at_grapheme_counted(&mut s, 1, "\u{0301}");
    assert_eq!(s, "e\u{0301}");
    assert_eq!(delta, 0);

    let mut s = String::new();
    let delta = insert_str_at_grapheme_counted(&mut s, 0, "\u{1112}\u{1161}\u{11AB}");
    assert_eq!(s, "\u{1112}\u{1161}\u{11AB}");
    assert_eq!(delta, 1);

    let mut s = String::new();
    let family = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F466}";
    let delta = insert_str_at_grapheme_counted(&mut s, 0, family);
    assert_eq!(s, family);
    assert_eq!(delta, 1);

    // Regression: inserting a base character before a standalone
    // combining mark must advance the cursor past the newly composed
    // cluster. The old total-count delta returned 0 because the buffer
    // had one grapheme both before and after the insertion.
    let mut s = String::from("\u{0301}");
    let delta = insert_str_at_grapheme_counted(&mut s, 0, "e");
    assert_eq!(s, "e\u{0301}");
    assert_eq!(delta, 1);

    // Same scenario with an additional trailing cluster: the cursor
    // stops after the composed "e + combining mark" cluster, not at
    // the end of the buffer.
    let mut s = String::from("\u{0301}x");
    let delta = insert_str_at_grapheme_counted(&mut s, 0, "e");
    assert_eq!(s, "e\u{0301}x");
    assert_eq!(delta, 1);
}

#[test]
pub fn test_grapheme_display_width() {
    do_grapheme_display_width();
}

pub fn do_grapheme_display_width() {
    // ASCII → one cell each.
    assert_eq!(grapheme_display_width(""), 0);
    assert_eq!(grapheme_display_width("abcd"), 4);
    assert_eq!(grapheme_display_width("❯ color"), 7);

    // Box-drawing characters (U+2500..) are ambiguous-width in Unicode
    // but render at one cell in the app's monospace chain. The table
    // deliberately keeps them at 1.
    assert_eq!(grapheme_display_width("╭─╮"), 3);
    assert_eq!(grapheme_display_width("│ │"), 3);

    // East-Asian-Wide: each CJK ideograph counts as two cells.
    assert_eq!(grapheme_display_width("日"), 2);
    assert_eq!(grapheme_display_width("日本語"), 6);

    // Hiragana / Katakana / Hangul.
    assert_eq!(grapheme_display_width("あい"), 4);
    assert_eq!(grapheme_display_width("가나"), 4);

    // Combining marks fold into their base: "café" as NFD (e + ´) is
    // still 4 cells because the combining acute is zero-width.
    assert_eq!(grapheme_display_width("cafe\u{0301}"), 4);

    // ZWJ emoji cluster (`🧑‍🚀`) has base 🧑 — outside the table's
    // wide ranges, so we return 1. This is a known under-measure for
    // terminal emoji; the app uses cosmic-text which renders the whole
    // ZWJ cluster at its own advance anyway, so the border-alignment
    // use-case is unaffected.
    assert_eq!(grapheme_display_width("🧑‍🚀"), 1);

    // Scalar-level spot checks.
    assert_eq!(scalar_display_width('a'), 1);
    assert_eq!(scalar_display_width('日'), 2);
    assert_eq!(scalar_display_width('\u{0301}'), 0); // combining acute
    assert_eq!(scalar_display_width('\u{200D}'), 0); // ZWJ
}

#[test]
pub fn test_truncate_to_display_width() {
    do_truncate_to_display_width();
}

pub fn do_truncate_to_display_width() {
    // ASCII: exact cell match.
    assert_eq!(truncate_to_display_width("abcdef", 3), "abc");
    assert_eq!(truncate_to_display_width("abcdef", 0), "");
    assert_eq!(truncate_to_display_width("abcdef", 100), "abcdef");

    // CJK: each char is 2 cells; an odd max cuts cleanly on the
    // grapheme boundary rather than mid-glyph.
    assert_eq!(truncate_to_display_width("日本語", 3), "日"); // 2+2>3 after "日"
    assert_eq!(truncate_to_display_width("日本語", 4), "日本");
    assert_eq!(truncate_to_display_width("日本語", 2), "日");
    assert_eq!(truncate_to_display_width("日本語", 0), "");

    // Mix: 3 ASCII + 1 CJK = 5 cells; trim to 4 drops the CJK.
    assert_eq!(truncate_to_display_width("abc日", 4), "abc");
    assert_eq!(truncate_to_display_width("abc日", 5), "abc日");

    // Combining marks: NFD "café" is 4 cells, e+́ folds into one cell.
    assert_eq!(truncate_to_display_width("cafe\u{0301}", 3), "caf");
    assert_eq!(truncate_to_display_width("cafe\u{0301}", 4), "cafe\u{0301}");
}

#[test]
pub fn test_delete_grapheme_at() {
    do_delete_grapheme_at();
}

pub fn do_delete_grapheme_at() {
    // ASCII delete in the middle.
    let mut s = String::from("abcd");
    delete_grapheme_at(&mut s, 1);
    assert_eq!(s, "acd");

    // Delete past the end is a no-op.
    let mut s = String::from("abc");
    delete_grapheme_at(&mut s, 99);
    assert_eq!(s, "abc");

    // Delete the last cluster.
    let mut s = String::from("abc");
    delete_grapheme_at(&mut s, 2);
    assert_eq!(s, "ab");

    // Multi-byte char and ZWJ cluster delete as one unit.
    let mut s = String::from("café");
    delete_grapheme_at(&mut s, 3);
    assert_eq!(s, "caf");
    let mut s = String::from("ab🧑‍🚀cd");
    delete_grapheme_at(&mut s, 2);
    assert_eq!(s, "abcd");
}

#[test]
pub fn test_word_left() {
    do_word_left();
}

pub fn do_word_left() {
    // Cursor at the end of a single word lands at the word's start.
    assert_eq!(word_left("hello", 5), 0);

    // Cursor at 0 is a no-op.
    assert_eq!(word_left("hello world", 0), 0);

    // Skip backwards past leading boundary chars then through the
    // preceding word — `"foo, bar"` cursor=8 (end) → 5 (start of
    // "bar"); cursor=5 (start of "bar") → 0 (start of "foo").
    assert_eq!(word_left("foo, bar", 8), 5);
    assert_eq!(word_left("foo, bar", 5), 0);

    // Walk through two words: `"foo bar"` cursor=7 → 4 → 0.
    assert_eq!(word_left("foo bar", 7), 4);
    assert_eq!(word_left("foo bar", 4), 0);

    // Empty string is a no-op for any cursor.
    assert_eq!(word_left("", 0), 0);
    assert_eq!(word_left("", 5), 0);

    // Cursor past the grapheme count is clamped to the count.
    assert_eq!(word_left("hello", 99), 0);

    // Single grapheme.
    assert_eq!(word_left("a", 1), 0);

    // Emoji is a non-word boundary — `"foo🍕bar"` cursor=7 → 4
    // (start of "bar" since 🍕 is one grapheme cluster between
    // graphemes 3 and 4).
    assert_eq!(word_left("foo🍕bar", 7), 4);

    // ZWJ cluster (👨‍👩‍👧) — first scalar `'\u{1F468}'` is non-
    // alphanumeric, so the entire cluster counts as one boundary
    // grapheme. `"foo👨‍👩‍👧bar"` cursor=7 → 4.
    assert_eq!(word_left("foo👨‍👩‍👧bar", 7), 4);

    // Combining-mark cluster (NFD `café` = "cafe\u{0301}") — the
    // base scalar is alphanumeric, so the cluster IS a word
    // grapheme. cursor=4 (end of "café") → 0.
    let cafe_nfd = "cafe\u{0301}";
    assert_eq!(word_left(cafe_nfd, 4), 0);

    // Regional-indicator pair (🇺🇸 = two scalars, one cluster) — no
    // scalar in a flag is alphanumeric, so the cluster is a
    // boundary. `"foo🇺🇸bar"` cursor=7 → 4.
    assert_eq!(word_left("foo🇺🇸bar", 7), 4);

    // UAX #29 `Prepend`: U+0600 ARABIC NUMBER SIGN is glued to the
    // digit after it by GB9b, so `"ab \u{600}5"` is four clusters —
    // "a", "b", " ", "\u{600}5". The cluster is a *number* on screen;
    // only its invisible first scalar is non-alphanumeric. A
    // first-scalar predicate calls it a boundary and walks the cursor
    // through the whole token back to the start of "ab"; reading any
    // scalar stops at the number where the reader expects it to.
    let prepend = "ab \u{600}5";
    assert_eq!(count_grapheme_clusters(prepend), 4);
    assert_eq!(word_left(prepend, 4), 3);
    assert_eq!(word_left(prepend, 3), 0);

    // The *second* class the two predicates disagree on, pinned
    // because `grapheme_is_word`'s doc names it and a reader is
    // entitled to check it: a non-alphanumeric base carrying an
    // `Other_Alphabetic` mark. U+0345 COMBINING GREEK YPOGEGRAMMENI is
    // one of 1 353 such marks, and `char::is_alphanumeric` reports it
    // as alphabetic, so `"-\u{345}"` is a word cluster under `any` and
    // a boundary under a first-scalar rule. Here `any` is the *less*
    // obviously right answer — the marked hyphen stops breaking the
    // word — and that is the price of fixing `Prepend`.
    let marked_hyphen = "key-\u{345}val";
    assert_eq!(count_grapheme_clusters(marked_hyphen), 7);
    assert_eq!(
        word_left(marked_hyphen, 7),
        0,
        "the marked hyphen is part of the word under the `any` rule, so the whole run is one word"
    );
    // The unmarked hyphen still breaks it, which is what makes the
    // above a consequence of the mark and not of the hyphen.
    assert_eq!(word_left("key-val", 7), 4);
}

#[test]
pub fn test_word_right() {
    do_word_right();
}

pub fn do_word_right() {
    // Cursor at 0 of a single word lands past the word's end.
    assert_eq!(word_right("hello", 0), 5);

    // Cursor at end is a no-op.
    assert_eq!(word_right("hello", 5), 5);

    // Cursor past the end is clamped at the grapheme count.
    assert_eq!(word_right("hello", 99), 5);

    // Skip leading boundary chars then through the following word.
    assert_eq!(word_right(", foo bar", 0), 5);
    assert_eq!(word_right("foo bar", 0), 3);
    assert_eq!(word_right("foo bar", 3), 7);

    // Empty string is a no-op for any cursor.
    assert_eq!(word_right("", 0), 0);
    assert_eq!(word_right("", 5), 0);

    // Single grapheme.
    assert_eq!(word_right("a", 0), 1);

    // Emoji boundary — `"foo🍕bar"` cursor=0 → 3 (end of "foo");
    // cursor=3 → 7 (past the boundary 🍕 and through "bar").
    assert_eq!(word_right("foo🍕bar", 0), 3);
    assert_eq!(word_right("foo🍕bar", 3), 7);

    // ZWJ cluster — `"foo👨‍👩‍👧bar"` cursor=3 → 7.
    assert_eq!(word_right("foo👨‍👩‍👧bar", 3), 7);

    // Combining-mark cluster — base scalar alphanumeric, so the
    // cluster IS a word grapheme. cursor=0 of `"café"` (NFD) → 4.
    let cafe_nfd = "cafe\u{0301}";
    assert_eq!(word_right(cafe_nfd, 0), 4);

    // Regional-indicator pair — `"foo🇺🇸bar"` cursor=3 → 7.
    assert_eq!(word_right("foo🇺🇸bar", 3), 7);

    // The `Prepend` case from `do_word_left`, walked the other way.
    // `"ab \u{600}5 cd"` is "a", "b", " ", "\u{600}5", " ", "c", "d".
    // From the space at 2 the motion must stop past the number; a
    // first-scalar predicate treats the number as punctuation, skips
    // it with the spaces around it, and overshoots into "cd".
    let prepend = "ab \u{600}5 cd";
    assert_eq!(count_grapheme_clusters(prepend), 7);
    assert_eq!(word_right(prepend, 2), 4);
    assert_eq!(word_right(prepend, 4), 7);
    assert_eq!(word_right(prepend, 0), 2);
}

lazy_static! {
    /// Truth table for [`first_non_whitespace_grapheme`]:
    /// `(case name, input, expected cluster index)`.
    ///
    /// The interesting rows are the ones where the byte offset, the
    /// `char` ordinal, and the grapheme index of the same answer all
    /// differ — those are precisely the inputs that used to panic or
    /// mis-place text in `GlyphLine::perform_op`.
    pub static ref FIRST_NON_WHITESPACE_GRAPHEME_TEST: Vec<(&'static str, &'static str, Option<usize>)> = vec![
        ("empty string has no content", "", None),
        ("all ASCII space", "    ", None),
        ("mixed ASCII whitespace only", " \t\n\r", None),
        ("content at the very front", "abc", Some(0)),
        ("ASCII indent", "  abc", Some(2)),
        // U+3000 is three bytes wide: the answer is cluster 2, but the
        // byte offset is 6. Feeding the cluster index to a byte-indexed
        // `String::split_off` panics mid-codepoint — the original bug.
        ("ideographic-space indent (multi-byte)", "\u{3000}\u{3000}abc", Some(2)),
        ("ideographic space is whitespace all the way", "\u{3000}\u{3000}", None),
        // CRLF is one cluster made of two chars, both whitespace: the
        // `char` ordinal here is 4, the grapheme index is 3.
        ("CRLF indent (multi-char single cluster)", "\r\n  abc", Some(3)),
        // Emoji are single clusters but several chars / many bytes, so
        // a char ordinal drifts the moment content is emoji.
        ("emoji at the front", "🍕🍕abc", Some(0)),
        ("emoji after an indent", "  🍕", Some(2)),
        ("ZWJ family cluster after an indent", "  👨‍👩‍👧", Some(2)),
        ("skin-tone modifier cluster after an indent", "  🙏🏻x", Some(2)),
        ("regional-indicator pair after an indent", "  🇺🇸", Some(2)),
        // A cluster counts as whitespace only when *every* scalar in it
        // is whitespace, so a space carrying a combining acute is
        // content, not indent.
        ("space plus combining mark is content", " \u{0301}abc", Some(0)),
        ("combining mark on a letter after an indent", "  e\u{0301}f", Some(2)),
        // U+200B ZERO WIDTH SPACE lacks the White_Space property, so it
        // is content — deliberately, since it occupies a cluster slot.
        ("zero-width space is not whitespace", " \u{200B}", Some(1)),
    ];
}

#[test]
fn test_first_non_whitespace_grapheme() {
    do_first_non_whitespace_grapheme();
}

/// Walks [`FIRST_NON_WHITESPACE_GRAPHEME_TEST`] and additionally
/// asserts the result is a valid cluster index into the input — the
/// property every caller relies on when it feeds the value straight
/// into [`split_off_graphemes`].
pub fn do_first_non_whitespace_grapheme() {
    for (name, input, expected) in FIRST_NON_WHITESPACE_GRAPHEME_TEST.iter() {
        let got = first_non_whitespace_grapheme(input);
        assert_eq!(got, *expected, "case `{}`", name);
        if let Some(idx) = got {
            assert!(
                idx < count_grapheme_clusters(input),
                "case `{}`: {} is not a cluster index into {:?}",
                name,
                idx,
                input
            );
            // Splitting there must leave the indent behind and start
            // the tail on real content — no mid-cluster cut.
            let mut buffer = input.to_string();
            let tail = split_off_graphemes(&mut buffer, idx);
            assert!(
                buffer.chars().all(char::is_whitespace),
                "case `{}`: prefix {:?} is not pure whitespace",
                name,
                buffer
            );
            assert_eq!(
                first_non_whitespace_grapheme(&tail),
                Some(0),
                "case `{}`: tail {:?} does not start on content",
                name,
                tail
            );
        }
    }
}

#[test]
pub fn test_take_graphemes() {
    do_take_graphemes();
}

/// [`take_graphemes`] must cut on cluster boundaries and report
/// overflow from the same walk. The adversarial cases are the ones
/// where a byte- or `char`-based `take` diverges: a skin-tone
/// modifier (base + modifier), a ZWJ family (three bases + two
/// joiners), a regional-indicator flag (two scalars), an NFD
/// combining mark, a CRLF pair, and U+200B, which is a real cluster
/// even though it renders as nothing.
pub fn do_take_graphemes() {
    // ASCII baseline, including both sides of the cap.
    assert_eq!(take_graphemes("abcdef", 3), ("abc", true));
    assert_eq!(take_graphemes("abc", 3), ("abc", false));
    assert_eq!(take_graphemes("ab", 3), ("ab", false));

    // n = 0 borrows nothing but still answers the overflow question.
    assert_eq!(take_graphemes("abc", 0), ("", true));
    assert_eq!(take_graphemes("", 0), ("", false));
    assert_eq!(take_graphemes("", 5), ("", false));

    // Skin-tone modifier: `🙏🏻` is base + U+1F3FB, one cluster.
    // Taking 1 must yield the whole thing, never the bare base.
    assert_eq!(take_graphemes("🙏🏻ab", 1), ("🙏🏻", true));
    assert_eq!(take_graphemes("🙏🏻🙏🏻", 1), ("🙏🏻", true));
    assert_eq!(take_graphemes("🙏🏻🙏🏻", 2), ("🙏🏻🙏🏻", false));

    // ZWJ family: five scalars, one cluster.
    assert_eq!(take_graphemes("👨‍👩‍👧x", 1), ("👨‍👩‍👧", true));
    assert_eq!(count_grapheme_clusters(take_graphemes("👨‍👩‍👧x", 1).0), 1);

    // Regional-indicator pair (flag): two scalars, one cluster.
    assert_eq!(take_graphemes("🇺🇸🇳🇴", 1), ("🇺🇸", true));
    assert_eq!(take_graphemes("🇺🇸🇳🇴", 2), ("🇺🇸🇳🇴", false));

    // Combining mark: NFD "café" is 5 scalars, 4 clusters, and the
    // acute must travel with its base.
    let cafe_nfd = "cafe\u{0301}";
    assert_eq!(take_graphemes(cafe_nfd, 4), (cafe_nfd, false));
    assert_eq!(take_graphemes(cafe_nfd, 3), ("caf", true));

    // Lone combining mark at index 0 — no base to attach to, so it
    // is its own cluster and `take(1)` must not return an empty
    // slice.
    assert_eq!(take_graphemes("\u{0301}abc", 1), ("\u{0301}", true));

    // CRLF is a single cluster under UAX #29.
    assert_eq!(take_graphemes("a\r\nb", 2), ("a\r\n", true));
    assert_eq!(take_graphemes("\r\n", 1), ("\r\n", false));

    // U+3000 IDEOGRAPHIC SPACE is three bytes but one cluster, and
    // U+200B ZERO WIDTH SPACE is a cluster despite rendering blank.
    assert_eq!(take_graphemes("\u{3000}ab", 1), ("\u{3000}", true));
    assert_eq!(take_graphemes("a\u{200B}b", 2), ("a\u{200B}", true));

    // Every prefix must be a valid, cluster-aligned slice of the
    // source — the property callers rely on when they format it.
    for input in ["", "abc", "🙏🏻👨‍👩‍👧🇺🇸", "a\r\nb", "\u{0301}x", "\u{3000}\u{200B}"]
    {
        let total = count_grapheme_clusters(input);
        for n in 0..=total + 2 {
            let (prefix, overflow) = take_graphemes(input, n);
            assert!(
                input.starts_with(prefix),
                "prefix {:?} is not a prefix of {:?}",
                prefix,
                input
            );
            assert_eq!(
                count_grapheme_clusters(prefix),
                n.min(total),
                "prefix of {:?} at n={} has the wrong cluster count",
                input,
                n
            );
            assert_eq!(overflow, total > n, "overflow flag for {:?} at n={}", input, n);
        }
    }
}

#[test]
pub fn test_line_bounds_at() {
    do_line_bounds_at();
}

/// [`line_bounds_at`] replaces the app crate's two hand-rolled
/// walks; these cases are ported from the editor tests it subsumed,
/// plus the CRLF case the old `g == "\n"` test walked straight past.
pub fn do_line_bounds_at() {
    // Single line: bounds are the whole buffer.
    assert_eq!(line_bounds_at("abc", 2), (0, 3));
    assert_eq!(line_bounds_at("abc", 0), (0, 3));

    // Empty buffer.
    assert_eq!(line_bounds_at("", 0), (0, 0));
    assert_eq!(line_bounds_at("", 7), (0, 0));

    // Three lines of "ab\ncd\nef": clusters 0..2, 3..5, 6..8.
    let s = "ab\ncd\nef";
    assert_eq!(line_bounds_at(s, 0), (0, 2));
    assert_eq!(line_bounds_at(s, 1), (0, 2));
    // A cursor sitting *on* the newline belongs to the line it ends.
    assert_eq!(line_bounds_at(s, 2), (0, 2));
    assert_eq!(line_bounds_at(s, 3), (3, 5));
    assert_eq!(line_bounds_at(s, 4), (3, 5));
    assert_eq!(line_bounds_at(s, 6), (6, 8));
    assert_eq!(line_bounds_at(s, 7), (6, 8));
    // Past the end clamps to the last line.
    assert_eq!(line_bounds_at(s, 99), (6, 8));

    // Trailing newline opens an empty final line.
    assert_eq!(line_bounds_at("abc\n", 4), (4, 4));
    assert_eq!(line_bounds_at("abc\n", 0), (0, 3));

    // A buffer of only newlines: every line is empty.
    assert_eq!(line_bounds_at("\n\n", 0), (0, 0));
    assert_eq!(line_bounds_at("\n\n", 1), (1, 1));
    assert_eq!(line_bounds_at("\n\n", 2), (2, 2));

    // CRLF: `\r\n` is ONE cluster, so "ab\r\ncd" is 5 clusters, not
    // 6, and the terminator sits at index 2. Testing `g == "\n"`
    // would miss it entirely and report one 5-cluster line.
    let crlf = "ab\r\ncd";
    assert_eq!(count_grapheme_clusters(crlf), 5);
    assert_eq!(line_bounds_at(crlf, 0), (0, 2));
    assert_eq!(line_bounds_at(crlf, 3), (3, 5));

    // Multi-byte and multi-scalar content: bounds are cluster
    // indices, so a line of emoji is as long as it looks.
    let emoji = "🙏🏻👨‍👩‍👧\n🇺🇸";
    assert_eq!(line_bounds_at(emoji, 0), (0, 2));
    assert_eq!(line_bounds_at(emoji, 3), (3, 4));

    // U+3000 and U+200B are ordinary content clusters, not
    // terminators.
    assert_eq!(line_bounds_at("a\u{3000}\u{200B}b", 0), (0, 4));

    // Property: for every cursor into every buffer the bounds must
    // bracket the cursor (once clamped) and never split a line.
    for input in ["", "a", "ab\ncd\nef", "abc\n", "\n\n", "ab\r\ncd", "🙏🏻\n🇺🇸"] {
        let total = count_grapheme_clusters(input);
        for cursor in 0..=total + 1 {
            let (start, end) = line_bounds_at(input, cursor);
            assert!(start <= end, "{:?} at {}: start > end", input, cursor);
            assert!(end <= total, "{:?} at {}: end past the buffer", input, cursor);
            assert!(
                start <= cursor.min(total),
                "{:?} at {}: start {} is past the cursor",
                input,
                cursor,
                start
            );
        }
    }
}

#[test]
pub fn test_prev_word_boundary_ws() {
    do_prev_word_boundary_ws();
}

/// [`prev_word_boundary_ws`] is the `Ctrl+W` motion the console's
/// `kill_word` used to hand-roll. It differs from [`word_left`]
/// wherever punctuation appears: `key=value` is one token here and
/// three there.
pub fn do_prev_word_boundary_ws() {
    // Cursor at 0, and empty buffers, are no-ops.
    assert_eq!(prev_word_boundary_ws("", 0), 0);
    assert_eq!(prev_word_boundary_ws("", 9), 0);
    assert_eq!(prev_word_boundary_ws("hello", 0), 0);

    // End of a single word lands at its start.
    assert_eq!(prev_word_boundary_ws("hello", 5), 0);
    assert_eq!(prev_word_boundary_ws("hello", 99), 0);

    // Trailing whitespace is skipped before the token is.
    assert_eq!(prev_word_boundary_ws("foo bar", 7), 4);
    assert_eq!(prev_word_boundary_ws("foo bar ", 8), 4);
    assert_eq!(prev_word_boundary_ws("foo bar   ", 10), 4);
    assert_eq!(prev_word_boundary_ws("foo bar", 4), 0);

    // The whole point of the `_ws` sibling: punctuation stays inside
    // the token, so a kv pair dies in one stroke. `word_left` would
    // stop at the `=`.
    assert_eq!(prev_word_boundary_ws("section key=value", 17), 8);
    assert_eq!(word_left("section key=value", 17), 12);
    assert_eq!(prev_word_boundary_ws("a/b/c", 5), 0);

    // Leading whitespace: walking back off the first token lands at 0.
    assert_eq!(prev_word_boundary_ws("   word", 7), 3);
    assert_eq!(prev_word_boundary_ws("   word", 3), 0);
    assert_eq!(prev_word_boundary_ws("   ", 3), 0);

    // U+3000 IDEOGRAPHIC SPACE is whitespace — and it is three bytes
    // and one cluster, so a byte-indexed implementation would land
    // two bytes into it.
    assert_eq!(prev_word_boundary_ws("foo\u{3000}bar", 7), 4);
    // Other exotic spaces behave the same.
    assert_eq!(prev_word_boundary_ws("foo\u{00A0}bar", 7), 4);

    // U+200B ZERO WIDTH SPACE is NOT whitespace (it is `Cf`), so it
    // holds the token together — the same rule
    // `first_non_whitespace_grapheme` applies.
    assert_eq!(prev_word_boundary_ws("foo\u{200B}bar", 7), 0);

    // Emoji are token content: a skin-toned emoji, a ZWJ family and
    // a flag are one cluster each and none is a separator.
    assert_eq!(prev_word_boundary_ws("ab🙏🏻cd", 5), 0);
    assert_eq!(prev_word_boundary_ws("x 👨‍👩‍👧", 3), 2);
    assert_eq!(prev_word_boundary_ws("x 🇺🇸", 3), 2);

    // Combining marks ride with their base: NFD "café" is one token.
    assert_eq!(prev_word_boundary_ws("cafe\u{0301}", 4), 0);
    // A lone combining mark at index 0 is content, not a separator.
    assert_eq!(prev_word_boundary_ws("\u{0301}ab", 3), 0);

    // A CRLF cluster is all-whitespace, so it separates tokens.
    assert_eq!(prev_word_boundary_ws("foo\r\nbar", 7), 4);

    // The rule is *every* scalar, not the first one. SPACE + U+0301
    // COMBINING ACUTE is a single cluster whose first scalar is
    // whitespace but which is not whitespace as a whole, so it is
    // token content and does not break the run. A first-scalar-only
    // predicate would report 4 here and 8 below — this is the pair
    // that pins the decision `first_non_whitespace_grapheme` also
    // makes.
    assert_eq!(prev_word_boundary_ws("foo \u{301}bar", 7), 0);
    assert_eq!(prev_word_boundary_ws("ab \u{301}", 4), 0);
    // And the converse direction: a cluster whose first scalar is
    // *not* whitespace is content no matter what follows, which the
    // NFD "café" case above already covers.
}

#[test]
pub fn test_token_start_ws() {
    do_token_start_ws();
}

/// [`token_start_ws`] is [`prev_word_boundary_ws`] without the
/// leading whitespace skip — the scan the completion popup needs, so
/// that a cursor parked after a space reports an *empty* token
/// rather than reaching back into the previous word.
pub fn do_token_start_ws() {
    assert_eq!(token_start_ws("", 0), 0);
    assert_eq!(token_start_ws("hello", 0), 0);
    assert_eq!(token_start_ws("hello", 5), 0);
    assert_eq!(token_start_ws("hello", 99), 0);

    // Mid-token.
    assert_eq!(token_start_ws("foo bar", 6), 4);
    assert_eq!(token_start_ws("foo bar", 7), 4);

    // The divergence from `prev_word_boundary_ws`: a cursor sitting
    // right after whitespace starts a fresh (empty) token.
    assert_eq!(token_start_ws("foo ", 4), 4);
    assert_eq!(prev_word_boundary_ws("foo ", 4), 0);
    assert_eq!(token_start_ws("foo   ", 6), 6);
    assert_eq!(token_start_ws("   ", 3), 3);

    // kv tokens stay whole so the caller can locate the `=` itself.
    assert_eq!(token_start_ws("section key=val", 15), 8);
    assert_eq!(token_start_ws("node color=#ff0000", 18), 5);

    // Same Unicode rules as its sibling.
    assert_eq!(token_start_ws("foo\u{3000}bar", 7), 4);
    assert_eq!(token_start_ws("foo\u{200B}bar", 7), 0);
    assert_eq!(token_start_ws("ab🙏🏻cd", 5), 0);
    assert_eq!(token_start_ws("cafe\u{0301}", 4), 0);
    assert_eq!(token_start_ws("\u{0301}ab", 3), 0);
    // "a\r\nb" is THREE clusters — the CRLF is one — so a cursor of
    // 4 clamps to 3 and the token is the final "b" at index 2.
    assert_eq!(token_start_ws("a\r\nb", 4), 2);

    // Separator-hood is decided over *every* scalar in the cluster,
    // not the first: SPACE + U+0301 COMBINING ACUTE is one cluster
    // that starts with whitespace and is not whitespace, so it holds
    // the token together. A first-scalar-only predicate reports 4.
    assert_eq!(token_start_ws("foo \u{301}bar", 7), 0);

    // Property: the reported start is always a cluster index at or
    // before the (clamped) cursor, and no cluster inside the token it
    // delimits is entirely whitespace. Note the shape of the
    // invariant — "contains no whitespace scalar" would be the wrong
    // one, and the `"x \u{301}y"` row is the case that tells them
    // apart.
    for input in [
        "",
        "foo bar",
        "a b  c",
        "key=value x",
        "\u{3000}q",
        "🙏🏻 👨‍👩‍👧",
        "x \u{301}y",
        "a\r\nb",
    ] {
        let total = count_grapheme_clusters(input);
        for cursor in 0..=total + 1 {
            let start = token_start_ws(input, cursor);
            let clamped = cursor.min(total);
            assert!(start <= clamped, "{:?} at {}: start past cursor", input, cursor);
            let from = find_byte_index_of_grapheme(input, start).unwrap_or(input.len());
            let to = find_byte_index_of_grapheme(input, clamped).unwrap_or(input.len());
            for cluster in split_graphemes_owned(&input[from..to]) {
                assert!(
                    cluster.chars().any(|c| !c.is_whitespace()),
                    "{:?} at {}: token {:?} contains the all-whitespace cluster {:?}",
                    input,
                    cursor,
                    &input[from..to],
                    cluster
                );
            }
        }
    }
}

#[test]
pub fn test_insert_spaces() {
    do_insert_spaces();
}

/// [`insert_spaces`] shipped `pub` with neither a test nor a bench
/// (issue #38 item 5) even though `GlyphMatrix::place_in` calls it
/// once per painted row. The cases that matter are the boundary ones:
/// `n = 0`, an index past the end, and an index that a byte- or
/// `char`-based insert would land inside a cluster.
pub fn do_insert_spaces() {
    // n = 0 leaves the string alone at every index.
    let mut s = String::from("abc");
    insert_spaces(&mut s, 0, 0);
    assert_eq!(s, "abc");
    insert_spaces(&mut s, 2, 0);
    assert_eq!(s, "abc");
    insert_spaces(&mut s, 99, 0);
    assert_eq!(s, "abc");

    // Front, middle, and exact-end insertion.
    let mut s = String::from("abc");
    insert_spaces(&mut s, 0, 2);
    assert_eq!(s, "  abc");
    let mut s = String::from("abc");
    insert_spaces(&mut s, 1, 3);
    assert_eq!(s, "a   bc");
    let mut s = String::from("abc");
    insert_spaces(&mut s, 3, 2);
    assert_eq!(s, "abc  ");

    // Past the end appends rather than panicking — the contract
    // `place_in` leans on when a row is shorter than the x-offset.
    let mut s = String::from("abc");
    insert_spaces(&mut s, 99, 2);
    assert_eq!(s, "abc  ");

    // Empty buffer.
    let mut s = String::new();
    insert_spaces(&mut s, 0, 3);
    assert_eq!(s, "   ");
    let mut s = String::new();
    insert_spaces(&mut s, 5, 0);
    assert_eq!(s, "");

    // Mid-emoji: index 1 of a skin-toned 🙏🏻 is *after* the whole
    // cluster. A `char`-indexed insert would wedge the spaces
    // between the base and its U+1F3FB modifier and change the
    // rendered glyph.
    let mut s = String::from("🙏🏻x");
    insert_spaces(&mut s, 1, 1);
    assert_eq!(s, "🙏🏻 x");
    assert_eq!(count_grapheme_clusters(&s), 3);

    // ZWJ family: five scalars, one cluster. Inserting at 1 must
    // not sever a joiner.
    let mut s = String::from("👨‍👩‍👧👨‍👩‍👧");
    insert_spaces(&mut s, 1, 2);
    assert_eq!(s, "👨‍👩‍👧  👨‍👩‍👧");
    assert_eq!(count_grapheme_clusters(&s), 4);

    // Regional-indicator pair.
    let mut s = String::from("🇺🇸🇳🇴");
    insert_spaces(&mut s, 1, 1);
    assert_eq!(s, "🇺🇸 🇳🇴");
    assert_eq!(count_grapheme_clusters(&s), 3);

    // Combining mark: inserting at 1 of NFD "é" goes after the
    // acute, not between base and mark.
    let mut s = String::from("e\u{0301}x");
    insert_spaces(&mut s, 1, 1);
    assert_eq!(s, "e\u{0301} x");

    // Lone combining mark at index 0 is its own cluster, so index 1
    // is past it.
    let mut s = String::from("\u{0301}ab");
    insert_spaces(&mut s, 1, 1);
    assert_eq!(s, "\u{0301} ab");
    // Inserting *before* it must not merge the space with the mark:
    // the space becomes the mark's base, fusing them into one
    // cluster. Documented rather than wished away — the byte layout
    // is what the caller asked for.
    let mut s = String::from("\u{0301}ab");
    insert_spaces(&mut s, 0, 1);
    assert_eq!(s, " \u{0301}ab");

    // CRLF is one cluster; index 1 is past the whole pair.
    let mut s = String::from("a\r\nb");
    insert_spaces(&mut s, 2, 1);
    assert_eq!(s, "a\r\n b");

    // U+3000 is one cluster of three bytes.
    let mut s = String::from("\u{3000}x");
    insert_spaces(&mut s, 1, 1);
    assert_eq!(s, "\u{3000} x");

    // Property: the cluster count always grows by exactly `n` when
    // the buffer has no cluster-merging boundary at `idx`.
    for input in ["", "abc", "🙏🏻👨‍👩‍👧", "a\r\nb", "\u{3000}\u{200B}"] {
        let before = count_grapheme_clusters(input);
        for idx in 0..=before + 1 {
            for n in 0..3 {
                let mut s = input.to_string();
                insert_spaces(&mut s, idx, n);
                assert_eq!(
                    count_grapheme_clusters(&s),
                    before + n,
                    "{:?}: insert_spaces(idx={}, n={}) changed the cluster count wrongly",
                    input,
                    idx,
                    n
                );
            }
        }
    }
}

#[test]
pub fn test_split_graphemes_owned() {
    do_split_graphemes_owned();
}

/// [`split_graphemes_owned`] is the border-pattern splitter lifted
/// out of `border_pattern.rs`. Its contract is that concatenating the
/// pieces reproduces the input and every piece is exactly one
/// cluster.
pub fn do_split_graphemes_owned() {
    assert_eq!(split_graphemes_owned(""), Vec::<String>::new());
    assert_eq!(split_graphemes_owned("abc"), vec!["a", "b", "c"]);

    // One cluster each, however many scalars they carry.
    assert_eq!(split_graphemes_owned("🙏🏻"), vec!["🙏🏻"]);
    assert_eq!(split_graphemes_owned("👨‍👩‍👧"), vec!["👨‍👩‍👧"]);
    assert_eq!(split_graphemes_owned("🇺🇸🇳🇴"), vec!["🇺🇸", "🇳🇴"]);
    assert_eq!(split_graphemes_owned("e\u{0301}"), vec!["e\u{0301}"]);
    assert_eq!(split_graphemes_owned("\u{0301}"), vec!["\u{0301}"]);
    assert_eq!(split_graphemes_owned("\r\n"), vec!["\r\n"]);
    assert_eq!(
        split_graphemes_owned("\u{3000}\u{200B}"),
        vec!["\u{3000}", "\u{200B}"]
    );

    // The two properties every caller depends on.
    for input in [
        "",
        "abc",
        "─┬─",
        "🙏🏻👨‍👩‍👧🇺🇸",
        "a\r\nb",
        "\u{0301}x",
        "\u{3000}\u{200B}",
    ] {
        let parts = split_graphemes_owned(input);
        assert_eq!(parts.concat(), input, "round-trip failed for {:?}", input);
        assert_eq!(
            parts.len(),
            count_grapheme_clusters(input),
            "count for {:?}",
            input
        );
        for p in &parts {
            assert_eq!(
                count_grapheme_clusters(p),
                1,
                "{:?} split off a non-cluster {:?}",
                input,
                p
            );
        }
    }
}

#[test]
pub fn test_join_graphemes() {
    do_join_graphemes();
}

/// [`join_graphemes`] backs the vertical border rail, which stacks
/// one cluster per row. Joining by `char` instead would drop a
/// combining mark onto the row below its base and shatter a ZWJ
/// emoji into its parts.
pub fn do_join_graphemes() {
    assert_eq!(join_graphemes("", "\n"), "");
    assert_eq!(join_graphemes("a", "\n"), "a");
    assert_eq!(join_graphemes("ab", "\n"), "a\nb");
    assert_eq!(join_graphemes("abc", "-"), "a-b-c");

    // An empty separator rebuilds the input verbatim.
    assert_eq!(join_graphemes("🙏🏻👨‍👩‍👧", ""), "🙏🏻👨‍👩‍👧");

    // Multi-scalar clusters stay whole and land on one row each.
    assert_eq!(join_graphemes("🙏🏻👨‍👩‍👧", "\n"), "🙏🏻\n👨‍👩‍👧");
    assert_eq!(join_graphemes("🇺🇸🇳🇴", "\n"), "🇺🇸\n🇳🇴");
    // The acute must stay with its base rather than starting a row.
    assert_eq!(join_graphemes("e\u{0301}x", "\n"), "e\u{0301}\nx");
    // A CRLF is one cluster, so it is one row, not two.
    assert_eq!(join_graphemes("a\r\nb", "|"), "a|\r\n|b");
    // Multi-character separators work too.
    assert_eq!(join_graphemes("ab", "<>"), "a<>b");

    // Property: the joined string always holds `n - 1` separators
    // and drops back to the input when they are stripped.
    for input in ["", "a", "abc", "🙏🏻👨‍👩‍👧🇺🇸", "e\u{0301}x", "\u{3000}\u{200B}"] {
        let n = count_grapheme_clusters(input);
        let joined = join_graphemes(input, "\n");
        assert_eq!(
            joined.matches('\n').count(),
            n.saturating_sub(1),
            "separator count for {:?}",
            input
        );
        assert_eq!(joined.replace('\n', ""), input, "round-trip for {:?}", input);
    }
}
