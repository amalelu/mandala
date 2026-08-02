// SPDX-License-Identifier: MPL-2.0

//! Node text / background / border / text-color / font-size
//! setters, plus the per-call-site contract of the shared
//! `nodes/undo_envelope.rs` envelope they all route through.
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::tests_common::{first_n_testament_node_ids, first_testament_node_id, load_test_doc};
use super::*;

use baumhard::mindmap::model::{MindNode, MindSection, NodeLayout, NodeStyle, Position, Size, TextRun};
use baumhard::util::grapheme_chad::count_grapheme_clusters;

#[test]
fn test_set_node_text_updates_text_and_collapses_runs() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let changed = doc.set_node_text(&nid, "Hello world".to_string());
    assert!(changed);
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(node.sections[0].text, "Hello world");
    assert_eq!(node.sections[0].text_runs.len(), 1);
    assert_eq!(node.sections[0].text_runs[0].start, 0);
    assert_eq!(
        node.sections[0].text_runs[0].end,
        count_grapheme_clusters("Hello world")
    );
    assert!(doc.dirty);
    assert!(matches!(
        doc.undo_stack.last(),
        Some(UndoAction::EditNodeText { .. })
    ));
}

/// `set_section_text(node, idx, text)` writes through to the
/// requested section — section 0 gets the same behaviour as the
/// pre-section `set_node_text`, sections 1+ stay untouched
/// unless explicitly targeted. Pins the section-aware setter's
/// addressing for the per-section text-edit path.
#[test]
fn test_set_section_text_targets_specific_section() {
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    // Materialise a multi-section node by appending a second
    // section to the existing testament root.
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), vec![]));
    }
    doc.undo_stack.clear();
    doc.dirty = false;

    // Edit section 1 only — section 0 must stay untouched.
    let s0_before = doc.mindmap.nodes.get(&nid).unwrap().sections[0].text.clone();
    assert!(doc.set_section_text(&nid, 1, "rewrote section 1".to_string()));
    let n = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(n.sections[0].text, s0_before, "section 0 untouched");
    assert_eq!(n.sections[1].text, "rewrote section 1");
    // Undo restores both sections.
    assert!(doc.undo());
    let n = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(n.sections[1].text, "second");
}

/// §T1 Unicode-edge: `set_section_text` round-trips ZWJ-emoji,
/// combining marks, and flag emoji byte-for-byte; the auto-
/// regenerated text-run's `end` matches grapheme-cluster count
/// (not codepoint or byte count). Catches the
/// `count_grapheme_clusters` accidentally being swapped for
/// `chars().count()` or `len()` — a regression that would
/// silently truncate emoji text on the next render.
#[test]
fn test_set_section_text_grapheme_handling_for_emoji_and_combining() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let zwj = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let combining = "e\u{0301}";
    let flag = "\u{1F1EF}\u{1F1F5}";
    let combined = format!("{zwj} {combining} {flag}");
    assert!(doc.set_section_text(&nid, 0, combined.clone()));
    let n = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(n.sections[0].text, combined, "text round-trips byte-for-byte");
    let cluster_count = count_grapheme_clusters(&combined);
    assert!(
        n.sections[0].text_runs.iter().all(|r| r.end <= cluster_count),
        "every run.end must fit within the {} grapheme clusters",
        cluster_count
    );
    // Tightened: every run.end must EQUAL the cluster count
    // (not just `<=`), so a regression that emits zero runs or
    // truncates the auto-collapsed run by even one grapheme
    // trips the test. The `<=` form would silently pass a
    // dropped trailing emoji.
    let runs = &n.sections[0].text_runs;
    assert!(!runs.is_empty(), "auto-collapsed run must exist");
    assert_eq!(runs[0].start, 0, "auto-collapsed run starts at grapheme index 0");
    assert_eq!(
        runs[0].end, cluster_count,
        "auto-collapsed run ends at the cluster count, not the codepoint or byte count"
    );
}

// ── Section offset / size setters ──────────────────────────────
//
// Validation rules + rejection messages mirror
// `crates/maptool/src/verify/sections.rs`. Shared fixture lives
// at `tests_common::pinned_two_section_node`.

#[test]
fn test_set_section_offset_writes_and_round_trips_through_undo() {
    use super::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    assert_eq!(doc.set_section_offset(&id, 1, 20.0, 25.0), Ok(true));
    let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
    assert_eq!(s.offset.x, 20.0);
    assert_eq!(s.offset.y, 25.0);
    assert!(doc.undo());
    let restored = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
    assert_eq!(restored.offset.x, 10.0, "undo restores prior offset");
    assert_eq!(restored.offset.y, 10.0);
}

#[test]
fn test_set_section_offset_idempotent_no_op() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert_eq!(doc.set_section_offset(&id, 1, 10.0, 10.0), Ok(false));
    assert!(doc.undo_stack.is_empty(), "no-op must not push undo");
    assert!(!doc.dirty);
}

#[test]
fn test_set_section_offset_rejects_nan_and_inf() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert!(doc
        .set_section_offset(&id, 1, f64::NAN, 0.0)
        .is_err_and(|m| m.contains("non-finite")));
    assert!(doc
        .set_section_offset(&id, 1, f64::INFINITY, 0.0)
        .is_err_and(|m| m.contains("non-finite")));
}

#[test]
fn test_set_section_offset_rejects_negative_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert!(doc
        .set_section_offset(&id, 1, -1.0, 0.0)
        .is_err_and(|m| m.contains("section[1].offset.x is negative")));
    assert!(doc
        .set_section_offset(&id, 1, 0.0, -2.0)
        .is_err_and(|m| m.contains("section[1].offset.y is negative")));
}

#[test]
fn test_set_section_offset_rejects_aabb_overflow_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // section[1] size 50×30; offset (160,0) → right=210 > 200.
    assert!(doc
        .set_section_offset(&id, 1, 160.0, 0.0)
        .is_err_and(|m| m.contains("extends past node right edge")));
    // offset (0,80) → bottom=110 > 100.
    assert!(doc
        .set_section_offset(&id, 1, 0.0, 80.0)
        .is_err_and(|m| m.contains("extends past node bottom edge")));
}

#[test]
fn test_set_section_offset_unknown_section_returns_false() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert_eq!(doc.set_section_offset(&id, 99, 0.0, 0.0), Ok(false));
}

#[test]
fn test_set_section_size_writes_and_round_trips_through_undo() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let new_size = Some(baumhard::mindmap::model::Size {
        width: 80.0,
        height: 40.0,
    });
    assert_eq!(doc.set_section_size(&id, 1, new_size.clone()), Ok(true));
    assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections[1].size, new_size);
    assert!(doc.undo());
    assert_eq!(
        doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .size
            .as_ref()
            .unwrap()
            .width,
        50.0,
        "undo restores prior size"
    );
}

#[test]
fn test_set_section_size_none_restores_fill_parent() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Flatten-to-fill-parent is only legal at offset (0, 0)
    // post the C3 effective-size fix; the fixture pins
    // section[1] at offset (10, 10), so reset before flattening.
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.sections[1].offset = baumhard::mindmap::model::Position { x: 0.0, y: 0.0 };
    }
    assert_eq!(doc.set_section_size(&id, 1, None), Ok(true));
    assert!(doc.mindmap.nodes.get(&id).unwrap().sections[1].size.is_none());
}

#[test]
fn test_set_section_size_rejects_zero_and_negative_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let zero = Some(baumhard::mindmap::model::Size {
        width: 0.0,
        height: 30.0,
    });
    assert!(doc
        .set_section_size(&id, 1, zero)
        .is_err_and(|m| m.contains("size.width is not positive")));
    let neg = Some(baumhard::mindmap::model::Size {
        width: 30.0,
        height: -5.0,
    });
    assert!(doc
        .set_section_size(&id, 1, neg)
        .is_err_and(|m| m.contains("size.height is not positive")));
}

#[test]
fn test_set_section_size_rejects_overflow_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Offset (10,10) + width 200 = 210 > node.size.width 200.
    let overflow = Some(baumhard::mindmap::model::Size {
        width: 200.0,
        height: 30.0,
    });
    assert!(doc
        .set_section_size(&id, 1, overflow)
        .is_err_and(|m| m.contains("extends past node right edge")));
}

#[test]
fn test_set_section_size_rejects_astronomical_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // node 200×100, 100× = 20000. 25000 trips the typo guard.
    let huge = Some(baumhard::mindmap::model::Size {
        width: 25000.0,
        height: 30.0,
    });
    assert!(doc
        .set_section_size(&id, 1, huge)
        .is_err_and(|m| m.contains("over 100× the node's width")));
}

/// Symmetric height-axis pin for the astronomical-typo guard —
/// ensures both width and height branches are reached.
#[test]
fn test_set_section_size_rejects_astronomical_height_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let huge = Some(baumhard::mindmap::model::Size {
        width: 30.0,
        height: 25000.0,
    });
    assert!(doc
        .set_section_size(&id, 1, huge)
        .is_err_and(|m| m.contains("over 100× the node's height")));
}

/// Non-finite size component rejected by `validate_section_aabb`.
/// Pin both width and height branches so a regression that drops
/// one ships visibly.
#[test]
fn test_set_section_size_rejects_non_finite_components() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let nan_w = Some(baumhard::mindmap::model::Size {
        width: f64::NAN,
        height: 30.0,
    });
    assert!(doc
        .set_section_size(&id, 1, nan_w)
        .is_err_and(|m| m.contains("size has non-finite component")));
    let inf_h = Some(baumhard::mindmap::model::Size {
        width: 30.0,
        height: f64::INFINITY,
    });
    assert!(doc
        .set_section_size(&id, 1, inf_h)
        .is_err_and(|m| m.contains("size has non-finite component")));
}

#[test]
fn test_set_section_size_idempotent_no_op() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let same = Some(baumhard::mindmap::model::Size {
        width: 50.0,
        height: 30.0,
    });
    assert_eq!(doc.set_section_size(&id, 1, same), Ok(false));
    assert!(doc.undo_stack.is_empty(), "no-op must not push undo");
}

/// `set_section_size(None)` rejects when the section's existing
/// offset is non-zero — flatten-to-fill-parent on a section
/// pinned at `(5, 0)` would produce an effective AABB
/// `((5, 0), node.size)` that overflows the parent's right
/// edge. Closes the symmetric hole to `set_section_offset`'s
/// effective-size check.
#[test]
fn test_set_section_size_rejects_none_when_offset_nonzero() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Move section[1] so it has an explicit non-zero offset
    // *and* an explicit size that fits at that offset.
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.sections[1].offset = baumhard::mindmap::model::Position { x: 5.0, y: 0.0 };
        node.sections[1].size = Some(baumhard::mindmap::model::Size {
            width: 50.0,
            height: 30.0,
        });
    }
    // Flatten to fill-parent — effective AABB becomes
    // ((5, 0), (200, 100)) — right=205 > node 200.
    assert!(doc
        .set_section_size(&id, 1, None)
        .is_err_and(|m| m.contains("extends past node right edge")));
}

/// `set_section_size(None)` accepts when offset is `(0, 0)` —
/// the canonical fill-parent shape.
#[test]
fn test_set_section_size_accepts_none_at_zero_offset() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.sections[1].offset = baumhard::mindmap::model::Position { x: 0.0, y: 0.0 };
    }
    assert_eq!(doc.set_section_size(&id, 1, None), Ok(true));
    assert!(doc.mindmap.nodes[&id].sections[1].size.is_none());
}

/// `set_section_offset` rejects non-zero offset on a `None`-
/// sized (fill-parent) section — the section's effective size
/// is `node.size`, so any non-zero offset stretches past the
/// node's right / bottom edge. Mirrors the verify rule.
#[test]
fn test_set_section_offset_rejects_nonzero_on_none_sized_section() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Section[0] is None-sized (fill-parent); the fixture only
    // pins section[1]'s explicit Some-size.
    assert!(
        doc.mindmap.nodes[&id].sections[0].size.is_none(),
        "fixture invariant"
    );
    let result = doc.set_section_offset(&id, 0, 5.0, 0.0);
    assert!(result.is_err_and(|m| m.contains("extends past node right edge")));
}

#[test]
fn test_set_section_offset_accepts_zero_on_none_sized_section() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert!(
        doc.mindmap.nodes[&id].sections[0].size.is_none(),
        "fixture invariant"
    );
    // Already at (0,0) → no-op false; not an error.
    let result = doc.set_section_offset(&id, 0, 0.0, 0.0);
    assert_eq!(result, Ok(false));
}

// ── set_node_size / set_node_aabb (atomic node resize) ───────────

#[test]
fn test_set_node_size_writes_and_round_trips_through_undo() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let before = doc.mindmap.nodes[&id].size;
    // Use a target large enough to fit any reasonable testament-
    // node text floor — `grow_one_node_to_fit_text` runs after
    // the setter and would bump a small target up to the text
    // floor, masking the round-trip pin.
    let target = Size {
        width: 800.0,
        height: 400.0,
    };
    assert_eq!(doc.set_node_size(&id, target), Ok(true));
    let after = doc.mindmap.nodes[&id].size;
    assert_eq!(after.width, 800.0);
    assert_eq!(after.height, 400.0);
    assert!(doc.undo());
    assert_eq!(doc.mindmap.nodes[&id].size, before, "undo restores prior size");
}

/// Setter applies `grow_one_node_to_fit_text` after the size
/// write, so a request below the measured-text floor lands at
/// the floor rather than the requested value.
#[test]
fn test_set_node_size_below_text_floor_lands_at_floor() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let tiny = Size {
        width: 5.0,
        height: 5.0,
    };
    assert_eq!(doc.set_node_size(&id, tiny), Ok(true));
    let after = doc.mindmap.nodes[&id].size;
    // Both axes must clear the requested tiny floor — a
    // regression that grows only one axis is the exact bug
    // shape we're guarding against.
    assert!(
        after.width > 5.0 && after.height > 5.0,
        "floor-respect must grow both axes above the tiny target ({}x{})",
        after.width, after.height
    );
}

#[test]
fn test_set_node_size_idempotent_no_op() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Land at a known size above the text floor first — the
    // post-grow no-op gate compares the post-mutation size
    // against the pre-mutation size, so the second call must
    // match the post-grow shape of the first.
    let target = Size {
        width: 800.0,
        height: 400.0,
    };
    assert_eq!(doc.set_node_size(&id, target), Ok(true));
    let undo_before = doc.undo_stack.len();
    // Second call with the same target — post-grow size will
    // match (no border-grow on this fixture's unframed root),
    // so the gate fires and we return Ok(false).
    assert_eq!(doc.set_node_size(&id, target), Ok(false));
    assert_eq!(doc.undo_stack.len(), undo_before);
}

#[test]
fn test_set_node_size_rejects_non_finite() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    assert!(doc
        .set_node_size(
            &id,
            Size {
                width: f64::NAN,
                height: 10.0
            }
        )
        .is_err_and(|m| m.contains("non-finite")));
    assert!(doc
        .set_node_size(
            &id,
            Size {
                width: 10.0,
                height: f64::INFINITY
            }
        )
        .is_err_and(|m| m.contains("non-finite")));
}

#[test]
fn test_set_node_size_rejects_non_positive() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    assert!(doc
        .set_node_size(
            &id,
            Size {
                width: 0.0,
                height: 10.0
            }
        )
        .is_err_and(|m| m.contains("is not positive")));
    assert!(doc
        .set_node_size(
            &id,
            Size {
                width: 10.0,
                height: -5.0
            }
        )
        .is_err_and(|m| m.contains("is not positive")));
}

/// Framed-node idempotency: `set_node_size` on a framed
/// node, where `grow_one_node_to_fit_border` inflates the
/// post-write size, must still no-op on a repeated identical
/// call. Pre-fix the no-op gate compared `new_size` against
/// pre-mutation `node.size`, missed on every post-first call,
/// and stacked `EditNodeAabb` undo entries.
#[test]
fn test_set_node_size_idempotent_on_framed_node() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    doc.mindmap.nodes.get_mut(&id).unwrap().style.show_frame = true;
    doc.undo_stack.clear();
    let target = Size {
        width: 800.0,
        height: 400.0,
    };
    assert_eq!(doc.set_node_size(&id, target), Ok(true));
    let after_first = doc.mindmap.nodes[&id].size;
    let undo_after_first = doc.undo_stack.len();
    // Second identical call must be a no-op even though the
    // border-grow likely inflated the post-write size past
    // `target`.
    assert_eq!(doc.set_node_size(&id, target), Ok(false));
    assert_eq!(
        doc.undo_stack.len(),
        undo_after_first,
        "framed-node set_node_size must not stack undo entries"
    );
    assert_eq!(doc.mindmap.nodes[&id].size, after_first);
}

/// Same framed-idempotency contract for `set_node_aabb`.
#[test]
fn test_set_node_aabb_idempotent_on_framed_node() {
    use baumhard::mindmap::model::{Position, Size};
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    doc.mindmap.nodes.get_mut(&id).unwrap().style.show_frame = true;
    doc.undo_stack.clear();
    let target_pos = Position { x: 100.0, y: 100.0 };
    let target_size = Size {
        width: 800.0,
        height: 400.0,
    };
    assert_eq!(doc.set_node_aabb(&id, target_pos, target_size), Ok(true));
    let after_first = doc.mindmap.nodes[&id].size;
    let undo_after_first = doc.undo_stack.len();
    assert_eq!(doc.set_node_aabb(&id, target_pos, target_size), Ok(false));
    assert_eq!(
        doc.undo_stack.len(),
        undo_after_first,
        "framed-node set_node_aabb must not stack undo entries"
    );
    assert_eq!(doc.mindmap.nodes[&id].size, after_first);
}

#[test]
fn test_set_node_size_rejects_astronomical_typo() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Absolute ceiling at 1_000_000 — value past it trips the
    // typo guard. Independent of the prior-size baseline so a
    // tiny-to-large drag at the gesture's release-commit isn't
    // silently rejected.
    let huge = Size {
        width: 2_000_000.0,
        height: 10.0,
    };
    assert!(doc.set_node_size(&id, huge).is_err_and(|m| m.contains("exceeds the")));
}

/// `set_node_aabb` writes both fields atomically and pushes one
/// `EditNodeAabb` undo entry. Used by the resize gesture's
/// release-commit. Uses a target large enough to fit testament
/// text so the floor-respect pass leaves the requested size
/// untouched and the round-trip pin is exact.
#[test]
fn test_set_node_aabb_writes_position_and_size_atomically() {
    use baumhard::mindmap::model::{Position, Size};
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let before_pos = doc.mindmap.nodes[&id].position;
    let before_size = doc.mindmap.nodes[&id].size;
    let new_pos = Position {
        x: before_pos.x + 10.0,
        y: before_pos.y + 5.0,
    };
    let new_size = Size {
        width: 800.0,
        height: 400.0,
    };
    let undo_before = doc.undo_stack.len();
    assert_eq!(doc.set_node_aabb(&id, new_pos, new_size), Ok(true));
    assert_eq!(doc.mindmap.nodes[&id].position, new_pos);
    assert_eq!(doc.mindmap.nodes[&id].size, new_size);
    assert_eq!(doc.undo_stack.len(), undo_before + 1);
    // Undo restores both.
    assert!(doc.undo());
    assert_eq!(doc.mindmap.nodes[&id].position, before_pos);
    assert_eq!(doc.mindmap.nodes[&id].size, before_size);
}

#[test]
fn test_set_node_aabb_idempotent_no_op() {
    use baumhard::mindmap::model::{Position, Size};
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Land above the text floor first — same shape as the
    // sibling `set_node_size` idempotency test.
    let target_pos = Position { x: 100.0, y: 100.0 };
    let target_size = Size {
        width: 800.0,
        height: 400.0,
    };
    assert_eq!(doc.set_node_aabb(&id, target_pos, target_size), Ok(true));
    let undo_before = doc.undo_stack.len();
    assert_eq!(doc.set_node_aabb(&id, target_pos, target_size), Ok(false));
    assert_eq!(doc.undo_stack.len(), undo_before);
}

#[test]
fn test_set_node_aabb_accepts_negative_position() {
    use baumhard::mindmap::model::{Position, Size};
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Nodes float freely on canvas; negative positions are legal.
    let result = doc.set_node_aabb(
        &id,
        Position { x: -50.0, y: -20.0 },
        Size {
            width: 60.0,
            height: 30.0,
        },
    );
    assert_eq!(result, Ok(true));
}

#[test]
fn test_set_node_aabb_rejects_non_finite_position() {
    use baumhard::mindmap::model::{Position, Size};
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let result = doc.set_node_aabb(
        &id,
        Position { x: f64::NAN, y: 0.0 },
        Size {
            width: 60.0,
            height: 30.0,
        },
    );
    assert!(result.is_err_and(|m| m.contains("non-finite")));
}

// ── compute_one_node_text_floor (the shared floor helper) ────────

/// Pinned section size dominates measured text in the floor
/// computation. Locks the "size as floor" contract directly on
/// the helper, not just through its consumers.
#[test]
fn test_compute_one_node_text_floor_pinned_size_acts_as_floor() {
    use super::compute_one_node_text_floor;
    use baumhard::mindmap::model::Size;
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Pin section[1] way past any text floor.
    doc.mindmap.nodes.get_mut(&id).unwrap().sections[1].size = Some(Size {
        width: 500.0,
        height: 200.0,
    });
    let node = &doc.mindmap.nodes[&id];
    let (w, h) = compute_one_node_text_floor(node);
    // section[1]'s offset+size = (10+500, 10+200) = (510, 210).
    assert!(w >= 510.0, "pinned width must propagate, got {}", w);
    assert!(h >= 210.0, "pinned height must propagate, got {}", h);
}

/// A non-finite section offset is skipped — the verifier flags
/// it elsewhere, and a NaN propagating into the floor would
/// corrupt every downstream `node.size` reader.
#[test]
fn test_compute_one_node_text_floor_skips_non_finite_offset() {
    use super::compute_one_node_text_floor;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    {
        let n = doc.mindmap.nodes.get_mut(&id).unwrap();
        n.sections[0].offset = baumhard::mindmap::model::Position {
            x: f64::NAN,
            y: 0.0,
        };
    }
    let (w, h) = compute_one_node_text_floor(&doc.mindmap.nodes[&id]);
    assert!(w.is_finite());
    assert!(h.is_finite());
}

// ── fit_node_to_content (auto-fit shrink path) ──────────────────

/// `fit_node_to_content` shrinks an over-sized node to its
/// measured-text floor and pushes one `EditNodeAabb` undo
/// entry. The path that lets users recover from a manual resize
/// that pinned the node larger than its content.
#[test]
fn test_fit_node_to_content_shrinks_to_floor() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    doc.mindmap.nodes.get_mut(&id).unwrap().size = Size {
        width: 5000.0,
        height: 5000.0,
    };
    doc.undo_stack.clear();
    assert_eq!(doc.fit_node_to_content(&id), Ok(true));
    let after = doc.mindmap.nodes[&id].size;
    assert!(
        after.width < 5000.0 && after.height < 5000.0,
        "fit-to-content must shrink the node"
    );
    // Undo restores the prior (over-sized) state.
    assert!(doc.undo());
    assert_eq!(doc.mindmap.nodes[&id].size.width, 5000.0);
}

#[test]
fn test_fit_node_to_content_idempotent_no_op() {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // First call lands at the floor.
    assert_eq!(doc.fit_node_to_content(&id), Ok(true));
    let undo_after_first = doc.undo_stack.len();
    // Second call is a no-op.
    assert_eq!(doc.fit_node_to_content(&id), Ok(false));
    assert_eq!(
        doc.undo_stack.len(),
        undo_after_first,
        "second fit-to-content must not push another undo entry"
    );
}

#[test]
fn test_fit_node_to_content_unknown_node_returns_false() {
    let mut doc = load_test_doc();
    assert_eq!(doc.fit_node_to_content("nope"), Ok(false));
}

#[test]
fn test_fit_node_to_content_pinned_section_size_acts_as_floor() {
    use baumhard::mindmap::model::Size;
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // section[1] is pinned at (10, 10) size 50×30 — its
    // contribution to the floor is offset+size = (60, 40).
    // The fit-to-content target is the max of the pinned-section
    // floor and section[0]'s text-driven size; assert that the
    // pinned axis floor survives. (Section[0] may pull width
    // past 60 via testament text, so we assert >= rather than
    // == on width; height has no large contributor in section[0]
    // beyond default padding so the pinned 40 is the dominant
    // axis there.)
    doc.mindmap.nodes.get_mut(&id).unwrap().size = Size {
        width: 5000.0,
        height: 5000.0,
    };
    doc.undo_stack.clear();
    assert_eq!(doc.fit_node_to_content(&id), Ok(true));
    let after = doc.mindmap.nodes[&id].size;
    assert!(
        after.width >= 60.0 && after.height >= 40.0,
        "pinned section[1]'s offset+size contribution must survive, got {}×{}",
        after.width,
        after.height,
    );
}

/// Idempotency must hold for **framed** nodes too. Pre-fix, a
/// framed node's `grow_one_node_to_fit_border` pulled `n.size`
/// past the bare text floor, so the no-op gate (which compared
/// against the bare floor) missed on every call after the
/// first — repeated `fit_node_to_content` calls stacked
/// `EditNodeAabb` undo entries. Post-fix, the gate compares the
/// *post-border-grow* size against `before_size`, which holds
/// across calls.
#[test]
fn test_fit_node_to_content_idempotent_on_framed_node() {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Force the testament root to wear a frame.
    doc.mindmap.nodes.get_mut(&id).unwrap().style.show_frame = true;
    doc.undo_stack.clear();
    // First call lands at the framed floor.
    assert_eq!(doc.fit_node_to_content(&id), Ok(true));
    let undo_after_first = doc.undo_stack.len();
    let size_after_first = doc.mindmap.nodes[&id].size;
    // Second call must be a no-op even though the border-grow
    // pulled the post-floor size up past the bare text floor.
    assert_eq!(doc.fit_node_to_content(&id), Ok(false));
    assert_eq!(
        doc.undo_stack.len(),
        undo_after_first,
        "framed-node fit-to-content must not stack undo entries"
    );
    assert_eq!(doc.mindmap.nodes[&id].size, size_after_first);
}

/// `fit_node_to_content` rejects with the verify-mirror-style
/// message when the floor is non-finite — exercises the
/// finite-check guard added in the self-audit fixup. We force
/// the rejection by clearing every section's text and runs;
/// `compute_one_node_text_floor` then yields a (pad-only,
/// pad-only) tuple. Empty-text sections still produce a finite
/// positive floor (pad), so this test in practice exercises the
/// idempotent-`<=0` rejection only when the loader-rejected
/// "every section empty" state is forced through the model
/// directly. Synthesize the unreachable state here to pin the
/// rejection-path coverage.
#[test]
fn test_fit_node_to_content_rejects_unmeasurable_floor() {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Construct an unreachable-via-loader state: a single
    // section with NaN offset. `compute_one_node_text_floor`
    // skips non-finite-offset sections, so floor stays (0, 0).
    {
        let n = doc.mindmap.nodes.get_mut(&id).unwrap();
        n.sections.clear();
        n.sections.push(baumhard::mindmap::model::MindSection::new_default(
            "x".into(),
            Vec::new(),
        ));
        n.sections[0].offset = baumhard::mindmap::model::Position {
            x: f64::NAN,
            y: 0.0,
        };
    }
    let result = doc.fit_node_to_content(&id);
    assert!(
        result.is_err_and(|m| m.contains("no measurable text")),
        "expected unmeasurable-floor error"
    );
}

/// Pinned `section.size` past the absolute typo ceiling
/// (`MAX_NODE_AXIS = 1_000_000`) propagates through the floor.
/// `fit_node_to_content` must route the candidate through
/// `check_node_size_typo` like the sibling node-size setters,
/// so the typo is caught even when it arrives via a pinned
/// section rather than a direct setter argument.
#[test]
fn test_fit_node_to_content_rejects_astronomical_pinned_section() {
    use baumhard::mindmap::model::Size;
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Pin section[1] to a width that exceeds the typo ceiling.
    doc.mindmap.nodes.get_mut(&id).unwrap().sections[1].size = Some(Size {
        width: 2_000_000.0,
        height: 30.0,
    });
    let result = doc.fit_node_to_content(&id);
    assert!(
        result.is_err_and(|m| m.contains("exceeds the")),
        "pinned-section typo must be caught at fit-to-content"
    );
}

// ── set_section_aabb (atomic offset+size for the resize gesture) ──

/// `set_section_aabb` accepts a W-grow gesture's final state —
/// section pinned at `offset.x = 90` with `size.width = 10` inside
/// a 100-wide node, gesture shrinks `offset.x` to 85 and grows
/// `size.width` to 15. Atomic validation against the **post-
/// mutation** AABB passes. The pre-fix `set_section_size` then
/// `set_section_offset` two-step rejected this transition because
/// `set_section_size(15)` validated against the *unchanged*
/// `offset.x = 90`, computing `right = 90 + 15 = 105 > 100`.
#[test]
fn test_set_section_aabb_accepts_w_grow_against_right_edge() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Reposition section[1] to be flush against the right edge.
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.sections[1].offset = Position { x: 90.0, y: 10.0 };
        node.sections[1].size = Some(Size {
            width: 10.0,
            height: 30.0,
        });
    }
    doc.undo_stack.clear();
    doc.dirty = false;
    // W-grow: offset.x 90 → 85, size.width 10 → 15. Right edge
    // stays at 100.
    let result = doc.set_section_aabb(
        &id,
        1,
        Position { x: 85.0, y: 10.0 },
        Size {
            width: 15.0,
            height: 30.0,
        },
    );
    assert_eq!(result, Ok(true));
    let n = &doc.mindmap.nodes[&id];
    assert_eq!(n.sections[1].offset.x, 85.0);
    assert_eq!(n.sections[1].size.as_ref().unwrap().width, 15.0);
}

/// `set_section_aabb` rejects post-mutation overflow with the
/// verify-mirror message — same shape as `set_section_size` /
/// `set_section_offset`.
#[test]
fn test_set_section_aabb_rejects_post_mutation_overflow() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let result = doc.set_section_aabb(
        &id,
        1,
        Position { x: 50.0, y: 10.0 },
        Size {
            width: 200.0,
            height: 30.0,
        },
    );
    assert!(result.is_err_and(|m| m.contains("extends past node right edge")));
}

#[test]
fn test_set_section_aabb_rejects_negative_offset() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let result = doc.set_section_aabb(
        &id,
        1,
        Position { x: -5.0, y: 10.0 },
        Size {
            width: 50.0,
            height: 30.0,
        },
    );
    assert!(result.is_err_and(|m| m.contains("offset.x is negative")));
}

#[test]
fn test_set_section_aabb_rejects_non_positive_size() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let result = doc.set_section_aabb(
        &id,
        1,
        Position { x: 10.0, y: 10.0 },
        Size {
            width: 0.0,
            height: 30.0,
        },
    );
    assert!(result.is_err_and(|m| m.contains("is not positive")));
}

#[test]
fn test_set_section_aabb_idempotent_no_op() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let same_offset = Position { x: 10.0, y: 10.0 };
    let same_size = Size {
        width: 50.0,
        height: 30.0,
    };
    let undo_before = doc.undo_stack.len();
    assert_eq!(doc.set_section_aabb(&id, 1, same_offset, same_size), Ok(false));
    assert_eq!(doc.undo_stack.len(), undo_before, "no-op must not push undo");
}

#[test]
fn test_set_section_aabb_writes_through_one_undo_entry() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    doc.undo_stack.clear();
    let result = doc.set_section_aabb(
        &id,
        1,
        Position { x: 20.0, y: 15.0 },
        Size {
            width: 40.0,
            height: 25.0,
        },
    );
    assert_eq!(result, Ok(true));
    assert_eq!(doc.undo_stack.len(), 1, "one undo entry per atomic AABB write");
}

// ── Auto-fit on Some-sized sections ────────────────────────────
//
// `grow_one_node_to_fit_text` contributes the larger of measured
// text and (when set) user-pinned size to the floor — user intent
// survives when text fits, text overflow still grows the parent.

#[test]
fn test_auto_fit_some_sized_section_grows_parent_when_text_overflows() {
    use super::grow_one_node_to_fit_text;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections.clear();
        node.sections
            .push(baumhard::mindmap::model::MindSection::new_default(
                "x".repeat(500),
                Vec::new(),
            ));
        node.sections[0].size = Some(baumhard::mindmap::model::Size {
            width: 10.0,
            height: 10.0,
        });
        node.size.width = 10.0;
        node.size.height = 10.0;
    }
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    grow_one_node_to_fit_text(node);
    // 500 'x' characters at 14pt should produce a measured text
    // block of hundreds of pixels (well over 100). A regression
    // that drops "grow to fit text" to "grow by 1 unit per call"
    // would pass the loose `> 10.0` form; this lower bound traps
    // it.
    assert!(
        node.size.width >= 100.0,
        "500-char text must grow parent past 100; got {}",
        node.size.width
    );
}

#[test]
fn test_auto_fit_some_sized_section_keeps_user_size_when_text_fits() {
    use super::grow_one_node_to_fit_text;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections.clear();
        node.sections
            .push(baumhard::mindmap::model::MindSection::new_default(
                String::new(),
                Vec::new(),
            ));
        node.sections[0].size = Some(baumhard::mindmap::model::Size {
            width: 200.0,
            height: 80.0,
        });
        node.size.width = 50.0;
        node.size.height = 50.0;
    }
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    grow_one_node_to_fit_text(node);
    assert!(
        node.size.width >= 200.0,
        "user-pinned section size must pull the parent floor up: width={}",
        node.size.width
    );
    assert!(
        node.size.height >= 80.0,
        "user-pinned section size must pull the parent floor up: height={}",
        node.size.height
    );
}

/// `Some`-sized section where text *also* fits within the user
/// size: ensures the floor takes max(text, user-size) — a
/// regression that always picks user-size and ignores text would
/// pass the previous two tests but fail here.
#[test]
fn test_auto_fit_some_sized_section_text_dominates_when_larger() {
    use super::grow_one_node_to_fit_text;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections.clear();
        node.sections
            .push(baumhard::mindmap::model::MindSection::new_default(
                "x".repeat(500),
                Vec::new(),
            ));
        // User pinned 50×50, but text needs much more — text
        // wins.
        node.sections[0].size = Some(baumhard::mindmap::model::Size {
            width: 50.0,
            height: 50.0,
        });
        node.size.width = 50.0;
        node.size.height = 50.0;
    }
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    grow_one_node_to_fit_text(node);
    assert!(
        node.size.width >= 100.0,
        "text must dominate the floor when larger than user size; got {}",
        node.size.width
    );
}

#[test]
fn test_auto_fit_none_sized_section_unchanged_regression() {
    use super::grow_one_node_to_fit_text;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections.clear();
        node.sections
            .push(baumhard::mindmap::model::MindSection::new_default(
                "x".repeat(200),
                Vec::new(),
            ));
        node.sections[0].size = None;
        node.size.width = 10.0;
        node.size.height = 10.0;
    }
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    grow_one_node_to_fit_text(node);
    assert!(
        node.size.width >= 100.0,
        "None-sized section auto-fit must grow parent past 100; got {}",
        node.size.width
    );
}

/// Out-of-range section index is a no-op — neither push undo
/// nor flip dirty. Mirrors `set_node_text` no-op contract.
#[test]
fn test_set_section_text_out_of_range_is_noop() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_section_text(&nid, 99, "nope".to_string()));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

/// `set_section_text(_, _, "")` produces an empty `text_runs`
/// vec, never a degenerate `TextRun { start: 0, end: 0 }`. The
/// degenerate run violates `text_run_ops`'s `start < end`
/// invariant (debug_assert_invariants in
/// `lib/baumhard/.../text_run_ops.rs`) and panics in debug
/// builds on subsequent slice / splice / find_run_containing
/// calls. Pin the empty case so the §4.5 console verb
/// `section text "" runs=clear` and the §4.6 keybind / macro
/// `Action::SetSectionText { text: "", runs_mode: "clear" }`
/// don't ship a debug-build crasher.
#[test]
fn test_set_section_text_empty_produces_empty_runs_not_degenerate() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    assert!(doc.set_section_text(&nid, 0, "".to_string()));
    let section = &doc.mindmap.nodes.get(&nid).unwrap().sections[0];
    assert!(section.text.is_empty());
    assert!(
        section.text_runs.is_empty(),
        "empty text must yield empty runs vec; got {:?}",
        section.text_runs
    );
}

#[test]
fn test_set_node_text_noop_on_unchanged() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let current = doc.mindmap.nodes.get(&nid).unwrap().sections[0].text.clone();
    doc.undo_stack.clear();
    doc.dirty = false;
    let changed = doc.set_node_text(&nid, current);
    assert!(!changed);
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

#[test]
fn test_set_node_text_undo_round_trip() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let before_text = doc.mindmap.nodes.get(&nid).unwrap().sections[0].text.clone();
    let before_runs_len = doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs.len();
    let before_first_run_color = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .first()
        .map(|r| r.color.clone());
    assert!(doc.set_node_text(&nid, "mutated".to_string()));
    assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().sections[0].text, "mutated");
    assert!(doc.undo());
    let restored = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(restored.sections[0].text, before_text);
    // TextRun doesn't implement PartialEq, so compare the parts
    // we care about: count + first run's color.
    assert_eq!(restored.sections[0].text_runs.len(), before_runs_len);
    assert_eq!(
        restored.sections[0].text_runs.first().map(|r| r.color.clone()),
        before_first_run_color
    );
}

#[test]
fn test_set_node_text_multiline_with_newlines() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    assert!(doc.set_node_text(&nid, "line 1\nline 2\nline 3".to_string()));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(node.sections[0].text, "line 1\nline 2\nline 3");
    // Collapsed single run spans the full char count, including newlines.
    assert_eq!(node.sections[0].text_runs.len(), 1);
    assert_eq!(
        node.sections[0].text_runs[0].end,
        count_grapheme_clusters("line 1\nline 2\nline 3")
    );
}

#[test]
fn test_set_node_text_unknown_id_returns_false() {
    let mut doc = load_test_doc();
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_node_text("nonexistent-id", "x".to_string()));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

#[test]
fn test_set_node_text_inherits_first_run_formatting() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    // Force a specific first-run formatting we can check for.
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        if node.sections[0].text_runs.is_empty() {
            let end = count_grapheme_clusters(&node.sections[0].text);
            node.sections[0].text_runs.push(TextRun {
                start: 0,
                end,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".to_string(),
                size_pt: 24,
                color: "#ffffff".to_string(),
                hyperlink: None,
            });
        }
        node.sections[0].text_runs[0].bold = true;
        node.sections[0].text_runs[0].color = "#abcdef".to_string();
        node.sections[0].text_runs[0].size_pt = 33;
    }
    assert!(doc.set_node_text(&nid, "rewritten".to_string()));
    let run = &doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs[0];
    assert!(run.bold);
    assert_eq!(run.color, "#abcdef");
    assert_eq!(run.size_pt, 33);
}

// -----------------------------------------------------------------
// Node style setters (bg / border / text color, font size)
// -----------------------------------------------------------------

#[test]
fn test_set_node_bg_color_round_trips_through_undo() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let before = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .style
        .background_color
        .clone();
    assert!(doc.set_node_bg_color(&nid, "#123456".to_string()));
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.background_color,
        "#123456"
    );
    assert!(matches!(
        doc.undo_stack.last(),
        Some(UndoAction::EditNodeStyle { .. })
    ));
    assert!(doc.undo());
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.background_color,
        before
    );
}

#[test]
fn test_set_node_bg_color_unchanged_is_noop() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let current = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .style
        .background_color
        .clone();
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_node_bg_color(&nid, current));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

#[test]
fn test_set_node_border_color_writes_frame_color() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    assert!(doc.set_node_border_color(&nid, "#ff00ff".to_string()));
    assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().style.frame_color, "#ff00ff");
}

/// First-edit materialization of `node.style.border` uses
/// `default_glyph_border_config()` (private to `nodes/border.rs`).
/// Pin the resulting `preset` to `"light"` so a regression to
/// `"rounded"` — the previous default — surfaces here. The
/// trigger is any kv edit that *touches a config field*; we
/// use `padding=` because it's a leaf field with no other
/// behaviour entanglement.
#[test]
fn test_default_border_config_first_edit_materialises_light_preset() {
    use crate::application::document::{BorderConfigEdits, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    // Strip any pre-existing per-node border so we exercise the
    // `get_or_insert_with(default_glyph_border_config)` path.
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.border = None;
    let mut edits = BorderConfigEdits::default();
    edits.padding = OptionEdit::Set(8.0);
    let outcome = doc.set_node_border_config(&nid, edits);
    assert!(outcome.changed);
    let cfg = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .style
        .border
        .as_ref()
        .expect("first-edit materialised the per-node config");
    assert_eq!(cfg.preset, "light");
}

/// Setting text color rewrites `style.text_color` and every run
/// whose color matched the pre-edit default. A run the user
/// colored by hand (mismatched) keeps its override.
#[test]
fn test_set_node_text_color_preserves_per_run_overrides() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    // Seed the node with a known default and two runs: one
    // matching the default, one hand-colored. Pin
    // `sections[0].text` to a string of known grapheme count so
    // the runs (`0..3`, `3..6`) survive the `clamp_runs_to_text`
    // pass `set_node_text_color` runs — without this, the second
    // run gets dropped when `first_testament_node_id` happens to
    // pick a node whose section text is shorter than 4 graphemes
    // (HashMap iteration order varies per process, so the test
    // was intermittently flaky).
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections[0].text = "abcdef".into();
        node.style.text_color = "#dddddd".into();
        node.sections[0].text_runs = vec![
            TextRun {
                start: 0,
                end: 3,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 24,
                color: "#dddddd".into(), // matches default
                hyperlink: None,
            },
            TextRun {
                start: 3,
                end: 6,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 24,
                color: "#abcdef".into(), // user override
                hyperlink: None,
            },
        ];
    }
    assert!(doc.set_node_text_color(&nid, "#111111".into()));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(node.style.text_color, "#111111");
    assert_eq!(
        node.sections[0].text_runs[0].color, "#111111",
        "default-following run should update"
    );
    assert_eq!(
        node.sections[0].text_runs[1].color, "#abcdef",
        "per-run override should be preserved"
    );
}

#[test]
fn test_set_node_text_color_round_trips_through_undo() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.style.text_color = "#dddddd".into();
        for run in node.sections[0].text_runs.iter_mut() {
            run.color = "#dddddd".into();
        }
    }
    let before_default = doc.mindmap.nodes.get(&nid).unwrap().style.text_color.clone();
    let before_run_colors: Vec<String> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.color.clone())
        .collect();
    assert!(doc.set_node_text_color(&nid, "#222222".into()));
    assert!(doc.undo());
    let restored = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(restored.style.text_color, before_default);
    let restored_colors: Vec<String> = restored.sections[0]
        .text_runs
        .iter()
        .map(|r| r.color.clone())
        .collect();
    assert_eq!(restored_colors, before_run_colors);
}

#[test]
fn test_set_node_font_size_writes_all_runs_and_round_trips() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let before_sizes: Vec<u32> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.size_pt)
        .collect();
    assert!(doc.set_node_font_size(&nid, 48.0));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(node.sections[0].text_runs.iter().all(|r| r.size_pt == 48));
    assert!(doc.undo());
    let after_sizes: Vec<u32> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.size_pt)
        .collect();
    assert_eq!(after_sizes, before_sizes);
}

#[test]
fn test_set_node_font_size_clamps_below_one() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    assert!(doc.set_node_font_size(&nid, 0.5));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(node.sections[0].text_runs.iter().all(|r| r.size_pt == 1));
}

#[test]
fn test_set_node_style_unknown_id_returns_false() {
    let mut doc = load_test_doc();
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_node_bg_color("nope", "#000".into()));
    assert!(!doc.set_node_border_color("nope", "#000".into()));
    assert!(!doc.set_node_text_color("nope", "#000".into()));
    assert!(!doc.set_node_font_size("nope", 10.0));
    assert!(!doc.set_node_font_family("nope", Some("Norse")));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

#[test]
fn test_set_node_font_family_writes_all_runs_and_round_trips() {
    baumhard::font::fonts::init();
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let before_fonts: Vec<String> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.font.clone())
        .collect();
    // Pick a loaded family that doesn't already match every
    // existing run — keeps the test self-healing against
    // future fixture changes.
    let target = baumhard::font::fonts::loaded_families_iter()
        .find(|f| !before_fonts.iter().any(|b| b == f))
        .map(str::to_string)
        .expect("at least one loaded family must differ from the fixture");
    assert!(doc.set_node_font_family(&nid, Some(&target)));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(node.sections[0].text_runs.iter().all(|r| r.font == target));
    // Idempotent re-set is a no-op.
    let stack_len = doc.undo_stack.len();
    assert!(!doc.set_node_font_family(&nid, Some(&target)));
    assert_eq!(doc.undo_stack.len(), stack_len);
    // Undo restores the prior heterogeneous state.
    assert!(doc.undo());
    let after_fonts: Vec<String> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.font.clone())
        .collect();
    assert_eq!(after_fonts, before_fonts);
}

/// Pinning a wide-advance face on a node previously sized for
/// a narrow monospace must grow the box so the new text fits.
/// The setter calls `grow_one_node_to_fit_text` after mutating
/// the runs; before the fix, font changes left the rect at its
/// prior size and the new text overflowed the right edge.
#[test]
fn test_set_node_font_family_grows_node_to_fit_new_face() {
    baumhard::font::fonts::init();
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());

    // Shrink the node *below* its measured floor so the per-edit
    // re-fit has something concrete to grow back. Note: the
    // production loader's `grow_node_sizes_to_fit_text` would
    // never leave a node this small, but the test fixture is
    // already loaded so we shrink in place to set up the
    // measurement.
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    node.size.width = 1.0;
    node.size.height = 1.0;

    // Use whatever family the fixture already references so the
    // setter doesn't bail out as "already". If the fixture's
    // first run carries the empty sentinel, pin to a real
    // family instead.
    let pin = baumhard::font::fonts::loaded_families_iter()
        .next()
        .map(str::to_string)
        .expect("at least one loaded family");
    assert!(doc.set_node_font_family(&nid, Some(&pin)));

    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(
        node.size.width > 1.0 && node.size.height > 1.0,
        "set_node_font_family must re-fit the node box; got {}×{}",
        node.size.width,
        node.size.height
    );
}

/// `set_node_font_size` likewise has to re-fit — the same
/// regression as the family case, just driven by the size
/// channel.
#[test]
fn test_set_node_font_size_grows_node_to_fit_new_size() {
    baumhard::font::fonts::init();
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    node.size.width = 1.0;
    node.size.height = 1.0;
    // Pick a size different from whatever the fixture's first
    // run uses so the setter actually applies. 96 pt is well
    // above any default.
    assert!(doc.set_node_font_size(&nid, 96.0));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(
        node.size.width > 1.0 && node.size.height > 1.0,
        "set_node_font_size must re-fit the node box; got {}×{}",
        node.size.width,
        node.size.height
    );
}

/// Pinning a wide display face must measure with that face, not
/// cosmic-text's default monospace. Pre-fix,
/// `measure_text_block_unbounded` shaped with `Attrs::new()`
/// regardless of the run's `font` field, so a node pinned to a
/// wide face under-measured by 30–60% and the box undersized.
/// This test compares the floor reached by two consecutive
/// font-family pins on the same fixture node — one to a face
/// with a known wide advance, one to a known narrow face — and
/// asserts the wide-face floor is strictly larger. If the
/// measurement reverts to font-blind, both pins land at the
/// monospace floor and the assertion fires.
#[test]
fn test_set_node_font_family_wide_face_grows_more_than_narrow() {
    baumhard::font::fonts::init();
    // Strategy: shape "MMMMMMMM" through every loaded face,
    // pick the narrowest and widest measured advance, and
    // compare the two floors. This is fixture-resilient — we
    // don't rely on any particular family being bundled, just
    // on at least two faces having distinct advances (which is
    // the case for the >40 bundled families).
    let families: Vec<String> = baumhard::font::fonts::loaded_families_iter()
        .map(str::to_string)
        .collect();
    if families.len() < 2 {
        // Not enough variety to discriminate; skip without
        // failing the suite.
        return;
    }

    // Measure each family's advance for "MMMMMMMM" at 14 pt;
    // pick narrowest and widest. Skip families that resolve to
    // None for app_font_by_family (shouldn't happen given the
    // iter source, but defensive).
    let mut measurements: Vec<(String, f32)> = Vec::new();
    for fam in &families {
        let app_font = match baumhard::font::fonts::app_font_by_family(fam) {
            Some(f) => f,
            None => continue,
        };
        let mut fs = baumhard::font::fonts::acquire_font_system_write("tests::wide_vs_narrow_measure");
        let block = baumhard::font::fonts::measure_text_block_unbounded(
            &mut fs,
            "MMMMMMMM",
            14.0,
            16.8,
            Some(app_font),
        );
        drop(fs);
        measurements.push((fam.clone(), block.width));
    }
    measurements.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    if measurements.len() < 2 || measurements.first().unwrap().1 <= 0.0 {
        return;
    }
    let narrow_fam = measurements.first().unwrap().0.clone();
    let wide_fam = measurements.last().unwrap().0.clone();
    if (measurements.last().unwrap().1 - measurements.first().unwrap().1).abs() < 1.0 {
        // Insufficient spread — bundled set may be pathologically
        // uniform. Don't assert.
        return;
    }

    // Apply each family in turn to a fresh node and read the
    // resulting size.
    let measure_floor = |fam: &str| -> f64 {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.size.width = 1.0;
        node.size.height = 1.0;
        assert!(doc.set_node_font_family(&nid, Some(fam)));
        doc.mindmap.nodes.get(&nid).unwrap().size.width
    };

    let narrow_floor = measure_floor(&narrow_fam);
    let wide_floor = measure_floor(&wide_fam);
    assert!(
        wide_floor > narrow_floor,
        "wide face '{}' floor ({}) should exceed narrow face '{}' floor ({}); \
             likely measure_text_block_unbounded reverted to font-blind",
        wide_fam,
        wide_floor,
        narrow_fam,
        narrow_floor
    );
}

/// `set_node_text` must re-fit on text change — pre-fix the
/// inline editor's commit path could overflow because the box
/// stayed at its prior size while the new text grew.
#[test]
fn test_set_node_text_grows_node_to_fit_longer_text() {
    baumhard::font::fonts::init();
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    node.size.width = 1.0;
    node.size.height = 1.0;
    let long_text = "this is some text that is meaningfully longer than a tiny box".to_string();
    assert!(doc.set_node_text(&nid, long_text));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(
        node.size.width > 1.0 && node.size.height > 1.0,
        "set_node_text must re-fit the node box; got {}×{}",
        node.size.width,
        node.size.height
    );
}

#[test]
fn test_set_node_font_family_none_clears_every_run() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    // Pin the runs to a known family first so the clear has
    // something to clear.
    baumhard::font::fonts::init();
    let target = baumhard::font::fonts::loaded_families_iter()
        .next()
        .map(str::to_string)
        .expect("at least one loaded family");
    assert!(doc.set_node_font_family(&nid, Some(&target)));
    // Now clear with None — every run should hold the empty
    // sentinel that the tree builder reads as "use default".
    assert!(doc.set_node_font_family(&nid, None));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(node.sections[0].text_runs.iter().all(|r| r.font.is_empty()));
    // Re-clear is a no-op.
    let stack_len = doc.undo_stack.len();
    assert!(!doc.set_node_font_family(&nid, None));
    assert_eq!(doc.undo_stack.len(), stack_len);
}

/// `grow_node_sizes_to_fit_borders` runs at finalize so a
/// map loaded with a wide static side pattern on a tiny node
/// grows the node automatically — the same monotonic posture
/// as `grow_node_sizes_to_fit_text`. Without this floor the
/// renderer would clip the static prefix at load time.
#[test]
fn finalize_grows_nodes_to_fit_border_static_parts() {
    use baumhard::mindmap::model::{Canvas, CustomBorderGlyphs, GlyphBorderConfig, MindMap};
    use std::collections::HashMap;

    let mut nodes = HashMap::new();
    let style = NodeStyle {
        background_color: "#000".into(),
        frame_color: "#fff".into(),
        text_color: "#fff".into(),
        shape: "rectangle".into(),
        corner_radius_percent: 0.0,
        frame_thickness: 1.0,
        show_frame: true,
        show_shadow: false,
        border: Some(GlyphBorderConfig {
            preset: "custom".into(),
            font: None,
            font_size_pt: 14.0,
            color: None,
            glyphs: Some(CustomBorderGlyphs {
                top: "##########(*)##########".into(),
                bottom: "-".into(),
                left: "|".into(),
                right: "|".into(),
                top_left: "<".into(),
                top_right: ">".into(),
                bottom_left: "<".into(),
                bottom_right: ">".into(),
            }),
            padding: 4.0,
            color_palette: None,
            color_palette_field: None,
        }),
    };
    nodes.insert(
        "0".into(),
        MindNode {
            id: "0".into(),
            parent_id: None,
            position: Position { x: 0.0, y: 0.0 },
            size: Size {
                width: 5.0,
                height: 5.0,
            },
            sections: vec![MindSection::new_default("n".into(), vec![])],
            style,
            layout: NodeLayout {
                layout_type: "map".into(),
                direction: "auto".into(),
                spacing: 0.0,
            },
            folded: false,
            notes: String::new(),
            color_schema: None,
            channel: 0,
            trigger_bindings: vec![],
            inline_mutations: vec![],
            inline_macros: Vec::new(),
            min_zoom_to_render: None,
            max_zoom_to_render: None,
        },
    );
    let map = MindMap {
        version: "1.0".into(),
        name: "fixture".into(),
        canvas: Canvas {
            background_color: "#000".into(),
            default_border: None,
            default_connection: None,
            default_section_frame_border: None,
            default_focused_section_frame_border: None,
            theme_variables: HashMap::new(),
            theme_variants: HashMap::new(),
        },
        palettes: HashMap::new(),
        nodes,
        edges: vec![],
        custom_mutations: vec![],
        macros: vec![],
    };
    // Round-trip through JSON to exercise the finalize hook
    // — `MindMapDocument::from_json_str` calls `finalize`,
    // which runs both grow passes. Direct construction skips
    // it.
    let json = serde_json::to_string(&map).expect("serialises");
    let doc = MindMapDocument::from_json_str(&json, None).expect("loads through finalize");
    let n = doc.mindmap.nodes.get("0").expect("node 0 exists");
    assert!(
        n.size.width > 5.0,
        "load-time floor must grow the box to fit the border statics; \
             got width={}",
        n.size.width,
    );
}

// ── Range-targeted section setters (Tier 2C-N4-B) ─────────────────

/// Set a colour on `[range_start, range_end)` inside one section
/// — pins the simplest happy path (range entirely inside one run).
#[test]
fn test_set_section_text_color_range_inside_one_run() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    // Apply blue to a sub-range and verify the section now has
    // three runs: original-colour | blue | original-colour.
    let applied = doc.set_section_text_color_range(&id, 0, 1, 9, "#abcdef".into());
    assert!(applied);
    let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    assert_eq!(runs.len(), 3, "expected three runs after range carve-out");
    assert_eq!(runs[1].color, "#abcdef");
}

/// Range that exactly matches an existing run's color is a
/// no-op — the range setter detects pre/post equality and pops
/// the spurious undo entry.
#[test]
fn test_set_section_text_color_range_no_op_no_undo() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    let original_color = "#ffffff".to_string();
    let undo_before = doc.undo_stack.len();
    let applied = doc.set_section_text_color_range(&id, 0, 1, 3, original_color);
    assert!(!applied, "no-op write must return false");
    assert_eq!(
        doc.undo_stack.len(),
        undo_before,
        "no-op write must not push an undo entry"
    );
}

/// Range setter clamps `range_end` to the section's grapheme
/// count. A range of `[2, 9999)` on a 10-grapheme section
/// behaves like `[2, 10)`.
#[test]
fn test_set_section_text_color_range_clamps_end_to_grapheme_count() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    let total = 10usize;
    let applied = doc.set_section_text_color_range(&id, 0, 1, total + 100, "#abcdef".into());
    assert!(applied, "clamped range must still apply");
    let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    let last = runs.last().expect("at least one run");
    assert!(
        last.end <= total,
        "post-mutation runs must respect grapheme count: last.end={} > total={}",
        last.end,
        total
    );
}

/// Range with empty bounds (`start == end`) is a no-op and
/// doesn't push an undo entry.
#[test]
fn test_set_section_text_color_range_empty_returns_false() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    let undo_before = doc.undo_stack.len();
    assert!(!doc.set_section_text_color_range(&id, 0, 5, 5, "#abcdef".into()));
    assert!(!doc.set_section_text_color_range(&id, 0, 7, 3, "#abcdef".into()));
    assert_eq!(doc.undo_stack.len(), undo_before);
}

/// Range setter on a missing section returns false without
/// crashing or pushing undo.
#[test]
fn test_set_section_text_color_range_missing_section_returns_false() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    assert!(!doc.set_section_text_color_range(&id, 99, 0, 5, "#abcdef".into()));
    assert!(!doc.set_section_text_color_range("does-not-exist", 0, 0, 5, "#abcdef".into()));
}

/// Range setter pushes one undo entry and Ctrl+Z restores the
/// pre-write run set byte-for-byte.
#[test]
fn test_set_section_text_color_range_undo_round_trip() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    let pre = doc.mindmap.nodes.get(&id).unwrap().sections[0]
        .text_runs
        .clone();
    assert!(doc.set_section_text_color_range(&id, 0, 1, 9, "#abcdef".into()));
    assert!(doc.undo());
    let post = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    assert_eq!(post, &pre, "undo must restore pre-write runs");
}

/// Range setter for font size carries through the
/// `grow_one_node_to_fit_text` re-measure. A larger size on a
/// sub-range can grow the node's AABB.
#[test]
fn test_set_section_font_size_range_triggers_grow() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    let pre_w = doc.mindmap.nodes.get(&id).unwrap().size.width;
    // Apply a much larger font to the whole section's range —
    // forces the grow pass and the post-write width should be
    // at least the pre-write width.
    assert!(doc.set_section_font_size_range(&id, 0, 0, 10, 96.0));
    let post_w = doc.mindmap.nodes.get(&id).unwrap().size.width;
    assert!(post_w >= pre_w, "grow pass must monotonically widen the node");
}

/// Range setter for font family clears / pins per-grapheme.
/// Pin: applying a family different from the section's runs
/// changes the in-range runs' `font` field.
#[test]
fn test_set_section_font_family_range_writes_in_range_only() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    // Override section[0].text + run to a known length so the
    // test isn't sensitive to which testament node `pinned_…`
    // happens to pick (HashMap iteration order isn't stable).
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "DejaVuSans");
    let original_font = "DejaVuSans".to_string();
    let target_font = "LiberationSans".to_string();
    assert!(doc.set_section_font_family_range(&id, 0, 1, 4, Some(&target_font)));
    let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    // Find the in-range run and the out-of-range runs.
    let in_range: Vec<_> = runs.iter().filter(|r| r.start >= 1 && r.end <= 4).collect();
    let out_of_range: Vec<_> = runs.iter().filter(|r| r.end <= 1 || r.start >= 4).collect();
    assert!(!in_range.is_empty(), "expected at least one in-range run");
    for r in in_range {
        assert_eq!(r.font, target_font);
    }
    for r in out_of_range {
        assert_eq!(r.font, original_font);
    }
}

/// Gap-fill: applying a colour on a range that falls in a gap
/// (no covering run) inserts a fresh run carrying the colour.
/// Pins the foundation gap N4-A.1's `insert_run` primitive
/// closes — without it, the user's "make graphemes 5..8 blue"
/// would silently no-op when no run covers that range.
#[test]
fn test_set_section_text_color_range_fills_gap() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    // Override section[0].text + run to a known length so the
    // test isn't sensitive to HashMap iteration order picking a
    // testament node with short section text.
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    {
        let n = doc.mindmap.nodes.get_mut(&id).unwrap();
        let s = &mut n.sections[0];
        // Shrink the run to [0, 3) so [3, 10) is a gap.
        s.text_runs[0].end = 3;
    }
    let runs_before = doc.mindmap.nodes.get(&id).unwrap().sections[0]
        .text_runs
        .len();
    assert!(doc.set_section_text_color_range(&id, 0, 5, 8, "#123456".into()));
    let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    assert!(
        runs.len() > runs_before,
        "gap-fill must add at least one run"
    );
    let new_run = runs.iter().find(|r| r.start == 5 && r.end == 8);
    assert!(new_run.is_some(), "expected a new run covering [5, 8)");
    assert_eq!(new_run.unwrap().color, "#123456");
}

/// Test helper: overwrite section[0]'s text with a known string
/// and replace its runs with a single full-coverage run carrying
/// the given font. Used by range-setter tests that need a
/// deterministic grapheme count — `first_testament_node_id` runs
/// over `HashMap` iteration order, which isn't stable across
/// test orderings, so the fixture's text length varies.
fn set_section_zero_text_and_single_run(
    doc: &mut MindMapDocument,
    node_id: &str,
    text: &str,
    font: &str,
) {
    let total = count_grapheme_clusters(text);
    let n = doc.mindmap.nodes.get_mut(node_id).expect("node exists");
    let s = &mut n.sections[0];
    s.text = text.to_string();
    s.text_runs.clear();
    s.text_runs.push(TextRun {
        start: 0,
        end: total,
        bold: false,
        italic: false,
        underline: false,
        font: font.to_string(),
        size_pt: 14,
        color: "#ffffff".to_string(),
        hyperlink: None,
    });
    // Reset undo so the round-trip test can probe `undo()` on
    // the range mutation alone.
    doc.undo_stack.clear();
    doc.dirty = false;
}

// ─── border preview ────────────────────────────────────────────
//
// `MindMapDocument::set_border_preview` /
// `cancel_border_preview` / `commit_border_preview` are the
// preview-substrate setters — runtime-only state, no model write
// until commit. The tests below mirror the discipline pinned for
// `color_picker_preview` (tests_edges_style.rs) and the
// node-border / section-frame / canvas auto-promotion contract.
// Scene-build threading lands in a later commit; these tests
// assert behaviour observable from the document layer alone.

/// Setting a preview must not push undo, flip `dirty`, or mutate
/// the model. Same discipline as `color_picker_preview` — preview
/// is a transient runtime substitution, not a model edit.
#[test]
fn test_border_preview_does_not_push_undo_or_dirty() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let undo_depth = doc.undo_stack.len();
    let before_node = doc.mindmap.nodes.get(&nid).cloned().unwrap();
    doc.dirty = false;

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);

    assert_eq!(doc.undo_stack.len(), undo_depth);
    assert!(!doc.dirty);
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.border.as_ref().map(|c| c.preset.clone()),
        before_node.style.border.as_ref().map(|c| c.preset.clone()),
        "model border slot must be byte-identical to pre-preview state"
    );
    assert!(doc.border_preview.is_some(), "preview slot populated");
}

/// Cancelling a preview returns to the pre-preview model state
/// without writing anything. Mirrors
/// `test_color_picker_preview_cleared_returns_to_committed`.
#[test]
fn test_border_preview_cleared_returns_to_committed() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let before_node = doc.mindmap.nodes.get(&nid).cloned().unwrap();
    doc.dirty = false;
    let undo_depth = doc.undo_stack.len();

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("double".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    let returned = doc.cancel_border_preview();

    assert!(returned, "cancel returns true when a preview was active");
    assert!(doc.border_preview.is_none(), "preview slot cleared");
    assert!(!doc.dirty);
    assert_eq!(doc.undo_stack.len(), undo_depth);
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.border.as_ref().map(|c| c.preset.clone()),
        before_node.style.border.as_ref().map(|c| c.preset.clone()),
        "model unchanged after preview-then-cancel"
    );
}

/// Commit dispatches to the underlying setter, which pushes one
/// undo entry per affected target and flips `dirty`. The preview
/// slot is cleared.
#[test]
fn test_border_preview_commit_pushes_undo_and_dirty() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let undo_depth = doc.undo_stack.len();
    doc.dirty = false;

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    let outcome = doc.commit_border_preview().expect("preview was active");

    assert!(outcome.changed);
    assert!(doc.dirty);
    assert!(doc.undo_stack.len() > undo_depth, "commit pushes at least one undo entry");
    let cfg = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .style
        .border
        .as_ref()
        .expect("border populated");
    assert_eq!(cfg.preset, "heavy");
}

/// Commit clears `border_preview` to `None`. A subsequent `commit`
/// returns `None` because no preview is active.
#[test]
fn test_border_preview_commit_clears_preview_slot() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid]), edits);
    let _ = doc.commit_border_preview().expect("preview was active");
    assert!(doc.border_preview.is_none(), "commit clears the preview slot");
    assert!(
        doc.commit_border_preview().is_none(),
        "second commit returns None — no preview to commit"
    );
}

/// A fresh `set_border_preview` replaces any prior preview
/// atomically. The new preview's edits are what commit will apply.
#[test]
fn test_border_preview_replaces_prior_preview() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());

    let mut first_edits = BorderConfigEdits::default();
    first_edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), first_edits);

    let mut second_edits = BorderConfigEdits::default();
    second_edits.preset = OptionEdit::Set("double".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), second_edits);

    let outcome = doc.commit_border_preview().expect("second preview active");
    assert!(outcome.changed);
    let cfg = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .style
        .border
        .as_ref()
        .expect("border populated");
    assert_eq!(
        cfg.preset, "double",
        "second preview wins; the first preview's heavy preset must not have committed"
    );
}

/// `cancel_border_preview` returns `true` when a preview was
/// active and `false` otherwise. The bool is what the verb / Esc
/// arm uses to decide whether the keystroke should fall through.
#[test]
fn test_border_preview_cancel_returns_true_when_active_and_false_when_inactive() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());

    assert!(
        !doc.cancel_border_preview(),
        "cancel returns false when no preview is active"
    );

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid]), edits);
    assert!(
        doc.cancel_border_preview(),
        "cancel returns true when a preview was active"
    );
    assert!(
        !doc.cancel_border_preview(),
        "subsequent cancel returns false again"
    );
}

/// Auto-promotion is reflected in the preview's outcome — the
/// verb surfaces the same auto-promote note up-front whether
/// the user runs `border preview preset=heavy top=…` or the
/// committing `border preset=heavy top=…`.
#[test]
fn test_border_preview_auto_promotes_preset_to_custom_in_outcome() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    // Ensure the pre-preview slot is non-custom so the helper
    // sees a real promotion.
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.border = None;

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    edits
        .with_side_pattern(crate::application::document::BorderSide::Top, "###(*)###")
        .expect("pattern parses");
    let outcome = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid]), edits);

    assert!(
        outcome.preset_auto_promoted,
        "side glyph + non-custom preset must auto-promote in the simulated outcome"
    );
    assert_eq!(outcome.requested_preset.as_deref(), Some("heavy"));
}

/// Selection drift: when the live selection no longer covers the
/// preview's `selection_snapshot`, the scene-build path renders
/// as if no preview were active. The actual slot empties at the
/// next `set_*` / `cancel_*` / `commit_*` call (defer-clear).
#[test]
fn test_border_preview_drift_clears_on_selection_change() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit, SelectionState,
    };
    let mut doc = load_test_doc();
    let nid_a = first_testament_node_id(&doc);
    // Pick any other node id distinct from `nid_a`.
    let nid_b = doc
        .mindmap
        .nodes
        .keys()
        .find(|id| id.as_str() != nid_a)
        .cloned()
        .expect("testament has multiple nodes");

    // Stage a preview against node A.
    doc.selection = SelectionState::Single(nid_a.clone());
    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid_a.clone()]), edits);
    assert!(doc.border_preview_covers_live_selection());

    // Change the selection to node B — drift.
    doc.selection = SelectionState::Single(nid_b);
    assert!(
        !doc.border_preview_covers_live_selection(),
        "live selection no longer covers the preview's target"
    );
    // The slot itself is still populated until the next setter
    // call — that's the defer-clear posture.
    assert!(doc.border_preview.is_some());

    // A subsequent cancel observes the drift and clears the slot,
    // returning false (nothing was actively rendering anyway).
    let cancelled = doc.cancel_border_preview();
    assert!(!cancelled, "drifted preview is treated as already-cleared");
    assert!(doc.border_preview.is_none());
}

/// A direct (non-preview) committing edit clears any active
/// preview. Without this rule, typing `border preset=double`
/// after `border preview preset=heavy` would render the heavy
/// preview *over* the just-committed double border — visibly
/// stale until the user manually cancelled. The implicit-cancel
/// fires on every committing setter:
/// `set_node_border_config`, `set_section_frame_border_config`,
/// `set_canvas_default_border`,
/// `set_canvas_default_section_frame_border_config`.
#[test]
fn test_committing_set_node_border_config_clears_active_preview() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());

    // Stage a preview.
    let mut preview_edits = BorderConfigEdits::default();
    preview_edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), preview_edits);
    assert!(doc.border_preview.is_some());

    // A direct committing edit on any of the four setters must
    // clear the preview before applying its own write. Test the
    // node-level setter path here; the section / canvas paths
    // are validated by the same implicit-cancel call site at the
    // top of each setter.
    let mut direct_edits = BorderConfigEdits::default();
    direct_edits.preset = OptionEdit::Set("double".into());
    let _ = doc.set_node_border_config(&nid, direct_edits);

    assert!(
        doc.border_preview.is_none(),
        "committing edit must clear an active preview"
    );
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.border.as_ref().unwrap().preset,
        "double",
        "the direct edit's value lands, not the preview's"
    );
}

/// **Parity contract:** `apply_view_to_slot` (baumhard, scene-side)
/// and `apply_glyph_border_edits_to_slot` (application, commit-side)
/// must produce byte-identical post-states for any committing edit.
/// Pre-fix `BorderConfigEditsView` collapsed both `Keep` and
/// `Clear` to "no edit" — preview rendered the field unchanged
/// while commit dropped it (Risk #1 in the plan). This test
/// runs every per-field axis (Set + Clear) through both helpers
/// and asserts the resulting `Option<GlyphBorderConfig>` matches
/// shape-for-shape.
///
/// Crosses the crate boundary deliberately — the parity contract
/// IS that the two helpers in two crates produce the same output;
/// a single-crate test wouldn't cover the projection step that
/// turns `BorderConfigEdits` into `BorderConfigEditsView`.
#[test]
fn test_border_preview_view_apply_matches_committing_apply_byte_for_byte() {
    use crate::application::document::{
        BorderConfigEdits, BorderEditOutcome, BorderSide, OptionEdit,
    };
    // The application-side slot helper lives in
    // `document/nodes/border.rs` as `pub(crate)`. The module is
    // private; re-export through `document/mod.rs` would be
    // wider than needed. Reach via the full path for the parity
    // test only.
    use crate::application::document::nodes_border_apply_glyph_border_edits_to_slot_for_test as apply_glyph_border_edits_to_slot;
    use baumhard::mindmap::border::{apply_view_to_slot, PaletteField};
    use baumhard::mindmap::model::GlyphBorderConfig;

    // Build a concrete starting slot the apply paths can mutate.
    let starting_slot = || -> Option<GlyphBorderConfig> {
        Some(GlyphBorderConfig {
            preset: "rounded".to_string(),
            font: Some("LiberationSans".to_string()),
            font_size_pt: 14.0,
            color: Some("#abcdef".to_string()),
            glyphs: None,
            padding: 4.0,
            color_palette: Some("rainbow".to_string()),
            color_palette_field: Some("frame".to_string()),
        })
    };

    // Each scenario: a `BorderConfigEdits` and a description.
    let scenarios: Vec<(&'static str, BorderConfigEdits)> = vec![
        ("Set preset to heavy", {
            let mut e = BorderConfigEdits::default();
            e.preset = OptionEdit::Set("heavy".into());
            e
        }),
        ("Clear font (Risk #1 case)", {
            let mut e = BorderConfigEdits::default();
            e.font = OptionEdit::Clear;
            e
        }),
        ("Clear color (Risk #1 case)", {
            let mut e = BorderConfigEdits::default();
            e.color = OptionEdit::Clear;
            e
        }),
        ("Clear color_palette (Risk #1 case)", {
            let mut e = BorderConfigEdits::default();
            e.color_palette = OptionEdit::Clear;
            e
        }),
        ("Clear color_palette_field (Risk #1 case)", {
            let mut e = BorderConfigEdits::default();
            e.color_palette_field = OptionEdit::Clear;
            e
        }),
        ("Set side top to a pattern (auto-promote to custom)", {
            let mut e = BorderConfigEdits::default();
            e.with_side_pattern(BorderSide::Top, "###(*)###").expect("parses");
            e
        }),
        ("Set padding", {
            let mut e = BorderConfigEdits::default();
            e.padding = OptionEdit::Set(8.0);
            e
        }),
        ("Combine preset=heavy + color=Clear", {
            let mut e = BorderConfigEdits::default();
            e.preset = OptionEdit::Set("heavy".into());
            e.color = OptionEdit::Clear;
            e
        }),
        ("Set palette + field", {
            let mut e = BorderConfigEdits::default();
            e.color_palette = OptionEdit::Set("summer".into());
            e.color_palette_field = OptionEdit::Set(PaletteField::Background);
            e
        }),
    ];

    for (label, edits) in scenarios {
        // Commit-side: in-place application via the document's helper.
        let mut commit_slot = starting_slot();
        let mut outcome = BorderEditOutcome::default();
        apply_glyph_border_edits_to_slot(&mut commit_slot, &edits, &mut outcome);

        // Preview-side: same edits projected to a borrowed view,
        // then applied via the scene-side helper.
        let view = crate::application::document::build_border_config_edits_view_for_test(&edits);
        let mut preview_slot = starting_slot();
        apply_view_to_slot(&mut preview_slot, &view);

        // Compare structurally — both `Option<GlyphBorderConfig>`
        // values should be identical post-apply.
        assert_eq!(
            commit_slot.is_some(),
            preview_slot.is_some(),
            "[{}] Option shape must match",
            label
        );
        if let (Some(c), Some(p)) = (commit_slot.as_ref(), preview_slot.as_ref()) {
            assert_eq!(c.preset, p.preset, "[{}] preset", label);
            assert_eq!(c.font, p.font, "[{}] font", label);
            assert_eq!(c.font_size_pt.to_bits(), p.font_size_pt.to_bits(), "[{}] font_size_pt", label);
            assert_eq!(c.color, p.color, "[{}] color", label);
            assert_eq!(c.padding.to_bits(), p.padding.to_bits(), "[{}] padding", label);
            assert_eq!(c.color_palette, p.color_palette, "[{}] color_palette", label);
            assert_eq!(c.color_palette_field, p.color_palette_field, "[{}] color_palette_field", label);
            assert_eq!(c.glyphs.is_some(), p.glyphs.is_some(), "[{}] glyphs Option shape", label);
            if let (Some(cg), Some(pg)) = (c.glyphs.as_ref(), p.glyphs.as_ref()) {
                assert_eq!(cg.top, pg.top, "[{}] glyphs.top", label);
                assert_eq!(cg.bottom, pg.bottom, "[{}] glyphs.bottom", label);
                assert_eq!(cg.left, pg.left, "[{}] glyphs.left", label);
                assert_eq!(cg.right, pg.right, "[{}] glyphs.right", label);
                assert_eq!(cg.top_left, pg.top_left, "[{}] glyphs.top_left", label);
                assert_eq!(cg.top_right, pg.top_right, "[{}] glyphs.top_right", label);
                assert_eq!(cg.bottom_left, pg.bottom_left, "[{}] glyphs.bottom_left", label);
                assert_eq!(cg.bottom_right, pg.bottom_right, "[{}] glyphs.bottom_right", label);
            }
        }
    }
}

/// **C8 regression** — the preview's `force_show_frame` flag
/// renders a frame on a node with committed `show_frame == false`,
/// but a naive commit would leave `show_frame == false` and the
/// frame would visibly disappear after commit. Commit now
/// auto-flips `style.show_frame = true` when the preview's edits
/// imply visibility (any field touched), so the user gets what
/// they previewed.
#[test]
fn test_border_preview_commit_force_shows_frame_on_hidden_node() {
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // Force the node into a hidden-frame state.
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.show_frame = false;
    doc.selection = SelectionState::Single(nid.clone());

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    let _ = doc.commit_border_preview().expect("preview was active");

    assert!(
        doc.mindmap.nodes.get(&nid).unwrap().style.show_frame,
        "commit must auto-flip `show_frame = true` when the preview's force-show fired \
         (otherwise the user sees the preview render then commit hides it)"
    );
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.border.as_ref().unwrap().preset,
        "heavy",
        "the preset still committed"
    );
}

/// Inverse of the C8 fix — explicit `visible=Some(false)` in the
/// preview edits survives the auto-flip rule.
#[test]
fn test_border_preview_commit_explicit_visibility_overrides_auto_flip() {
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.show_frame = false;
    doc.selection = SelectionState::Single(nid.clone());

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    edits.visible = Some(false);
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    let _ = doc.commit_border_preview();

    assert!(
        !doc.mindmap.nodes.get(&nid).unwrap().style.show_frame,
        "explicit `visible=Some(false)` must survive the auto-flip"
    );
}

/// Undo after commit restores the pre-preview model state. The
/// preview itself never pushed undo — the undo entry was pushed
/// by the underlying setter at commit time.
#[test]
fn test_border_preview_undo_after_commit_restores_pre_preview() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    // Ensure a known starting point.
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.border = None;
    doc.undo_stack.clear();
    doc.dirty = false;
    let before_preset = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .style
        .border
        .as_ref()
        .map(|c| c.preset.clone());

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    let _ = doc.commit_border_preview().expect("preview was active");
    assert!(doc.undo());
    let after = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .style
        .border
        .as_ref()
        .map(|c| c.preset.clone());
    assert_eq!(
        before_preset, after,
        "undo after commit restores the pre-preview border config"
    );
}

/// **C20 regression** — commit on a `Multi(ids)` selection
/// fans out to every targeted node. Each node gets the staged
/// preset applied through `set_node_border_config` (one undo
/// entry per node, matching the committing-path posture
/// documented on `commit_border_preview`).
#[test]
fn test_border_preview_commit_fans_out_to_all_nodes_in_multi_selection() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let ids = first_n_testament_node_ids(&doc, 3);
    // Clear baseline border slots so the post-commit assertion
    // is unambiguous.
    for id in &ids {
        doc.mindmap.nodes.get_mut(id).unwrap().style.border = None;
    }
    doc.selection = SelectionState::Multi(ids.clone());
    doc.undo_stack.clear();
    doc.dirty = false;

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(ids.clone()), edits);
    let outcome = doc
        .commit_border_preview()
        .expect("preview was active before commit");

    // Every node should now carry the staged preset.
    for id in &ids {
        assert_eq!(
            doc.mindmap.nodes.get(id).unwrap().style.border.as_ref().unwrap().preset,
            "heavy",
            "commit must fan out to every node in Multi(ids); missed {}",
            id
        );
    }
    // N undo entries, one per fanned-out node — same posture
    // as today's `apply_edits` and as documented on
    // `commit_border_preview`.
    assert_eq!(
        doc.undo_stack.len(),
        ids.len(),
        "Multi commit must push one undo entry per node ({}); pushed {}",
        ids.len(),
        doc.undo_stack.len()
    );
    assert!(doc.dirty, "Multi commit must flip dirty");
    // Outcome's `changed` reflects the fan-out total — pinned
    // so a future "merge into one undo entry" change doesn't
    // silently regress the user-visible commit count.
    assert!(
        outcome.changed,
        "outcome.changed must be true after Multi commit"
    );
}

/// **C20 regression** — commit on a `SectionRange` selection
/// fans out to every section in the range. The section path
/// uses `set_section_frame_border_config` per (node_id,
/// section_idx) pair; each pushes its own undo entry.
#[test]
fn test_border_preview_commit_fans_out_to_section_range() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit, SectionSel,
    };
    let mut doc = load_test_doc();
    // Pick a node with at least 2 sections — testament's node 3.7
    // has multiple by construction; fall back to any node with
    // .sections.len() >= 2.
    let node_id: String = doc
        .mindmap
        .nodes
        .iter()
        .filter(|(_, n)| n.sections.len() >= 2)
        .map(|(id, _)| id.clone())
        .min()
        .expect("testament map has a multi-section node");
    let n_sections = doc.mindmap.nodes.get(&node_id).unwrap().sections.len();
    let last_section_idx = (n_sections - 1).min(2); // up to 3 sections
    // Clear baseline frame_border slots on the targeted range.
    for i in 0..=last_section_idx {
        doc.mindmap.nodes.get_mut(&node_id).unwrap().sections[i].frame_border = None;
    }
    doc.selection = SelectionState::SectionRange {
        sel: SectionSel {
            node_id: node_id.clone(),
            section_idx: 0,
        },
        range: (0, last_section_idx),
    };
    doc.undo_stack.clear();
    doc.dirty = false;

    let pairs: Vec<(String, usize)> = (0..=last_section_idx)
        .map(|i| (node_id.clone(), i))
        .collect();
    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Sections(pairs.clone()), edits);
    let _ = doc
        .commit_border_preview()
        .expect("preview was active before commit");

    // Every section in the range now carries `heavy`.
    for i in 0..=last_section_idx {
        assert_eq!(
            doc.mindmap.nodes.get(&node_id).unwrap().sections[i]
                .frame_border
                .as_ref()
                .unwrap()
                .preset,
            "heavy",
            "commit must fan out to every section in the range; missed section[{}]",
            i
        );
    }
    // Same per-target undo posture.
    assert_eq!(
        doc.undo_stack.len(),
        pairs.len(),
        "SectionRange commit must push one undo entry per section pair ({}); pushed {}",
        pairs.len(),
        doc.undo_stack.len()
    );
}

/// **C19 regression** — `Action::SetBorderPreview` /
/// `CommitBorderPreview` / `CancelBorderPreview` arms route to
/// the corresponding document setters with the typed
/// `BorderPreviewTargetKind` discriminator. The dispatch arms
/// can't be exercised without a `Renderer`
/// (`TEST_CONVENTIONS.md §T8`), so this test pins the
/// document-side contract `apply_set_border_preview` ultimately
/// invokes — `target_kind: Node` resolves to a
/// `BorderPreviewTarget::Nodes` against the live selection.
#[test]
fn test_border_preview_target_kind_node_resolves_against_live_selection() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit, SelectionState,
    };
    let mut doc = load_test_doc();
    let ids = first_n_testament_node_ids(&doc, 2);
    doc.selection = SelectionState::Multi(ids.clone());

    // Mimic the resolver `apply_set_border_preview` runs for
    // `BorderPreviewTargetKind::Node`: ids come from
    // `nodes_in_selection(&doc.selection, ...)` and feed
    // `BorderPreviewTarget::Nodes(...)`.
    let resolved_ids = crate::application::console::commands::border::nodes_in_selection(
        &doc.selection,
        "border preview",
    )
    .expect("Multi selection resolves to ids");
    assert_eq!(resolved_ids.len(), ids.len(), "all selected ids carried through");
    for id in &ids {
        assert!(
            resolved_ids.contains(id),
            "live selection id {} must appear in resolved target",
            id
        );
    }

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let outcome = doc.set_border_preview(BorderPreviewTarget::Nodes(resolved_ids), edits);
    assert!(doc.border_preview.is_some(), "preview slot populated");
    assert!(
        !outcome.preset_auto_promoted,
        "plain preset edit must not auto-promote"
    );
}

/// **Important review finding** — `set_node_border_visible`
/// (the `border on` / `border off` setter) was missing the
/// implicit-cancel rule the file's module doc claims is
/// universal. With the fix, flipping a node's frame
/// visibility while a `Nodes(_)` preview targets that node
/// clears the preview first — same scope-gating as
/// `set_node_border_config`.
///
/// Pre-fix: `border preview preset=heavy` then `border off`
/// would leave the preview rendering through `force_show_frame`
/// on top of the `show_frame=false` commit, so the user sees
/// the border they just hid still on screen.
#[test]
fn test_border_on_off_clears_active_node_preview() {
    use crate::application::document::{
        BorderConfigEdits, BorderPreviewTarget, OptionEdit,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());

    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("heavy".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    assert!(doc.border_preview.is_some(), "preview was set");

    // `border off` (visibility flip) must clear the preview.
    doc.set_node_border_visible(&nid, false);
    assert!(
        doc.border_preview.is_none(),
        "border off must clear an active per-node preview (implicit-cancel rule)"
    );

    // Same for `border on`: stage another preview, flip back to
    // visible, the preview is gone.
    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set("double".into());
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    assert!(doc.border_preview.is_some());
    doc.set_node_border_visible(&nid, true);
    assert!(
        doc.border_preview.is_none(),
        "border on must clear an active per-node preview (implicit-cancel rule)"
    );
}

// ── Envelope contract, pinned at each call site ────────────────
//
// `nodes/undo_envelope.rs` owns the snapshot → verdict →
// undo-push → auto-fit sequence for every node-scoped setter.
// Testing the envelope in isolation is not enough: the bug class
// this replaces was always a *caller* that wired the sequence up
// slightly differently from its siblings. So each committing
// setter gets its no-op contract pinned here, at the public API
// the app actually calls.

/// Run `call` against a fresh document with a clean undo stack
/// and `dirty` cleared, and assert the call reported no change,
/// pushed no undo entry, did not dirty the document, and left the
/// node byte-identical.
///
/// The four-part no-op contract every setter in this module
/// shares. Named per call site by the tests below so a failure
/// says which setter drifted.
fn assert_setter_no_op<F>(label: &str, call: F)
where
    F: FnOnce(&mut MindMapDocument, &str) -> bool,
{
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.undo_stack.clear();
    doc.dirty = false;
    let before = doc.mindmap.nodes.get(&nid).expect("node").clone();

    let changed = call(&mut doc, &nid);

    assert!(!changed, "{label}: expected a no-op verdict");
    assert!(doc.undo_stack.is_empty(), "{label}: no-op pushed an undo entry");
    assert!(!doc.dirty, "{label}: no-op dirtied the document");
    let after = doc.mindmap.nodes.get(&nid).expect("node");
    assert_eq!(
        node_fingerprint(&before),
        node_fingerprint(after),
        "{label}: no-op mutated the node"
    );
}

/// Structural fingerprint of a node for whole-value comparison.
/// `NodeStyle` and `MindSection` deliberately do not implement
/// `PartialEq` (they are data-model records, not value types), so
/// the tests compare their serialized form — which also catches a
/// field a future `PartialEq` impl might forget.
fn node_fingerprint(node: &MindNode) -> String {
    serde_json::to_string(node).expect("MindNode serializes")
}

/// Every style / text setter that is handed the value it already
/// holds must report no change and leave nothing behind. One
/// assertion per call site, because a shared envelope only helps
/// if the callers actually reach it.
#[test]
fn test_node_setters_are_no_ops_when_value_is_unchanged() {
    assert_setter_no_op("set_node_bg_color", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().style.background_color.clone();
        doc.set_node_bg_color(nid, current)
    });
    assert_setter_no_op("set_node_border_color", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().style.frame_color.clone();
        doc.set_node_border_color(nid, current)
    });
    assert_setter_no_op("set_node_text_color", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().style.text_color.clone();
        doc.set_node_text_color(nid, current)
    });
    assert_setter_no_op("set_node_border_visible", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().style.show_frame;
        doc.set_node_border_visible(nid, current)
    });
    assert_setter_no_op("set_node_text", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().sections[0].text.clone();
        doc.set_node_text(nid, current)
    });
    assert_setter_no_op("set_section_text", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().sections[0].text.clone();
        doc.set_section_text(nid, 0, current)
    });
    assert_setter_no_op("set_section_text_preserving_runs", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().sections[0].text.clone();
        doc.set_section_text_preserving_runs(nid, 0, current)
    });
    assert_setter_no_op("set_node_font_size", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().sections[0].text_runs[0].size_pt;
        doc.set_node_font_size(nid, current as f32)
    });
    assert_setter_no_op("set_section_font_size", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().sections[0].text_runs[0].size_pt;
        doc.set_section_font_size(nid, 0, current as f32)
    });
    assert_setter_no_op("set_node_font_family", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().sections[0].text_runs[0]
            .font
            .clone();
        doc.set_node_font_family(nid, Some(&current))
    });
    assert_setter_no_op("set_section_font_family", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().sections[0].text_runs[0]
            .font
            .clone();
        doc.set_section_font_family(nid, 0, Some(&current))
    });
    assert_setter_no_op("set_section_offset", |doc, nid| {
        let o = doc.mindmap.nodes.get(nid).unwrap().sections[0].offset;
        doc.set_section_offset(nid, 0, o.x, o.y).expect("valid offset")
    });
    assert_setter_no_op("set_section_size", |doc, nid| {
        let s = doc.mindmap.nodes.get(nid).unwrap().sections[0].size;
        doc.set_section_size(nid, 0, s).expect("valid size")
    });
    assert_setter_no_op("set_section_text_color", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().sections[0].text_runs[0]
            .color
            .clone();
        doc.set_section_text_color(nid, 0, current)
    });
    assert_setter_no_op("set_node_border_config", |doc, nid| {
        doc.set_node_border_config(nid, BorderConfigEdits::default())
            .changed
    });
    assert_setter_no_op("set_section_frame_border_config", |doc, nid| {
        doc.set_section_frame_border_config(nid, 0, BorderConfigEdits::default())
            .changed
    });
    assert_setter_no_op("apply_section_payload", |doc, nid| {
        let section = &doc.mindmap.nodes.get(nid).unwrap().sections[0];
        let text = section.text.clone();
        let payload = SectionPayload::from_section(section);
        doc.apply_section_payload(nid, 0, text, &payload)
    });
    assert_setter_no_op("set_section_text_color_range", |doc, nid| {
        let color = doc.mindmap.nodes.get(nid).unwrap().sections[0].text_runs[0]
            .color
            .clone();
        doc.set_section_text_color_range(nid, 0, 0, 3, color)
    });
    assert_setter_no_op("set_section_font_size_range", |doc, nid| {
        let size = doc.mindmap.nodes.get(nid).unwrap().sections[0].text_runs[0].size_pt;
        doc.set_section_font_size_range(nid, 0, 0, 3, size as f32)
    });
    assert_setter_no_op("set_section_font_family_range", |doc, nid| {
        let font = doc.mindmap.nodes.get(nid).unwrap().sections[0].text_runs[0]
            .font
            .clone();
        doc.set_section_font_family_range(nid, 0, 0, 3, Some(&font))
    });
}

/// The same battery against a node id that does not exist. Every
/// setter must degrade to the no-op verdict rather than panicking
/// — `CODE_CONVENTIONS.md` §9, and the reason the id lookup now
/// lives inside the envelope.
#[test]
fn test_node_setters_no_op_on_unknown_node_id() {
    let mut doc = load_test_doc();
    doc.undo_stack.clear();
    doc.dirty = false;
    let ghost = "no-such-node-id";

    assert!(!doc.set_node_bg_color(ghost, "#123456".into()));
    assert!(!doc.set_node_border_color(ghost, "#123456".into()));
    assert!(!doc.set_node_text_color(ghost, "#123456".into()));
    assert!(!doc.set_node_border_visible(ghost, true));
    assert!(!doc.set_node_text(ghost, "hi".into()));
    assert!(!doc.set_section_text(ghost, 0, "hi".into()));
    assert!(!doc.set_section_text_preserving_runs(ghost, 0, "hi".into()));
    assert!(!doc.set_node_font_size(ghost, 33.0));
    assert!(!doc.set_section_font_size(ghost, 0, 33.0));
    assert!(!doc.set_node_font_family(ghost, Some("Norse")));
    assert!(!doc.set_section_font_family(ghost, 0, Some("Norse")));
    assert!(!doc.set_section_text_color_range(ghost, 0, 0, 3, "#123456".into()));
    assert!(!doc.set_section_font_size_range(ghost, 0, 0, 3, 33.0));
    assert!(!doc.set_section_font_family_range(ghost, 0, 0, 3, Some("Norse")));
    assert!(
        !doc.set_node_border_config(ghost, BorderConfigEdits::default())
            .changed
    );
    assert!(
        !doc.set_section_frame_border_config(ghost, 0, BorderConfigEdits::default())
            .changed
    );
    assert!(!doc.set_section_offset(ghost, 0, 1.0, 1.0).expect("no error"));
    assert!(!doc
        .set_node_size(
            ghost,
            baumhard::mindmap::model::Size {
                width: 50.0,
                height: 50.0
            }
        )
        .expect("no error"));
    assert!(!doc.fit_node_to_content(ghost).expect("no error"));

    assert!(doc.undo_stack.is_empty(), "unknown id must push no undo entry");
    assert!(!doc.dirty, "unknown id must not dirty the document");
}

/// A section index past the end is the same no-op as an unknown
/// node — the section wrapper folds the lookup in so no caller
/// can index `sections` out of range. Pre-fix
/// `mutate_section_with_style_undo` indexed directly and would
/// have panicked in an interactive path.
#[test]
fn test_section_setters_no_op_on_out_of_range_section_index() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let ghost_idx = 9_999;
    doc.undo_stack.clear();
    doc.dirty = false;

    assert!(!doc.set_section_text(&nid, ghost_idx, "hi".into()));
    assert!(!doc.set_section_text_preserving_runs(&nid, ghost_idx, "hi".into()));
    assert!(!doc.set_section_text_color(&nid, ghost_idx, "#123456".into()));
    assert!(!doc.set_section_font_size(&nid, ghost_idx, 33.0));
    assert!(!doc.set_section_font_family(&nid, ghost_idx, Some("Norse")));
    assert!(!doc.set_section_text_color_range(&nid, ghost_idx, 0, 3, "#123456".into()));
    assert!(!doc
        .set_section_offset(&nid, ghost_idx, 1.0, 1.0)
        .expect("no error"));
    assert!(
        !doc.set_section_frame_border_config(&nid, ghost_idx, BorderConfigEdits::default())
            .changed
    );

    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

/// One labelled node setter in a table-driven battery: the name
/// to report on failure, and a boxed call taking the document and
/// a node id. Aliased because the bare tuple trips
/// `clippy::type_complexity`.
type NodeSetterCase = (&'static str, Box<dyn Fn(&mut MindMapDocument, &str) -> bool>);

/// A real change through any of these setters pushes exactly one
/// undo entry, and one `undo()` puts the node back. The other
/// half of the envelope contract: the no-op path must not be so
/// eager that it swallows genuine edits.
#[test]
fn test_node_setters_push_exactly_one_undo_entry_and_round_trip() {
    let cases: Vec<NodeSetterCase> = vec![
        (
            "set_node_bg_color",
            Box::new(|doc: &mut MindMapDocument, nid: &str| doc.set_node_bg_color(nid, "#0b0b0b".into())),
        ),
        (
            "set_node_text_color",
            Box::new(|doc: &mut MindMapDocument, nid: &str| doc.set_node_text_color(nid, "#0b0b0b".into())),
        ),
        (
            "set_node_text",
            Box::new(|doc: &mut MindMapDocument, nid: &str| {
                doc.set_node_text(nid, "a wholly different string".into())
            }),
        ),
        (
            "set_section_text",
            Box::new(|doc: &mut MindMapDocument, nid: &str| {
                doc.set_section_text(nid, 0, "another different string".into())
            }),
        ),
        (
            "set_node_font_size",
            Box::new(|doc: &mut MindMapDocument, nid: &str| doc.set_node_font_size(nid, 37.0)),
        ),
        (
            "set_node_font_family",
            Box::new(|doc: &mut MindMapDocument, nid: &str| doc.set_node_font_family(nid, Some("Norse"))),
        ),
        (
            "set_section_font_size_range",
            Box::new(|doc: &mut MindMapDocument, nid: &str| {
                doc.set_section_font_size_range(nid, 0, 0, 3, 41.0)
            }),
        ),
    ];

    for (label, call) in cases {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        doc.undo_stack.clear();
        let before = doc.mindmap.nodes.get(&nid).expect("node").clone();

        assert!(call(&mut doc, &nid), "{label}: expected a real change");
        assert_eq!(
            doc.undo_stack.len(),
            1,
            "{label}: expected exactly one undo entry"
        );
        assert!(doc.dirty, "{label}: a real change must dirty the document");

        assert!(doc.undo(), "{label}: undo must succeed");
        let after = doc.mindmap.nodes.get(&nid).expect("node");
        assert_eq!(
            node_fingerprint(&before),
            node_fingerprint(after),
            "{label}: undo did not restore the node"
        );
    }
}

/// Clearing a node's text to `""` must leave *no* runs rather
/// than a degenerate `TextRun { start: 0, end: 0 }`, which
/// violates the `text_run_ops` `start < end` invariant and panics
/// in debug builds on the next slice / splice call.
///
/// `set_section_text` has always guarded this; `set_node_text`
/// did not — the guard never made it across the copy. Regression
/// test for the drift, named after the symptom.
#[test]
fn test_set_node_text_to_empty_leaves_no_degenerate_run() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    assert!(doc.set_node_text(&nid, String::new()));
    let runs = &doc.mindmap.nodes.get(&nid).expect("node").sections[0].text_runs;
    assert!(runs.is_empty(), "empty text must yield zero runs, got {runs:?}");
    // And the sibling setter still agrees.
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    assert!(doc.set_section_text(&nid, 0, String::new()));
    assert!(doc.mindmap.nodes.get(&nid).expect("node").sections[0]
        .text_runs
        .is_empty());
}

/// Switching a frame on runs the border-fit pass, so a node too
/// small for its frame glyphs grows to fit — the same tail
/// `set_node_border_config` runs. Pre-fix `border on` alone
/// skipped the grow while `border on preset=…` performed it, so
/// the same user intent sized the node differently depending on
/// whether an unrelated kv rode along.
#[test]
fn test_set_node_border_visible_on_runs_the_border_fit_pass() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).expect("node");
        node.style.show_frame = false;
        node.sections[0].text = String::new();
        node.sections[0].text_runs.clear();
        node.size = Size {
            width: 4.0,
            height: 4.0,
        };
    }
    assert!(doc.set_node_border_visible(&nid, true));
    let grown = doc.mindmap.nodes.get(&nid).expect("node").size;
    assert!(
        grown.width > 4.0,
        "turning the frame on must grow the node to fit its border glyphs, got {grown:?}"
    );
    // Undo restores both the flag and the grown size in one step.
    assert!(doc.undo());
    let restored = doc.mindmap.nodes.get(&nid).expect("node");
    assert!(!restored.style.show_frame);
    assert_eq!(restored.size.width, 4.0);
}

/// Turning a frame *off* must not grow anything — the border-fit
/// pass returns early on `show_frame == false`, so the tail is
/// harmless in that direction. Guards the other half of the
/// `NodeEditTail::Border` choice above.
#[test]
fn test_set_node_border_visible_off_does_not_grow() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).expect("node");
        node.style.show_frame = true;
        node.sections[0].text = String::new();
        node.sections[0].text_runs.clear();
        node.size = Size {
            width: 4.0,
            height: 4.0,
        };
    }
    assert!(doc.set_node_border_visible(&nid, false));
    assert_eq!(doc.mindmap.nodes.get(&nid).expect("node").size.width, 4.0);
}

/// A color-only setter must not run the text-fit pass: a node
/// the user deliberately shrank below its text floor stays where
/// they put it. Pins the `NodeEditTail::None` choice, which is
/// otherwise invisible until someone "helpfully" upgrades it to
/// `Grow`.
#[test]
fn test_color_setters_do_not_run_the_text_fit_pass() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).expect("node");
        node.style.show_frame = false;
        node.size = Size {
            width: 3.0,
            height: 3.0,
        };
    }
    assert!(doc.set_node_bg_color(&nid, "#0b0b0b".into()));
    assert_eq!(doc.mindmap.nodes.get(&nid).expect("node").size.width, 3.0);
    assert!(doc.set_node_text_color(&nid, "#0c0c0c".into()));
    assert_eq!(doc.mindmap.nodes.get(&nid).expect("node").size.width, 3.0);
}

/// A range setter whose mutation lands on runs that already carry
/// the value backs the mutation out and pushes nothing — and,
/// critically, leaves `dirty` where it found it.
///
/// Pre-fix `mutate_section_runs_in_range` committed through the
/// envelope and then reached for `undo_stack.pop()`, which
/// removed the entry but left `dirty = true` behind: the document
/// reported unsaved changes for an edit that never happened. The
/// file header condemned that exact anti-pattern while the
/// function below it used it.
#[test]
fn test_range_setter_no_op_does_not_leak_dirty() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // Give the section a uniform, known color so the range
    // rewrite below is provably a no-op.
    assert!(doc.set_section_text_color(&nid, 0, "#abcdef".into()));
    doc.undo_stack.clear();
    doc.dirty = false;
    let before = doc.mindmap.nodes.get(&nid).expect("node").sections[0]
        .text_runs
        .clone();

    let changed = doc.set_section_text_color_range(&nid, 0, 0, 3, "#abcdef".into());

    assert!(!changed, "re-applying the same color must be a no-op");
    assert!(doc.undo_stack.is_empty(), "no-op must push no undo entry");
    assert!(!doc.dirty, "no-op must not leave dirty set");
    assert_eq!(
        &before,
        &doc.mindmap.nodes.get(&nid).expect("node").sections[0].text_runs,
        "no-op must leave the runs exactly as they were"
    );
}

/// A range setter that *does* change something still commits
/// normally through the envelope — one entry, one undo.
#[test]
fn test_range_setter_real_change_commits_once() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.undo_stack.clear();
    let before = doc.mindmap.nodes.get(&nid).expect("node").sections[0]
        .text_runs
        .clone();

    assert!(doc.set_section_text_color_range(&nid, 0, 0, 3, "#fedcba".into()));
    assert_eq!(doc.undo_stack.len(), 1);
    assert!(doc.dirty);
    assert!(doc.undo());
    assert_eq!(
        &before,
        &doc.mindmap.nodes.get(&nid).expect("node").sections[0].text_runs
    );
}

/// Repeating an AABB write on a *framed* node no-ops, because the
/// envelope's verdict is computed after the border-fit pass
/// inflates the size past what was asked for. Checking before the
/// pass would report a change on every call and stack undo
/// entries — a bug this codebase has already shipped once.
#[test]
fn test_set_node_size_is_idempotent_on_a_framed_node() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.mindmap.nodes.get_mut(&nid).expect("node").style.show_frame = true;
    let target = Size {
        width: 30.0,
        height: 12.0,
    };
    assert!(doc.set_node_size(&nid, target).expect("valid"));
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(
        !doc.set_node_size(&nid, target).expect("valid"),
        "second identical resize must no-op after the border-fit pass"
    );
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

/// The AABB envelope's post-tail verdict is shared, so the other
/// two `EditNodeAabb` setters inherit the same idempotency. Each
/// is settled with one call, then repeated: the repeat must
/// no-op.
///
/// These are deliberately *not* in the "unchanged value" battery
/// above: handing `set_node_size` the size a node currently has
/// is not necessarily a no-op, because the auto-fit pass may
/// legitimately grow a node that is sitting below its text floor.
/// Settling first is what makes the second call comparable.
#[test]
fn test_aabb_setters_are_idempotent_once_settled() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let (pos, size) = {
        let n = doc.mindmap.nodes.get(&nid).expect("node");
        (n.position, n.size)
    };
    // Settle: the first call may grow past `size`.
    let _ = doc.set_node_aabb(&nid, pos, size).expect("valid");
    let settled = doc.mindmap.nodes.get(&nid).expect("node").size;
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(
        !doc.set_node_aabb(&nid, pos, settled).expect("valid"),
        "set_node_aabb: repeating a settled AABB must no-op"
    );
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);

    // `fit_node_to_content` shrinks to the measured text floor;
    // running it twice must land the second call on the same
    // post-border-grow size.
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let _ = doc.fit_node_to_content(&nid).expect("has measurable text");
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(
        !doc.fit_node_to_content(&nid).expect("has measurable text"),
        "fit_node_to_content: a second fit must no-op"
    );
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

/// The structural mutators keep their `GrowAndCleanup` tail: a
/// section selection stranded past the end of the shortened
/// `sections` vec is repaired, and undo restores it. Pins the one
/// tail that does more than auto-fit.
#[test]
fn test_delete_section_repairs_stranded_selection_and_undo_restores_it() {
    let (mut doc, nid) = super::tests_common::pinned_two_section_node();
    doc.selection = SelectionState::Section(SectionSel {
        node_id: nid.clone(),
        section_idx: 1,
    });
    doc.undo_stack.clear();

    doc.delete_section(&nid, 1).expect("delete ok");
    assert!(
        matches!(&doc.selection, SelectionState::Single(id) if *id == nid),
        "a selection on the deleted section must demote to the node, got {:?}",
        doc.selection
    );

    assert!(doc.undo());
    assert!(
        matches!(&doc.selection, SelectionState::Section(s) if s.section_idx == 1),
        "undo must restore the pre-mutation selection, got {:?}",
        doc.selection
    );
}
