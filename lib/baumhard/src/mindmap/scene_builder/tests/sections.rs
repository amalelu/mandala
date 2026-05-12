// SPDX-License-Identifier: MPL-2.0

//! Per-section text-element emission tests for the scene builder.
//! Every visible [`MindSection`](crate::mindmap::model::MindSection)
//! with non-empty text emits one `TextElement`; empty-text
//! sections skip emission. Multi-section nodes produce as many
//! `TextElement`s as they have non-empty sections, each carrying
//! the section's index for downstream selection / hit-test
//! routing.

use super::fixtures::*;
use crate::mindmap::model::{MindSection, Position, Size};
use crate::mindmap::scene_builder::build_scene;

#[test]
fn test_one_text_element_per_non_empty_section() {
    let mut node = synthetic_node("n", 0.0, 0.0, 200.0, 200.0, false);
    node.sections = vec![
        MindSection::new_default("alpha".into(), vec![]),
        MindSection::new_default("beta".into(), vec![]),
        MindSection::new_default("".into(), vec![]), // empty -> skipped
    ];
    let map = synthetic_map(vec![node], vec![]);
    let scene = build_scene(&map, 1.0);

    assert_eq!(scene.text_elements.len(), 2, "two non-empty sections");
    let texts: Vec<&str> = scene.text_elements.iter().map(|t| t.text.as_str()).collect();
    assert!(texts.contains(&"alpha"));
    assert!(texts.contains(&"beta"));
    // The skipped empty section must not appear as a text element.
    assert!(!texts.iter().any(|t| t.is_empty()));
}

#[test]
fn test_text_element_carries_section_idx() {
    let mut node = synthetic_node("n", 0.0, 0.0, 200.0, 200.0, false);
    node.sections = vec![
        MindSection::new_default("first".into(), vec![]),
        MindSection::new_default("second".into(), vec![]),
    ];
    let map = synthetic_map(vec![node], vec![]);
    let scene = build_scene(&map, 1.0);

    let by_text: std::collections::HashMap<&str, usize> = scene
        .text_elements
        .iter()
        .map(|t| (t.text.as_str(), t.section_idx))
        .collect();
    assert_eq!(by_text.get("first"), Some(&0));
    assert_eq!(by_text.get("second"), Some(&1));
}

#[test]
fn test_section_offset_resolves_to_absolute_position() {
    let mut node = synthetic_node("n", 100.0, 200.0, 300.0, 300.0, false);
    node.sections[0].text = "shifted".into();
    node.sections[0].offset = Position { x: 25.0, y: 40.0 };
    node.sections[0].size = Some(Size {
        width: 50.0,
        height: 30.0,
    });
    let map = synthetic_map(vec![node], vec![]);
    let scene = build_scene(&map, 1.0);

    assert_eq!(scene.text_elements.len(), 1);
    let elem = &scene.text_elements[0];
    assert!((elem.position.0 - 125.0).abs() < 1e-3, "x: {}", elem.position.0);
    assert!((elem.position.1 - 240.0).abs() < 1e-3, "y: {}", elem.position.1);
    assert!((elem.size.0 - 50.0).abs() < 1e-3);
    assert!((elem.size.1 - 30.0).abs() < 1e-3);
}

/// Sections with explicit zero-width or zero-height size skip
/// emission — degenerate bounds would produce a 0-area buffer
/// that confuses both renderer shaping and hit-test math. The
/// verifier flags these so authors can fix the source.
#[test]
fn test_zero_size_section_skipped() {
    let mut node = synthetic_node("n", 0.0, 0.0, 200.0, 200.0, false);
    node.sections[0].text = "valid".into();
    node.sections[0].size = Some(Size {
        width: 100.0,
        height: 50.0,
    });
    let mut bad = MindSection::new_default("zero".into(), vec![]);
    bad.size = Some(Size {
        width: 0.0,
        height: 30.0,
    });
    node.sections.push(bad);
    let map = synthetic_map(vec![node], vec![]);
    let scene = build_scene(&map, 1.0);

    let texts: Vec<&str> = scene.text_elements.iter().map(|t| t.text.as_str()).collect();
    assert!(texts.contains(&"valid"), "valid section must emit");
    assert!(!texts.contains(&"zero"), "zero-width section must skip emission");
}

#[test]
fn test_negative_size_section_skipped() {
    let mut node = synthetic_node("n", 0.0, 0.0, 200.0, 200.0, false);
    let mut bad = MindSection::new_default("neg".into(), vec![]);
    bad.size = Some(Size {
        width: 30.0,
        height: -5.0,
    });
    node.sections = vec![bad];
    let map = synthetic_map(vec![node], vec![]);
    let scene = build_scene(&map, 1.0);
    assert!(
        scene.text_elements.is_empty(),
        "negative-height section must skip emission"
    );
}

#[test]
fn test_nan_offset_section_skipped() {
    let mut node = synthetic_node("n", 0.0, 0.0, 200.0, 200.0, false);
    node.sections[0].text = "nan".into();
    node.sections[0].offset = Position { x: f64::NAN, y: 0.0 };
    let map = synthetic_map(vec![node], vec![]);
    let scene = build_scene(&map, 1.0);
    assert!(
        scene.text_elements.is_empty(),
        "NaN-offset section must skip emission"
    );
}

/// §T1 Unicode-edge: scene builder must hand grapheme-cluster
/// strings to the renderer untouched — no slicing on UTF-8
/// boundaries, no truncation, no normalisation.
#[test]
fn test_section_text_round_trips_zwj_combining_and_flag_emoji() {
    let mut node = synthetic_node("n", 0.0, 0.0, 200.0, 200.0, false);
    let zwj = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let combining = "e\u{0301}";
    let flag = "\u{1F1EF}\u{1F1F5}";
    let combined = format!("{zwj} {combining} {flag}");
    node.sections[0].text = combined.clone();
    let map = synthetic_map(vec![node], vec![]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.text_elements.len(), 1);
    assert_eq!(
        scene.text_elements[0].text, combined,
        "scene-emitted text must match the section source byte-for-byte"
    );
}

#[test]
fn test_empty_section_renders_no_text_but_keeps_aabb_for_borders() {
    // A node whose only section has empty text should still emit a
    // border element + clip AABB (the border layout happens at the
    // node level, not the section level).
    let mut node = synthetic_node("n", 0.0, 0.0, 80.0, 40.0, true);
    node.sections[0].text = "".into();
    let map = synthetic_map(vec![node], vec![]);
    let scene = build_scene(&map, 1.0);

    assert!(scene.text_elements.is_empty(), "no text → no text elements");
    assert_eq!(scene.border_elements.len(), 1, "border emission stays node-level");
}
