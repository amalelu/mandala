use lazy_static::lazy_static;

use crate::util::grapheme_chad::{
   count_grapheme_clusters, count_number_lines, delete_back_unicode, delete_front_unicode,
   delete_grapheme_at, find_byte_index_of_grapheme, find_nth_line_byte_range,
   find_nth_line_grapheme_range, insert_new_lines, insert_str_at_grapheme, push_spaces,
   replace_graphemes_until_newline, slice_to_newline, split_off_graphemes,
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

        pub static ref NTH_LINE_BYTE_INDICES_TEST: Vec<(&'static str, usize, Option<(usize, usize)>)> = vec![
            ("\n", 0, Some((0, 0))),
            ("", 0, None),
            ("a", 0, Some((0,1))),
            ("a\n", 1, None),
            ("a\n", 0, Some((0,1))),
            ("", 1, None),
            ("Hello\nxxxxxxxxxxqqqqqqqqqqxxxxxxxxxxqqqqqqqqqq\n", 1, Some((6, 46))),
            ("Hello\nxxxxxxxxxxqqqqqqqqqqxxxxxxxxxxqqqqqqqqqq", 1, Some((6, 46))),
            ("\n\n", 1, Some((1, 1))),
            ("\nhi\n", 1, Some((1, 3))),
            ("\nhi\n", 2, None),
            ("\nhi\na\nb\nc\nd", 4, Some((8, 9))),
            ("\n🙏🏻\n🙏🏻\n", 2, Some((1+8+1, 1+8+1+8))),
            ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 1, Some((8*10+1, (8*10+1)+8*10))),
            ("🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻", 2, Some((20*8 + 2, (20*8 + 2) + 10*8))),
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
      if expected_byte_index.is_some() && expected_byte_index.clone().unwrap() >= string.len()
      {
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
      replace_graphemes_until_newline(&mut result, idx, &source);
      assert_eq!(result, expected);
   }
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
pub fn test_find_nth_line_byte_indices() {
   do_find_nth_line_byte_indices();
}

pub fn do_find_nth_line_byte_indices() {
   for (str, n, idx) in NTH_LINE_BYTE_INDICES_TEST.clone() {
      let result = find_nth_line_byte_range(str, n);
      assert_eq!(result, idx);
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
