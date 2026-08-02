// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::gfx_structs::model::GlyphModel`] — line/matrix
//! construction and component layout (§T1).

use crate::core::primitives::{Applicable, ApplyOperation, ColorFontRegion, ColorFontRegions, Range};
use crate::font::fonts::AppFont;
use crate::gfx_structs::model::{
    DeltaGlyphModel, GlyphComponent, GlyphLine, GlyphMatrix, GlyphModel, GlyphModelField,
};
use crate::util::color::Color;
use crate::util::grapheme_chad::{count_grapheme_clusters, count_number_lines, find_byte_index_of_grapheme};

/// The tests are written in a non-test-annotated function and then wrapped by an annotated test function
/// So that they can be reused for benchmarking

#[test]
pub fn test_matrix_place_in_1() {
    matrix_place_in_1();
}

pub fn matrix_place_in_1() {
    let mut matrix = GlyphMatrix::new();
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));

    let mut regions = ColorFontRegions::new_empty();
    let mut my_string = String::new();

    matrix.place_in(&mut my_string, &mut regions, (10, 10));
    assert_matrix_place_in(&my_string, &regions);
    // Just asserting that the operation is idempotent
    matrix.place_in(&mut my_string, &mut regions, (10, 10));
    assert_matrix_place_in(&my_string, &regions);
    matrix.place_in(&mut my_string, &mut regions, (10, 10));
    assert_matrix_place_in(&my_string, &regions);
    matrix.place_in(&mut my_string, &mut regions, (10, 10));
    assert_matrix_place_in(&my_string, &regions);
    matrix.place_in(&mut my_string, &mut regions, (10, 10));
    assert_matrix_place_in(&my_string, &regions);
}

fn assert_matrix_place_in(my_string: &String, regions: &ColorFontRegions) {
    assert_eq!(
        my_string,
        "\n\n\n\n\n\n\n\n\n\n          ##########\n          ##########\n          ##########"
    );
    assert_eq!(regions.num_regions(), 3);
    let first_region = regions.regions.first();
    assert_eq!(first_region.is_some(), true);
    let unwrapped_first_region = first_region.unwrap();
    assert_eq!(unwrapped_first_region.range, Range::new(20, 30));
    assert_eq!(unwrapped_first_region.color.unwrap(), Color::black().to_float());
    assert_eq!(unwrapped_first_region.font.unwrap(), AppFont::Evilz);
    let second_region = regions.get(Range::new(41, 51));
    assert_eq!(second_region.is_some(), true);
    let unwrapped_second_region = second_region.unwrap();
    assert_eq!(unwrapped_second_region.range, Range::new(41, 51));
    assert_eq!(unwrapped_second_region.color.unwrap(), Color::black().to_float());
    assert_eq!(unwrapped_second_region.font.unwrap(), AppFont::Evilz);
    let third_region = regions.get(Range::new(62, 72));
    assert_eq!(third_region.is_some(), true);
    let unwrapped_third_region = third_region.unwrap();
    assert_eq!(unwrapped_third_region.range, Range::new(62, 72));
    assert_eq!(unwrapped_third_region.color.unwrap(), Color::black().to_float());
    assert_eq!(unwrapped_third_region.font.unwrap(), AppFont::Evilz);
}

#[test]
pub fn test_matrix_place_in_2() {
    matrix_place_in_2();
}

pub fn matrix_place_in_2() {
    let mut matrix_a = GlyphMatrix::new();
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));

    let mut matrix_b = GlyphMatrix::new();
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));

    let mut regions = ColorFontRegions::new_empty();
    let mut my_string = String::new();
    matrix_a.place_in(&mut my_string, &mut regions, (0, 0));
    assert_eq!(my_string, "##########\n##########\n##########");
    assert_eq!(regions.num_regions(), 3);
    {
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(11, 21)).unwrap();
        let _region_3 = regions.get(Range::new(22, 32)).unwrap();
    }
    matrix_b.place_in(&mut my_string, &mut regions, (10, 0));
    assert_eq!(
        my_string,
        "##########@@@@@@@@@@\n##########@@@@@@@@@@\n##########@@@@@@@@@@"
    );
    assert_eq!(regions.num_regions(), 6);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        // New regions
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
    }
    matrix_a.place_in(&mut my_string, &mut regions, (10, 3));
    assert_eq!(my_string, "##########@@@@@@@@@@\n##########@@@@@@@@@@\n##########@@@@@@@@@@\n          ##########\n          ##########\n          ##########");
    assert_eq!(regions.num_regions(), 9);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
        // new regions
        let _region_7 = regions.get(Range::new(73, 83)).unwrap();
        let _region_8 = regions.get(Range::new(94, 104)).unwrap();
        let _region_9 = regions.get(Range::new(115, 125)).unwrap();
    }
    matrix_b.place_in(&mut my_string, &mut regions, (0, 3));
    assert_eq!(my_string, "##########@@@@@@@@@@\n##########@@@@@@@@@@\n##########@@@@@@@@@@\n@@@@@@@@@@##########\n@@@@@@@@@@##########\n@@@@@@@@@@##########");
    assert_eq!(regions.num_regions(), 12);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
        let _region_7 = regions.get(Range::new(73, 83)).unwrap();
        let _region_8 = regions.get(Range::new(94, 104)).unwrap();
        let _region_9 = regions.get(Range::new(115, 125)).unwrap();
        // new regions
        let _region_10 = regions.get(Range::new(63, 73)).unwrap();
        let _region_11 = regions.get(Range::new(84, 94)).unwrap();
        let _region_12 = regions.get(Range::new(105, 115)).unwrap();
    }
}

#[test]
pub fn test_matrix_place_in_3() {
    matrix_place_in_3();
}

pub fn matrix_place_in_3() {
    let mut matrix_a = GlyphMatrix::new();
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
        AppFont::AlphaMusicMan,
        Color::black(),
    )));
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
        AppFont::AliceInWonderland,
        Color::black(),
    )));

    let mut matrix_b = GlyphMatrix::new();
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));

    let mut regions = ColorFontRegions::new_empty();
    let mut my_string = String::new();
    matrix_a.place_in(&mut my_string, &mut regions, (0, 0));
    assert_eq!(
        my_string,
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"
    );
    assert_eq!(regions.num_regions(), 3);
    {
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(11, 21)).unwrap();
        let _region_3 = regions.get(Range::new(22, 32)).unwrap();
    }
    matrix_b.place_in(&mut my_string, &mut regions, (10, 0));
    assert_eq!(
        my_string,
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@"
    );
    assert_eq!(regions.num_regions(), 6);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        // New regions
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
    }
    matrix_a.place_in(&mut my_string, &mut regions, (10, 3));
    assert_eq!(my_string,
              "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
    assert_eq!(regions.num_regions(), 9);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
        // new regions
        let _region_7 = regions.get(Range::new(73, 83)).unwrap();
        let _region_8 = regions.get(Range::new(94, 104)).unwrap();
        let _region_9 = regions.get(Range::new(115, 125)).unwrap();
    }
    matrix_b.place_in(&mut my_string, &mut regions, (0, 3));
    assert_eq!(my_string, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
    assert_eq!(regions.num_regions(), 12);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
        let _region_7 = regions.get(Range::new(73, 83)).unwrap();
        let _region_8 = regions.get(Range::new(94, 104)).unwrap();
        let _region_9 = regions.get(Range::new(115, 125)).unwrap();
        // new regions
        let _region_10 = regions.get(Range::new(63, 73)).unwrap();
        let _region_11 = regions.get(Range::new(84, 94)).unwrap();
        let _region_12 = regions.get(Range::new(105, 115)).unwrap();
    }
}

#[test]
pub fn test_matrix_place_in_multiline_component() {
    matrix_place_in_multiline_component();
}

/// A `GlyphComponent`'s text is arbitrary and may itself contain
/// newlines. Painting such a row grows the target by a line, and
/// every row after it has to move down with it.
///
/// Before `replace_graphemes_until_newline` reported
/// `LineReplacement::added_lines` (issue #38 item 4) `place_in` had no
/// way to know: it kept addressing target row `n + 1` as
/// `line_num + offset.1`, which after a multi-line paint pointed
/// *inside* the text it had just inserted, and row 1 overwrote the
/// tail of row 0.
///
/// The CRLF cases guard the other half of that agreement. `place_in`
/// counts drift in `\n` *characters* (via `count_number_lines`) but
/// addresses rows through `find_nth_line_grapheme_range`; UAX #29
/// fuses `\r\n` into a single cluster, so unless the addresser treats
/// any cluster ending in `\n` as a terminator the two disagree and
/// every row after a CRLF-bearing one is looked up a line too far
/// down, falls off the end, and is appended onto its predecessor.
pub fn matrix_place_in_multiline_component() {
    let mut matrix = GlyphMatrix::new();
    // Row 0 carries an author line break inside one component.
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "AA\nBB",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "CC",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "DD",
        AppFont::Evilz,
        Color::black(),
    )));

    let mut regions = ColorFontRegions::new_empty();
    let mut my_string = String::new();
    matrix.place_in(&mut my_string, &mut regions, (0, 0));

    // Four target lines for three matrix rows: row 0 contributed two
    // of them. Rows 1 and 2 land *below* row 0's second line rather
    // than on top of it.
    assert_eq!(my_string, "AA\nBB\nCC\nDD");
    assert_eq!(count_number_lines(&my_string), 4);

    // Regions stay in whole-buffer grapheme coordinates and none of
    // them was clobbered by the drift.
    assert_eq!(regions.num_regions(), 3);
    assert!(
        regions.get(Range::new(0, 5)).is_some(),
        "row 0 spans its own newline"
    );
    assert!(
        regions.get(Range::new(6, 8)).is_some(),
        "row 1 shifted past row 0"
    );
    assert!(
        regions.get(Range::new(9, 11)).is_some(),
        "row 2 shifted past both"
    );

    // The same paint with a y-offset keeps the drift relative to the
    // offset rather than swallowing it.
    let mut regions = ColorFontRegions::new_empty();
    let mut offset_string = String::new();
    matrix.place_in(&mut offset_string, &mut regions, (0, 2));
    assert_eq!(offset_string, "\n\nAA\nBB\nCC\nDD");
    assert_eq!(regions.num_regions(), 3);

    // Two newlines in one component drift the rows below by two.
    let mut deep = GlyphMatrix::new();
    deep.push(GlyphLine::new_with(GlyphComponent::text(
        "A\nB\nC",
        AppFont::Evilz,
        Color::black(),
    )));
    deep.push(GlyphLine::new_with(GlyphComponent::text(
        "D",
        AppFont::Evilz,
        Color::black(),
    )));
    let mut regions = ColorFontRegions::new_empty();
    let mut deep_string = String::new();
    deep.place_in(&mut deep_string, &mut regions, (0, 0));
    assert_eq!(deep_string, "A\nB\nC\nD");

    // An x-offset still pads each row it actually paints into.
    let mut regions = ColorFontRegions::new_empty();
    let mut padded = String::new();
    matrix.place_in(&mut padded, &mut regions, (2, 0));
    assert_eq!(padded, "  AA\nBB\n  CC\n  DD");

    // The same three rows with a Windows line ending in row 0. One
    // cluster, one added line — rows 1 and 2 must still land on their
    // own lines rather than collide at the end of the buffer.
    let mut crlf = GlyphMatrix::new();
    for text in ["AA\r\nBB", "CC", "DD"] {
        crlf.push(GlyphLine::new_with(GlyphComponent::text(
            text,
            AppFont::Evilz,
            Color::black(),
        )));
    }

    let mut regions = ColorFontRegions::new_empty();
    let mut crlf_string = String::new();
    crlf.place_in(&mut crlf_string, &mut regions, (0, 0));
    assert_eq!(crlf_string, "AA\r\nBB\nCC\nDD");
    assert_eq!(count_number_lines(&crlf_string), 4);
    assert_eq!(regions.num_regions(), 3);
    assert!(
        regions.get(Range::new(0, 5)).is_some(),
        "CRLF row 0 spans its own terminator"
    );
    assert!(
        regions.get(Range::new(6, 8)).is_some(),
        "CRLF row 1 shifted past row 0"
    );
    assert!(
        regions.get(Range::new(9, 11)).is_some(),
        "CRLF row 2 shifted past both"
    );

    let mut regions = ColorFontRegions::new_empty();
    let mut crlf_offset = String::new();
    crlf.place_in(&mut crlf_offset, &mut regions, (0, 1));
    assert_eq!(crlf_offset, "\nAA\r\nBB\nCC\nDD");

    let mut regions = ColorFontRegions::new_empty();
    let mut crlf_padded = String::new();
    crlf.place_in(&mut crlf_padded, &mut regions, (2, 0));
    assert_eq!(crlf_padded, "  AA\r\nBB\n  CC\n  DD");

    // A component that is nothing but a CRLF still contributes
    // exactly one line of drift.
    let mut bare = GlyphMatrix::new();
    for text in ["\r\n", "CC", "DD"] {
        bare.push(GlyphLine::new_with(GlyphComponent::text(
            text,
            AppFont::Evilz,
            Color::black(),
        )));
    }
    let mut regions = ColorFontRegions::new_empty();
    let mut bare_string = String::new();
    bare.place_in(&mut bare_string, &mut regions, (0, 0));
    assert_eq!(bare_string, "\r\n\nCC\nDD");

    // Two CRLFs in one component drift the rows below by two.
    let mut deep_crlf = GlyphMatrix::new();
    for text in ["A\r\nB\r\nC", "DD", "EE"] {
        deep_crlf.push(GlyphLine::new_with(GlyphComponent::text(
            text,
            AppFont::Evilz,
            Color::black(),
        )));
    }
    let mut regions = ColorFontRegions::new_empty();
    let mut deep_crlf_string = String::new();
    deep_crlf.place_in(&mut deep_crlf_string, &mut regions, (0, 0));
    assert_eq!(deep_crlf_string, "A\r\nB\r\nC\nDD\nEE");

    // A lone CR is not a line terminator and must not be counted as
    // drift by either side of the bookkeeping.
    let mut lone_cr = GlyphMatrix::new();
    for text in ["AA\rBB", "CC", "DD"] {
        lone_cr.push(GlyphLine::new_with(GlyphComponent::text(
            text,
            AppFont::Evilz,
            Color::black(),
        )));
    }
    let mut regions = ColorFontRegions::new_empty();
    let mut lone_cr_string = String::new();
    lone_cr.place_in(&mut lone_cr_string, &mut regions, (0, 0));
    assert_eq!(lone_cr_string, "AA\rBB\nCC\nDD");

    // Painting into a target that *already* carries a Windows line
    // ending. This is the composition case the module header names —
    // several models sharing one buffer — and it is where addressing
    // rows by the cluster line model while writing them by the raw
    // `\n` byte comes apart: the overwritten line tail would swallow
    // the CR, turning the CRLF into a bare LF and under-reporting the
    // region shift by one cluster.
    let mut over_crlf = GlyphMatrix::new();
    over_crlf.push(GlyphLine::new_with(GlyphComponent::text(
        "ABCDEFG",
        AppFont::Evilz,
        Color::black(),
    )));
    let mut regions = ColorFontRegions::new_empty();
    // The pre-existing region sits on the second line, after the
    // terminator, so the paint must slide it right by the full
    // seven-minus-four = three clusters the first line gained.
    regions.submit_region(ColorFontRegion::new(
        Range::new(5, 9),
        Some(AppFont::Evilz),
        Some(Color::black().to_float()),
    ));
    let mut existing = String::from("XXXX\r\nYYYY");
    over_crlf.place_in(&mut existing, &mut regions, (0, 0));
    assert_eq!(
        existing, "ABCDEFG\r\nYYYY",
        "the target's CRLF terminator must survive being painted over"
    );
    assert!(
        regions.get(Range::new(8, 12)).is_some(),
        "the trailing region must shift by the real cluster growth (3), not by 2"
    );

    // The same paint at a *non-zero* x-offset. `place_in` pads short
    // rows out to the offset with `insert_spaces`, which puts clusters
    // into the middle of the buffer — every row but the last has text
    // after it. Until the padding was reported to `regions`, the
    // caller span below simply stopped pointing at its text: it stayed
    // at 5..9 and sliced to "   A" instead of "YYYY". The `(0, 0)`
    // case above walks straight past that, which is why this one
    // exists.
    let mut x_offset = GlyphMatrix::new();
    x_offset.push(GlyphLine::new_with(GlyphComponent::text(
        "AB",
        AppFont::Evilz,
        Color::black(),
    )));
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(5, 9),
        Some(AppFont::Evilz),
        Some(Color::black().to_float()),
    ));
    let mut padded = String::from("XXXX\r\nYYYY");
    x_offset.place_in(&mut padded, &mut regions, (8, 0));
    assert_eq!(padded, "XXXX    AB\r\nYYYY");
    assert_eq!(count_grapheme_clusters(&padded), 15);
    assert!(
        regions.get(Range::new(11, 15)).is_some(),
        "the caller span must ride the four inserted padding clusters, landing on \"YYYY\" again"
    );
    assert!(
        regions.get(Range::new(8, 10)).is_some(),
        "the painted component owns the two cells it wrote"
    );
    // Spelled out against the buffer so a future change to the region
    // arithmetic cannot keep the index while losing the text.
    let span_start = find_byte_index_of_grapheme(&padded, 11).unwrap();
    assert_eq!(&padded[span_start..], "YYYY");

    // The same paint with the caller span anchored **exactly at** the
    // padding point rather than after it. This is the case that tells
    // the two candidate primitives apart: the padding lands at cell 4
    // and the span starts at cell 4, so `start >= idx` moves it and
    // `shift_regions_after`'s strict `start > idx` does not. The span
    // above starts at 5 and rides either rule, which is why swapping
    // `place_in`'s padding call for `shift_regions_after` used to
    // leave the whole suite green.
    //
    // The component's own write is the second half of the same
    // question. It lands at cell 8 into an *empty* line tail — a pure
    // insertion, not an overwrite — so the caller span, now sitting at
    // 8..10, has to move again. `shift_regions_after` left it at 8..10
    // and the component's `submit_region(8..10)` then evicted it: the
    // region set is keyed on the range alone, so the caller's pin
    // vanished from a buffer that still contained its text.
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(4, 6),
        Some(AppFont::Evilz),
        Some(Color::black().to_float()),
    ));
    let mut at_pad = String::from("XXXX\r\nYYYY");
    x_offset.place_in(&mut at_pad, &mut regions, (8, 0));
    assert_eq!(at_pad, "XXXX    AB\r\nYYYY");
    assert_eq!(
        regions.num_regions(),
        2,
        "the caller's span and the component's span both survive; neither evicts the other"
    );
    assert!(
        regions.get(Range::new(10, 12)).is_some(),
        "a caller span anchored at the padding point rides the four padding cells and then the \
         two the component inserted: 4..6 -> 8..10 -> 10..12"
    );
    assert!(
        regions.get(Range::new(8, 10)).is_some(),
        "the component still owns exactly the two cells it wrote"
    );
    let span_start = find_byte_index_of_grapheme(&at_pad, 10).unwrap();
    let span_end = find_byte_index_of_grapheme(&at_pad, 12).unwrap();
    assert_eq!(
        &at_pad[span_start..span_end],
        "\r\nY",
        "and it still covers the same two clusters it started on"
    );

    // The same paint with the caller span **straddling** the padding
    // point. This is the third seam, and the one a `start >= idx`
    // shift gets wrong: the padding lands at cell 4 in the middle of a
    // span covering 2..6, so two of the cells it describes stay where
    // they are and two move four cells right. Shifting the whole span
    // is not an option (its head did not move) and leaving it whole is
    // not either — it would then cover `"XX  "`, two of its own cells
    // plus two indent blanks, and drop the `"\r\nY"` that moved. The
    // run has to split around the blanks, which is what
    // `split_and_separate` is for.
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(2, 6),
        Some(AppFont::Evilz),
        Some(Color::black().to_float()),
    ));
    let mut straddling = String::from("XXXX\r\nYYYY");
    x_offset.place_in(&mut straddling, &mut regions, (8, 0));
    assert_eq!(straddling, "XXXX    AB\r\nYYYY");
    assert_eq!(
        regions.num_regions(),
        3,
        "the caller's span is now two — head and tail — plus the component's own"
    );
    assert!(
        regions.get(Range::new(2, 4)).is_some(),
        "the head keeps the two cells left of the padding, which never moved"
    );
    assert!(
        regions.get(Range::new(10, 12)).is_some(),
        "and the two that did move ride the four padding cells and then the two the component \
         inserted: 4..6 -> 8..10 -> 10..12"
    );
    assert!(
        regions.get(Range::new(8, 10)).is_some(),
        "the component still owns exactly the two cells it wrote"
    );
    // Spelled out against the buffer: head plus tail is the caller's
    // original "XX\r\nY", with the indent blanks it never covered
    // excluded rather than swallowed.
    let head_start = find_byte_index_of_grapheme(&straddling, 2).unwrap();
    let head_end = find_byte_index_of_grapheme(&straddling, 4).unwrap();
    let tail_start = find_byte_index_of_grapheme(&straddling, 10).unwrap();
    let tail_end = find_byte_index_of_grapheme(&straddling, 12).unwrap();
    assert_eq!(&straddling[head_start..head_end], "XX");
    assert_eq!(&straddling[tail_start..tail_end], "\r\nY");

    // The padding must *shift* spans without letting one that merely
    // ends at the padding point absorb the blanks. The blanks are the
    // indent of the row about to be painted — they belong to no run
    // and to no component — so absorbing them stretches a span across
    // a line terminator onto another row's leading whitespace.
    //
    // Three rows, the first of which is a bare CR, painted at x-offset
    // 2. Row 0's component lands at cell 2, fuses with the padding LF
    // into one `"\r\n"` cluster, and correctly owns 2..3. Row 1 then
    // needs two indent cells at cell 3 — exactly where row 0's span
    // ends. `insert_regions_at`'s left-adjacent absorption grew it to
    // 2..5, pinning row 0's font and color onto row 1's indent.
    let mut cr_rows = GlyphMatrix::new();
    for text in ["\r", "", ""] {
        cr_rows.push(GlyphLine::new_with(GlyphComponent::text(
            text,
            AppFont::Evilz,
            Color::black(),
        )));
    }
    let mut regions = ColorFontRegions::new_empty();
    let mut cr_padded = String::new();
    cr_rows.place_in(&mut cr_padded, &mut regions, (2, 0));
    assert_eq!(cr_padded, "  \r\n  \n  ");
    assert_eq!(
        regions.num_regions(),
        1,
        "only row 0 wrote a cell; the two empty rows own nothing and emit nothing"
    );
    assert!(
        regions.get(Range::new(2, 3)).is_some(),
        "row 0's span must stop at its own terminator, not absorb row 1's indent"
    );
    let span_start = find_byte_index_of_grapheme(&cr_padded, 2).unwrap();
    let span_end = find_byte_index_of_grapheme(&cr_padded, 3).unwrap();
    assert_eq!(
        &cr_padded[span_start..span_end],
        "\r\n",
        "the CR the component wrote plus the LF it fused with — and nothing of the next row"
    );
}

#[test]
pub fn test_matrix_place_in_fusing_component() {
    matrix_place_in_fusing_component();
}

/// A component whose text begins with a cluster-extending scalar
/// fuses into the cell before it, so the buffer *shrinks* where the
/// arithmetic says it grows. That is the negative half of
/// `LineReplacement::growth` and the `shrink_regions_after` arm of
/// `place_in` — reachable in three lines, and unexercised by every
/// other test in this file, all of whose components start on a
/// non-combining scalar.
pub fn matrix_place_in_fusing_component() {
    let mut matrix = GlyphMatrix::new();
    let mut line = GlyphLine::new();
    line.push(GlyphComponent::text("ab", AppFont::Evilz, Color::black()));
    // U+0301 COMBINING ACUTE ACCENT: not a cell of its own, it
    // extends the cell before it.
    line.push(GlyphComponent::text("\u{301}", AppFont::Evilz, Color::black()));
    matrix.push(line);

    let mut regions = ColorFontRegions::new_empty();
    // A caller span on the text after the paint. The buffer loses one
    // cluster to the fusion, so this must move *left* by one.
    regions.submit_region(ColorFontRegion::new(
        Range::new(4, 8),
        Some(AppFont::Evilz),
        Some(Color::black().to_float()),
    ));
    let mut buffer = String::from("XXXXXXXX");
    matrix.place_in(&mut buffer, &mut regions, (0, 0));

    assert_eq!(buffer, "ab\u{301}XXXXX");
    assert_eq!(
        count_grapheme_clusters(&buffer),
        7,
        "\"b\" and the accent are one cell: eight clusters became seven"
    );
    assert!(
        regions.get(Range::new(3, 7)).is_some(),
        "the caller span must follow the shrink; a grow-only `place_in` leaves it at 4..8, one cell past its text"
    );
    assert!(
        regions.get(Range::new(4, 8)).is_none(),
        "the pre-shrink span must not survive"
    );
    let span_start = find_byte_index_of_grapheme(&buffer, 3).unwrap();
    assert_eq!(
        &buffer[span_start..],
        "XXXX",
        "the span 3..7 still covers the four X's the caller pinned, now one cell to the left"
    );
    assert!(
        regions.get(Range::new(0, 2)).is_some(),
        "\"ab\" owns the two cells it wrote, including the one the accent joined"
    );
    assert!(
        regions.get(Range::new(2, 2)).is_none(),
        "a component that only extends the cell before it owns no cell of its own, so no region is \
         emitted for it at all — a degenerate `2..2` would violate the region primitives' \
         \"non-degenerate\" precondition by construction"
    );
    assert_eq!(
        regions.num_regions(),
        2,
        "one component span plus the caller's, and nothing else"
    );

    // A third component makes the *head* observable. Until this row
    // existed, `comp_head` could be reverted to `comp_head +=
    // component.length()` — isolated cluster arithmetic, the exact
    // quantity the measured span replaced — and the suite stayed
    // green, because the fusing component was last in its line and
    // nothing ever consumed the head it left behind.
    let mut matrix = GlyphMatrix::new();
    let mut line = GlyphLine::new();
    line.push(GlyphComponent::text("ab", AppFont::Evilz, Color::black()));
    line.push(GlyphComponent::text("\u{301}", AppFont::Evilz, Color::black()));
    line.push(GlyphComponent::text("Z", AppFont::Evilz, Color::black()));
    matrix.push(line);

    let mut regions = ColorFontRegions::new_empty();
    let mut buffer = String::from("XXXXXXXX");
    matrix.place_in(&mut buffer, &mut regions, (0, 0));

    assert_eq!(buffer, "ab\u{301}ZXXXX");
    assert_eq!(
        regions.num_regions(),
        2,
        "\"ab\" and \"Z\"; the accent owns no cell"
    );
    assert!(regions.get(Range::new(0, 2)).is_some());
    assert!(
        regions.get(Range::new(2, 3)).is_some(),
        "\"Z\" lands on cell 2, the cell after the fused one. With an arithmetic head the accent \
         would have advanced it to 3 and \"Z\" would claim a cell of the target instead"
    );
    let span_start = find_byte_index_of_grapheme(&buffer, 2).unwrap();
    let span_end = find_byte_index_of_grapheme(&buffer, 3).unwrap();
    assert_eq!(&buffer[span_start..span_end], "Z");

    // Two components that both fuse into the same cell. Both own no
    // cell, so both emit nothing — which is also what keeps the region
    // set free of key collisions: while `place_in` emitted a
    // degenerate `k..k` for each, the second silently evicted the
    // first's font and color pin from the set, because
    // `ColorFontRegions` is keyed on the range alone.
    let mut matrix = GlyphMatrix::new();
    let mut line = GlyphLine::new();
    line.push(GlyphComponent::text("ab", AppFont::Evilz, Color::black()));
    line.push(GlyphComponent::text("\u{301}", AppFont::African, Color::white()));
    line.push(GlyphComponent::text("\u{302}", AppFont::Evilz, Color::black()));
    matrix.push(line);

    let mut regions = ColorFontRegions::new_empty();
    let mut buffer = String::from("XXXXXXXX");
    matrix.place_in(&mut buffer, &mut regions, (0, 0));

    assert_eq!(buffer, "ab\u{301}\u{302}XXXX");
    assert_eq!(
        regions.num_regions(),
        1,
        "only \"ab\" owns cells; two cell-less components collapsed into one set slot before, \
         which discarded a pin that had been submitted"
    );
    assert!(regions.get(Range::new(0, 2)).is_some());
    assert!(regions.get(Range::new(2, 2)).is_none());
}

#[test]
pub fn test_matrix_add_assign_2() {
    matrix_add_assign_2();
}

pub fn matrix_add_assign_2() {
    let mut matrix = GlyphMatrix::new();

    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));

    let mut modifier_matrix = GlyphMatrix::new();
    modifier_matrix.push(GlyphLine::new());
    modifier_matrix.push(GlyphLine::new());
    modifier_matrix.push(GlyphLine::new());

    modifier_matrix.push(GlyphLine::new_with_vec(
        vec![
            GlyphComponent::space(3),
            GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
        ],
        true,
    ));

    modifier_matrix.push(GlyphLine::new_with_vec(
        vec![
            GlyphComponent::space(3),
            GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
        ],
        true,
    ));

    modifier_matrix.push(GlyphLine::new_with_vec(
        vec![
            GlyphComponent::space(3),
            GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
        ],
        true,
    ));

    modifier_matrix.push(GlyphLine::new_with_vec(
        vec![
            GlyphComponent::space(3),
            GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
        ],
        true,
    ));

    matrix += modifier_matrix;

    assert_eq!(matrix.get(0).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(1).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(2).unwrap().get(0).unwrap().text, "##########");

    assert_eq!(matrix.get(3).unwrap().get(0).unwrap().text, "###");
    assert_eq!(matrix.get(3).unwrap().get(1).unwrap().text, "HELP");
    assert_eq!(matrix.get(3).unwrap().get(2).unwrap().text, "###");

    assert_eq!(matrix.get(4).unwrap().get(0).unwrap().text, "###");
    assert_eq!(matrix.get(4).unwrap().get(1).unwrap().text, "HELP");
    assert_eq!(matrix.get(4).unwrap().get(2).unwrap().text, "###");

    assert_eq!(matrix.get(5).unwrap().get(0).unwrap().text, "###");
    assert_eq!(matrix.get(5).unwrap().get(1).unwrap().text, "HELP");
    assert_eq!(matrix.get(5).unwrap().get(2).unwrap().text, "###");

    assert_eq!(matrix.get(6).unwrap().get(0).unwrap().text, "###");
    assert_eq!(matrix.get(6).unwrap().get(1).unwrap().text, "HELP");
    assert_eq!(matrix.get(6).unwrap().get(2).unwrap().text, "###");

    assert_eq!(matrix.get(7).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(8).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(9).unwrap().get(0).unwrap().text, "##########");
}

#[test]
pub fn test_matrix_add_assign_1() {
    matrix_add_assign_1();
}

pub fn matrix_add_assign_1() {
    let mut matrix = create_default_matrix();

    let mut modifier_matrix = GlyphMatrix::new();
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));

    matrix += modifier_matrix;

    assert_eq!(matrix.get(0).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(0).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(1).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(1).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(2).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(2).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(3).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(3).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(4).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(5).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(6).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(7).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(8).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(9).unwrap().get(0).unwrap().text, "##########");
}

#[test]
pub fn test_matrix_mul_assign_1() {
    matrix_mul_assign_1();
}

pub fn matrix_mul_assign_1() {
    let mut matrix = create_default_matrix();

    let mut modifier_matrix = GlyphMatrix::new();
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));

    matrix *= modifier_matrix;

    assert_eq!(matrix.get(0).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(0).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(1).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(1).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(2).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(2).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(3).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(3).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(4).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(5).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(6).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(7).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(8).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(9).unwrap().get(0).unwrap().text, "##########");
}

fn create_default_matrix() -> GlyphMatrix {
    let mut matrix = GlyphMatrix::new();

    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));

    return matrix;
}

#[test]
pub fn test_line_add_assign_1() {
    line_add_assign_1();
}

// Pretty straight forward, simple test case
pub fn line_add_assign_1() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text(
        "##########",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "**********",
        AppFont::AppleTea,
        Color::black(),
    ));

    let mut modifier_line = GlyphLine::new();
    modifier_line.push(GlyphComponent::text(
        "!!!!!!!!!!!",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line += modifier_line;
    assert_eq!(glyph_line.get(0).unwrap().text, "!!!!!!!!!!!");
    assert_eq!(glyph_line.get(1).unwrap().text, "@@@@@@@@@");
    assert_eq!(glyph_line.get(2).unwrap().text, "**********");
}

#[test]
pub fn test_line_add_assign_2() {
    line_add_assign_2();
}

// Here we test the ability to ignore initial whitespace, while also respecting the indices
pub fn line_add_assign_2() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text(
        "##########",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "**********",
        AppFont::AppleTea,
        Color::black(),
    ));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(20));
    modifier_line.push(GlyphComponent::text(
        "!!!!!!!!!!",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line += modifier_line;
    assert_eq!(glyph_line.get(0).unwrap().text, "##########");
    assert_eq!(glyph_line.get(1).unwrap().text, "@@@@@@@@@@");
    assert_eq!(glyph_line.get(2).unwrap().text, "!!!!!!!!!!");
}

#[test]
pub fn test_line_add_assign_3() {
    line_add_assign_3();
}

// Here we test the ability to handle emojis
pub fn line_add_assign_3() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text(
        "##########",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "**********",
        AppFont::AppleTea,
        Color::black(),
    ));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(20));
    modifier_line.push(GlyphComponent::text(
        "🍕🍕🍕🍕🍕🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line += modifier_line;
    assert_eq!(glyph_line.get(0).unwrap().text, "##########");
    assert_eq!(glyph_line.get(1).unwrap().text, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
    assert_eq!(glyph_line.get(2).unwrap().text, "🍕🍕🍕🍕🍕🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
}

#[test]
pub fn test_line_add_assign_4() {
    line_add_assign_4();
}

pub fn line_add_assign_4() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text(
        "##############################",
        AppFont::AppleTea,
        Color::black(),
    ));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(10));
    modifier_line.push(GlyphComponent::text(
        "!!!!!!!!!!",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line += modifier_line;
    assert_eq!(glyph_line.get(0).unwrap().text, "##########");
    assert_eq!(glyph_line.get(1).unwrap().text, "!!!!!!!!!!");
    assert_eq!(glyph_line.get(2).unwrap().text, "##########");
}
#[test]
pub fn test_component_of_index() {
    component_of_index();
}

pub fn component_of_index() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10)); //0
    glyph_line.push(GlyphComponent::space(10)); //1
    glyph_line.push(GlyphComponent::space(10)); //2
    let a = glyph_line.component_of_index(0);
    assert_eq!(a, 0);
    let b = glyph_line.component_of_index(9);
    assert_eq!(b, 0);
    let c = glyph_line.component_of_index(10);
    assert_eq!(c, 1);
    let d = glyph_line.component_of_index(15);
    assert_eq!(d, 1);
    let e = glyph_line.component_of_index(20);
    assert_eq!(e, 2);
    let f = glyph_line.component_of_index(29);
    assert_eq!(f, 2);
}

#[test]
pub fn test_index_of_component() {
    index_of_component();
}

pub fn index_of_component() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10)); //0
    glyph_line.push(GlyphComponent::space(10)); //1
    glyph_line.push(GlyphComponent::space(10)); //2
    let a = glyph_line.index_of_component(0);
    assert_eq!(a, 0);
    let b = glyph_line.index_of_component(1);
    assert_eq!(b, 10);
    let c = glyph_line.index_of_component(2);
    assert_eq!(c, 20);
}

#[test]
pub fn test_expanding_insert_1() {
    expanding_insert_1();
}

pub fn expanding_insert_1() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10)); //0
    glyph_line.push(GlyphComponent::text("12", AppFont::AppleTea, Color::black())); //1
    glyph_line.push(GlyphComponent::space(10)); //2
    assert_eq!(glyph_line.line.len(), 3);
    // This should now insert itself between the 1 and the 2
    glyph_line.expanding_insert(
        11,
        &GlyphComponent::text("onetwo", AppFont::Evilz, Color::black()),
    );
    // Two space comps + "1", and "2", and "onetwo" = 5
    assert_eq!(glyph_line.line.len(), 5);
    assert_eq!(glyph_line.get(1).unwrap().text, "1");
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(2).unwrap().text, "onetwo");
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::Evilz);
    assert_eq!(glyph_line.get(3).unwrap().text, "2");
    assert_eq!(glyph_line.get(3).unwrap().font, AppFont::AppleTea);
}

#[test]
pub fn test_expanding_insert_2() {
    expanding_insert_2();
}

pub fn expanding_insert_2() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10)); //0
    glyph_line.push(GlyphComponent::text("12", AppFont::AppleTea, Color::black())); //1
    glyph_line.push(GlyphComponent::space(10)); //2
    assert_eq!(glyph_line.line.len(), 3);

    glyph_line.expanding_insert(
        10,
        &GlyphComponent::text("onetwo", AppFont::Evilz, Color::black()),
    );

    assert_eq!(glyph_line.line.len(), 4);
    assert_eq!(glyph_line.get(1).unwrap().text, "onetwo");
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Evilz);
    assert_eq!(glyph_line.get(2).unwrap().text, "12");
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AppleTea);
}

#[test]
pub fn test_expanding_insert_3() {
    expanding_insert_3();
}

pub fn expanding_insert_3() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10)); //0
    glyph_line.push(GlyphComponent::text("12", AppFont::AppleTea, Color::black())); //1
    glyph_line.push(GlyphComponent::space(10)); //2
    assert_eq!(glyph_line.line.len(), 3);

    glyph_line.expanding_insert(
        12,
        &GlyphComponent::text("onetwo", AppFont::Evilz, Color::black()),
    );

    assert_eq!(glyph_line.line.len(), 4);

    assert_eq!(glyph_line.get(1).unwrap().text, "12");
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(2).unwrap().text, "onetwo");
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::Evilz);
}

#[test]
pub fn test_expanding_insert_4() {
    expanding_insert_4();
}

pub fn expanding_insert_4() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.expanding_insert(0, &GlyphComponent::text("123", AppFont::African, Color::black()));
    assert_eq!(glyph_line.line.len(), 1);
    assert_eq!(glyph_line.line.get(0).unwrap().text, "123");
}

#[test]
pub fn test_expanding_insert_5() {
    expanding_insert_5();
}

pub fn expanding_insert_5() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.expanding_insert(10, &GlyphComponent::text("123", AppFont::African, Color::black()));
    assert_eq!(glyph_line.line.len(), 2);
    assert_eq!(
        glyph_line.line.get(0).unwrap().text,
        GlyphComponent::space(10).text
    );
    assert_eq!(glyph_line.line.get(1).unwrap().text, "123");
}

#[test]
pub fn test_expanding_insert_6() {
    expanding_insert_6();
}

pub fn expanding_insert_6() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(5));
    glyph_line.expanding_insert(10, &GlyphComponent::text("123", AppFont::African, Color::black()));
    assert_eq!(glyph_line.line.len(), 2);
    assert_eq!(
        glyph_line.line.get(0).unwrap().text,
        GlyphComponent::space(10).text
    );
    assert_eq!(glyph_line.line.get(1).unwrap().text, "123");
}

#[test]
pub fn test_expanding_insert_7() {
    expanding_insert_7();
}

pub fn expanding_insert_7() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10));
    glyph_line.expanding_insert(10, &GlyphComponent::text("123", AppFont::African, Color::black()));
    assert_eq!(glyph_line.line.len(), 2);
    assert_eq!(
        glyph_line.line.get(0).unwrap().text,
        GlyphComponent::space(10).text
    );
    assert_eq!(glyph_line.line.get(1).unwrap().text, "123");
}

#[test]
pub fn test_overriding_insert_1() {
    overriding_insert_1();
}

pub fn overriding_insert_1() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(10, &GlyphComponent::space(40));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().length(), 10);
    assert_eq!(glyph_line.get(1).unwrap().length(), 40);
    assert_eq!(glyph_line.get(2).unwrap().length(), 10);
}
#[test]
pub fn test_overriding_insert_2() {
    overriding_insert_2();
}

pub fn overriding_insert_2() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    glyph_line.push(GlyphComponent::space(20)); //3
    glyph_line.push(GlyphComponent::space(20)); //4
    glyph_line.push(GlyphComponent::space(20)); //5
    assert_eq!(glyph_line.line.len(), 6);
    glyph_line.overriding_insert(10, &GlyphComponent::space(100));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().length(), 10);
    assert_eq!(glyph_line.get(1).unwrap().length(), 100);
    assert_eq!(glyph_line.get(2).unwrap().length(), 10);
}

#[test]
pub fn test_overriding_insert_3() {
    overriding_insert_3();
}

pub fn overriding_insert_3() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(60, &GlyphComponent::space(40));
    assert_eq!(glyph_line.line.len(), 4);
    assert_eq!(glyph_line.get(0).unwrap().length(), 20);
    assert_eq!(glyph_line.get(1).unwrap().length(), 20);
    assert_eq!(glyph_line.get(2).unwrap().length(), 20);
    assert_eq!(glyph_line.get(3).unwrap().length(), 40);
}

#[test]
pub fn test_overriding_insert_4() {
    overriding_insert_4();
}

pub fn overriding_insert_4() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(0, &GlyphComponent::space(40));
    assert_eq!(glyph_line.line.len(), 2);
    assert_eq!(glyph_line.get(0).unwrap().length(), 40);
    assert_eq!(glyph_line.get(1).unwrap().length(), 20);
}

#[test]
pub fn test_overriding_insert_5() {
    overriding_insert_5();
}

pub fn overriding_insert_5() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(0, &GlyphComponent::space(20));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().length(), 20);
    assert_eq!(glyph_line.get(1).unwrap().length(), 20);
    assert_eq!(glyph_line.get(2).unwrap().length(), 20);
}

#[test]
pub fn test_overriding_insert_6() {
    overriding_insert_6();
}

pub fn overriding_insert_6() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    assert_eq!(glyph_line.line.len(), 3);
    for _i in 0..3639 {
        glyph_line.overriding_insert(0, &GlyphComponent::space(20));
    }
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().length(), 20);
    assert_eq!(glyph_line.get(1).unwrap().length(), 20);
    assert_eq!(glyph_line.get(2).unwrap().length(), 20);
}

#[test]
pub fn test_overriding_insert_7() {
    overriding_insert_7();
}

pub fn overriding_insert_7() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(1)); //0
    glyph_line.push(GlyphComponent::space(1)); //1
    glyph_line.push(GlyphComponent::space(1)); //2
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(3, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 4);

    glyph_line.overriding_insert(4, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 5);

    glyph_line.overriding_insert(5, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 6);

    glyph_line.overriding_insert(6, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 7);

    glyph_line.overriding_insert(7, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 8);

    glyph_line.overriding_insert(8, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 9);

    assert_eq!(glyph_line.get(0).unwrap().length(), 1);
    assert_eq!(glyph_line.get(1).unwrap().length(), 1);
    assert_eq!(glyph_line.get(2).unwrap().length(), 1);
    assert_eq!(glyph_line.get(3).unwrap().length(), 1);
    assert_eq!(glyph_line.get(4).unwrap().length(), 1);
    assert_eq!(glyph_line.get(5).unwrap().length(), 1);
    assert_eq!(glyph_line.get(6).unwrap().length(), 1);
    assert_eq!(glyph_line.get(7).unwrap().length(), 1);
    assert_eq!(glyph_line.get(8).unwrap().length(), 1);
}

#[test]
pub fn test_overriding_insert_8() {
    overriding_insert_8();
}

pub fn overriding_insert_8() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "abcdefghij",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    assert_eq!(glyph_line.line.len(), 2);
    glyph_line.overriding_insert(10, &GlyphComponent::text("x🙏🏻🍕", AppFont::Any, Color::black()));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), "x🙏🏻🍕");
    assert_eq!(glyph_line.get(2).unwrap().as_str(), "defghij");
    assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AliceInWonderland);
}

#[test]
pub fn test_overriding_insert_9() {
    overriding_insert_9();
}

pub fn overriding_insert_9() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "abcdefghij",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    assert_eq!(glyph_line.line.len(), 2);
    glyph_line.overriding_insert(7, &GlyphComponent::text("x🙏🏻🍕", AppFont::Any, Color::black()));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), "x🙏🏻🍕");
    assert_eq!(glyph_line.get(2).unwrap().as_str(), "abcdefghij");
    assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AliceInWonderland);
    assert_eq!(glyph_line.get(0).unwrap().color, Color::black());
    assert_eq!(glyph_line.get(1).unwrap().color, Color::black());
    assert_eq!(glyph_line.get(2).unwrap().color, Color::white());
}

#[test]
pub fn test_overriding_insert_10() {
    overriding_insert_10();
}

pub fn overriding_insert_10() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "abcdefghij",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line.push(GlyphComponent::text(
        "Ook? Ook! Ook? Ook.",
        AppFont::Evilz,
        Color::black(),
    ));
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(
        7,
        &GlyphComponent::text("Nanananananananana", AppFont::Any, Color::black()),
    );

    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), "Nanananananananana");
    assert_eq!(glyph_line.get(2).unwrap().as_str(), "Ook! Ook? Ook.");
    assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::Evilz);
    assert_eq!(glyph_line.get(0).unwrap().color, Color::black());
    assert_eq!(glyph_line.get(1).unwrap().color, Color::black());
    assert_eq!(glyph_line.get(2).unwrap().color, Color::black());
}

#[test]
pub fn test_overriding_insert_11() {
    overriding_insert_11();
}

pub fn overriding_insert_11() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    assert_eq!(glyph_line.line.len(), 1);
    glyph_line.overriding_insert(10, &GlyphComponent::text("10", AppFont::Any, Color::black()));
    assert_eq!(glyph_line.line.len(), 2);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), "10");
}

#[test]
pub fn test_overriding_insert_12() {
    overriding_insert_12();
}

pub fn overriding_insert_12() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    assert_eq!(glyph_line.line.len(), 1);
    glyph_line.overriding_insert(
        11,
        &GlyphComponent::text("10", AppFont::AlphaMusicMan, Color::black()),
    );
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), " ");
    assert_eq!(glyph_line.get(2).unwrap().as_str(), "10");
    assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
    assert_eq!(glyph_line.get(1).unwrap().color, Color::invisible());
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AlphaMusicMan);
    assert_eq!(glyph_line.get(2).unwrap().color, Color::black());
}

#[test]
pub fn test_overriding_insert_13() {
    overriding_insert_13();
}

pub fn overriding_insert_13() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    assert_eq!(glyph_line.line.len(), 1);
    glyph_line.overriding_insert(15, &GlyphComponent::text("10", AppFont::AppleTea, Color::black()));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), "     ");
    assert_eq!(glyph_line.get(2).unwrap().as_str(), "10");
    assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
    assert_eq!(glyph_line.get(1).unwrap().color, Color::invisible());
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(2).unwrap().color, Color::black());
}
// ── Mutation-surface completeness: model deltas (P1-10) ─────────────

/// `GlyphModelField::GlyphLine` deltas must apply through the
/// `Applicable` path, not just when hand-applied. Regression for the
/// missing branch in `GlyphModel::apply_operation`.
#[test]
pub fn test_delta_glyph_line_assign_applies_through_apply_to() {
    do_delta_glyph_line_assign_applies_through_apply_to();
}

pub fn do_delta_glyph_line_assign_applies_through_apply_to() {
    let mut model = GlyphModel::new();
    model.add_line(GlyphLine::new_with(GlyphComponent::text(
        "first",
        AppFont::AppleTea,
        Color::black(),
    )));

    let replacement = GlyphLine::new_with(GlyphComponent::text("replaced", AppFont::Evilz, Color::white()));
    let delta = DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Assign),
        GlyphModelField::GlyphLine(1, replacement),
    ]);

    delta.apply_to(&mut model);

    assert_eq!(model.glyph_matrix.matrix.len(), 2);
    assert_eq!(
        model.glyph_matrix.get(0).unwrap().get(0).unwrap().as_str(),
        "first"
    );
    assert_eq!(
        model.glyph_matrix.get(1).unwrap().get(0).unwrap().as_str(),
        "replaced"
    );
}

/// `GlyphModelField::GlyphLines` deltas must apply through the
/// `Applicable` path. Mirrors the single-line regression test above.
#[test]
pub fn test_delta_glyph_lines_assign_applies_through_apply_to() {
    do_delta_glyph_lines_assign_applies_through_apply_to();
}

pub fn do_delta_glyph_lines_assign_applies_through_apply_to() {
    let mut model = GlyphModel::new();

    let lines = vec![
        (
            0,
            GlyphLine::new_with(GlyphComponent::text(
                "line zero",
                AppFont::AppleTea,
                Color::black(),
            )),
        ),
        (
            2,
            GlyphLine::new_with(GlyphComponent::text("line two", AppFont::Evilz, Color::white())),
        ),
    ];
    let delta = DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Assign),
        GlyphModelField::GlyphLines(lines),
    ]);

    delta.apply_to(&mut model);

    assert_eq!(model.glyph_matrix.matrix.len(), 3);
    assert_eq!(
        model.glyph_matrix.get(0).unwrap().get(0).unwrap().as_str(),
        "line zero"
    );
    assert_eq!(model.glyph_matrix.get(1).unwrap().line.len(), 0);
    assert_eq!(
        model.glyph_matrix.get(2).unwrap().get(0).unwrap().as_str(),
        "line two"
    );
}

/// `ApplyOperation::Delete` on a `GlyphLine` delta clears the target
/// line. Part of the per-field operation-table work for P1-10.
#[test]
pub fn test_delta_glyph_line_delete_clears_line() {
    do_delta_glyph_line_delete_clears_line();
}

pub fn do_delta_glyph_line_delete_clears_line() {
    let mut model = GlyphModel::new();
    model.add_line(GlyphLine::new_with(GlyphComponent::text(
        "to clear",
        AppFont::AppleTea,
        Color::black(),
    )));

    let delta = DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Delete),
        GlyphModelField::GlyphLine(0, GlyphLine::new()),
    ]);

    delta.apply_to(&mut model);

    assert_eq!(model.glyph_matrix.get(0).unwrap().line.len(), 0);
}

/// Destructive operations on a single-line delta must not grow an
/// absent row. They mirror `GlyphMatrix::{sub,mul}_assign`, where
/// missing rows fall away.
#[test]
pub fn test_delta_glyph_line_destructive_ops_do_not_grow_missing_line() {
    do_delta_glyph_line_destructive_ops_do_not_grow_missing_line();
}

pub fn do_delta_glyph_line_destructive_ops_do_not_grow_missing_line() {
    for operation in [
        ApplyOperation::Subtract,
        ApplyOperation::Multiply,
        ApplyOperation::Delete,
    ] {
        let mut model = GlyphModel::new();
        model.add_line(GlyphLine::new_with(GlyphComponent::text(
            "kept",
            AppFont::AppleTea,
            Color::black(),
        )));

        let delta = DeltaGlyphModel::new(vec![
            GlyphModelField::Operation(operation),
            GlyphModelField::GlyphLine(
                3,
                GlyphLine::new_with(GlyphComponent::text("stale", AppFont::Any, Color::white())),
            ),
        ]);

        delta.apply_to(&mut model);

        assert_eq!(model.glyph_matrix.matrix.len(), 1);
        assert_eq!(
            model.glyph_matrix.get(0).unwrap().get(0).unwrap().as_str(),
            "kept"
        );
    }
}

/// Multi-line deltas use the same absent-row rule as single-line
/// deltas for destructive operations.
#[test]
pub fn test_delta_glyph_lines_destructive_ops_do_not_grow_missing_lines() {
    do_delta_glyph_lines_destructive_ops_do_not_grow_missing_lines();
}

pub fn do_delta_glyph_lines_destructive_ops_do_not_grow_missing_lines() {
    for operation in [
        ApplyOperation::Subtract,
        ApplyOperation::Multiply,
        ApplyOperation::Delete,
    ] {
        let mut model = GlyphModel::new();
        model.add_line(GlyphLine::new_with(GlyphComponent::text(
            "kept",
            AppFont::AppleTea,
            Color::black(),
        )));

        let delta = DeltaGlyphModel::new(vec![
            GlyphModelField::Operation(operation),
            GlyphModelField::GlyphLines(vec![(
                3,
                GlyphLine::new_with(GlyphComponent::text("stale", AppFont::Any, Color::white())),
            )]),
        ]);

        delta.apply_to(&mut model);

        assert_eq!(model.glyph_matrix.matrix.len(), 1);
        assert_eq!(
            model.glyph_matrix.get(0).unwrap().get(0).unwrap().as_str(),
            "kept"
        );
    }
}

/// `Subtract` on the layer field must saturate at zero instead of
/// underflowing `usize`. Regression for the raw `operation.apply`
/// path in `GlyphModel::apply_operation`.
#[test]
pub fn test_model_layer_subtract_saturates_at_zero() {
    do_model_layer_subtract_saturates_at_zero();
}

pub fn do_model_layer_subtract_saturates_at_zero() {
    let mut model = GlyphModel::new();
    model.layer = 2;

    let delta = DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Subtract),
        GlyphModelField::Layer(5),
    ]);

    delta.apply_to(&mut model);

    assert_eq!(model.layer, 0);
}

/// A `Noop` matrix delta must leave the target matrix untouched, and
/// a `Delete` matrix delta must clear it — both without reading the
/// payload. Regression for `DeltaGlyphModel::glyph_matrix` returning
/// a deep clone on every apply (P1-24): the borrow-plus-`apply_ref`
/// path only clones on the arms that consume the payload, so these
/// two operations must still produce exactly the documented result.
#[test]
pub fn test_delta_glyph_matrix_noop_and_delete_ignore_payload() {
    do_delta_glyph_matrix_noop_and_delete_ignore_payload();
}

pub fn do_delta_glyph_matrix_noop_and_delete_ignore_payload() {
    let payload = || {
        let mut matrix = GlyphMatrix::new();
        matrix.push(GlyphLine::new_with(GlyphComponent::text(
            "payload",
            AppFont::AppleTea,
            Color::white(),
        )));
        matrix
    };

    let mut noop_model = GlyphModel::new();
    noop_model.add_line(GlyphLine::new_with(GlyphComponent::text(
        "original",
        AppFont::AppleTea,
        Color::black(),
    )));
    DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Noop),
        GlyphModelField::GlyphMatrix(payload()),
    ])
    .apply_to(&mut noop_model);
    assert_eq!(noop_model.glyph_matrix.matrix.len(), 1);
    assert_eq!(
        noop_model.glyph_matrix.get(0).unwrap().get(0).unwrap().as_str(),
        "original"
    );

    let mut delete_model = GlyphModel::new();
    delete_model.add_line(GlyphLine::new_with(GlyphComponent::text(
        "original",
        AppFont::AppleTea,
        Color::black(),
    )));
    DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Delete),
        GlyphModelField::GlyphMatrix(payload()),
    ])
    .apply_to(&mut delete_model);
    assert!(delete_model.glyph_matrix.matrix.is_empty());
}

/// The same delta applies more than once with the same result — the
/// borrowing accessors hand out references, so nothing about the
/// delta is consumed by an apply.
#[test]
pub fn test_delta_glyph_matrix_applies_repeatedly() {
    do_delta_glyph_matrix_applies_repeatedly();
}

pub fn do_delta_glyph_matrix_applies_repeatedly() {
    let mut payload = GlyphMatrix::new();
    payload.push(GlyphLine::new_with(GlyphComponent::text(
        "assigned",
        AppFont::AppleTea,
        Color::white(),
    )));

    let delta = DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Assign),
        GlyphModelField::GlyphMatrix(payload.clone()),
    ]);

    // The accessors borrow, so the payload is still readable between
    // applies and identical afterwards.
    assert_eq!(delta.glyph_matrix(), Some(&payload));

    let mut first = GlyphModel::new();
    let mut second = GlyphModel::new();
    delta.apply_to(&mut first);
    delta.apply_to(&mut second);

    assert_eq!(first.glyph_matrix, payload);
    assert_eq!(second.glyph_matrix, payload);
    assert_eq!(delta.glyph_matrix(), Some(&payload));
}

/// `GlyphMatrix::add_assign` moves surplus rhs rows into self rather
/// than cloning them, and keeps the row order. Locks the
/// `into_iter()` rewrite (the previous `insert(i, ..)` would panic if
/// a caller ever grew self non-contiguously).
#[test]
pub fn test_matrix_add_assign_absorbs_taller_rhs() {
    do_matrix_add_assign_absorbs_taller_rhs();
}

pub fn do_matrix_add_assign_absorbs_taller_rhs() {
    let mut matrix = GlyphMatrix::new();
    let mut rhs = GlyphMatrix::new();
    for text in ["one", "two", "three"] {
        rhs.push(GlyphLine::new_with(GlyphComponent::text(
            text,
            AppFont::AppleTea,
            Color::black(),
        )));
    }

    matrix += rhs;

    assert_eq!(matrix.matrix.len(), 3);
    assert_eq!(matrix.get(0).unwrap().get(0).unwrap().as_str(), "one");
    assert_eq!(matrix.get(1).unwrap().get(0).unwrap().as_str(), "two");
    assert_eq!(matrix.get(2).unwrap().get(0).unwrap().as_str(), "three");
}

/// `GlyphMatrix::{sub,mul}_assign` drop rhs rows that self does not
/// have, and leave self's row count alone.
#[test]
pub fn test_matrix_destructive_assigns_drop_surplus_rhs_rows() {
    do_matrix_destructive_assigns_drop_surplus_rhs_rows();
}

pub fn do_matrix_destructive_assigns_drop_surplus_rhs_rows() {
    for subtract in [true, false] {
        let mut matrix = GlyphMatrix::new();
        matrix.push(GlyphLine::new_with(GlyphComponent::text(
            "keep",
            AppFont::AppleTea,
            Color::black(),
        )));

        let mut rhs = GlyphMatrix::new();
        for text in ["a", "b", "c"] {
            rhs.push(GlyphLine::new_with(GlyphComponent::text(
                text,
                AppFont::AppleTea,
                Color::black(),
            )));
        }

        if subtract {
            matrix -= rhs;
        } else {
            matrix *= rhs;
        }

        assert_eq!(matrix.matrix.len(), 1);
    }
}

/// `GlyphComponent::add_assign` appends the rhs run in place. Locks
/// the `push_str` rewrite that removed a whole-string clone per
/// concatenation.
#[test]
pub fn test_component_add_assign_appends_text() {
    do_component_add_assign_appends_text();
}

pub fn do_component_add_assign_appends_text() {
    let mut left = GlyphComponent::text("héllo ", AppFont::AppleTea, Color::black());
    let right = GlyphComponent::text("wörld🌍", AppFont::Evilz, Color::black());

    left += right.clone();
    assert_eq!(left.text, "héllo wörld🌍");

    // An empty rhs must not touch the lhs text.
    let mut untouched = GlyphComponent::text("stay", AppFont::AppleTea, Color::black());
    untouched += GlyphComponent::text("", AppFont::AppleTea, Color::black());
    assert_eq!(untouched.text, "stay");

    // `Add` reuses the same path without cloning the lhs first.
    let summed = GlyphComponent::text("a", AppFont::AppleTea, Color::black()) + right;
    assert_eq!(summed.text, "awörld🌍");
}

/// `GlyphModelCommand::Rotate` swings the model's position around a
/// displaced pivot. Uses the geometry epsilon rather than bit
/// equality — the trig is `f32`, so `model_block_commands` can only
/// assert the pivot-is-the-position fixed point exactly.
#[test]
pub fn test_model_rotate_moves_position_around_pivot() {
    do_model_rotate_moves_position_around_pivot();
}

pub fn do_model_rotate_moves_position_around_pivot() {
    use crate::gfx_structs::model::GlyphModelCommand;
    use crate::util::geometry::almost_equal_vec2;
    use glam::Vec2;

    let mut model = GlyphModel::new();
    model.position = crate::util::ordered_vec2::OrderedVec2::new_f32(10.0, 0.0);

    GlyphModelCommand::Rotate {
        pivot: Vec2::ZERO,
        degrees: 90.0,
    }
    .apply_to(&mut model);

    // Clockwise 90° takes `+x` onto `-y` (screen space has `+y` down).
    assert!(almost_equal_vec2(model.position.to_vec2(), Vec2::new(0.0, -10.0)));
}

/////////////////////////////////////////////////////////////////
// `GlyphLine::perform_op` — the `ignore_initial_space` path.
//
// Every case below reached `perform_op` through a real `*Assign`
// operator, which is how the mutation pipeline reaches it: a
// `ModelDelta` carrying a `GlyphLine`/`GlyphMatrix` payload runs
// `ApplyOperation::apply_ref` straight into these impls, and the
// payload is deserialized from user-authored JSON — so
// `ignore_initial_space` is reachable from a `.mindmap.json`
// without a single Rust caller setting it.
/////////////////////////////////////////////////////////////////

/// Per-run text of a line, in order — the shape the
/// `ignore_initial_space` assertions compare against.
fn line_component_texts(line: &GlyphLine) -> Vec<&str> {
    line.line.iter().map(|comp| comp.text.as_str()).collect()
}

/// The line's rendered text, runs concatenated. The run *partition*
/// is an implementation detail of `overriding_insert`; what the
/// reader sees is this string, so offset assertions are made against
/// it rather than against a component list that a future splice
/// change could legitimately reshape.
fn line_text(line: &GlyphLine) -> String {
    line.line.iter().map(|comp| comp.text.as_str()).collect()
}

/// An rhs made of one `indent`-wide leading space run followed by one
/// run per entry in `contents`, with `ignore_initial_space` set.
///
/// The indent is a *single* run on purpose. That is what leaves the
/// trailing loop starting at run ordinal 1 while `self` has already
/// been repartitioned by the head paint — the shape in which the run
/// ordinal and the column offset disagree. Split the same indent into
/// several one-space runs and the ordinals happen to line back up,
/// which is exactly how the defect hid.
fn indent_runs_rhs(indent: usize, contents: &[&str]) -> GlyphLine {
    let mut line = GlyphLine::new();
    line.ignore_initial_space = true;
    line.push(GlyphComponent::space(indent));
    for text in contents {
        line.push(GlyphComponent::text(
            text,
            AppFont::AliceInWonderland,
            Color::white(),
        ));
    }
    line
}

/// A ten-cluster ASCII line to paint onto.
fn hash_line() -> GlyphLine {
    GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::AppleTea,
        Color::black(),
    ))
}

/// A one-component rhs whose own leading whitespace must be counted
/// into the paint offset but not written.
fn indented_rhs(text: &str) -> GlyphLine {
    let mut line = GlyphLine::new_with(GlyphComponent::text(
        text,
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    line.ignore_initial_space = true;
    line
}

#[test]
pub fn test_line_ignore_initial_space_multibyte_indent() {
    do_line_ignore_initial_space_multibyte_indent();
}

/// Four U+3000 IDEOGRAPHIC SPACEs: cluster 4, byte 12. Feeding the
/// old `char` ordinal to the byte-indexed `String::split_off` landed
/// mid-codepoint and panicked. The content must land at cluster 4.
pub fn do_line_ignore_initial_space_multibyte_indent() {
    let mut glyph_line = hash_line();
    glyph_line += indented_rhs("\u{3000}\u{3000}\u{3000}\u{3000}!!!");

    assert_eq!(line_component_texts(&glyph_line), vec!["####", "!!!", "###"]);
    assert_eq!(glyph_line.length(), 10);
}

#[test]
pub fn test_line_ignore_initial_space_crlf_indent() {
    do_line_ignore_initial_space_crlf_indent();
}

/// CRLF is one grapheme cluster made of two chars, so an indent of
/// `"\r\n  "` is cluster 3 but char ordinal 4. The old code paid the
/// char ordinal into a cluster-indexed insert and painted one column
/// too far right.
pub fn do_line_ignore_initial_space_crlf_indent() {
    let mut glyph_line = hash_line();
    glyph_line += indented_rhs("\r\n  🍕🍕🍕");

    assert_eq!(line_component_texts(&glyph_line), vec!["###", "🍕🍕🍕", "####"]);
    assert_eq!(glyph_line.length(), 10);
}

#[test]
pub fn test_line_ignore_initial_space_zwj_content() {
    do_line_ignore_initial_space_zwj_content();
}

/// A ZWJ family sequence is one cluster of eight chars. The offset
/// must stay in clusters on both sides of the split so the painted
/// run occupies two columns, not eight.
pub fn do_line_ignore_initial_space_zwj_content() {
    let mut glyph_line = hash_line();
    glyph_line += indented_rhs("  👨‍👩‍👧🍕");

    assert_eq!(line_component_texts(&glyph_line), vec!["##", "👨‍👩‍👧🍕", "######"]);
    assert_eq!(glyph_line.length(), 10);
}

#[test]
pub fn test_line_ignore_initial_space_sub_assign_rhs_longer_than_lhs() {
    do_line_ignore_initial_space_sub_assign_rhs_longer_than_lhs();
}

/// `self` and `rhs` need not agree on how their text is cut into
/// runs, so the rhs run index may name no lhs run at all. The
/// `SubAssign` color arm indexed `self.line[i]` unguarded and
/// panicked; it now falls back to the rhs color.
pub fn do_line_ignore_initial_space_sub_assign_rhs_longer_than_lhs() {
    let mut glyph_line = GlyphLine::new_with(GlyphComponent::text("##", AppFont::AppleTea, Color::black()));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(2));
    modifier_line.push(GlyphComponent::text(
        "!!",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line -= modifier_line;

    assert_eq!(line_component_texts(&glyph_line), vec!["##", "!!"]);
    assert_eq!(glyph_line.get(1).unwrap().color, Color::white());
}

#[test]
pub fn test_line_ignore_initial_space_mul_assign_rhs_longer_than_lhs() {
    do_line_ignore_initial_space_mul_assign_rhs_longer_than_lhs();
}

/// Same shape as the `SubAssign` case for the `MulAssign` arm, which
/// carried the identical unguarded index.
pub fn do_line_ignore_initial_space_mul_assign_rhs_longer_than_lhs() {
    let mut glyph_line = GlyphLine::new_with(GlyphComponent::text("##", AppFont::AppleTea, Color::black()));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(2));
    modifier_line.push(GlyphComponent::text(
        "!!",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line *= modifier_line;

    assert_eq!(line_component_texts(&glyph_line), vec!["##", "!!"]);
    assert_eq!(glyph_line.get(1).unwrap().color, Color::white());
}

#[test]
pub fn test_line_ignore_initial_space_sub_assign_uses_lhs_color_when_present() {
    do_line_ignore_initial_space_sub_assign_uses_lhs_color_when_present();
}

/// The guard must not swallow the arm it guards: when the lhs *does*
/// have a run at that index, `SubAssign` still subtracts the rhs
/// color from it rather than taking the fallback.
pub fn do_line_ignore_initial_space_sub_assign_uses_lhs_color_when_present() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text("ab", AppFont::AppleTea, Color::white()));
    glyph_line.push(GlyphComponent::text("cd", AppFont::AppleTea, Color::white()));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(2));
    modifier_line.push(GlyphComponent::text(
        "!!",
        AppFont::AliceInWonderland,
        Color::black(),
    ));
    glyph_line -= modifier_line;

    let painted = glyph_line
        .line
        .iter()
        .find(|comp| comp.text == "!!")
        .expect("the rhs content run must have been painted");
    assert_eq!(painted.color, Color::white() - Color::black());
}

#[test]
pub fn test_line_ignore_initial_space_surplus_rhs_runs_append() {
    do_line_ignore_initial_space_surplus_rhs_runs_append();
}

/// With the leading-space runs skipped, the trailing loop starts at a
/// component index the lhs may be several slots short of.
/// `Vec::insert(i, …)` panicked there. Onto an empty lhs the three
/// skipped space runs still occupy columns 0..3, so the content lands
/// at 3 and 4 behind a three-space pad.
pub fn do_line_ignore_initial_space_surplus_rhs_runs_append() {
    let mut glyph_line = GlyphLine::new();

    // Three separate one-space runs, not one three-space run: this is
    // the shape that drove `begin_comp` past the end of an empty lhs
    // and made the old `Vec::insert(i, …)` panic.
    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(1));
    modifier_line.push(GlyphComponent::space(1));
    modifier_line.push(GlyphComponent::space(1));
    modifier_line.push(GlyphComponent::text(
        "A",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    modifier_line.push(GlyphComponent::text(
        "B",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line += modifier_line;

    assert_eq!(line_component_texts(&glyph_line), vec!["   ", "A", "B"]);
    assert_eq!(line_text(&glyph_line), "   AB");
}

#[test]
pub fn test_line_runs_paint_by_column_with_the_flag_off() {
    do_line_runs_paint_by_column_with_the_flag_off();
}

/// The column-offset rewrite governs the `ignore_initial_space ==
/// false` path too, and that is the likelier real-world trigger: a
/// plain `ModelDelta` `GlyphMatrix` payload reaches it with the flag
/// never set at all.
///
/// With the flag off there is no head paint, but the partitions still
/// diverge — `overriding_insert` resplits `self` as it goes, so from
/// the second run onward the rhs run ordinal names the wrong place in
/// `self`. On `main` each row below either panics outright or
/// resurrects text the insert should have covered:
///
/// - `["ab","cd"] += ["W","XYZ"]` panicked in `index_of_component`.
/// - `["ab","cd","ef"] += ["W","X"]` panicked likewise.
/// - `["a","b","c","d","e","f"] += ["WX","YZ"]` produced `WXYZcef`,
///   with the overwritten `c` resurrected between them.
///
/// Painting by column makes each case just "write the rhs text over
/// the lhs from column 0", independent of either partition.
pub fn do_line_runs_paint_by_column_with_the_flag_off() {
    // (lhs runs, rhs runs, expected text)
    let cases: Vec<(Vec<&str>, Vec<&str>, &str)> = vec![
        (vec!["ab", "cd"], vec!["W", "XYZ"], "WXYZ"),
        (vec!["ab", "cd", "ef"], vec!["W", "X"], "WXcdef"),
        (vec!["a", "b", "c", "d", "e", "f"], vec!["WX", "YZ"], "WXYZef"),
        // rhs shorter than lhs, boundaries aligned: the tail survives.
        (vec!["abcd", "efgh"], vec!["WX"], "WXcdefgh"),
        // rhs longer than lhs: the surplus extends the line.
        (vec!["ab"], vec!["W", "X", "YZ"], "WXYZ"),
        // Emoji runs — columns are clusters on both sides.
        (vec!["####", "@@@@"], vec!["🍕🍕", "🙏🏻"], "🍕🍕🙏🏻#@@@@"),
    ];

    for (lhs_runs, rhs_runs, expected) in cases {
        let mut glyph_line = GlyphLine::new();
        for run in &lhs_runs {
            glyph_line.push(GlyphComponent::text(run, AppFont::AppleTea, Color::black()));
        }
        let mut modifier_line = GlyphLine::new();
        // Explicitly off — this is the whole point of the case.
        modifier_line.ignore_initial_space = false;
        for run in &rhs_runs {
            modifier_line.push(GlyphComponent::text(
                run,
                AppFont::AliceInWonderland,
                Color::white(),
            ));
        }

        glyph_line += modifier_line;

        assert_eq!(
            line_text(&glyph_line),
            expected,
            "lhs {:?} += rhs {:?}",
            lhs_runs,
            rhs_runs
        );
    }
}

#[test]
pub fn test_line_surplus_runs_paint_at_their_own_rhs_offsets() {
    do_line_surplus_runs_paint_at_their_own_rhs_offsets();
}

/// Surplus runs must land at *their own* grapheme offset inside the
/// rhs, in order.
///
/// The trailing loop used to look up `self.index_of_component(i)`
/// with the **rhs** run ordinal `i`. By the time it runs, the head
/// path has already split and padded `self`, so the two run
/// partitions have diverged and that lookup means nothing: with a
/// one-cluster lhs and an rhs of `["  ", "A", "B", "C"]` the runs
/// came out reordered as `# BCA` instead of `# ABC`.
///
/// Offsets here: the two skipped indent columns put `A` at 2, `B` at
/// 3, `C` at 4; the lhs contributes its single `#` at 0, and
/// `overriding_insert` pads column 1.
pub fn do_line_surplus_runs_paint_at_their_own_rhs_offsets() {
    let mut glyph_line = GlyphLine::new_with(GlyphComponent::text("#", AppFont::AppleTea, Color::black()));
    glyph_line += indent_runs_rhs(2, &["A", "B", "C"]);

    assert_eq!(line_text(&glyph_line), "# ABC");
    assert_eq!(line_component_texts(&glyph_line), vec!["#", " ", "A", "B", "C"]);
}

#[test]
pub fn test_overriding_insert_is_partition_independent() {
    do_overriding_insert_is_partition_independent();
}

/// `overriding_insert` addresses the line in grapheme columns, so its
/// result must not depend on how that line happens to be cut into
/// runs. The run partition is a rendering detail — two lines with the
/// same text and different run boundaries are the same line.
///
/// It was not: `seek_comp_begin`'s "this whole comp can be spared"
/// arm hard-coded `should_overwrite = false`, a judgment it could not
/// actually make, because whether the *following* run is consumed
/// depends on the item's length against a component that arm has not
/// seen. So an insert landing exactly on a run boundary and exactly
/// consuming the run after it overwrote nothing and grew the line by
/// a column, while the identical insert over the identical text as a
/// single run correctly overwrote. `perform_op`'s trailing loop is
/// what surfaced it — it paints at column offsets onto a line the
/// head path has already repartitioned — but the defect is
/// `overriding_insert`'s and is reachable from
/// `GlyphModelCommand::RudeInsert` and `GlyphMatrix::overriding_insert`
/// with no `GlyphLine` operator involved.
pub fn do_overriding_insert_is_partition_independent() {
    // Every partition below spells "####@@@@" — eight columns.
    let partitions: Vec<Vec<&str>> = vec![
        vec!["####@@@@"],
        vec!["####", "@@@@"],
        vec!["##", "##", "@@@@"],
        vec!["###", "#", "@@@@"],
        vec!["####", "@", "@@@"],
        vec!["#", "#", "#", "#", "@", "@", "@", "@"],
    ];
    // (insert column, item text, expected line text)
    let cases: Vec<(usize, &str, &str)> = vec![
        (0, "B", "B###@@@@"),
        (2, "B", "##B#@@@@"),
        (3, "B", "###B@@@@"),
        (4, "B", "####B@@@"),
        (7, "B", "####@@@B"),
        (3, "BB", "###BB@@@"),
        (2, "BBBB", "##BBBB@@"),
        (8, "B", "####@@@@B"),
        (3, "🍕", "###🍕@@@@"),
    ];

    for partition in &partitions {
        for (column, item, expected) in &cases {
            let mut line = GlyphLine::new();
            for run in partition {
                line.push(GlyphComponent::text(run, AppFont::AppleTea, Color::black()));
            }
            line.overriding_insert(
                *column,
                &GlyphComponent::text(item, AppFont::AliceInWonderland, Color::white()),
            );
            assert_eq!(
                line_text(&line),
                *expected,
                "partition {:?}, insert {:?} at column {}",
                partition,
                item,
                column
            );
            assert_eq!(
                line.length(),
                count_grapheme_clusters(expected),
                "partition {:?}, insert {:?} at column {}: column count drifted",
                partition,
                item,
                column
            );
        }
    }
}

#[test]
pub fn test_line_surplus_runs_overwrite_a_longer_lhs_in_place() {
    do_line_surplus_runs_overwrite_a_longer_lhs_in_place();
}

/// The same divergence, but with an lhs long enough that the surplus
/// runs overwrite rather than extend — so a misplaced run shows up as
/// corrupted text rather than as a reorder.
///
/// `####@@@@` with `A` at column 2 and `B` at column 3 is `##AB@@@@`:
/// eight columns in, eight columns out. The old ordinal lookup wrote
/// `##AB#@@@@` — nine columns, the overwritten `#` resurrected.
pub fn do_line_surplus_runs_overwrite_a_longer_lhs_in_place() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text("####", AppFont::AppleTea, Color::black()));
    glyph_line.push(GlyphComponent::text("@@@@", AppFont::AppleTea, Color::black()));

    glyph_line += indent_runs_rhs(2, &["A", "B"]);

    assert_eq!(line_text(&glyph_line), "##AB@@@@");
    assert_eq!(glyph_line.length(), 8);
}

#[test]
pub fn test_line_ignore_initial_space_all_whitespace_rhs_paints_nothing() {
    do_line_ignore_initial_space_all_whitespace_rhs_paints_nothing();
}

/// Every run of an all-whitespace rhs is leading whitespace, so under
/// the flag's contract the whole rhs is transparent. It used to be
/// opaque — the same run blanked the lhs when nothing followed it and
/// showed through when something did.
pub fn do_line_ignore_initial_space_all_whitespace_rhs_paints_nothing() {
    let mut glyph_line = hash_line();

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(2));
    modifier_line.push(GlyphComponent::space(2));
    glyph_line += modifier_line;

    assert_eq!(line_component_texts(&glyph_line), vec!["##########"]);
}
