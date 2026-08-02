// SPDX-License-Identifier: MPL-2.0

//! Per-section text projection. Every renderable
//! [`MindSection`](crate::mindmap::model::MindSection) of a
//! non-folded node becomes one section `GlyphArea` under the node's
//! container, carrying the section text verbatim and the section's
//! absolute AABB. Sections whose authored geometry is degenerate
//! (non-finite offset, non-finite / non-positive explicit size)
//! are skipped entirely — see
//! [`super::super::node::renderable_section`] for why.
//!
//! Empty-text sections are *not* skipped: an empty section still
//! owns the rectangle the user's next keystroke lands in.

use super::fixtures::*;
use crate::mindmap::model::{MindSection, Position, Size};
use crate::mindmap::tree_builder::build_mindmap_tree;

/// One projected section area, reduced to the three things these
/// assertions look at.
struct SectionArea {
    text: String,
    position: (f32, f32),
    bounds: (f32, f32),
}

/// Every section area in the tree, keyed by `(node_id,
/// section_idx)` — the shape these assertions want without
/// repeating the arena walk.
fn section_areas(
    map: &crate::mindmap::model::MindMap,
) -> std::collections::HashMap<(String, usize), SectionArea> {
    let tree = build_mindmap_tree(map);
    tree.section_ids()
        .map(|((mind_id, idx), arena_id)| {
            let area = tree
                .tree
                .arena
                .get(arena_id)
                .and_then(|n| n.get().glyph_area())
                .expect("section arena id must resolve to a GlyphArea");
            (
                (mind_id.to_string(), idx),
                SectionArea {
                    text: area.text.clone(),
                    position: (area.position.x.0, area.position.y.0),
                    bounds: (area.render_bounds.x.0, area.render_bounds.y.0),
                },
            )
        })
        .collect()
}

#[test]
fn test_one_section_area_per_section_including_empty() {
    let mut node = sized_node("n", 0.0, 0.0, 200.0, 200.0, false);
    node.sections = vec![
        MindSection::new_default("alpha".into(), vec![]),
        MindSection::new_default("beta".into(), vec![]),
        MindSection::new_default("".into(), vec![]),
    ];
    let map = synthetic_map(vec![node], vec![]);
    let areas = section_areas(&map);

    assert_eq!(areas.len(), 3, "every renderable section gets an area");
    assert_eq!(areas[&("n".into(), 0)].text, "alpha");
    assert_eq!(areas[&("n".into(), 1)].text, "beta");
    assert_eq!(
        areas[&("n".into(), 2)].text,
        "",
        "an empty section keeps its area — it is where the next keystroke lands"
    );
}

#[test]
fn test_section_area_is_keyed_by_section_idx() {
    let mut node = sized_node("n", 0.0, 0.0, 200.0, 200.0, false);
    node.sections = vec![
        MindSection::new_default("first".into(), vec![]),
        MindSection::new_default("second".into(), vec![]),
    ];
    let map = synthetic_map(vec![node], vec![]);
    let areas = section_areas(&map);
    assert_eq!(areas[&("n".into(), 0)].text, "first");
    assert_eq!(areas[&("n".into(), 1)].text, "second");
}

#[test]
fn test_section_offset_resolves_to_absolute_position() {
    let mut node = sized_node("n", 100.0, 200.0, 300.0, 300.0, false);
    node.sections[0].text = "shifted".into();
    node.sections[0].offset = Position { x: 25.0, y: 40.0 };
    node.sections[0].size = Some(Size {
        width: 50.0,
        height: 30.0,
    });
    let map = synthetic_map(vec![node], vec![]);
    let areas = section_areas(&map);

    let area = &areas[&("n".into(), 0)];
    assert!((area.position.0 - 125.0).abs() < 1e-3, "x: {}", area.position.0);
    assert!((area.position.1 - 240.0).abs() < 1e-3, "y: {}", area.position.1);
    assert!((area.bounds.0 - 50.0).abs() < 1e-3);
    assert!((area.bounds.1 - 30.0).abs() < 1e-3);
}

/// Sections with explicit zero-width or zero-height size skip
/// emission — degenerate bounds would produce a 0-area buffer that
/// confuses renderer shaping, the subtree-AABB cache, and hit-test
/// math alike. The verifier flags these so authors can fix the
/// source.
#[test]
fn test_zero_size_section_skipped() {
    let mut node = sized_node("n", 0.0, 0.0, 200.0, 200.0, false);
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
    let areas = section_areas(&map);

    assert_eq!(areas[&("n".into(), 0)].text, "valid", "valid section must emit");
    assert!(
        !areas.contains_key(&("n".into(), 1)),
        "zero-width section must skip emission"
    );
}

#[test]
fn test_negative_size_section_skipped() {
    let mut node = sized_node("n", 0.0, 0.0, 200.0, 200.0, false);
    let mut bad = MindSection::new_default("neg".into(), vec![]);
    bad.size = Some(Size {
        width: 30.0,
        height: -5.0,
    });
    node.sections = vec![bad];
    let map = synthetic_map(vec![node], vec![]);
    assert!(
        section_areas(&map).is_empty(),
        "negative-height section must skip emission"
    );
}

#[test]
fn test_nan_offset_section_skipped() {
    let mut node = sized_node("n", 0.0, 0.0, 200.0, 200.0, false);
    node.sections[0].text = "nan".into();
    node.sections[0].offset = Position { x: f64::NAN, y: 0.0 };
    let map = synthetic_map(vec![node], vec![]);
    assert!(
        section_areas(&map).is_empty(),
        "NaN-offset section must skip emission"
    );
}

/// §T1 Unicode-edge: the projection must hand grapheme-cluster
/// strings to the renderer untouched — no slicing on UTF-8
/// boundaries, no truncation, no normalization.
#[test]
fn test_section_text_round_trips_zwj_combining_and_flag_emoji() {
    let mut node = sized_node("n", 0.0, 0.0, 200.0, 200.0, false);
    let zwj = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let combining = "e\u{0301}";
    let flag = "\u{1F1EF}\u{1F1F5}";
    let combined = format!("{zwj} {combining} {flag}");
    node.sections[0].text = combined.clone();
    let map = synthetic_map(vec![node], vec![]);
    let areas = section_areas(&map);
    assert_eq!(areas.len(), 1);
    assert_eq!(
        areas[&("n".into(), 0)].text,
        combined,
        "projected text must match the section source byte-for-byte"
    );
}

/// A node whose only section has empty text still contributes a
/// border (border layout is node-level, not section-level) and a
/// clip AABB.
#[test]
fn test_empty_section_still_yields_border_and_clip_aabb() {
    let mut node = sized_node("n", 0.0, 0.0, 80.0, 40.0, true);
    node.sections[0].text = "".into();
    let map = synthetic_map(vec![node], vec![]);
    let projected = project(&map, 1.0);

    assert_eq!(
        projected.border_nodes.len(),
        1,
        "border emission stays node-level"
    );
    assert_eq!(projected.node_aabbs.len(), 1, "the node still clips connections");
}
