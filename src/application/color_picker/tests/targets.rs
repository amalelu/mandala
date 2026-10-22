// SPDX-License-Identifier: MPL-2.0

//! `targets.rs` unit tests — `ColorTarget` resolution and
//! `current_color_at` cascade for each `PickerHandle` shape.

use crate::application::color_picker::{current_color_at, ColorTarget, PickerHandle, SectionColorAxis};
use crate::application::document::tests_common::{
    first_testament_node_id, load_test_doc, make_two_section_node_with_pinned_runs,
};
use crate::application::document::GraphemeRange;

/// Build a node with two sections, each carrying a distinct
/// uniform run color so the cascade-primary read on either
/// section returns a different value (`#111111` for section 0,
/// `#222222` for section 1). Returns `(doc, node_id)`.
fn doc_with_two_uniform_sections() -> (crate::application::document::MindMapDocument, String) {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // text_color_default is set to one of the per-section colors
    // so the section-1 cascade falls back to it cleanly when a
    // disagreement test appends a contrarian run.
    make_two_section_node_with_pinned_runs(
        &mut doc,
        &id,
        "#abcdef",
        ["#111111", "#222222"],
        "LiberationSans",
        14.0,
    );
    (doc, id)
}

/// `current_color_at` on a Section handle returns the unanimous
/// run color when every run on the section shares one (cascade
/// primary).
#[test]
fn current_color_at_section_reads_unanimous_run_color() {
    let (doc, id) = doc_with_two_uniform_sections();
    let handle = PickerHandle::Section {
        range: None,
        node_id: id,
        section_idx: 1,
        axis: SectionColorAxis::Text,
    };
    assert_eq!(
        current_color_at(&doc, &handle).as_deref(),
        Some("#222222"),
        "section 1's unanimous run color should be returned"
    );
}

/// When a section's runs disagree on color, the cascade falls
/// back to the node's `style.text_color` default. Pins Item 8
/// (cascade fallback). Mirrors the write-side cascade source
/// `set_section_text_color` reads.
#[test]
fn current_color_at_section_falls_back_to_node_default_on_run_disagreement() {
    let (mut doc, id) = doc_with_two_uniform_sections();
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.style.text_color = "#abcdef".into();
        // Append a second run on section 1 with a different color
        // so the runs no longer agree.
        let section = node.sections.get_mut(1).expect("section 1 exists");
        section.text_runs.push(baumhard::mindmap::model::TextRun {
            start: 0,
            end: 1,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14.0,
            color: "#999999".into(),
            hyperlink: None,
        });
    }
    let handle = PickerHandle::Section {
        range: None,
        node_id: id,
        section_idx: 1,
        axis: SectionColorAxis::Text,
    };
    assert_eq!(
        current_color_at(&doc, &handle).as_deref(),
        Some("#abcdef"),
        "disagreement between runs should fall back to node.style.text_color"
    );
}

/// `ColorTarget::Section.resolve()` produces a matching
/// `PickerHandle::Section` when the node + section index resolve.
/// Stale-index defensive check is exercised by the negative test
/// below.
#[test]
fn color_target_section_resolves_to_picker_handle() {
    let (doc, id) = doc_with_two_uniform_sections();
    let target = ColorTarget::Section {
        range: None,
        node_id: id.clone(),
        section_idx: 1,
        axis: SectionColorAxis::Text,
    };
    match target.resolve(&doc) {
        Some(PickerHandle::Section {
            range: None,
            node_id,
            section_idx,
            axis,
        }) => {
            assert_eq!(node_id, id);
            assert_eq!(section_idx, 1);
            assert_eq!(axis, SectionColorAxis::Text);
        }
        other => panic!("expected PickerHandle::Section, got {:?}", other),
    }
}

/// A section index past the end of `node.sections` resolves to
/// `None` rather than producing a handle that would later panic
/// in `current_color_at`. Mirrors the Edge variant's
/// stale-position defensive check.
#[test]
fn color_target_section_resolves_to_none_when_index_out_of_range() {
    let (doc, id) = doc_with_two_uniform_sections();
    let target = ColorTarget::Section {
        range: None,
        node_id: id,
        section_idx: 99,
        axis: SectionColorAxis::Text,
    };
    assert!(target.resolve(&doc).is_none());
}

/// `current_color_at` over a sub-range scans only the in-range
/// runs. With section 1 set up so different ranges yield
/// different unanimous colors, the picker reads each correctly.
/// Pins the N4-C.b.1 range-aware seed.
#[test]
fn current_color_at_section_range_reads_in_range_runs() {
    use baumhard::mindmap::model::TextRun;
    let (mut doc, id) = doc_with_two_uniform_sections();
    // Replace section 1's runs: [0..3 #aaaaaa, 3..7 #bbbbbb, 7..10 #cccccc]
    {
        let s = &mut doc.mindmap.nodes.get_mut(&id).unwrap().sections[1];
        s.text = "abcdefghij".into();
        s.text_runs.clear();
        for (start, end, color) in [(0, 3, "#aaaaaa"), (3, 7, "#bbbbbb"), (7, 10, "#cccccc")] {
            s.text_runs.push(TextRun {
                start,
                end,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 14.0,
                color: color.into(),
                hyperlink: None,
            });
        }
    }
    // Range [3, 7) = the middle run only → unanimous #bbbbbb.
    let handle_in_range = PickerHandle::Section {
        node_id: id.clone(),
        section_idx: 1,
        axis: SectionColorAxis::Text,
        range: Some(GraphemeRange::new(3, 7)),
    };
    assert_eq!(
        current_color_at(&doc, &handle_in_range).as_deref(),
        Some("#bbbbbb"),
        "range-restricted cascade reads only the in-range run's color"
    );
    // Range [0, 7) = first two runs disagree (#aaaaaa, #bbbbbb)
    // → cascade falls back to node.style.text_color (the
    // fixture's default is "#abcdef").
    let handle_disagree = PickerHandle::Section {
        node_id: id,
        section_idx: 1,
        axis: SectionColorAxis::Text,
        range: Some(GraphemeRange::new(0, 7)),
    };
    assert_eq!(
        current_color_at(&doc, &handle_disagree).as_deref(),
        Some("#abcdef"),
        "non-unanimous in-range runs fall back to node default"
    );
}

/// **Gap coverage check.** A range that covers a single run
/// AND a gap (uncovered grapheme range) must NOT report the
/// run's color as the picker seed — the gap's effective
/// color is the node default, so the range is non-unanimous.
/// Pre-fix the trivial `iter().all` on a one-element slice
/// would have passed and seeded the picker with the run's
/// color. Pin the fall-back-to-default behavior.
#[test]
fn current_color_at_section_range_falls_back_when_range_crosses_gap() {
    use baumhard::mindmap::model::TextRun;
    let (mut doc, id) = doc_with_two_uniform_sections();
    // Replace section 1's runs: ONE run at [3..6 red], rest of
    // [0..10) is gap. Range [0..8) covers the gap [0,3),
    // run [3,6) red, gap [6,8) — mixed coverage → fall back.
    {
        let s = &mut doc.mindmap.nodes.get_mut(&id).unwrap().sections[1];
        s.text = "abcdefghij".into();
        s.text_runs.clear();
        s.text_runs.push(TextRun {
            start: 3,
            end: 6,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14.0,
            color: "#ff0000".into(),
            hyperlink: None,
        });
    }
    let handle = PickerHandle::Section {
        node_id: id,
        section_idx: 1,
        axis: SectionColorAxis::Text,
        range: Some(GraphemeRange::new(0, 8)),
    };
    assert_eq!(
        current_color_at(&doc, &handle).as_deref(),
        Some("#abcdef"),
        "range that crosses a gap must fall back to node default, \
         not seed with the partial run's color"
    );
}

/// Range entirely in a gap (no covering run) → falls back.
#[test]
fn current_color_at_section_range_falls_back_when_range_in_pure_gap() {
    use baumhard::mindmap::model::TextRun;
    let (mut doc, id) = doc_with_two_uniform_sections();
    {
        let s = &mut doc.mindmap.nodes.get_mut(&id).unwrap().sections[1];
        s.text = "abcdefghij".into();
        s.text_runs.clear();
        // Single run far from the queried range — [0..3) is
        // entirely gap.
        s.text_runs.push(TextRun {
            start: 7,
            end: 10,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14.0,
            color: "#00ff00".into(),
            hyperlink: None,
        });
    }
    let handle = PickerHandle::Section {
        node_id: id,
        section_idx: 1,
        axis: SectionColorAxis::Text,
        range: Some(GraphemeRange::new(0, 3)),
    };
    assert_eq!(
        current_color_at(&doc, &handle).as_deref(),
        Some("#abcdef"),
        "range entirely in a gap must fall back to node default"
    );
}

/// Range covering multiple consecutive runs with the same
/// color reads unanimous. Pins the `iter().all` branch with
/// `len > 1` (today's other range tests cover only `len == 1`
/// and the disagree case).
#[test]
fn current_color_at_section_range_unanimous_across_multiple_adjacent_runs() {
    use baumhard::mindmap::model::TextRun;
    let (mut doc, id) = doc_with_two_uniform_sections();
    {
        let s = &mut doc.mindmap.nodes.get_mut(&id).unwrap().sections[1];
        s.text = "abcdefghij".into();
        s.text_runs.clear();
        // Two adjacent runs covering [0..6), both yellow. Range
        // [0..6) → two runs, both same color → unanimous.
        for (start, end) in [(0, 3), (3, 6)] {
            s.text_runs.push(TextRun {
                start,
                end,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 14.0,
                color: "#ffff00".into(),
                hyperlink: None,
            });
        }
    }
    let handle = PickerHandle::Section {
        node_id: id,
        section_idx: 1,
        axis: SectionColorAxis::Text,
        range: Some(GraphemeRange::new(0, 6)),
    };
    assert_eq!(
        current_color_at(&doc, &handle).as_deref(),
        Some("#ffff00"),
        "unanimous color across multiple fully-covering adjacent runs"
    );
}

/// A Node handle seeds from the **effective** color, not the
/// authored `style` field. On a themed node those differ — `style`
/// is the tier the palette shadows — and seeding from `style`
/// opens the wheel on a color the node is not drawn in, so the
/// user's first hover jumps.
#[test]
fn current_color_at_node_reads_through_the_palette_cascade() {
    use crate::application::color_picker::NodeColorAxis;
    use crate::application::document::tests_common::theme_node_with_probe_palette;
    use baumhard::mindmap::model::ColorGroup;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    theme_node_with_probe_palette(
        &mut doc,
        &id,
        "picker-probe",
        ColorGroup {
            background: "#a10000".into(),
            frame: "#a20000".into(),
            text: "#a30000".into(),
            title: String::new(),
        },
    );
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.style.background_color = "#111111".into();
        node.style.frame_color = "#222222".into();
        node.style.text_color = "#333333".into();
    }
    for (axis, expected) in [
        (NodeColorAxis::Bg, "#a10000"),
        (NodeColorAxis::Border, "#a20000"),
        (NodeColorAxis::Text, "#a30000"),
    ] {
        let handle = PickerHandle::Node { id: id.clone(), axis };
        assert_eq!(
            current_color_at(&doc, &handle).as_deref(),
            Some(expected),
            "{axis:?} must seed from the palette group, not style"
        );
    }

    // A per-node override outranks the group, and the picker has to
    // seed from that too — it is what the node is drawn in.
    assert!(doc.set_node_bg_color(&id, Some("#00ff00")));
    let handle = PickerHandle::Node {
        id: id.clone(),
        axis: NodeColorAxis::Bg,
    };
    assert_eq!(current_color_at(&doc, &handle).as_deref(), Some("#00ff00"));
}

/// A section whose runs all decline to name a color defers to the
/// node, so the picker seeds from the node's effective text color
/// rather than opening on the empty string (which parses to
/// nothing and drops the wheel to its mid-gray fallback).
#[test]
fn current_color_at_section_falls_through_when_every_run_defers() {
    let (mut doc, id) = doc_with_two_uniform_sections();
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.style.text_color = "#abcdef".into();
        for run in node.sections[1].text_runs.iter_mut() {
            run.color = String::new();
        }
    }
    let handle = PickerHandle::Section {
        node_id: id,
        section_idx: 1,
        axis: SectionColorAxis::Text,
        range: None,
    };
    assert_eq!(
        current_color_at(&doc, &handle).as_deref(),
        Some("#abcdef"),
        "a deferring run seeds the picker from the node, not from the empty string"
    );
}
