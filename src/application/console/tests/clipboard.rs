// SPDX-License-Identifier: MPL-2.0

//! Clipboard trait dispatch tests for `TargetView`. Covers the
//! `HandlesCopy`, `HandlesPaste`, and `HandlesCut` impls per
//! component variant (Node / Edge / EdgeLabel / PortalLabel /
//! PortalText).

use super::fixtures::{first_node_id, load_test_doc, select_first_edge};
use crate::application::console::traits::{
    view_for, ClipboardContent, HandlesCopy, HandlesCut, HandlesPaste, Outcome, TargetId,
};

// ── Node ─────────────────────────────────────────────────────────

#[test]
fn node_copy_returns_node_text() {
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    let original = doc.mindmap.nodes.get(&nid).unwrap().display_text().into_owned();
    let tid = TargetId::Node(nid);
    let view = view_for(&mut doc, &tid);
    match view.clipboard_copy() {
        ClipboardContent::Text(t) => assert_eq!(t, original),
        other => panic!("expected Text, got {:?}", other),
    }
}

#[test]
fn node_copy_empty_text_returns_empty() {
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    doc.set_node_text(&nid, String::new());
    let tid = TargetId::Node(nid);
    let view = view_for(&mut doc, &tid);
    assert_eq!(view.clipboard_copy(), ClipboardContent::Empty);
}

#[test]
fn node_paste_replaces_text_and_pushes_undo() {
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    let undo_before = doc.undo_stack.len();
    let tid = TargetId::Node(nid.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("pasted text")
    };
    assert_eq!(outcome, Outcome::Applied);
    assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().display_text(), "pasted text");
    assert_eq!(doc.undo_stack.len(), undo_before + 1);
}

#[test]
fn node_paste_unchanged_text_reports_unchanged() {
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    // Pre-trim the node's text. The paste handler trims trailing
    // whitespace (paragraph-copy ergonomics); pasting raw `original`
    // on a node whose text happens to end in whitespace would
    // report `Applied`, and HashMap iteration order picks the
    // "first" node non-deterministically. Normalising first pins
    // the assertion to the round-trip we actually care about.
    let original = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .display_text()
        .trim_end()
        .to_string();
    doc.set_node_text(&nid, original.clone());
    let tid = TargetId::Node(nid);
    let mut view = view_for(&mut doc, &tid);
    assert_eq!(view.clipboard_paste(&original), Outcome::Unchanged);
}

#[test]
fn node_cut_returns_text_and_clears_node() {
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    let original = doc.mindmap.nodes.get(&nid).unwrap().display_text().into_owned();
    assert!(!original.is_empty(), "fixture node should have text");
    let tid = TargetId::Node(nid.clone());
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert_eq!(cut, ClipboardContent::Text(original));
    assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().display_text(), "");
}

/// Multi-section node `Node`-target cut: clears EVERY section's
/// text. Pre-fix only `section[0]` was cleared via
/// `set_node_text`, leaving `sections[1..]` populated as zombie
/// content not on the clipboard. Pin the loop-over-every-section
/// behaviour so a future revert is loud.
#[test]
fn node_cut_clears_every_section_on_multi_section_node() {
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second-stratum".into(), Vec::new()));
        node.sections
            .push(MindSection::new_default("third-stratum".into(), Vec::new()));
    }
    let original = doc.mindmap.nodes.get(&nid).unwrap().display_text().into_owned();
    assert!(original.contains("second-stratum"));
    assert!(original.contains("third-stratum"));

    let tid = TargetId::Node(nid.clone());
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert_eq!(cut, ClipboardContent::Text(original));
    let post = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(post.sections.len(), 3, "section count preserved");
    for (idx, section) in post.sections.iter().enumerate() {
        assert!(
            section.text.is_empty(),
            "section[{}].text must be empty post-cut, was {:?}",
            idx,
            section.text
        );
    }
}

/// Section paste collapses the section's `text_runs` to a single
/// run, inheriting the first original run's `font` / `size_pt` /
/// `color` / `bold` / `italic` / `underline`. Pre-Tier-2B the
/// clipboard payload is `String`-only; structured per-run paste
/// is captured in the plan tracker. This test pins the CURRENT
/// lossy behaviour so a future structured payload doesn't
/// regress to the unstructured branch silently — the assertion
/// is "if you paste plain text into a section that had multi-run
/// formatting, you lose the per-run structure but keep the
/// first-run template's formatting on the new single run."
#[test]
fn section_paste_collapses_runs_inheriting_first_run_template() {
    use crate::application::clipboard::clear_section_clipboard;
    use crate::application::console::traits::{HandlesPaste, TargetView};
    use baumhard::mindmap::model::{MindSection, TextRun};
    // Clear so a prior test on the same worker thread doesn't
    // leak a buffer entry matching our probe and route us through
    // `apply_section_payload` instead of the lossy fall-back.
    clear_section_clipboard();
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), Vec::new()));
        // Section 1 starts with two distinctly-formatted runs.
        // The first run's template (font="LiberationSans" / 18pt
        // bold / "#ff0000") is what `set_section_text` should
        // inherit onto the post-paste single run.
        node.sections[1].text = "before".into();
        node.sections[1].text_runs = vec![
            TextRun {
                start: 0,
                end: 3,
                bold: true,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 18,
                color: "#ff0000".into(),
                hyperlink: None,
            },
            TextRun {
                start: 3,
                end: 6,
                bold: false,
                italic: true,
                underline: false,
                font: "Other".into(),
                size_pt: 24,
                color: "#00ff00".into(),
                hyperlink: None,
            },
        ];
    }
    let id = nid.clone();
    {
        let mut view = TargetView::Section { range: None,
            doc: &mut doc,
            id,
            section_idx: 1,
        };
        let _ = view.clipboard_paste("pasted");
    }
    let section = &doc.mindmap.nodes.get(&nid).unwrap().sections[1];
    assert_eq!(section.text, "pasted");
    assert_eq!(
        section.text_runs.len(),
        1,
        "paste collapses multi-run sections to a single run (lossy by spec until Tier 2B)"
    );
    let r = &section.text_runs[0];
    assert_eq!(r.font, "LiberationSans", "inherits first-run font");
    assert_eq!(r.size_pt, 18, "inherits first-run size_pt");
    assert_eq!(r.color, "#ff0000", "inherits first-run color");
    assert!(r.bold, "inherits first-run bold flag");
    assert!(!r.italic, "second-run italic flag must NOT bleed in");
}

/// Section paste with a stale `section_idx` (a custom mutation
/// shrunk `node.sections` between the click that captured the
/// Section selection and the paste) clamps to the last existing
/// section instead of silently no-op'ing through
/// `set_section_text`'s bounds check.
#[test]
fn section_paste_clamps_stale_idx_to_last_section() {
    use crate::application::clipboard::clear_section_clipboard;
    use crate::application::console::traits::{HandlesPaste, TargetView};
    use baumhard::mindmap::model::MindSection;
    clear_section_clipboard();
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), Vec::new()));
    }
    let id = nid.clone();
    {
        let mut view = TargetView::Section { range: None,
            doc: &mut doc,
            id,
            section_idx: 5, // way past the end
        };
        let _ = view.clipboard_paste("after-clamp");
    }
    let post = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(
        post.sections[1].text, "after-clamp",
        "stale paste must land in the last existing section, not silently no-op"
    );
}

// ── Section structured clipboard ────────────────────────────────

/// Section copy emits the structured `Section` variant carrying
/// both plain text (OS-clipboard cross-app fallback) and the full
/// per-section payload (in-process within-app round-trip).
#[test]
fn section_copy_emits_structured_payload() {
    use crate::application::document::tests_common::make_two_section_node_with_pinned_runs;
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    make_two_section_node_with_pinned_runs(
        &mut doc,
        &nid,
        "#aaaaaa",
        ["#aaaaaa", "#abcdef"],
        "LiberationSans",
        18,
    );
    let tid = TargetId::Section { range: None,
        node_id: nid.clone(),
        section_idx: 1,
    };
    let view = view_for(&mut doc, &tid);
    match view.clipboard_copy() {
        ClipboardContent::Section { text: _, payload } => {
            assert_eq!(payload.text_runs.len(), 1);
            assert_eq!(payload.text_runs[0].color, "#abcdef");
            assert_eq!(payload.text_runs[0].size_pt, 18);
            assert_eq!(payload.text_runs[0].font, "LiberationSans");
        }
        other => panic!("expected ClipboardContent::Section, got {:?}", other),
    }
}

/// Round-trip pin: paste consults the buffer, finds a matching
/// payload, writes structured (per-run formatting + chrome
/// preserved). Closes audit Q5 for the within-app path.
#[test]
fn section_paste_with_matching_buffer_preserves_runs() {
    use crate::application::clipboard::{clear_section_clipboard, write_section_clipboard};
    use crate::application::console::traits::TargetView;
    use crate::application::document::tests_common::make_two_section_node_with_pinned_runs;
    use crate::application::document::SectionPayload;
    use baumhard::mindmap::model::TextRun;
    clear_section_clipboard();
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    make_two_section_node_with_pinned_runs(&mut doc, &nid, "#ffffff", ["#ffffff", "#ffffff"], "Seed", 14);
    let payload = SectionPayload {
        text_runs: vec![TextRun {
            start: 0,
            end: 3,
            bold: false,
            italic: true,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 22,
            color: "#112233".into(),
            hyperlink: None,
        }],
        offset: baumhard::mindmap::model::Position { x: 5.0, y: 7.0 },
        size: None,
        channel: Some(3),
        trigger_bindings: Vec::new(),
    };
    write_section_clipboard("src".into(), payload);

    let id = nid.clone();
    {
        let mut view = TargetView::Section { range: None,
            doc: &mut doc,
            id,
            section_idx: 1,
        };
        assert_eq!(view.clipboard_paste("src"), Outcome::Applied);
    }
    let section = &doc.mindmap.nodes.get(&nid).unwrap().sections[1];
    assert_eq!(section.text, "src");
    assert_eq!(section.text_runs.len(), 1);
    assert_eq!(section.text_runs[0].size_pt, 22);
    assert!(section.text_runs[0].italic);
    assert_eq!(section.text_runs[0].color, "#112233");
    assert_eq!(section.offset.x, 5.0);
    assert_eq!(section.channel, Some(3));
}

/// Trailing-newline round-trip pin: a section whose text ends in
/// `\n` (trivially produced by the inline editor's Enter key) must
/// still match the buffer's snapshot — probing with the untrimmed
/// `content` keeps the consistency check honest. Pre-fix, the
/// paste side's `content.trim_end()` falsely missed any section
/// whose text ended in whitespace.
#[test]
fn section_paste_buffer_match_survives_trailing_newline() {
    use crate::application::clipboard::{clear_section_clipboard, write_section_clipboard};
    use crate::application::console::traits::TargetView;
    use crate::application::document::tests_common::make_two_section_node_with_pinned_runs;
    use crate::application::document::SectionPayload;
    use baumhard::mindmap::model::TextRun;
    clear_section_clipboard();
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    make_two_section_node_with_pinned_runs(&mut doc, &nid, "#ffffff", ["#ffffff", "#ffffff"], "Seed", 14);
    let payload = SectionPayload {
        text_runs: vec![TextRun {
            start: 0,
            end: 5,
            bold: false,
            italic: false,
            underline: false,
            font: "PreservedFont".into(),
            size_pt: 30,
            color: "#aabbcc".into(),
            hyperlink: None,
        }],
        offset: baumhard::mindmap::model::Position::default(),
        size: None,
        channel: None,
        trigger_bindings: Vec::new(),
    };
    write_section_clipboard("hello\n".into(), payload);

    let id = nid.clone();
    {
        let mut view = TargetView::Section { range: None,
            doc: &mut doc,
            id,
            section_idx: 1,
        };
        assert_eq!(view.clipboard_paste("hello\n"), Outcome::Applied);
    }
    let section = &doc.mindmap.nodes.get(&nid).unwrap().sections[1];
    assert_eq!(section.text_runs.len(), 1);
    assert_eq!(
        section.text_runs[0].font, "PreservedFont",
        "trailing-newline copy must round-trip via the structured buffer, not the trimmed plain-text fallback"
    );
}

/// Buffer mismatch (user copied from another app between Mandala
/// copy and paste) falls through to the plain-text branch with
/// template inheritance from the destination's first run.
#[test]
fn section_paste_with_mismatched_buffer_falls_back_to_plain() {
    use crate::application::clipboard::{clear_section_clipboard, write_section_clipboard};
    use crate::application::console::traits::TargetView;
    use crate::application::document::tests_common::make_two_section_node_with_pinned_runs;
    use crate::application::document::SectionPayload;
    use baumhard::mindmap::model::TextRun;
    clear_section_clipboard();
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    make_two_section_node_with_pinned_runs(&mut doc, &nid, "#ffffff", ["#ffffff", "#ffffff"], "Seed", 14);
    let payload = SectionPayload {
        text_runs: vec![TextRun {
            start: 0,
            end: 1,
            bold: true,
            italic: false,
            underline: false,
            font: "ShouldNotApply".into(),
            size_pt: 99,
            color: "#deadbe".into(),
            hyperlink: None,
        }],
        offset: baumhard::mindmap::model::Position::default(),
        size: None,
        channel: None,
        trigger_bindings: Vec::new(),
    };
    write_section_clipboard("OLD-COPY".into(), payload);

    let id = nid.clone();
    {
        let mut view = TargetView::Section { range: None,
            doc: &mut doc,
            id,
            section_idx: 1,
        };
        let _ = view.clipboard_paste("from-elsewhere");
    }
    let section = &doc.mindmap.nodes.get(&nid).unwrap().sections[1];
    assert_eq!(section.text, "from-elsewhere");
    assert_eq!(section.text_runs.len(), 1);
    assert_eq!(section.text_runs[0].font, "Seed");
    assert_eq!(section.text_runs[0].size_pt, 14);
}

/// Cut snapshots the structured payload, clears text + runs, and
/// preserves offset / size / channel / bindings on the source.
#[test]
fn section_cut_emits_structured_payload_and_clears_text_runs_only() {
    use crate::application::console::traits::TargetView;
    use crate::application::document::tests_common::make_two_section_node_with_pinned_runs;
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    make_two_section_node_with_pinned_runs(
        &mut doc,
        &nid,
        "#ffffff",
        ["#ffffff", "#abc123"],
        "LiberationSans",
        14,
    );
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections[1].channel = Some(7);
        node.sections[1].offset = baumhard::mindmap::model::Position { x: 1.0, y: 2.0 };
    }
    let id = nid.clone();
    let cut = {
        let mut view = TargetView::Section { range: None,
            doc: &mut doc,
            id,
            section_idx: 1,
        };
        view.clipboard_cut()
    };
    match cut {
        ClipboardContent::Section { text: _, payload } => {
            assert_eq!(payload.text_runs.len(), 1);
            assert_eq!(payload.text_runs[0].color, "#abc123");
            assert_eq!(payload.channel, Some(7));
            assert_eq!(payload.offset.x, 1.0);
        }
        other => panic!("expected ClipboardContent::Section, got {:?}", other),
    }
    let section = &doc.mindmap.nodes.get(&nid).unwrap().sections[1];
    assert_eq!(section.text, "");
    assert!(section.text_runs.is_empty());
    assert_eq!(section.channel, Some(7));
    assert_eq!(section.offset.x, 1.0);
}

/// `apply_section_payload` round-trips through undo — one Ctrl+Z
/// restores all six fields atomically.
#[test]
fn apply_section_payload_round_trips_through_undo() {
    use crate::application::document::tests_common::make_two_section_node_with_pinned_runs;
    use crate::application::document::SectionPayload;
    use baumhard::mindmap::model::TextRun;
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    make_two_section_node_with_pinned_runs(&mut doc, &nid, "#ffffff", ["#ffffff", "#111111"], "Before", 14);
    {
        doc.mindmap.nodes.get_mut(&nid).unwrap().sections[1].channel = Some(2);
    }
    let snapshot_text = doc.mindmap.nodes.get(&nid).unwrap().sections[1].text.clone();
    let snapshot_channel = doc.mindmap.nodes.get(&nid).unwrap().sections[1].channel;

    let after = SectionPayload {
        text_runs: vec![TextRun {
            start: 0,
            end: 5,
            bold: true,
            italic: true,
            underline: false,
            font: "After".into(),
            size_pt: 22,
            color: "#222222".into(),
            hyperlink: None,
        }],
        offset: baumhard::mindmap::model::Position { x: 9.0, y: 9.0 },
        size: None,
        channel: Some(99),
        trigger_bindings: Vec::new(),
    };
    assert!(doc.apply_section_payload(&nid, 1, "after".into(), &after));
    assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().sections[1].channel, Some(99));

    assert!(doc.undo());
    let restored = &doc.mindmap.nodes.get(&nid).unwrap().sections[1];
    assert_eq!(restored.text, snapshot_text);
    assert_eq!(restored.channel, snapshot_channel);
    assert_eq!(restored.text_runs[0].font, "Before");
}

// ── Edge (body) ──────────────────────────────────────────────────
//
// Clipboard semantics: copy / cut return the resolved edge color
// hex; paste sets the edge color from a hex. Label text copy /
// paste is gone (edge labels are edited through the inline modal,
// which owns its own OS-clipboard surface).

#[test]
fn edge_copy_returns_resolved_color_hex() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_color(&er, Some("#abcdef"));
    let tid = TargetId::Edge(er);
    let view = view_for(&mut doc, &tid);
    match view.clipboard_copy() {
        ClipboardContent::Text(hex) => assert_eq!(hex, "#abcdef"),
        other => panic!("expected Text with hex, got {:?}", other),
    }
}

#[test]
fn edge_paste_valid_hex_sets_color() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::Edge(er.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("#112233")
    };
    assert_eq!(outcome, Outcome::Applied);
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(
        edge.glyph_connection.as_ref().and_then(|c| c.color.as_deref()),
        Some("#112233")
    );
}

#[test]
fn edge_paste_valid_short_hex_sets_color() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::Edge(er.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("#abc")
    };
    assert_eq!(outcome, Outcome::Applied);
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(
        edge.glyph_connection.as_ref().and_then(|c| c.color.as_deref()),
        Some("#abc")
    );
}

#[test]
fn edge_paste_invalid_content_reports_invalid() {
    // The paste path rejects arbitrary text — it expects a hex
    // code or `var(--name)` — so garbage produces `Invalid`
    // rather than silently losing a colour edit.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::Edge(er);
    let mut view = view_for(&mut doc, &tid);
    match view.clipboard_paste("not a color") {
        Outcome::Invalid(msg) => assert!(msg.contains("not a color")),
        other => panic!("expected Invalid, got {:?}", other),
    }
}

#[test]
fn edge_paste_rejects_bare_hex_like_words() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    for word in ["bad", "fade", "decade"] {
        let tid = TargetId::Edge(er.clone());
        let mut view = view_for(&mut doc, &tid);
        match view.clipboard_paste(word) {
            Outcome::Invalid(_) => {}
            other => panic!("expected Invalid for bare hex-like word {word:?}, got {other:?}"),
        }
    }
}

#[test]
fn edge_paste_rejects_malformed_var_forms() {
    // Tightened `is_valid_color_literal`: reject trailing
    // garbage after the closing `)`, empty var name, and nested
    // parens. The previous `starts_with / ends_with` pair let
    // `var(--accent)extra)` slip through and be stored verbatim
    // on the color field — the renderer then fell back to its
    // "malformed hex" path silently.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    // Note: the paste path trims leading/trailing whitespace
    // before validation, so trailing-space cases aren't tested
    // here — they normalise to a valid form.
    for malformed in [
        "var(--accent)extra)", // trailing garbage before the final `)`
        "var(--)",             // empty name
        "var(--foo(bar))",     // nested paren
        "var",                 // no name at all
    ] {
        let tid = TargetId::Edge(er.clone());
        let mut view = view_for(&mut doc, &tid);
        match view.clipboard_paste(malformed) {
            Outcome::Invalid(_) => {}
            other => panic!("expected Invalid for {:?}, got {:?}", malformed, other),
        }
    }
}

#[test]
fn edge_paste_accepts_mixed_case_hex() {
    // CSS-style mixed-case hex (`#AbCdEf`) parses as an ordinary
    // 6-digit hex code. Important that the validator doesn't
    // reject uppercase letters; `is_ascii_hexdigit` covers both
    // cases but the matcher needs to stay case-insensitive.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::Edge(er);
    let mut view = view_for(&mut doc, &tid);
    assert_eq!(view.clipboard_paste("#AbCdEf"), Outcome::Applied);
}

#[test]
fn edge_paste_empty_clears_color_override() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_color(&er, Some("#abcdef"));
    let tid = TargetId::Edge(er.clone());
    {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("");
    }
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(edge
        .glyph_connection
        .as_ref()
        .and_then(|c| c.color.as_deref())
        .is_none());
}

#[test]
fn edge_cut_returns_hex_and_clears_override() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_color(&er, Some("#abcdef"));
    let tid = TargetId::Edge(er.clone());
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert_eq!(cut, ClipboardContent::Text("#abcdef".into()));
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(edge
        .glyph_connection
        .as_ref()
        .and_then(|c| c.color.as_deref())
        .is_none());
}

// ── EdgeLabel ───────────────────────────────────────────────────

#[test]
fn edge_label_copy_returns_resolved_label_color_hex() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_label_color(&er, Some("#ff8800"));
    let tid = TargetId::EdgeLabel(er);
    let view = view_for(&mut doc, &tid);
    match view.clipboard_copy() {
        ClipboardContent::Text(hex) => assert_eq!(hex, "#ff8800"),
        other => panic!("expected Text with hex, got {:?}", other),
    }
}

#[test]
fn edge_label_copy_falls_back_to_edge_color_when_override_absent() {
    // With no `label_config.color` override the cascade resolves
    // through `glyph_connection.color` → `edge.color`; copy
    // reports the final concrete hex (no "Empty" — there's always
    // a resolved colour).
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_color(&er, Some("#445566"));
    // Ensure no label-specific override is set.
    doc.set_edge_label_color(&er, None);
    let tid = TargetId::EdgeLabel(er);
    let view = view_for(&mut doc, &tid);
    match view.clipboard_copy() {
        ClipboardContent::Text(hex) => assert_eq!(hex, "#445566"),
        other => panic!("expected Text with fallback hex, got {:?}", other),
    }
}

#[test]
fn edge_label_paste_valid_hex_sets_label_color_only() {
    // Pasting a colour onto an `EdgeLabel` selection must NOT
    // touch the edge body's own colour cascade — that's the
    // whole point of a separate label channel.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_color(&er, Some("#000000"));
    let tid = TargetId::EdgeLabel(er.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("#ff00ff")
    };
    assert_eq!(outcome, Outcome::Applied);
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(
        edge.label_config.as_ref().and_then(|c| c.color.as_deref()),
        Some("#ff00ff"),
        "label color should land in label_config"
    );
    assert_eq!(
        edge.glyph_connection.as_ref().and_then(|c| c.color.as_deref()),
        Some("#000000"),
        "edge body color must remain unchanged"
    );
}

#[test]
fn edge_label_paste_invalid_content_reports_invalid() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::EdgeLabel(er);
    let mut view = view_for(&mut doc, &tid);
    match view.clipboard_paste("not a color") {
        Outcome::Invalid(msg) => assert!(msg.contains("not a color")),
        other => panic!("expected Invalid, got {:?}", other),
    }
}

#[test]
fn edge_label_cut_returns_hex_and_clears_override() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_label_color(&er, Some("#ff8800"));
    let tid = TargetId::EdgeLabel(er.clone());
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert_eq!(cut, ClipboardContent::Text("#ff8800".into()));
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(edge
        .label_config
        .as_ref()
        .and_then(|c| c.color.as_deref())
        .is_none());
}

// ── PortalText ──────────────────────────────────────────────────

#[test]
fn portal_text_paste_valid_hex_sets_text_color_only() {
    use baumhard::mindmap::model::{is_portal_edge, DISPLAY_MODE_PORTAL};
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    // Convert to portal mode so the endpoint state is meaningful.
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
    assert!(is_portal_edge(&doc.mindmap.edges[idx]));
    let endpoint = doc.mindmap.edges[idx].from_id.clone();
    doc.set_portal_label_color(&er, &endpoint, Some("#000000"));

    let tid = TargetId::PortalText {
        edge: er.clone(),
        endpoint_node_id: endpoint.clone(),
    };
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("#99ccff")
    };
    assert_eq!(outcome, Outcome::Applied);
    // Confirm `text_color` got the paste and the icon `color`
    // was not touched — the two channels are independent by
    // design.
    let state = baumhard::mindmap::model::portal_endpoint_state(&doc.mindmap.edges[idx], &endpoint)
        .expect("endpoint state should exist");
    assert_eq!(state.text_color.as_deref(), Some("#99ccff"));
    assert_eq!(state.color.as_deref(), Some("#000000"));
}

#[test]
fn portal_text_cut_returns_hex_and_clears_text_override() {
    use baumhard::mindmap::model::DISPLAY_MODE_PORTAL;
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
    let endpoint = doc.mindmap.edges[idx].from_id.clone();
    doc.set_portal_label_text_color(&er, &endpoint, Some("#99ccff"));

    let tid = TargetId::PortalText {
        edge: er.clone(),
        endpoint_node_id: endpoint.clone(),
    };
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert_eq!(cut, ClipboardContent::Text("#99ccff".into()));
    let state = baumhard::mindmap::model::portal_endpoint_state(&doc.mindmap.edges[idx], &endpoint);
    assert!(state.and_then(|s| s.text_color.as_deref()).is_none());
}

// Portal-mode icon (PortalLabel) clipboard continues to work as
// before — covered indirectly through the PortalLabel variant
// sharing the `set_portal_label_color` setter. No dedicated
// PortalLabel copy/paste tests are added here because the
// behaviour was unchanged in this commit.
