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
/// requested section — section 0 gets the same behavior as the
/// pre-section `set_node_text`, sections 1+ stay untouched
/// unless explicitly targeted. Pins the section-aware setter's
/// addressing for the per-section text-edit path.
#[test]
fn test_set_section_text_targets_specific_section() {
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    // Materialize a multi-section node by appending a second
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
    assert_eq!(doc.set_section_size(&id, 1, new_size), Ok(true));
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
        after.width,
        after.height
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
    assert!(doc
        .set_node_size(&id, huge)
        .is_err_and(|m| m.contains("exceeds the")));
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

/// **Past the budget, the floor must beat both samples.**
///
/// The width sample was once the first `MEASURED_LINE_BUDGET`
/// lines, which sized a node from a prefix and clipped a long line
/// past it. Replacing that with the widest-by-column-proxy lines
/// fixed that case and broke another: the proxy counts columns
/// while cosmic-text shapes advances against a proportional face,
/// so `"i".repeat(30)` outranks `"W".repeat(20)` and shapes far
/// narrower. On this input the *replacement* measured ~46% of what
/// the node needed — worse than the prefix it replaced.
///
/// So both samples are measured and the wider wins, and this is the
/// input that distinguishes that from either one alone: the widest
/// line lives in the first 512 (so the proxy sample misses it) and
/// the proxy's picks are narrower (so taking the proxy alone
/// regresses). Asserted as a strict inequality against the narrow
/// sample, because "not worse" is the whole property.
#[test]
fn test_text_floor_past_budget_beats_both_samples() {
    use super::{compute_one_node_text_floor, MEASURED_LINE_BUDGET};

    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);

    // 512 wide-shaping lines, then 600 that win the column ranking
    // while shaping narrower.
    let wide = "W".repeat(20);
    let narrow = "i".repeat(30);
    let mut text = String::new();
    for _ in 0..MEASURED_LINE_BUDGET {
        text.push_str(&wide);
        text.push('\n');
    }
    for i in 0..600 {
        text.push_str(&narrow);
        if i < 599 {
            text.push('\n');
        }
    }
    {
        let n = doc.mindmap.nodes.get_mut(&id).unwrap();
        n.sections.truncate(1);
        n.sections[0].text = text.clone();
        n.sections[0].text_runs.clear();
        n.sections[0].size = None;
        n.sections[0].offset = baumhard::mindmap::model::Position { x: 0.0, y: 0.0 };
    }
    let (floor_w, _) = compute_one_node_text_floor(&doc.mindmap.nodes[&id]);

    // What the node would measure if only the narrow (proxy-picked)
    // lines were sampled — the regression this guards.
    let mut narrow_only = doc.mindmap.nodes[&id].clone();
    narrow_only.sections[0].text = narrow.clone();
    let (narrow_w, _) = compute_one_node_text_floor(&narrow_only);

    // And what the widest line actually needs.
    let mut wide_only = doc.mindmap.nodes[&id].clone();
    wide_only.sections[0].text = wide.clone();
    let (wide_w, _) = compute_one_node_text_floor(&wide_only);

    assert!(
        wide_w > narrow_w,
        "fixture is wrong: the wide line must shape wider than the narrow one ({wide_w} vs {narrow_w})"
    );
    assert!(
        floor_w >= wide_w,
        "the floor must cover the widest line, got {floor_w} < {wide_w}"
    );
    assert!(
        floor_w > narrow_w,
        "the floor must beat the proxy-only sample, got {floor_w} <= {narrow_w}"
    );

    // The other direction, and the one the first version of this
    // test missed. Above, every wide-shaping line sits inside the
    // first MEASURED_LINE_BUDGET, so the positional prefix alone
    // already measures them — deleting the widest sample left the
    // whole suite green. Here the widest line sits PAST the budget
    // and the proxy ranks it top, so the prefix cannot see it and
    // only the widest sample can. Between the two cases, deleting
    // either sample fails.
    let long = "W".repeat(300);
    let mut past = String::new();
    for _ in 0..MEASURED_LINE_BUDGET {
        past.push_str("x\n");
    }
    past.push_str(&long);
    {
        let n = doc.mindmap.nodes.get_mut(&id).unwrap();
        n.sections[0].text = past;
    }
    let (past_floor_w, _) = compute_one_node_text_floor(&doc.mindmap.nodes[&id]);

    let mut long_only = doc.mindmap.nodes[&id].clone();
    long_only.sections[0].text = long.clone();
    let (long_w, _) = compute_one_node_text_floor(&long_only);

    let mut short_only = doc.mindmap.nodes[&id].clone();
    short_only.sections[0].text = "x".to_string();
    let (short_w, _) = compute_one_node_text_floor(&short_only);

    assert!(
        long_w > short_w,
        "fixture is wrong: the long line must shape wider ({long_w} vs {short_w})"
    );
    assert!(
        past_floor_w >= long_w,
        "the floor must cover a widest line that sits past the budget, got {past_floor_w} < {long_w}"
    );
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
        n.sections[0].offset = baumhard::mindmap::model::Position { x: f64::NAN, y: 0.0 };
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
        n.sections
            .push(baumhard::mindmap::model::MindSection::new_default(
                "x".into(),
                Vec::new(),
            ));
        n.sections[0].offset = baumhard::mindmap::model::Position { x: f64::NAN, y: 0.0 };
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
                size_pt: 24.0,
                color: "#ffffff".to_string(),
                hyperlink: None,
            });
        }
        node.sections[0].text_runs[0].bold = true;
        node.sections[0].text_runs[0].color = "#abcdef".to_string();
        node.sections[0].text_runs[0].size_pt = 33.0;
    }
    assert!(doc.set_node_text(&nid, "rewritten".to_string()));
    let run = &doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs[0];
    assert!(run.bold);
    assert_eq!(run.color, "#abcdef");
    assert_eq!(run.size_pt, 33.0);
}

// -----------------------------------------------------------------
// Node style setters (bg / border / text color, font size)
// -----------------------------------------------------------------

/// Asserted through the cascade, not off `style`: every testament
/// node is themed, so its fill comes from its palette group and
/// `style.background_color` is a value nothing reads. A write that
/// landed there would leave this node rendering the palette color
/// while the setter reported success.
#[test]
fn test_set_node_bg_color_round_trips_through_undo() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(
        node.color_schema.is_some(),
        "the fixture node must be themed for this test to mean anything"
    );
    let before = doc.mindmap.node_background_color(node).to_string();
    assert!(doc.set_node_bg_color(&nid, Some("#123456")));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(doc.mindmap.node_background_color(node), "#123456");
    assert!(matches!(
        doc.undo_stack.last(),
        Some(UndoAction::EditNodeStyle { .. })
    ));
    assert!(doc.undo());
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(doc.mindmap.node_background_color(node), before);
}

/// The no-op verdict is decided against the value the setter would
/// write, which on a themed node is the override slot rather than
/// `style` — so "unchanged" here means "already overridden to this
/// exact color", and the fixture has to be put in that state first.
/// A themed node with *no* override that is handed its palette's
/// own color does change: it stops tracking the palette.
#[test]
fn test_set_node_bg_color_unchanged_is_noop() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    assert!(doc.set_node_bg_color(&nid, Some("#123456")));
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_node_bg_color(&nid, Some("#123456")));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

/// Same reasoning as `test_set_node_bg_color_round_trips_through_undo`:
/// the frame channel is read through the cascade, so that is where
/// the assertion has to look.
#[test]
fn test_set_node_border_color_writes_frame_color() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    assert!(doc.set_node_border_color(&nid, Some("#ff00ff")));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(doc.mindmap.node_frame_color(node), "#ff00ff");
}

/// First-edit materialization of `node.style.border` uses
/// `GlyphBorderConfig::default()`.
/// Pin the resulting `preset` to `"light"` so a regression to
/// `"rounded"` — the previous default — surfaces here. The
/// trigger is any kv edit that *touches a config field*; we
/// use `padding=` because it's a leaf field with no other
/// behavior entanglement.
#[test]
fn test_default_border_config_first_edit_materializes_light_preset() {
    use crate::application::document::{BorderConfigEdits, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    // Strip any pre-existing per-node border so we exercise the
    // `get_or_insert_with(GlyphBorderConfig::default)` path.
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.border = None;
    let edits = BorderConfigEdits {
        padding: OptionEdit::Set(8.0),
        ..Default::default()
    };
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
        .expect("first-edit materialized the per-node config");
    assert_eq!(cfg.preset, "light");
}

/// Setting text color rewrites the node's default and every run
/// whose color matched the pre-edit *effective* default. A run the
/// user colored by hand (mismatched) keeps its override.
///
/// Unthemed, so `style.text_color` really is the effective default
/// — the themed half is
/// `test_set_node_text_color_on_a_themed_node_follows_the_palette`.
#[test]
fn test_set_node_text_color_preserves_per_run_overrides() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    doc.mindmap.nodes.get_mut(&nid).unwrap().color_schema = None;
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
                size_pt: 24.0,
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
                size_pt: 24.0,
                color: "#abcdef".into(), // user override
                hyperlink: None,
            },
        ];
    }
    assert!(doc.set_node_text_color(&nid, Some("#111111")));
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
        node.color_schema = None;
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
    assert!(doc.set_node_text_color(&nid, Some("#222222")));
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

/// The themed half of
/// `test_set_node_text_color_preserves_per_run_overrides`. On a
/// node bound to a palette, the *effective* default is the group's
/// `text`, not `style.text_color` — so that is the value a run has
/// to match to count as default-following, and the new default
/// lands in the node's own overrides where the read path looks.
#[test]
fn test_set_node_text_color_on_a_themed_node_follows_the_palette() {
    use super::tests_common::theme_node_with_probe_palette;
    use baumhard::mindmap::model::ColorGroup;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    theme_node_with_probe_palette(
        &mut doc,
        &nid,
        "write-probe",
        ColorGroup {
            background: "#101010".into(),
            frame: "#202020".into(),
            text: "#303030".into(),
            title: String::new(),
        },
    );
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        // `style.text_color` is a stale copy the palette shadows —
        // the shape every migrated node is in.
        node.style.text_color = "#dddddd".into();
        node.sections[0].text = "abcdefghi".into();
        node.sections[0].text_runs = vec![
            TextRun {
                start: 0,
                end: 3,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 24.0,
                color: "#303030".into(), // baked copy of the palette default
                hyperlink: None,
            },
            TextRun {
                start: 3,
                end: 6,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 24.0,
                color: "#abcdef".into(), // hand-colored
                hyperlink: None,
            },
            TextRun {
                start: 6,
                end: 9,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 24.0,
                color: String::new(), // defers to the node
                hyperlink: None,
            },
        ];
    }
    assert!(doc.set_node_text_color(&nid, Some("#111111")));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(
        doc.mindmap.node_text_color(node),
        "#111111",
        "the node's effective text color must be the one just set"
    );
    assert_eq!(
        node.style.text_color, "#dddddd",
        "the stale style copy is not the tier the read path consults and must not be touched"
    );
    let runs = &node.sections[0].text_runs;
    assert_eq!(
        runs[0].color, "#111111",
        "a run baked from the old default follows"
    );
    assert_eq!(runs[1].color, "#abcdef", "a hand-colored run keeps its override");
    assert_eq!(
        runs[2].color, "",
        "a deferring run stays deferring — it already follows the new default"
    );
}

/// A per-node fill / frame write on a themed node has to be
/// *visible*: the verb reporting success while the node keeps
/// painting its palette color is the exact failure this cascade
/// nearly shipped. Also pins that the write did not disturb the
/// node's palette binding or its other channels.
#[test]
fn test_per_node_color_writes_are_visible_on_a_themed_node() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let (palette_fill, palette_frame, palette_text) = {
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert!(node.color_schema.is_some(), "testament nodes are all themed");
        (
            doc.mindmap.node_background_color(node).to_string(),
            doc.mindmap.node_frame_color(node).to_string(),
            doc.mindmap.node_text_color(node).to_string(),
        )
    };
    assert!(doc.set_node_bg_color(&nid, Some("#00ff00")));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(doc.mindmap.node_background_color(node), "#00ff00");
    assert_ne!(
        palette_fill, "#00ff00",
        "the fixture must start on a different fill or this proves nothing"
    );
    assert_eq!(
        doc.mindmap.node_frame_color(node),
        palette_frame,
        "a fill write must not move the frame channel off the palette"
    );
    assert_eq!(
        doc.mindmap.node_text_color(node),
        palette_text,
        "a fill write must not move the text channel off the palette"
    );
    assert!(
        node.color_schema.is_some(),
        "the node keeps its palette binding — only one channel excepted it"
    );

    assert!(doc.set_node_border_color(&nid, Some("#0000ff")));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(doc.mindmap.node_frame_color(node), "#0000ff");
    assert_eq!(doc.mindmap.node_background_color(node), "#00ff00");
}

/// Undo of a per-node color write on a themed node puts the node
/// back on its palette — not on the stale `style` value, and not
/// on an empty override that would read as "no opinion" but still
/// leave the key in the file.
#[test]
fn test_per_node_color_write_on_a_themed_node_undoes_back_to_the_palette() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let before = {
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        doc.mindmap.node_background_color(node).to_string()
    };
    assert!(doc.set_node_bg_color(&nid, Some("#00ff00")));
    assert!(doc.undo());
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(doc.mindmap.node_background_color(node), before);
    assert!(
        node.color_schema
            .as_ref()
            .is_some_and(|schema| schema.overrides.is_empty()),
        "undo must clear the override, not merely blank it"
    );
    // ...and the node tracks its palette again, which is the
    // property "restores what was there" actually means here.
    let group_fill = doc
        .mindmap
        .resolve_theme_colors(node)
        .expect("a themed node resolves")
        .background
        .clone();
    assert_eq!(doc.mindmap.node_background_color(node), group_fill);
}

/// An override outranks the group even when the schema's palette
/// has gone missing. "I painted this node green" does not stop
/// being true because the theme it excepts broke.
#[test]
fn test_a_color_override_survives_a_dangling_palette_reference() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    assert!(doc.set_node_bg_color(&nid, Some("#00ff00")));
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.color_schema.as_mut().unwrap().palette = "no-such-palette".into();
    }
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(doc.mindmap.resolve_theme_colors(node).is_none());
    assert_eq!(doc.mindmap.node_background_color(node), "#00ff00");
}

/// The literal reproduction from the review: node `0` of
/// `maps/palette_cascade.mindmap.json` draws its palette's
/// `#a10000` while `style.background_color` says `#111111`.
/// `color bg=#00ff00` on it must move the pixels.
///
/// This fixture exists precisely because `testament`'s baked
/// `style` values have drifted toward its palette — here the two
/// tiers are unmistakably different, so a write to the wrong one
/// cannot accidentally look right.
#[test]
fn test_bg_write_moves_the_fill_on_the_palette_cascade_fixture() {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("maps/palette_cascade.mindmap.json");
    let map =
        baumhard::mindmap::loader::load_from_file(&path).expect("the palette-cascade fixture must load");
    let mut doc = MindMapDocument::from_mindmap(map, None);
    {
        let node = doc.mindmap.nodes.get("0").expect("fixture node 0");
        assert_eq!(node.style.background_color, "#111111");
        assert_eq!(doc.mindmap.node_background_color(node), "#a10000");
    }
    assert!(doc.set_node_bg_color("0", Some("#00ff00")));
    let node = doc.mindmap.nodes.get("0").unwrap();
    assert_eq!(
        doc.mindmap.node_background_color(node),
        "#00ff00",
        "the verb reported success, so the node must be drawn in the new color"
    );
    assert!(doc.undo());
    let node = doc.mindmap.nodes.get("0").unwrap();
    assert_eq!(
        doc.mindmap.node_background_color(node),
        "#a10000",
        "undo restores what was actually there — the palette color, not #111111"
    );
}

/// A themed node recolored by hand survives a save / load round
/// trip with its override intact — the write is not a runtime-only
/// effect. The unrecolored siblings keep the key out of their JSON.
#[test]
fn test_a_color_override_round_trips_through_save_and_load() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    assert!(doc.set_node_bg_color(&nid, Some("#00ff00")));
    let dir = baumhard::util::test_temp::TempDir::new("color-override-round-trip");
    let path = dir.join("overridden.mindmap.json");
    baumhard::mindmap::loader::save_to_file(&path, &doc.mindmap).expect("save must succeed");
    let reloaded = baumhard::mindmap::loader::load_from_file(&path).expect("reload must succeed");
    let node = reloaded.nodes.get(&nid).expect("node survives the round trip");
    assert_eq!(reloaded.node_background_color(node), "#00ff00");
    let untouched = reloaded
        .nodes
        .values()
        .filter(|n| n.id != nid)
        .filter_map(|n| n.color_schema.as_ref())
        .all(|schema| schema.overrides.is_empty());
    assert!(untouched, "only the recolored node carries an override");
}

#[test]
fn test_set_node_font_size_writes_all_runs_and_round_trips() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let before_sizes: Vec<f32> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.size_pt)
        .collect();
    assert!(doc.set_node_font_size(&nid, 48.0));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(node.sections[0].text_runs.iter().all(|r| r.size_pt == 48.0));
    assert!(doc.undo());
    let after_sizes: Vec<f32> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
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
    assert!(node.sections[0].text_runs.iter().all(|r| r.size_pt == 1.0));
}

#[test]
fn test_set_node_style_unknown_id_returns_false() {
    let mut doc = load_test_doc();
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_node_bg_color("nope", Some("#000")));
    assert!(!doc.set_node_border_color("nope", Some("#000")));
    assert!(!doc.set_node_text_color("nope", Some("#000")));
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
            ..Canvas::default()
        },
        palettes: HashMap::new(),
        nodes,
        edges: vec![],
        custom_mutations: vec![],
        macros: vec![],
        unknown_keys: Default::default(),
        skipped_constructs: Default::default(),
    };
    // Round-trip through JSON to exercise the finalize hook
    // — `MindMapDocument::from_json_str` calls `finalize`,
    // which runs both grow passes. Direct construction skips
    // it.
    let json = serde_json::to_string(&map).expect("serializes");
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

/// Set a color on `[range_start, range_end)` inside one section
/// — pins the simplest happy path (range entirely inside one run).
#[test]
fn test_set_section_text_color_range_inside_one_run() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    // Apply blue to a sub-range and verify the section now has
    // three runs: original-color | blue | original-color.
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
    let pre = doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs.clone();
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

/// Gap-fill: applying a color on a range that falls in a gap
/// (no covering run) inserts a fresh run carrying the color.
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
    let runs_before = doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs.len();
    assert!(doc.set_section_text_color_range(&id, 0, 5, 8, "#123456".into()));
    let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    assert!(runs.len() > runs_before, "gap-fill must add at least one run");
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
fn set_section_zero_text_and_single_run(doc: &mut MindMapDocument, node_id: &str, text: &str, font: &str) {
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
        size_pt: 14.0,
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
// assert behavior observable from the document layer alone.

/// Setting a preview must not push undo, flip `dirty`, or mutate
/// the model. Same discipline as `color_picker_preview` — preview
/// is a transient runtime substitution, not a model edit.
#[test]
fn test_border_preview_does_not_push_undo_or_dirty() {
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let undo_depth = doc.undo_stack.len();
    let before_node = doc.mindmap.nodes.get(&nid).cloned().unwrap();
    doc.dirty = false;

    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);

    assert_eq!(doc.undo_stack.len(), undo_depth);
    assert!(!doc.dirty);
    assert_eq!(
        doc.mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .style
            .border
            .as_ref()
            .map(|c| c.preset.clone()),
        before_node.style.border.as_ref().map(|c| c.preset.clone()),
        "model border slot must be byte-identical to pre-preview state"
    );
    assert!(doc.border_preview.is_some(), "preview slot populated");
}

/// Canceling a preview returns to the pre-preview model state
/// without writing anything. Mirrors
/// `test_color_picker_preview_cleared_returns_to_committed`.
#[test]
fn test_border_preview_cleared_returns_to_committed() {
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let before_node = doc.mindmap.nodes.get(&nid).cloned().unwrap();
    doc.dirty = false;
    let undo_depth = doc.undo_stack.len();

    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("double".into()),
        ..Default::default()
    };
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    let returned = doc.cancel_border_preview();

    assert!(returned, "cancel returns true when a preview was active");
    assert!(doc.border_preview.is_none(), "preview slot cleared");
    assert!(!doc.dirty);
    assert_eq!(doc.undo_stack.len(), undo_depth);
    assert_eq!(
        doc.mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .style
            .border
            .as_ref()
            .map(|c| c.preset.clone()),
        before_node.style.border.as_ref().map(|c| c.preset.clone()),
        "model unchanged after preview-then-cancel"
    );
}

/// Commit dispatches to the underlying setter, which pushes one
/// undo entry per affected target and flips `dirty`. The preview
/// slot is cleared.
#[test]
fn test_border_preview_commit_pushes_undo_and_dirty() {
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let undo_depth = doc.undo_stack.len();
    doc.dirty = false;

    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    let outcome = doc.commit_border_preview().expect("preview was active");

    assert!(outcome.changed);
    assert!(doc.dirty);
    assert!(
        doc.undo_stack.len() > undo_depth,
        "commit pushes at least one undo entry"
    );
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
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
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
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());

    let first_edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), first_edits);

    let second_edits = BorderConfigEdits {
        preset: OptionEdit::Set("double".into()),
        ..Default::default()
    };
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
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());

    assert!(
        !doc.cancel_border_preview(),
        "cancel returns false when no preview is active"
    );

    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
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
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    // Ensure the pre-preview slot is non-custom so the helper
    // sees a real promotion.
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.border = None;

    let mut edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
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

/// Asking for `preset=custom` is not a promotion — it is the thing
/// promotion promotes *to*. The outcome must not claim otherwise.
///
/// Fails when: `detect_preset_auto_promote` asks only whether a
/// `preset=` kv was present. It did, and the console printed
/// `note: preset='custom' auto-promoted to 'custom' (a side or
/// corner glyph was set; non-custom presets ignore the per-node
/// glyph override)` on all four border surfaces — two false clauses
/// in one line, since nothing was promoted and, on the bare form,
/// no glyph was set.
///
/// Negative control: the same node, the same call, with `heavy` in
/// place of `custom` and a side glyph alongside it, is a real
/// promotion and must still report one. Without that row a
/// `detect_preset_auto_promote` that returned early always would
/// pass the first half.
#[test]
fn test_asking_for_preset_custom_is_not_reported_as_a_promotion_to_custom() {
    use crate::application::document::{BorderConfigEdits, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // A non-custom starting point, so `was_already_custom` is not
    // what suppresses the note.
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.border = None;

    let outcome = doc.set_node_border_config(
        &nid,
        BorderConfigEdits {
            preset: OptionEdit::Set("custom".into()),
            ..Default::default()
        },
    );
    assert!(
        !outcome.preset_auto_promoted,
        "`preset=custom` is what the user asked for, not a promotion away from it"
    );
    assert_eq!(
        outcome.requested_preset.as_deref(),
        Some("custom"),
        "the requested preset is still reported — only the promotion claim is dropped"
    );

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.border = None;
    let mut edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
    edits
        .with_side_pattern(crate::application::document::BorderSide::Top, "###(*)###")
        .expect("pattern parses");
    let control = doc.set_node_border_config(&nid, edits);
    assert!(
        control.preset_auto_promoted,
        "control: a side glyph over a non-custom preset is still a promotion"
    );
}

/// Selection drift: when the live selection no longer covers the
/// preview's `selection_snapshot`, the scene-build path renders
/// as if no preview were active. The actual slot empties at the
/// next `set_*` / `cancel_*` / `commit_*` call (defer-clear).
#[test]
fn test_border_preview_drift_clears_on_selection_change() {
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit, SelectionState};
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
    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
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
    let canceled = doc.cancel_border_preview();
    assert!(!canceled, "drifted preview is treated as already-cleared");
    assert!(doc.border_preview.is_none());
}

/// A direct (non-preview) committing edit clears any active
/// preview. Without this rule, typing `border preset=double`
/// after `border preview preset=heavy` would render the heavy
/// preview *over* the just-committed double border — visibly
/// stale until the user manually canceled. The implicit-cancel
/// fires on every committing setter:
/// `set_node_border_config`, `set_section_frame_border_config`,
/// `set_canvas_default_border`,
/// `set_canvas_default_section_frame_border_config`.
#[test]
fn test_committing_set_node_border_config_clears_active_preview() {
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());

    // Stage a preview.
    let preview_edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), preview_edits);
    assert!(doc.border_preview.is_some());

    // A direct committing edit on any of the four setters must
    // clear the preview before applying its own write. Test the
    // node-level setter path here; the section / canvas paths
    // are validated by the same implicit-cancel call site at the
    // top of each setter.
    let direct_edits = BorderConfigEdits {
        preset: OptionEdit::Set("double".into()),
        ..Default::default()
    };
    let _ = doc.set_node_border_config(&nid, direct_edits);

    assert!(
        doc.border_preview.is_none(),
        "committing edit must clear an active preview"
    );
    assert_eq!(
        doc.mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .style
            .border
            .as_ref()
            .unwrap()
            .preset,
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
    use crate::application::document::{BorderConfigEdits, BorderEditOutcome, BorderSide, OptionEdit};
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
        (
            "Set preset to heavy",
            BorderConfigEdits {
                preset: OptionEdit::Set("heavy".into()),
                ..Default::default()
            },
        ),
        (
            "Clear font (Risk #1 case)",
            BorderConfigEdits {
                font: OptionEdit::Clear,
                ..Default::default()
            },
        ),
        (
            "Clear color (Risk #1 case)",
            BorderConfigEdits {
                color: OptionEdit::Clear,
                ..Default::default()
            },
        ),
        (
            "Clear color_palette (Risk #1 case)",
            BorderConfigEdits {
                color_palette: OptionEdit::Clear,
                ..Default::default()
            },
        ),
        (
            "Clear color_palette_field (Risk #1 case)",
            BorderConfigEdits {
                color_palette_field: OptionEdit::Clear,
                ..Default::default()
            },
        ),
        ("Set side top to a pattern (auto-promote to custom)", {
            let mut e = BorderConfigEdits::default();
            e.with_side_pattern(BorderSide::Top, "###(*)###").expect("parses");
            e
        }),
        (
            "Set padding",
            BorderConfigEdits {
                padding: OptionEdit::Set(8.0),
                ..Default::default()
            },
        ),
        (
            "Combine preset=heavy + color=Clear",
            BorderConfigEdits {
                preset: OptionEdit::Set("heavy".into()),
                color: OptionEdit::Clear,
                ..Default::default()
            },
        ),
        (
            "Set palette + field",
            BorderConfigEdits {
                color_palette: OptionEdit::Set("summer".into()),
                color_palette_field: OptionEdit::Set(PaletteField::Background),
                ..Default::default()
            },
        ),
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
            assert_eq!(
                c.font_size_pt.to_bits(),
                p.font_size_pt.to_bits(),
                "[{}] font_size_pt",
                label
            );
            assert_eq!(c.color, p.color, "[{}] color", label);
            assert_eq!(c.padding.to_bits(), p.padding.to_bits(), "[{}] padding", label);
            assert_eq!(c.color_palette, p.color_palette, "[{}] color_palette", label);
            assert_eq!(
                c.color_palette_field, p.color_palette_field,
                "[{}] color_palette_field",
                label
            );
            assert_eq!(
                c.glyphs.is_some(),
                p.glyphs.is_some(),
                "[{}] glyphs Option shape",
                label
            );
            if let (Some(cg), Some(pg)) = (c.glyphs.as_ref(), p.glyphs.as_ref()) {
                assert_eq!(cg.top, pg.top, "[{}] glyphs.top", label);
                assert_eq!(cg.bottom, pg.bottom, "[{}] glyphs.bottom", label);
                assert_eq!(cg.left, pg.left, "[{}] glyphs.left", label);
                assert_eq!(cg.right, pg.right, "[{}] glyphs.right", label);
                assert_eq!(cg.top_left, pg.top_left, "[{}] glyphs.top_left", label);
                assert_eq!(cg.top_right, pg.top_right, "[{}] glyphs.top_right", label);
                assert_eq!(cg.bottom_left, pg.bottom_left, "[{}] glyphs.bottom_left", label);
                assert_eq!(
                    cg.bottom_right, pg.bottom_right,
                    "[{}] glyphs.bottom_right",
                    label
                );
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

    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    let _ = doc.commit_border_preview().expect("preview was active");

    assert!(
        doc.mindmap.nodes.get(&nid).unwrap().style.show_frame,
        "commit must auto-flip `show_frame = true` when the preview's force-show fired \
         (otherwise the user sees the preview render then commit hides it)"
    );
    assert_eq!(
        doc.mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .style
            .border
            .as_ref()
            .unwrap()
            .preset,
        "heavy",
        "the preset still committed"
    );
}

/// **Every** field the force-show coupling claims to cover
/// actually fires it, not just `preset`.
///
/// The coupling asks `edits_touch_cfg_field` whether the preview
/// changed anything; until #48 `commit_border_preview` open-coded
/// that predicate's field list instead of calling it, so a field
/// added to the setter's copy and not to the commit's copy would
/// preview a frame and then hide it at commit — visible only on a
/// node whose committed `show_frame` is already `false`, which is
/// why the single-field `preset` test above never saw it.
///
/// Fails when: a field drops out of `edits_touch_cfg_field` (its
/// row stops flipping `show_frame`), or when the coupling stops
/// consulting the predicate at all (every row fails).
///
/// Control on the same path: the last row stages nothing at all
/// and must leave `show_frame` false. Without it every row above
/// is satisfied by a commit that force-shows unconditionally.
///
/// The exhaustive destructuring below is what keeps the table
/// honest: a new field on `BorderConfigEdits` does not compile
/// until it is named here, and the two fields deliberately absent
/// from the table are named as absent rather than forgotten.
#[test]
fn test_border_preview_commit_force_shows_for_every_field_the_predicate_covers() {
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    use baumhard::mindmap::border::PaletteField;

    // `visible` is the field the coupling *writes*, and `clear` is
    // covered by its own row below rather than by the predicate.
    let BorderConfigEdits {
        preset: _,
        font: _,
        font_size_pt: _,
        color: _,
        padding: _,
        color_palette: _,
        color_palette_field: _,
        side_top: _,
        side_bottom: _,
        side_left: _,
        side_right: _,
        corner_top_left: _,
        corner_top_right: _,
        corner_bottom_left: _,
        corner_bottom_right: _,
        visible: _,
        clear: _,
    } = BorderConfigEdits::default();

    type Stage = fn(&mut BorderConfigEdits);
    let rows: &[(&str, Stage)] = &[
        ("preset", |e| e.preset = OptionEdit::Set("heavy".into())),
        ("font", |e| e.font = OptionEdit::Set("DejaVu Sans Mono".into())),
        ("font_size_pt", |e| e.font_size_pt = OptionEdit::Set(11.0)),
        ("color", |e| e.color = OptionEdit::Set("#ff8800".into())),
        ("padding", |e| e.padding = OptionEdit::Set(3.0)),
        ("color_palette", |e| {
            e.color_palette = OptionEdit::Set("sunset".into())
        }),
        ("color_palette_field", |e| {
            e.color_palette_field = OptionEdit::Set(PaletteField::Background)
        }),
        ("side_top", |e| e.side_top = OptionEdit::Set("=".into())),
        ("side_bottom", |e| e.side_bottom = OptionEdit::Set("=".into())),
        ("side_left", |e| e.side_left = OptionEdit::Set("|".into())),
        ("side_right", |e| e.side_right = OptionEdit::Set("|".into())),
        ("corner_top_left", |e| {
            e.corner_top_left = OptionEdit::Set("+".into())
        }),
        ("corner_top_right", |e| {
            e.corner_top_right = OptionEdit::Set("+".into())
        }),
        ("corner_bottom_left", |e| {
            e.corner_bottom_left = OptionEdit::Set("+".into())
        }),
        ("corner_bottom_right", |e| {
            e.corner_bottom_right = OptionEdit::Set("+".into())
        }),
        // Not a config field, but the commit path couples it the
        // same way: `border reset` on a hidden-frame node has to
        // leave the node showing its (now default) border.
        ("clear", |e| e.clear = true),
    ];

    for (label, stage) in rows {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        doc.mindmap.nodes.get_mut(&nid).unwrap().style.show_frame = false;
        doc.selection = SelectionState::Single(nid.clone());

        let mut edits = BorderConfigEdits::default();
        stage(&mut edits);
        let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
        let _ = doc
            .commit_border_preview()
            .unwrap_or_else(|| panic!("{label}: the preview must be active at commit"));

        assert!(
            doc.mindmap.nodes.get(&nid).unwrap().style.show_frame,
            "{label}: a preview touching this field renders a frame, so committing it \
             must leave the frame shown"
        );
    }

    // Control: a preview that stages nothing must not force-show.
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.show_frame = false;
    doc.selection = SelectionState::Single(nid.clone());
    let _ = doc.set_border_preview(
        BorderPreviewTarget::Nodes(vec![nid.clone()]),
        BorderConfigEdits::default(),
    );
    let _ = doc.commit_border_preview();
    assert!(
        !doc.mindmap.nodes.get(&nid).unwrap().style.show_frame,
        "an empty preview touches no field, so nothing implies visibility — if this \
         flips, the rows above are passing on an unconditional force-show"
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

    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        visible: Some(false),
        ..Default::default()
    };
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
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
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

    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
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
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
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

    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
    let _ = doc.set_border_preview(BorderPreviewTarget::Nodes(ids.clone()), edits);
    let outcome = doc
        .commit_border_preview()
        .expect("preview was active before commit");

    // Every node should now carry the staged preset.
    for id in &ids {
        assert_eq!(
            doc.mindmap
                .nodes
                .get(id)
                .unwrap()
                .style
                .border
                .as_ref()
                .unwrap()
                .preset,
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
    assert!(outcome.changed, "outcome.changed must be true after Multi commit");
}

/// **C20 regression** — commit on a `SectionRange` selection
/// fans out to every section in the range. The section path
/// uses `set_section_frame_border_config` per (node_id,
/// section_idx) pair; each pushes its own undo entry.
#[test]
fn test_border_preview_commit_fans_out_to_section_range() {
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit, SectionSel};
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
    // The span carries the fan-out meaning here: sections
    // 0..=last_section_idx of the owning node. The grapheme range
    // is incidental to this test (border commits never read it).
    doc.selection = SelectionState::SectionRange {
        sel: SectionSel {
            node_id: node_id.clone(),
            section_idx: 0,
        },
        section_span: SectionSpan::new(0, last_section_idx),
        grapheme_range: GraphemeRange::new(0, 1),
    };
    doc.undo_stack.clear();
    doc.dirty = false;

    let pairs: Vec<(String, usize)> = (0..=last_section_idx).map(|i| (node_id.clone(), i)).collect();
    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
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
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit, SelectionState};
    let mut doc = load_test_doc();
    let ids = first_n_testament_node_ids(&doc, 2);
    doc.selection = SelectionState::Multi(ids.clone());

    // Mimic the resolver `apply_set_border_preview` runs for
    // `BorderPreviewTargetKind::Node`: ids come from
    // `nodes_in_selection(&doc.selection, ...)` and feed
    // `BorderPreviewTarget::Nodes(...)`.
    let resolved_ids =
        crate::application::console::commands::border::nodes_in_selection(&doc.selection, "border preview")
            .expect("Multi selection resolves to ids");
    assert_eq!(resolved_ids.len(), ids.len(), "all selected ids carried through");
    for id in &ids {
        assert!(
            resolved_ids.contains(id),
            "live selection id {} must appear in resolved target",
            id
        );
    }

    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
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
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());

    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("heavy".into()),
        ..Default::default()
    };
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
    let edits = BorderConfigEdits {
        preset: OptionEdit::Set("double".into()),
        ..Default::default()
    };
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
    assert_setter_no_op_after(label, |_, _| {}, call);
}

/// [`assert_setter_no_op`] with a `prepare` step that runs
/// **before** the snapshot, for setters whose "already at this
/// value" state has to be arranged first.
///
/// The three per-node color setters need it: on a themed node they
/// write the node's `color_schema.overrides`, so "unchanged" means
/// "already overridden to this color" and a fixture node that has
/// never been recolored is not in that state. Preparing before the
/// snapshot keeps the whole-node fingerprint comparison honest —
/// arranging inside `call` would make the setup itself look like
/// the mutation.
fn assert_setter_no_op_after<P, F>(label: &str, prepare: P, call: F)
where
    P: FnOnce(&mut MindMapDocument, &str),
    F: FnOnce(&mut MindMapDocument, &str) -> bool,
{
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    prepare(&mut doc, &nid);
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
    // The three color setters get the contract pinned in both
    // worlds, because they write to two different places. Unthemed:
    // `style` is the live tier, so re-writing what is already there
    // is the no-op.
    let unthemed = |doc: &mut MindMapDocument, nid: &str| {
        doc.mindmap.nodes.get_mut(nid).unwrap().color_schema = None;
    };
    assert_setter_no_op_after("set_node_bg_color/unthemed", unthemed, |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().style.background_color.clone();
        doc.set_node_bg_color(nid, Some(&current))
    });
    assert_setter_no_op_after("set_node_border_color/unthemed", unthemed, |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().style.frame_color.clone();
        doc.set_node_border_color(nid, Some(&current))
    });
    assert_setter_no_op_after("set_node_text_color/unthemed", unthemed, |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().style.text_color.clone();
        doc.set_node_text_color(nid, Some(&current))
    });
    // Themed: the override slot is the live tier, so the node has
    // to already carry the override for the second write to be the
    // no-op. Every testament node is themed, so no setup beyond the
    // first write is needed.
    assert_setter_no_op_after(
        "set_node_bg_color/themed",
        |doc, nid| {
            doc.set_node_bg_color(nid, Some("#123456"));
        },
        |doc, nid| doc.set_node_bg_color(nid, Some("#123456")),
    );
    assert_setter_no_op_after(
        "set_node_border_color/themed",
        |doc, nid| {
            doc.set_node_border_color(nid, Some("#123456"));
        },
        |doc, nid| doc.set_node_border_color(nid, Some("#123456")),
    );
    assert_setter_no_op_after(
        "set_node_text_color/themed",
        |doc, nid| {
            doc.set_node_text_color(nid, Some("#123456"));
        },
        |doc, nid| doc.set_node_text_color(nid, Some("#123456")),
    );
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
        doc.set_node_font_size(nid, current)
    });
    assert_setter_no_op("set_section_font_size", |doc, nid| {
        let current = doc.mindmap.nodes.get(nid).unwrap().sections[0].text_runs[0].size_pt;
        doc.set_section_font_size(nid, 0, current)
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
        doc.set_section_font_size_range(nid, 0, 0, 3, size)
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

    assert!(!doc.set_node_bg_color(ghost, Some("#123456")));
    assert!(!doc.set_node_border_color(ghost, Some("#123456")));
    assert!(!doc.set_node_text_color(ghost, Some("#123456")));
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

/// One labeled node setter in a table-driven battery: the name
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
            Box::new(|doc: &mut MindMapDocument, nid: &str| doc.set_node_bg_color(nid, Some("#0b0b0b"))),
        ),
        (
            "set_node_text_color",
            Box::new(|doc: &mut MindMapDocument, nid: &str| doc.set_node_text_color(nid, Some("#0b0b0b"))),
        ),
        (
            "set_node_border_color",
            Box::new(|doc: &mut MindMapDocument, nid: &str| doc.set_node_border_color(nid, Some("#0b0b0b"))),
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
    assert!(doc.set_node_bg_color(&nid, Some("#0b0b0b")));
    assert_eq!(doc.mindmap.nodes.get(&nid).expect("node").size.width, 3.0);
    assert!(doc.set_node_text_color(&nid, Some("#0c0c0c")));
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

/// `SectionRange` structural cleanup repairs each field per its
/// own meaning (#47 part C): the **section span** clamps against
/// the shortened `sections` vec, while the **grapheme range**
/// stays as authored — it addresses text inside the surviving
/// anchor section, which the delete did not touch. When the
/// anchor section itself is gone, the whole variant demotes and
/// the grapheme range dies with it.
#[test]
fn test_delete_section_clamps_section_range_span_and_keeps_grapheme_range() {
    use crate::application::document::{GraphemeRange, SectionSpan};
    use baumhard::mindmap::model::MindSection;

    let (mut doc, nid) = super::tests_common::pinned_two_section_node();
    doc.mindmap
        .nodes
        .get_mut(&nid)
        .unwrap()
        .sections
        .push(MindSection::new_default("third".into(), Vec::new()));
    doc.selection = SelectionState::SectionRange {
        sel: SectionSel {
            node_id: nid.clone(),
            section_idx: 0,
        },
        section_span: SectionSpan::new(0, 2),
        grapheme_range: GraphemeRange::new(1, 3),
    };

    doc.delete_section(&nid, 2).expect("delete ok");
    match &doc.selection {
        SelectionState::SectionRange {
            sel,
            section_span,
            grapheme_range,
        } => {
            assert_eq!(sel.section_idx, 0, "the surviving anchor stays");
            assert_eq!(
                *section_span,
                SectionSpan::new(0, 1),
                "the span must clamp to the shortened section count"
            );
            assert_eq!(
                *grapheme_range,
                GraphemeRange::new(1, 3),
                "the grapheme range addresses the anchor section's text and must not be clamped \
                 by a section-count change"
            );
        }
        other => panic!("a clamped range with a live anchor stays a SectionRange, got {other:?}"),
    }

    // Anchor gone: the variant demotes to the closest surviving
    // section — no stale grapheme range survives onto text it was
    // never swept over.
    doc.selection = SelectionState::SectionRange {
        sel: SectionSel {
            node_id: nid.clone(),
            section_idx: 1,
        },
        section_span: SectionSpan::single(1),
        grapheme_range: GraphemeRange::new(0, 2),
    };
    doc.delete_section(&nid, 1).expect("delete ok");
    assert!(
        matches!(&doc.selection, SelectionState::Section(s) if s.section_idx == 0),
        "a dead anchor demotes to the closest surviving section, got {:?}",
        doc.selection
    );
}

/// **The load-time text floor must not lay out an unbounded number
/// of lines.** Section text arrives from an untrusted file, and this
/// measurement runs on every section at load, before a frame is
/// drawn — so a map carrying millions of newlines would build
/// millions of cosmic-text lines here, each an owned `String`, an
/// `AttrsList`, and two layout caches.
///
/// The bound loses nothing real: `TextBlockSize::height` is
/// `line_count * line_height`, and with an unbounded measuring width
/// nothing wraps, so counting newlines gives the same number without
/// a layout pass. This pins that the count stays exact past the
/// budget while the shaped prefix stops growing.
#[test]
fn test_measured_prefix_bounds_layout_but_not_the_line_count() {
    use crate::application::document::{measured_prefix, MEASURED_LINE_BUDGET};

    // Under budget: the whole string is measured, count is exact.
    let short = "alpha\nbeta\ngamma";
    let (measured, total) = measured_prefix(short, MEASURED_LINE_BUDGET);
    assert_eq!(measured, short, "short text is measured whole");
    assert_eq!(total, 3);

    // Over budget: the shaped slice is capped, the count is not.
    let many = "x\n".repeat(MEASURED_LINE_BUDGET * 4);
    let (measured, total) = measured_prefix(&many, MEASURED_LINE_BUDGET);
    assert_eq!(
        total,
        MEASURED_LINE_BUDGET * 4,
        "the line count must stay exact past the budget — it is what the height is derived from"
    );
    assert_eq!(
        measured.lines().count(),
        MEASURED_LINE_BUDGET,
        "only the budgeted prefix is handed to the shaper"
    );
    assert!(measured.len() < many.len(), "the prefix must actually be shorter");

    // Degenerate inputs stay total.
    assert_eq!(measured_prefix("", MEASURED_LINE_BUDGET), ("", 0));
    assert_eq!(measured_prefix("solo", MEASURED_LINE_BUDGET), ("solo", 1));
}

/// **The budget bounds the work; it must not bias the answer.**
///
/// With an unbounded measuring width nothing wraps, so a block's
/// width is its widest line. Measuring the first `MEASURED_LINE_BUDGET`
/// lines therefore answered a different question than the one asked:
/// a section of 512 short lines followed by one long line was sized
/// as though the long line did not exist, and the node clipped it —
/// the exact failure `grow_node_sizes_to_fit_text` exists to prevent.
///
/// It was excused on the grounds that a node past the budget is
/// clamped at `MAX_NODE_AXIS` anyway. At the default 14 pt, 513
/// lines is 8,618 pt against a 1,000,000 ceiling — 0.86% of it — so
/// the clamp does not cover for this until roughly 1,624 pt, and the
/// whole 513..~59,500-line range shipped an under-measured width.
#[test]
fn test_widest_lines_picks_by_width_not_by_position() {
    use crate::application::document::{widest_lines, MEASURED_LINE_BUDGET};

    // The shape that broke: the widest line sits past the budget.
    let long = "W".repeat(300);
    let text = format!("{}{}", "x\n".repeat(MEASURED_LINE_BUDGET), long);
    let picked = widest_lines(&text, MEASURED_LINE_BUDGET);
    assert!(
        picked.lines().any(|l| l == long),
        "the widest line must be measured even when it falls past the budget"
    );
    assert_eq!(
        picked.lines().count(),
        MEASURED_LINE_BUDGET,
        "the sample must still respect the budget"
    );

    // Selection is by width, and ties keep source order.
    let ranked = widest_lines("a\nbbbb\ncc\nddd", 2);
    assert_eq!(ranked, "bbbb\nddd", "the two widest, in source order");

    // Ranking is display width, not byte length — chosen so the two
    // disagree. "日本語" is 9 bytes but 6 columns; "abcdefgh" is 8
    // bytes and 8 columns. Byte length would pick the CJK line;
    // column width picks the ASCII one, and column width is what
    // decides how wide the node has to be.
    let by_columns = widest_lines("日本語\nabcdefgh", 1);
    assert_eq!(by_columns, "abcdefgh", "ranking is display width, not bytes");

    // And the double-width half of that: three CJK glyphs are 6
    // columns and must beat a 4-column ASCII line despite both being
    // short.
    let wide_script = widest_lines("日本語\nabcd", 1);
    assert_eq!(wide_script, "日本語", "a CJK glyph counts as two columns");

    // Under budget nothing is dropped.
    assert_eq!(widest_lines("one\ntwo", MEASURED_LINE_BUDGET), "one\ntwo");

    // Degenerate inputs stay total.
    assert_eq!(widest_lines("", MEASURED_LINE_BUDGET), "");
    assert_eq!(widest_lines("solo", 0), "");
}

/// **A file the editor writes must be a file the editor can reopen.**
///
/// The loader now rejects rather than repairs, which makes every
/// writer that can exceed the new domain a lockout candidate: the
/// user edits, saves, and their own map stops opening. The console
/// parsers are deliberately permissive (`parse_finite_pt` accepts
/// `0.001`, the `spacing` verb accepts any finite float), so the
/// clamps have to live at the document setters — the same posture
/// the reverse converter takes with `clamp_run_size_pt`.
///
/// This drives the setters with values well outside the domain and
/// then round-trips through the real save and the real strict load.
#[test]
fn test_extreme_editor_writes_still_reload() {
    use crate::application::document::{BorderConfigEdits, OptionEdit};
    use baumhard::mindmap::loader::{load_from_file, save_to_file};

    let dir = baumhard::util::test_temp::TempDir::new("editor-write-reload");

    // Every case gets its own document and its own round trip. An
    // earlier version drove several values into *one* document
    // before saving, which meant each was overwritten by the next
    // and only the last one was ever tested — the low-end font case
    // never reached the loader at all.
    let round_trip = |label: &str, edit: &dyn Fn(&mut crate::application::document::MindMapDocument)| {
        let mut doc = load_test_doc();
        edit(&mut doc);
        let path = dir.join(&format!("{label}.mindmap.json"));
        save_to_file(&path, &doc.mindmap).expect("save must succeed");
        load_from_file(&path).unwrap_or_else(|e| {
            panic!("{label}: the editor wrote a map its own loader refuses — the lockout case: {e}")
        })
    };

    let node_id = load_test_doc().mindmap.root_nodes()[0].id.clone();
    let edge_ref = |doc: &crate::application::document::MindMapDocument| {
        doc.mindmap
            .edges
            .first()
            .map(|e| crate::application::document::EdgeRef {
                from_id: e.from_id.clone(),
                to_id: e.to_id.clone(),
                edge_type: e.edge_type.clone(),
            })
            .expect("fixture has edges")
    };
    let border_of = |map: &baumhard::mindmap::model::MindMap, id: &str| {
        map.nodes[id]
            .style
            .border
            .as_ref()
            .expect("border override was authored")
            .clone()
    };
    let spacing_of = |map: &baumhard::mindmap::model::MindMap| {
        map.edges[0]
            .glyph_connection
            .as_ref()
            .map(|c| c.spacing)
            .expect("spacing override was authored")
    };

    // A decorative hairline and an absurd ceiling, from the two ends
    // the console will happily parse — each round-tripped alone.
    for (label, requested) in [("hairline", 0.001_f32), ("giant", 5000.0)] {
        let id = node_id.clone();
        let reloaded = round_trip(label, &move |doc| {
            doc.set_node_border_config(
                &id,
                BorderConfigEdits {
                    font_size_pt: OptionEdit::Set(requested),
                    visible: Some(true),
                    ..Default::default()
                },
            );
        });
        let border = border_of(&reloaded, &node_id);
        assert!(
            border.font_size_pt >= baumhard::font::fonts::MIN_FONT_SIZE_PT
                && border.font_size_pt <= baumhard::font::fonts::MAX_FONT_SIZE_PT,
            "{label}: the setter must clamp into the domain the loader accepts, got {}",
            border.font_size_pt
        );
    }

    let max_axis = baumhard::mindmap::model::MAX_NODE_AXIS as f32;

    // The magnitude bounds, which the first pass missed: the loader
    // caps both `style.border.padding` and
    // `glyph_connection.spacing` at `MAX_NODE_AXIS`, while
    // `border padding=` and `spacing` accept any finite float. Both
    // reported success and wrote a map that would not reopen.
    let id = node_id.clone();
    let reloaded = round_trip("padding", &move |doc| {
        doc.set_node_border_config(
            &id,
            BorderConfigEdits {
                padding: OptionEdit::Set(1.0e30),
                visible: Some(true),
                ..Default::default()
            },
        );
    });
    let padding = border_of(&reloaded, &node_id).padding;
    assert!(
        padding.abs() <= max_axis,
        "padding must be clamped into the loader's bound, got {padding}"
    );

    let reloaded = round_trip("spacing-huge", &|doc| {
        let r = edge_ref(doc);
        doc.set_edge_spacing(&r, 1.0e30);
    });
    let spacing = spacing_of(&reloaded);
    assert!(
        spacing.abs() <= max_axis,
        "spacing must be clamped into the loader's bound, got {spacing}"
    );

    // The text-run font size, on all three setters that write it.
    // The edge and border font channels were clamped in an earlier
    // pass and these were missed, which left `font size=5000` — the
    // plainest thing to type — writing a map that would not reopen.
    let max_run_pt = baumhard::font::fonts::MAX_FONT_SIZE_PT;

    let id = node_id.clone();
    let reloaded = round_trip("node-font-size", &move |doc| {
        doc.set_node_font_size(&id, 5000.0);
    });
    for run in reloaded.nodes[&node_id]
        .sections
        .iter()
        .flat_map(|s| s.text_runs.iter())
    {
        assert!(
            run.size_pt <= max_run_pt,
            "set_node_font_size must clamp into the loader's run domain, got {}",
            run.size_pt
        );
    }

    let id = node_id.clone();
    let reloaded = round_trip("section-font-size", &move |doc| {
        doc.set_section_font_size(&id, 0, 5000.0);
    });
    for run in reloaded.nodes[&node_id].sections[0].text_runs.iter() {
        assert!(
            run.size_pt <= max_run_pt,
            "set_section_font_size must clamp, got {}",
            run.size_pt
        );
    }

    let id = node_id.clone();
    let reloaded = round_trip("section-font-size-range", &move |doc| {
        doc.set_section_font_size_range(&id, 0, 0, 1, 5000.0);
    });
    for run in reloaded.nodes[&node_id].sections[0].text_runs.iter() {
        assert!(
            run.size_pt <= max_run_pt,
            "set_section_font_size_range must clamp, got {}",
            run.size_pt
        );
    }

    // An ordinary size still round-trips exactly — the clamp bounds
    // the extremes, it does not perturb normal edits. Fractional
    // sizes are first-class now that `size_pt` is an `f32`, so 12.7
    // is stored and reloaded as 12.7, not rounded.
    let id = node_id.clone();
    let reloaded = round_trip("node-font-ordinary", &move |doc| {
        doc.set_node_font_size(&id, 12.7);
    });
    assert!(
        reloaded.nodes[&node_id]
            .sections
            .iter()
            .flat_map(|s| s.text_runs.iter())
            .all(|r| r.size_pt == 12.7),
        "an ordinary fractional size must survive the round trip unchanged"
    );

    // The eight border glyphs. The loader *rejects* these rather
    // than clamping, so the writer refuses the edit — and the map
    // that reaches disk is the unmodified one, which reloads.
    let id = node_id.clone();
    let over = "=".repeat(baumhard::mindmap::model::validate::MAX_BORDER_GLYPH_CLUSTERS + 1);
    let reloaded = round_trip("border-glyph", &move |doc| {
        let outcome = doc.set_node_border_config(
            &id,
            BorderConfigEdits {
                side_top: OptionEdit::Set(over.clone()),
                visible: Some(true),
                ..Default::default()
            },
        );
        assert!(
            !outcome.rejected.is_empty(),
            "an over-long border glyph must be refused, not written"
        );
        assert!(
            !outcome.changed,
            "the refusal must be atomic — nothing on the node changed"
        );
    });
    if let Some(border) = reloaded.nodes[&node_id].style.border.as_ref() {
        if let Some(glyphs) = border.glyphs.as_ref() {
            assert!(
                baumhard::util::grapheme_chad::count_grapheme_clusters(&glyphs.top)
                    <= baumhard::mindmap::model::validate::MAX_BORDER_GLYPH_CLUSTERS,
                "a refused glyph must not have reached the file"
            );
        }
    }

    // ...on **every** surface that writes a `GlyphBorderConfig`, not
    // just the per-node one. The loader screens a section's
    // `frame_border` and all three canvas slots with the same
    // `border_config_violations`, so each of these authored an
    // unopenable map for as long as only `set_node_border_config`
    // screened. That the per-node case above passed is exactly what
    // made the gap invisible.
    let over = "=".repeat(baumhard::mindmap::model::validate::MAX_BORDER_GLYPH_CLUSTERS + 1);
    let glyph_edits = |g: &str| BorderConfigEdits {
        side_top: OptionEdit::Set(g.to_string()),
        ..Default::default()
    };

    let id = node_id.clone();
    let g = over.clone();
    let reloaded = round_trip("section-frame-glyph", &move |doc| {
        let outcome = doc.set_section_frame_border_config(&id, 0, glyph_edits(&g));
        assert!(
            !outcome.rejected.is_empty() && !outcome.changed,
            "`section frame top=` must refuse an over-long glyph atomically"
        );
    });
    assert!(
        reloaded.nodes[&node_id].sections[0]
            .frame_border
            .as_ref()
            .and_then(|b| b.glyphs.as_ref())
            .is_none_or(|g| baumhard::util::grapheme_chad::count_grapheme_clusters(&g.top)
                <= baumhard::mindmap::model::validate::MAX_BORDER_GLYPH_CLUSTERS),
        "a refused section-frame glyph must not have reached the file"
    );

    // The three canvas slots. These matter more than the per-element
    // ones, not less: a canvas default is the fallback for every
    // node or section that does not override it.
    let canvas_cases: [(&str, Option<bool>); 3] = [
        ("canvas-default-glyph", None),
        ("canvas-section-frame-glyph", Some(false)),
        ("canvas-section-frame-focused-glyph", Some(true)),
    ];
    for (label, focused) in canvas_cases {
        let g = over.clone();
        let reloaded = round_trip(label, &move |doc| {
            let outcome = match focused {
                None => doc.set_canvas_default_border(glyph_edits(&g)),
                Some(f) => doc.set_canvas_default_section_frame_border_config(f, glyph_edits(&g)),
            };
            assert!(
                !outcome.rejected.is_empty() && !outcome.changed,
                "{label}: an over-long canvas border glyph must be refused atomically"
            );
        });
        let slot = match focused {
            None => reloaded.canvas.default_border.as_ref(),
            Some(false) => reloaded.canvas.default_section_frame_border.as_ref(),
            Some(true) => reloaded.canvas.default_focused_section_frame_border.as_ref(),
        };
        assert!(
            slot.and_then(|b| b.glyphs.as_ref())
                .is_none_or(|g| baumhard::util::grapheme_chad::count_grapheme_clusters(&g.top)
                    <= baumhard::mindmap::model::validate::MAX_BORDER_GLYPH_CLUSTERS),
            "{label}: a refused glyph must not have reached the file"
        );
    }

    // A negative gap is a legitimate tightening, so it must survive
    // the round trip unchanged rather than be clamped away.
    let reloaded = round_trip("spacing-tight", &|doc| {
        let r = edge_ref(doc);
        doc.set_edge_spacing(&r, -2.0);
    });
    assert_eq!(
        spacing_of(&reloaded),
        -2.0,
        "a negative gap is authorable and must survive unclamped"
    );
}

/// **A refused glyph must not commit as a success.**
///
/// The four `… preview` verbs stage edits and commit them through the
/// same four setters the direct verbs use, so the *write* has been
/// refused since the screen moved to the chokepoint. The **report**
/// was not: `merge_outcome` folded `changed`, `preset_auto_promoted`
/// and `requested_preset` across the committed targets and dropped
/// `rejected` on the floor, so `commit_border_preview` handed back an
/// outcome that said nothing was refused. The verb then printed
/// success — or, worse, "no change", which is true of the model and
/// says nothing about why.
///
/// The user-visible consequence is the one this whole branch is about,
/// one step removed: the editor tells you your border was applied, the
/// map on disk does not have it, and nothing said so.
#[test]
fn test_a_preview_commit_reports_a_refused_glyph_instead_of_success() {
    use crate::application::document::{BorderConfigEdits, BorderPreviewTarget, OptionEdit};

    let over = "=".repeat(baumhard::mindmap::model::validate::MAX_BORDER_GLYPH_CLUSTERS + 1);
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());

    let edits = BorderConfigEdits {
        side_top: OptionEdit::Set(over.clone()),
        ..Default::default()
    };
    let staged = doc.set_border_preview(BorderPreviewTarget::Nodes(vec![nid.clone()]), edits);
    assert!(
        !staged.rejected.is_empty(),
        "the preview itself must refuse — staging a glyph the commit will decline invites          the user to look at a border they cannot keep"
    );
    assert!(
        doc.border_preview.is_none(),
        "a refused preview must not be recorded, or the scene renders the over-ceiling glyph"
    );

    // And the commit path, reached by staging a legal preview and then
    // committing an illegal edit through the same fold.
    let mut doc = load_test_doc();
    doc.selection = SelectionState::Single(nid.clone());
    let outcome = doc.set_node_border_config(
        &nid,
        BorderConfigEdits {
            side_top: OptionEdit::Set(over),
            ..Default::default()
        },
    );
    let mut merged = crate::application::document::BorderEditOutcome::default();
    crate::application::document::nodes::merge_outcome(&mut merged, outcome);
    assert!(
        !merged.rejected.is_empty(),
        "merge_outcome must carry the refusal — dropping it is what let a refused commit          report success"
    );
}

/// **The backstop screen, observed directly.**
///
/// `apply_glyph_border_edits_to_slot` is where the eight glyph fields
/// are screened, and the four public setters each refuse earlier so a
/// declined edit does not also discard a live preview. That layering
/// left the backstop untested: deleting the screen at the chokepoint
/// left every test in the workspace green, because the early refusals
/// caught the same fixtures first. A review round found it by deleting
/// the screen and watching nothing happen.
///
/// The reason the backstop matters is precisely that it is *not* the
/// early refusals: a fifth writer added later reaches the applier
/// without going through any of them, and the applier is what has to
/// stop it. So this drives the applier directly, past the setters, and
/// is the only test that fails when the chokepoint screen is removed
/// and the early refusals are left in place.
#[test]
fn test_the_chokepoint_screen_refuses_without_help_from_any_setter() {
    use crate::application::document::nodes::apply_glyph_border_edits_to_slot;
    use crate::application::document::{BorderConfigEdits, BorderEditOutcome, OptionEdit};

    let over = "=".repeat(baumhard::mindmap::model::validate::MAX_BORDER_GLYPH_CLUSTERS + 1);
    let mut slot: Option<baumhard::mindmap::model::GlyphBorderConfig> = None;
    let mut outcome = BorderEditOutcome::default();
    let changed = apply_glyph_border_edits_to_slot(
        &mut slot,
        &BorderConfigEdits {
            side_top: OptionEdit::Set(over),
            ..Default::default()
        },
        &mut outcome,
    );

    assert!(
        !outcome.rejected.is_empty(),
        "the chokepoint must refuse an over-ceiling glyph on its own, with no setter above it"
    );
    assert!(
        !changed,
        "a refusal must report no change, so no caller pushes an undo entry"
    );
    assert!(
        slot.is_none(),
        "the refusal must be atomic — the slot must not even be allocated"
    );

    // The negative control: an ordinary glyph still writes, so the
    // assertions above cannot be passing because the applier refuses
    // everything.
    let mut slot = None;
    let mut outcome = BorderEditOutcome::default();
    let changed = apply_glyph_border_edits_to_slot(
        &mut slot,
        &BorderConfigEdits {
            side_top: OptionEdit::Set("◆·".to_string()),
            ..Default::default()
        },
        &mut outcome,
    );
    assert!(
        outcome.rejected.is_empty() && changed,
        "an ordinary glyph must still be written"
    );
    assert_eq!(
        slot.and_then(|c| c.glyphs).map(|g| g.top).as_deref(),
        Some("◆·"),
        "and must land in the slot"
    );
}

/// **The writer-side invariant, checked mechanically.**
///
/// `format/validation.md` states the property this pins: a value the
/// editor can write must be a value the loader accepts. It is not
/// enforced by the type system — it holds because each setter
/// screens or clamps — and it has now been broken four separate
/// times, each time by adding a bound at the loader and forgetting
/// the writer. `font size=`, `border padding=`, `spacing`, node
/// position and the eight border glyphs were each found that way,
/// by a review round rather than by a test.
///
/// The tests that were supposed to catch it enumerate *setters*, so
/// they only ever cover the ones somebody remembered. This
/// enumerates the **bounds** instead.
///
/// # The bound set is derived, not listed
///
/// An earlier version of this test read one file —
/// `model/validate.rs` — for `pub const`. That was wrong in four
/// ways at once, and a review round demonstrated each against a
/// modified tree:
///
/// - **Four of its own rows named constants declared elsewhere**
///   (`fonts.rs`, `model/node.rs`, `loader.rs`), so those rows were
///   inert prose. The two bounds behind two of the historical
///   lockouts live in the unscanned files — this test would not have
///   caught either of the bugs it cites as its motivation.
/// - **`pub(crate) const` and `pub static` were invisible**, since
///   only the literal prefix `pub const ` was matched.
/// - **Membership was a substring test**, so a new constant whose
///   name is a substring of an existing row was absorbed silently.
/// - **The writer column was never read**, so a row naming a deleted
///   function stayed green — and the row for the border glyphs was
///   false at the moment it was written.
///
/// So the set is derived instead. A loader-enforced bound is a
/// constant the rejection path *consults*: every `const` / `static`
/// declared anywhere in baumhard, intersected with the identifiers
/// `model/validate.rs` and `mindmap/loader.rs` reference outside
/// their own test modules. That reaches bounds in five files today
/// and follows one that moves to a sixth tomorrow, with nothing to
/// update here.
///
/// # What it still cannot see
///
/// Stated so this reads as a decision rather than a claim of
/// completeness — the *previous* version of this test was described
/// as closing the class, and a live fourth instance was sitting in
/// the same delta while it passed:
///
/// - **An inlined literal.** A bound written as `1.0e9` at the
///   comparison site is not a constant and is not derivable here.
/// - **A rejection path outside those two files.** The scan reads
///   `validate.rs` and `loader.rs`; a module that grows its own
///   rejection is invisible until it is added to `REJECTION_PATHS`.
/// - **Whether the named writer is the *only* writer.** The row for
///   the border glyphs was true of one writer and false of three
///   others when it was written. What is checked is that each named
///   symbol exists; that it is exhaustive is a claim the prose
///   makes and `test_extreme_editor_writes_still_reload` exercises
///   per surface.
///
/// This is a registry, not a behavior test — the behavior is pinned
/// by `test_extreme_editor_writes_still_reload`, which drives the
/// writers. What this catches is the *omission*: a bound added with
/// no writer at all.
#[test]
fn test_every_loader_bound_names_its_writer_side_guard() {
    // (constant, the symbols that keep the editor inside it, prose)
    //
    // The symbol list is read: each name must exist as a function in
    // the workspace, so a row naming something deleted or renamed
    // fails rather than sitting green. An empty list means "no
    // editor writer" and the prose must say why.
    let registry: &[(&str, &[&str], &str)] = &[
        (
            "MIN_FONT_SIZE_PT",
            &["clamp_font_metric", "clamp_run_size_pt", "resolve_font_triple"],
            "the same three clamps as MAX_FONT_SIZE_PT — it is the lower half of one window",
        ),
        (
            "MAX_FONT_SIZE_PT",
            &["clamp_font_metric", "clamp_run_size_pt", "resolve_font_triple"],
            "GlyphArea::set_*_clamped + apply_operation; clamp_run_size_pt for the three \
             text-run setters; resolve_font_triple for the edge channels; clamp_font_metric \
             for the border font size",
        ),
        (
            "MAX_CANVAS_COORD",
            &[
                "validate_node_position",
                "set_position_clamped",
                "offset_position_clamped",
            ],
            "validate_node_position rejects an *authored* position — one a caller \
             supplied, at the loader and at set_node_aabb. Every *computed* one goes \
             through MindNode::set_position_clamped or its offset sibling, and \
             test_every_node_position_write_goes_through_the_clamp fails the build for a \
             writer that does not. This row named two computed writers and was false when \
             it was written: the drag, two nudge handlers, the lerp and the \
             custom-mutation sync-back were all unclamped, and the last is reachable from \
             a map's own trigger bindings",
        ),
        (
            "MAX_NODE_AXIS",
            &["clamp_node_size_to_ceiling", "clamp_to_bound"],
            "clamp_node_size_to_ceiling for node size; clamp_to_bound for border padding \
             and edge spacing",
        ),
        (
            "MAX_BORDER_GLYPH_CLUSTERS",
            &["apply_glyph_border_edits_to_slot", "border_glyph_edit_violations"],
            "screened at apply_glyph_border_edits_to_slot — the chokepoint every border \
             writer funnels through, so all four surfaces (node style.border, section \
             frame_border, canvas default_border, canvas section-frame) are covered by \
             one screen. An earlier row named the per-node setter instead, which is \
             precisely how the other three stayed unguarded",
        ),
        (
            "MAX_BORDER_GLYPH_BYTES",
            &["apply_glyph_border_edits_to_slot", "border_glyph_edit_violations"],
            "same screen as MAX_BORDER_GLYPH_CLUSTERS — border_glyph_violations checks \
             both ceilings in one call",
        ),
        (
            "MAX_CONNECTION_GLYPH_GRAPHEMES",
            &[],
            "no editor writer — the connection body/cap glyphs are authored in the file \
             only; there is no console verb or setter that writes them",
        ),
        (
            "MAX_CONNECTION_GLYPH_BYTES",
            &[],
            "no editor writer — same field as MAX_CONNECTION_GLYPH_GRAPHEMES, second ceiling",
        ),
        (
            "MAX_ANIMATION_MS",
            &[],
            "no editor writer — animation timings are authored in the file only",
        ),
        (
            "MAX_UNKNOWN_KEYS",
            &[],
            "no editor writer — the count of keys this build has no field for is a property \
             of the document it read, not a value any setter writes. `None` on native: it \
             was a resource ceiling standing in for a cost defect, and once the capture and \
             the write-back were both made linear there was nothing left for it to bound. \
             `Some` on wasm32, where a 32-bit address space is physics",
        ),
        (
            "MAX_SECTIONS_PER_NODE",
            &["add_section"],
            "add_section refuses past the cap",
        ),
        (
            "MAX_MAP_BYTES",
            &[],
            "no editor writer — a saved map's size is a consequence, not a set field",
        ),
        (
            "INVERTED_SIZE_WINDOW",
            &["clamp_node_size_to_ceiling"],
            "not a magnitude bound but a consistency one: the loader rejects a min/max \
             window whose ends are crossed. No setter writes both ends of a window in one \
             call, and clamp_node_size_to_ceiling keeps the size it does write inside \
             MAX_NODE_AXIS",
        ),
        (
            "INVERTED_ZOOM_WINDOW",
            &[],
            "no editor writer — the zoom-visibility window is authored in the file only; \
             no console verb or setter writes either end",
        ),
    ];

    // The files whose contents *are* the rejection. A constant one of
    // these consults is a bound a hostile or mistaken value is
    // measured against, which is exactly the set that needs a
    // writer-side guard.
    const REJECTION_PATHS: &[&str] = &[
        "lib/baumhard/src/mindmap/model/validate.rs",
        "lib/baumhard/src/mindmap/loader.rs",
        // `MAX_UNKNOWN_KEYS` refuses a load and was invisible to this
        // registry for two commits, because `loader.rs` calls
        // `unknown_key_count_violation` and never names the constant.
        // That is the "a rejection path outside those files" limitation
        // in the docstring, met in practice within a day of it being
        // written down — which is the argument for reading this list as
        // a known-incomplete enumeration rather than a definition.
        "lib/baumhard/src/mindmap/unknown_keys.rs",
    ];

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    // Every `const` / `static` declared anywhere in baumhard, at any
    // visibility. `pub(crate) const` and `pub static` were both
    // invisible to the previous scan.
    let mut declared: std::collections::BTreeMap<String, String> = Default::default();
    let mut stack = vec![root.join("lib/baumhard/src")];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}"));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("read a baumhard source file");
            for line in src.lines() {
                let line = line.trim_start();
                // Step past any visibility qualifier, then require
                // `const` or `static` and an ALL-CAPS name.
                let rest = match line.strip_prefix("pub") {
                    Some(after) => after
                        .trim_start()
                        .strip_prefix('(')
                        .map_or(after, |p| p.split_once(')').map_or(p, |(_, tail)| tail)),
                    None => line,
                }
                .trim_start();
                let rest = match rest
                    .strip_prefix("const ")
                    .or_else(|| rest.strip_prefix("static "))
                {
                    Some(r) => r,
                    None => continue,
                };
                let name: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                    .collect();
                // Require the declaration to actually be `NAME:` —
                // otherwise `const fn foo` and friends leak in.
                if name.len() > 2 && rest.trim_start()[name.len()..].trim_start().starts_with(':') {
                    declared.entry(name).or_insert_with(|| path.display().to_string());
                }
            }
        }
    }
    assert!(
        declared.len() > 50,
        "the declaration scan found only {} constants — the parse, not the crate, is what \
         broke",
        declared.len()
    );

    // Comments and string literals are not references. `validate.rs`
    // *documents* `MAX_BORDER_SIDE_BYTES` and `MAX_PATH_SAMPLES` in
    // prose while consulting neither, so a raw text scan reports two
    // bounds that have no rejection behind them — and the registry
    // grows two rows describing a check that does not exist.
    //
    // Over-stripping is the dangerous direction (a swallowed
    // reference silently shrinks the bound set), so the two controls
    // below hold this to a known-code and a known-prose mention.
    fn code_only(src: &str) -> String {
        let b: Vec<char> = src.chars().collect();
        let mut out = String::with_capacity(src.len());
        let mut i = 0;
        while i < b.len() {
            // Line comment.
            if b[i] == '/' && b.get(i + 1) == Some(&'/') {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            // Block comment, nested per Rust's rules.
            if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                let mut depth = 1usize;
                i += 2;
                while i < b.len() && depth > 0 {
                    if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                out.push(' ');
                continue;
            }
            // Char literal — checked before the raw-string and string
            // arms so a `'"'` cannot open a string that eats the code
            // after it. A lifetime (`'a`) has no closing quote and
            // falls through to the copy arm.
            if b[i] == '\'' && (b.get(i + 2) == Some(&'\'') || b.get(i + 1) == Some(&'\\')) {
                i += 1;
                if b.get(i) == Some(&'\\') {
                    i += 1;
                }
                while i < b.len() && b[i] != '\'' {
                    i += 1;
                }
                i += 1;
                out.push(' ');
                continue;
            }
            // Raw string, any hash count.
            if b[i] == 'r' && matches!(b.get(i + 1), Some('"') | Some('#')) {
                let mut j = i + 1;
                let mut hashes = 0usize;
                while b.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if b.get(j) == Some(&'"') {
                    j += 1;
                    while j < b.len() {
                        if b[j] == '"' {
                            let mut k = j + 1;
                            let mut n = 0;
                            while n < hashes && b.get(k) == Some(&'#') {
                                k += 1;
                                n += 1;
                            }
                            if n == hashes {
                                j = k;
                                break;
                            }
                        }
                        j += 1;
                    }
                    out.push(' ');
                    i = j;
                    continue;
                }
            }
            // Ordinary string.
            if b[i] == '"' {
                i += 1;
                while i < b.len() {
                    if b[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if b[i] == '"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                out.push(' ');
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        out
    }

    // Which of them the rejection path consults.
    let mut bounds: Vec<&String> = Vec::new();
    let mut rejection_src = String::new();
    for rel in REJECTION_PATHS {
        let src = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("read the rejection path {rel}: {e}"));
        // Everything above the file's own test module — a constant a
        // test mentions is not thereby a loader bound.
        rejection_src.push_str(&code_only(src.split("#[cfg(test)]").next().unwrap_or(&src)));
        rejection_src.push('\n');
    }
    // The stripper's two controls, against this exact source: a
    // constant used in code survives, a constant named only in a doc
    // comment does not. Both were verified by hand against
    // `validate.rs` — `INVERTED_ZOOM_WINDOW` is passed to
    // `ordered_pair`, `MAX_BORDER_SIDE_BYTES` appears only inside
    // `MAX_BORDER_GLYPH_BYTES`'s doc comment.
    assert!(
        rejection_src.contains("INVERTED_ZOOM_WINDOW"),
        "the comment stripper ate a real code reference — every bound below it would go \
         unlisted and this test would pass by seeing nothing"
    );
    assert!(
        !rejection_src.contains("MAX_BORDER_SIDE_BYTES"),
        "the comment stripper left a doc-comment mention in place, so prose about a bound \
         reads as a rejection consulting it"
    );
    for name in declared.keys() {
        // Word-boundary match: `MAX_BORDER_GLYPH_BYTES` must not be
        // found inside a longer identifier that merely contains it.
        if rejection_src.match_indices(name.as_str()).any(|(at, _)| {
            let before = rejection_src[..at].chars().next_back();
            let after = rejection_src[at + name.len()..].chars().next();
            let continues = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            !continues(before) && !continues(after)
        }) {
            bounds.push(name);
        }
    }
    assert!(
        bounds.len() >= 13,
        "the rejection path consults only {} of baumhard's constants — that is fewer than \
         the bounds this registry already knows about, so the scan is what broke:\n  {:?}",
        bounds.len(),
        bounds
    );

    // Every derived bound needs a row, matched by **name**, not by
    // substring — a shorter new constant must not be absorbed by an
    // existing row that happens to contain its name.
    for name in &bounds {
        let covered = registry.iter().any(|(constant, _, _)| constant == &name.as_str());
        assert!(
            covered,
            "`{name}` (declared in {}) is consulted by the loader's rejection path and has \
             no row in this registry.\n\
             Add one naming the writer that keeps the editor inside it — or, if no writer \
             can reach the field, say so explicitly and leave the symbol list empty. Four \
             lockout bugs on this branch were a bound added without a writer; this is the \
             check that makes that loud.",
            declared[*name]
        );
    }

    // Every row must name a bound that still exists, so a row does
    // not outlive the constant it describes.
    for (constant, _, _) in registry {
        assert!(
            bounds.iter().any(|b| b.as_str() == *constant),
            "the registry has a row for `{constant}`, which the loader's rejection path no \
             longer consults. Delete the row, or fix the rejection that stopped reading it."
        );
    }

    // And every named writer must still exist. This is the half that
    // was missing: the border-glyph row named a real function that
    // guarded one of four writers, and no amount of re-reading the
    // constant column could have shown that. Reading the symbol
    // column at least catches the row that names nothing at all.
    let mut workspace_src = String::new();
    let mut stack = vec![
        root.join("src"),
        root.join("lib/baumhard/src"),
        root.join("crates"),
    ];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                workspace_src.push_str(&std::fs::read_to_string(&path).expect("read a source"));
                workspace_src.push('\n');
            }
        }
    }
    for (constant, guards, prose) in registry {
        for guard in *guards {
            // `fn {guard}(` rather than `fn {guard}`: the bare form is
            // satisfied by this registry's own prose, by a doc comment
            // naming the function, and by a `#[test] fn` — all three
            // were demonstrated against it. The paren requires a
            // declaration or a call. What it still cannot tell is
            // *shipped* code from a test module; `util::source_scan`
            // answers that, and is `#[cfg(test)]` inside baumhard, so
            // it is not reachable from here.
            let declared = workspace_src.contains(&format!("fn {guard}("));
            assert!(
                declared,
                "the row for `{constant}` names `{guard}` as its writer-side guard, and no \
                 `fn {guard}(` exists in the workspace. A renamed or deleted guard must fail \
                 here rather than leave the row quietly describing nothing."
            );
        }
        assert!(
            !guards.is_empty() || prose.starts_with("no editor writer"),
            "`{constant}` names no guard, so its prose must begin \"no editor writer\" and \
             say why the field is out of every writer's reach"
        );
    }
}

/// **The magnitude half of the position guard, pinned.**
///
/// `validate_node_position` gained a `MAX_CANVAS_COORD` rejection
/// after a review found `set_node_aabb` accepting `x: 1e30` and
/// saving a map that would not reopen. Nothing exercised it — the
/// guard shipped on the strength of the argument for it.
#[test]
fn test_set_node_aabb_rejects_out_of_bound_positions() {
    use baumhard::mindmap::model::validate::MAX_CANVAS_COORD;
    use baumhard::mindmap::model::{Position, Size};

    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let size = Size {
        width: 100.0,
        height: 50.0,
    };

    // Inside the bound: accepted.
    let ok = doc.set_node_aabb(
        &id,
        Position {
            x: MAX_CANVAS_COORD,
            y: -MAX_CANVAS_COORD,
        },
        size,
    );
    assert!(ok.is_ok(), "a position at the bound must be accepted: {ok:?}");

    // Past it: refused, and the node is untouched.
    let before = doc.mindmap.nodes[&id].position;
    let err = doc.set_node_aabb(&id, Position { x: 1.0e30, y: 0.0 }, size);
    assert!(err.is_err(), "a position past the bound must be refused");
    assert_eq!(
        doc.mindmap.nodes[&id].position, before,
        "a refused position must leave the node where it was"
    );

    // Non-finite is still refused, as it was before the magnitude
    // half existed.
    assert!(doc
        .set_node_aabb(&id, Position { x: f64::NAN, y: 0.0 }, size)
        .is_err());
}

/// **The blank-line case in `widest_lines`' separator.**
///
/// The separator was driven by `out.is_empty()`, so a selected blank
/// line left `out` empty and suppressed the newline before the next
/// one — splicing two selected lines into one and losing a line from
/// a sample whose contract is a fixed count of them. Fixed by
/// counting emissions instead; this is the case that distinguishes
/// the two.
#[test]
fn test_widest_lines_keeps_blank_lines_separate() {
    use crate::application::document::widest_lines;

    // A leading blank line is selected (everything is, at this
    // budget), and must not swallow the line after it.
    assert_eq!(widest_lines("\nabc\nde", 3), "\nabc\nde");

    // Blank first *and* the widest line after it, with a budget that
    // forces a choice: the blank still cannot merge with its
    // neighbor.
    let picked = widest_lines("\nlonger\nxy", 2);
    assert_eq!(
        picked.lines().count(),
        2,
        "two lines selected must stay two lines: {picked:?}"
    );
    assert!(
        picked.contains("longer"),
        "the widest line must be there: {picked:?}"
    );
}

/// **`reset` on a themed node has to give the node back to its
/// palette, not pin it to a literal.**
///
/// `ColorValue::Reset` is documented as *clear any local override*.
/// While the three node setters wrote `style` — the tier the
/// palette shadows — resolving it to `#141414` / `#ffffff` was
/// inert, and no test ever noticed. Writing the override tier makes
/// the literal win: the node goes flat dark gray, permanently
/// excepted from its theme, with no verb left that could lift the
/// exception. All three channels, because the resolution lived in
/// one helper and was wrong in all three.
#[test]
fn test_reset_on_a_themed_node_returns_it_to_its_palette() {
    use super::tests_common::theme_node_with_probe_palette;
    use crate::application::console::traits::{
        ColorValue, HasBgColor, HasBorderColor, HasTextColor, Outcome, TargetView,
    };
    use baumhard::mindmap::model::ColorGroup;

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let group = theme_node_with_probe_palette(
        &mut doc,
        &nid,
        "reset-probe",
        ColorGroup {
            background: "#a9decb".into(),
            frame: "#30b082".into(),
            text: "#0f0f0f".into(),
            title: String::new(),
        },
    );

    // Paint all three channels by hand first, so the reset has
    // something to clear and the assertion is not vacuous.
    {
        let mut view = TargetView::Node {
            doc: &mut doc,
            id: nid.clone(),
        };
        assert_eq!(
            view.set_bg_color(ColorValue::Hex("#111111".into())),
            Outcome::Applied
        );
        assert_eq!(
            view.set_border_color(ColorValue::Hex("#222222".into())),
            Outcome::Applied
        );
        assert_eq!(
            view.set_text_color(ColorValue::Hex("#333333".into())),
            Outcome::Applied
        );
    }
    {
        let overrides = &doc.mindmap.nodes[&nid]
            .color_schema
            .as_ref()
            .expect("themed")
            .overrides;
        assert_eq!(overrides.background.as_deref(), Some("#111111"));
        assert_eq!(overrides.frame.as_deref(), Some("#222222"));
        assert_eq!(overrides.text.as_deref(), Some("#333333"));
    }

    {
        let mut view = TargetView::Node {
            doc: &mut doc,
            id: nid.clone(),
        };
        assert_eq!(view.set_bg_color(ColorValue::Reset), Outcome::Applied);
        assert_eq!(view.set_border_color(ColorValue::Reset), Outcome::Applied);
        assert_eq!(view.set_text_color(ColorValue::Reset), Outcome::Applied);
    }

    let node = &doc.mindmap.nodes[&nid];
    let overrides = &node.color_schema.as_ref().expect("still themed").overrides;
    assert!(
        overrides.is_empty(),
        "reset must leave no override behind, got {overrides:?}"
    );
    assert_eq!(doc.mindmap.node_background_color(node), group.background);
    assert_eq!(doc.mindmap.node_frame_color(node), group.frame);
    assert_eq!(doc.mindmap.node_text_color(node), group.text);

    // The node is back *on* the palette, not merely showing the
    // same pixels: a palette edit reaches it again. That is the
    // property a literal destroys.
    doc.mindmap
        .palettes
        .get_mut("reset-probe")
        .expect("palette inserted above")
        .groups[0]
        .background = "#00ff00".into();
    let node = &doc.mindmap.nodes[&nid];
    assert_eq!(doc.mindmap.node_background_color(node), "#00ff00");
}

/// The unthemed half of the same gesture. There is no tier below
/// `style` to fall back to, so "the natural default" has to be
/// named — and the only defensible name is the color every node
/// this application creates already carries
/// ([`default_orphan_node`](super::defaults::default_orphan_node)).
#[test]
fn test_reset_on_an_unthemed_node_writes_the_natural_default() {
    use super::defaults::{DEFAULT_NODE_BACKGROUND_COLOR, DEFAULT_NODE_FRAME_COLOR, DEFAULT_NODE_TEXT_COLOR};
    use crate::application::console::traits::{
        ColorValue, HasBgColor, HasBorderColor, HasTextColor, Outcome, TargetView,
    };

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.color_schema = None;
        node.style.background_color = "#111111".into();
        node.style.frame_color = "#222222".into();
        node.style.text_color = "#333333".into();
    }
    {
        let mut view = TargetView::Node {
            doc: &mut doc,
            id: nid.clone(),
        };
        assert_eq!(view.set_bg_color(ColorValue::Reset), Outcome::Applied);
        assert_eq!(view.set_border_color(ColorValue::Reset), Outcome::Applied);
        assert_eq!(view.set_text_color(ColorValue::Reset), Outcome::Applied);
    }
    let node = &doc.mindmap.nodes[&nid];
    assert_eq!(node.style.background_color, DEFAULT_NODE_BACKGROUND_COLOR);
    assert_eq!(node.style.frame_color, DEFAULT_NODE_FRAME_COLOR);
    assert_eq!(node.style.text_color, DEFAULT_NODE_TEXT_COLOR);
    assert!(
        node.color_schema.is_none(),
        "a reset must not invent a schema for an unthemed node"
    );
}

/// A `reset` of the text channel un-bakes the runs that were
/// following the old default instead of re-baking them onto the
/// new one. The empty string is the run tier's own "follow the
/// node", so the graphemes rejoin the cascade; rewriting them to
/// the palette's hex would render identically today and opt them
/// out of the next retheme, which is the trap `DEFAULT_RUN_COLOR`
/// exists to avoid.
#[test]
fn test_reset_text_color_unbakes_the_runs_that_followed_the_default() {
    use super::defaults::default_text_run;
    use super::tests_common::theme_node_with_probe_palette;
    use baumhard::mindmap::model::ColorGroup;

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    theme_node_with_probe_palette(
        &mut doc,
        &nid,
        "unbake-probe",
        ColorGroup {
            background: "#101010".into(),
            frame: "#202020".into(),
            text: "#303030".into(),
            title: String::new(),
        },
    );
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections[0].text = "abcdef".into();
        node.sections[0].text_runs = vec![
            TextRun {
                start: 0,
                end: 3,
                color: "#909090".into(),
                ..default_text_run(3)
            },
            TextRun {
                start: 3,
                end: 6,
                color: "#abcdef".into(),
                ..default_text_run(6)
            },
        ];
    }
    // Pin the node's text to #909090 — run 0 is now a baked copy
    // of the effective default, run 1 is a hand-picked override.
    assert!(doc.set_node_text_color(&nid, Some("#909090")));
    assert!(doc.set_node_text_color(&nid, None));

    let node = &doc.mindmap.nodes[&nid];
    assert_eq!(
        doc.mindmap.node_text_color(node),
        "#303030",
        "the node is back on the palette's text"
    );
    assert_eq!(
        node.sections[0].text_runs[0].color, "",
        "the baked run must rejoin the cascade, not be re-pinned to the palette hex"
    );
    assert_eq!(
        node.sections[0].text_runs[1].color, "#abcdef",
        "a hand-picked run is not the node's business"
    );
}

/// …and the un-bake alone is enough to be a change, which is why
/// a `reset` of the text channel on an unthemed node **already**
/// sitting at the authoring default still reports `true` and still
/// pushes an undo entry.
///
/// Nothing on screen moves: the run rendered `#ffffff` before and
/// renders the node's `#ffffff` after. The model moves, though —
/// those graphemes were opted out of the cascade and are now back
/// in it, so the next `color text=` carries them along instead of
/// stranding them, and undo has to be able to put the bake back.
/// "Changed nothing on screen" is not the test the setters answer;
/// "changed nothing in the model" is. The second call finds
/// nothing left to un-bake and is the no-op, so the gesture
/// converges instead of growing the stack.
#[test]
fn test_reset_text_color_at_the_default_unbakes_once_and_then_settles() {
    use super::defaults::{default_text_run, DEFAULT_NODE_TEXT_COLOR};

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.color_schema = None;
        node.style.text_color = DEFAULT_NODE_TEXT_COLOR.into();
        node.sections[0].text = "abcdef".into();
        node.sections[0].text_runs = vec![
            TextRun {
                start: 0,
                end: 3,
                color: DEFAULT_NODE_TEXT_COLOR.into(),
                ..default_text_run(3)
            },
            TextRun {
                start: 3,
                end: 6,
                color: "#abcdef".into(),
                ..default_text_run(6)
            },
        ];
    }
    doc.undo_stack.clear();

    assert!(
        doc.set_node_text_color(&nid, None),
        "the baked run still has to be un-baked, so this is not a no-op"
    );
    assert_eq!(
        doc.undo_stack.len(),
        1,
        "and undo must be able to put the bake back"
    );
    {
        let node = &doc.mindmap.nodes[&nid];
        assert_eq!(node.style.text_color, DEFAULT_NODE_TEXT_COLOR);
        assert_eq!(node.sections[0].text_runs[0].color, "");
        assert_eq!(node.sections[0].text_runs[1].color, "#abcdef");
        assert_eq!(
            doc.mindmap.node_text_color(node),
            DEFAULT_NODE_TEXT_COLOR,
            "the graphemes render exactly as they did before"
        );
    }

    assert!(
        !doc.set_node_text_color(&nid, None),
        "with nothing left to un-bake the gesture converges"
    );
    assert_eq!(doc.undo_stack.len(), 1, "and pushes no second entry");

    assert!(doc.undo());
    assert_eq!(
        doc.mindmap.nodes[&nid].sections[0].text_runs[0].color, DEFAULT_NODE_TEXT_COLOR,
        "undo restores the bake it removed"
    );
}

/// **An empty color on `frame` or `text` is not a color, and the
/// setter must not pretend it wrote one.**
///
/// Both readers run an override through `non_empty` exactly as they
/// run the palette group through it, so `overrides.frame = Some("")`
/// is skipped and the palette keeps painting. Reporting `true` for
/// that write — undo entry pushed, document dirtied, nothing on
/// screen — is the same defect on the override tier that the whole
/// branch exists to close on the `style` tier. The honest write is
/// the clear.
#[test]
fn test_empty_frame_and_text_clear_rather_than_writing_a_hole() {
    use super::tests_common::theme_node_with_probe_palette;
    use baumhard::mindmap::model::ColorGroup;

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let group = theme_node_with_probe_palette(
        &mut doc,
        &nid,
        "empty-probe",
        ColorGroup {
            background: "#a9decb".into(),
            frame: "#30b082".into(),
            text: "#0f0f0f".into(),
            title: String::new(),
        },
    );

    // Nothing overridden yet: an empty write has nothing to clear
    // and must report no change at all.
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(
        !doc.set_node_border_color(&nid, Some("")),
        "an empty frame write on an un-overridden node changes nothing and must say so"
    );
    assert!(
        !doc.set_node_text_color(&nid, Some("")),
        "same for the text channel"
    );
    assert!(doc.undo_stack.is_empty(), "no undo entry for a no-op");
    assert!(!doc.dirty, "no dirty flag for a no-op");

    // With an override in place, the empty write clears it and the
    // palette comes back.
    assert!(doc.set_node_border_color(&nid, Some("#ff00ff")));
    assert!(doc.set_node_text_color(&nid, Some("#ff00ff")));
    assert!(doc.set_node_border_color(&nid, Some("")));
    assert!(doc.set_node_text_color(&nid, Some("")));
    let node = &doc.mindmap.nodes[&nid];
    let overrides = &node.color_schema.as_ref().expect("themed").overrides;
    assert_eq!(overrides.frame, None);
    assert_eq!(overrides.text, None);
    assert_eq!(doc.mindmap.node_frame_color(node), group.frame);
    assert_eq!(doc.mindmap.node_text_color(node), group.text);
}

/// The fill channel is the exception, and it has to stay one: an
/// empty `background` is the format's spelling for "no fill, let
/// the canvas show through", every reader passes it along, and it
/// is therefore a value rather than an absence.
#[test]
fn test_empty_background_is_transparent_not_a_clear() {
    use super::tests_common::theme_node_with_probe_palette;
    use baumhard::mindmap::model::ColorGroup;

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    theme_node_with_probe_palette(
        &mut doc,
        &nid,
        "transparent-probe",
        ColorGroup {
            background: "#a9decb".into(),
            frame: "#30b082".into(),
            text: "#0f0f0f".into(),
            title: String::new(),
        },
    );
    assert!(doc.set_node_bg_color(&nid, Some("")));
    let node = &doc.mindmap.nodes[&nid];
    assert_eq!(
        node.color_schema.as_ref().expect("themed").overrides.background,
        Some(String::new()),
        "an empty fill is an authored transparent, not a missing opinion"
    );
    assert_eq!(
        doc.mindmap.node_background_color(node),
        "",
        "and the reader passes it through instead of falling back to the group"
    );
}

/// The themed counterpart of
/// `test_set_node_text_color_round_trips_through_undo`. Undo of a
/// text-color write **with run rewrites** has to restore two things
/// the unthemed case cannot exercise together: the override slot
/// the write created on `color_schema`, and the run colors the
/// rewrite changed underneath it. `EditNodeStyle` carries both, and
/// this is the test that would fail if it stopped carrying either.
#[test]
fn test_set_node_text_color_round_trips_through_undo_on_a_themed_node() {
    use super::defaults::default_text_run;
    use super::tests_common::theme_node_with_probe_palette;
    use baumhard::mindmap::model::ColorGroup;

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(nid.clone());
    let group = theme_node_with_probe_palette(
        &mut doc,
        &nid,
        "undo-probe",
        ColorGroup {
            background: "#101010".into(),
            frame: "#202020".into(),
            text: "#303030".into(),
            title: String::new(),
        },
    );
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        // The migrated shape: a stale `style` copy the palette
        // shadows, plus a run baked from the palette's own text.
        node.style.text_color = "#dddddd".into();
        node.sections[0].text = "abcdef".into();
        node.sections[0].text_runs = vec![
            TextRun {
                start: 0,
                end: 3,
                color: "#303030".into(),
                ..default_text_run(3)
            },
            TextRun {
                start: 3,
                end: 6,
                color: "#abcdef".into(),
                ..default_text_run(6)
            },
        ];
    }
    doc.undo_stack.clear();

    assert!(doc.set_node_text_color(&nid, Some("#222222")));
    {
        let node = &doc.mindmap.nodes[&nid];
        assert_eq!(doc.mindmap.node_text_color(node), "#222222");
        assert_eq!(node.sections[0].text_runs[0].color, "#222222");
    }
    assert!(doc.undo());

    let node = &doc.mindmap.nodes[&nid];
    assert_eq!(
        node.color_schema
            .as_ref()
            .expect("undo must not drop the schema")
            .overrides
            .text,
        None,
        "undo restores the whole color_schema, so the override goes away with it"
    );
    assert_eq!(
        doc.mindmap.node_text_color(node),
        group.text,
        "which puts the node back on its palette rather than on the stale style copy"
    );
    assert_eq!(
        node.style.text_color, "#dddddd",
        "the shadowed style copy was never the write target and must be unchanged"
    );
    assert_eq!(
        node.sections[0].text_runs[0].color, "#303030",
        "the rewritten run comes back too"
    );
    assert_eq!(node.sections[0].text_runs[1].color, "#abcdef");
}

/// The themed sibling of the two-section pinned-run family. The
/// unthemed helper is the determinate anchor the section setters
/// need, but it drops `color_schema` — so every one of its call
/// sites tests the tier a per-node write does *not* land in. This
/// pins the other one: the node-level write goes to the overrides,
/// the section-level write stays on the runs, and neither disturbs
/// the other.
#[test]
fn test_node_and_section_color_writes_stay_in_their_tiers_on_a_themed_node() {
    use super::tests_common::make_two_section_node_with_pinned_runs_themed;
    use baumhard::mindmap::model::ColorGroup;

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let group = make_two_section_node_with_pinned_runs_themed(
        &mut doc,
        &nid,
        "#aaaaaa",
        ["#aaaaaa", "#aaaaaa"],
        "LiberationSans",
        14.0,
        ColorGroup {
            background: "#101010".into(),
            frame: "#202020".into(),
            text: "#303030".into(),
            title: String::new(),
        },
    );
    // The palette, not `style.text_color`, is what the node reads
    // as — the premise the rest of the test rests on.
    assert_eq!(doc.mindmap.node_text_color(&doc.mindmap.nodes[&nid]), group.text);

    // Section-scoped: only section 1's runs move, and the node's
    // own channels are untouched.
    assert!(doc.set_section_text_color(&nid, 1, "#00ff00".into()));
    {
        let node = &doc.mindmap.nodes[&nid];
        assert!(node.sections[0].text_runs.iter().all(|r| r.color == "#aaaaaa"));
        assert!(node.sections[1].text_runs.iter().all(|r| r.color == "#00ff00"));
        assert!(
            node.color_schema.as_ref().expect("themed").overrides.is_empty(),
            "a section write must not reach the node's override tier"
        );
    }

    // Node-scoped: the override tier takes the write, `style`
    // stays the stale shadowed copy, and the runs that were baked
    // copies of the *effective* default follow.
    assert!(doc.set_node_bg_color(&nid, Some("#0000ff")));
    let node = &doc.mindmap.nodes[&nid];
    assert_eq!(
        node.color_schema
            .as_ref()
            .expect("themed")
            .overrides
            .background
            .as_deref(),
        Some("#0000ff")
    );
    assert_eq!(doc.mindmap.node_background_color(node), "#0000ff");
    assert_eq!(
        node.style.background_color, "#141414",
        "the shadowed style tier is not where a themed write lands"
    );
    assert_eq!(
        doc.mindmap.node_frame_color(node),
        group.frame,
        "the other channels still resolve through the palette"
    );
}

// ── Uniform font setters over run-less sections ────────────────
//
// `MindSection::text_runs` documents a section with text and no
// runs as a legal shape that renders at the cascade defaults, and
// the loader synthesizes nothing, so an authored map reaches these
// setters in that state. `all` over an empty run list is vacuously
// true, so every uniform font setter reported "already at the
// target value" and returned `false` without touching anything —
// while the *range* siblings, which fill uncovered ranges from
// `default_text_run`, honored the same request.

/// Strip `node_id`'s first section down to the run-less shape a
/// map can be authored in: text present, `text_runs` empty.
/// Returns the section's grapheme count so a test can assert the
/// created run spans it.
fn make_section_run_less(doc: &mut MindMapDocument, node_id: &str, text: &str) -> usize {
    let node = doc.mindmap.nodes.get_mut(node_id).expect("node");
    node.sections[0].text = text.to_string();
    node.sections[0].text_runs = Vec::new();
    count_grapheme_clusters(text)
}

/// The four uniform font setters each author a run onto a
/// run-less section rather than silently declining.
///
/// Fails when: any setter goes back to `text_runs.iter().all(..)`
/// — the vacuous-truth shape — since that returns `false` and
/// leaves the section run-less. Asserting on the run's `start` /
/// `end` as well as the value is what distinguishes "a run was
/// created spanning the text" from "a degenerate run was pushed".
///
/// Control on the same path: the run-less section is re-made
/// before each row, and the section is asserted run-less *before*
/// the call, so a row cannot pass by finding a run an earlier row
/// left behind.
#[test]
fn test_uniform_font_setters_author_a_run_onto_a_run_less_section() {
    let text = "run-less";

    // Whole-node size.
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let count = make_section_run_less(&mut doc, &nid, text);
    assert!(doc.mindmap.nodes[&nid].sections[0].text_runs.is_empty());
    assert!(
        doc.set_node_font_size(&nid, 33.0),
        "set_node_font_size must report the change it makes on a run-less section"
    );
    let runs = &doc.mindmap.nodes[&nid].sections[0].text_runs;
    assert_eq!(runs.len(), 1, "one run spanning the section's text");
    assert_eq!((runs[0].start, runs[0].end), (0, count));
    assert_eq!(runs[0].size_pt, 33.0);

    // Whole-node family.
    let mut doc = load_test_doc();
    let count = make_section_run_less(&mut doc, &nid, text);
    assert!(doc.set_node_font_family(&nid, Some("DejaVu Sans Mono")));
    let runs = &doc.mindmap.nodes[&nid].sections[0].text_runs;
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].start, runs[0].end), (0, count));
    assert_eq!(runs[0].font, "DejaVu Sans Mono");

    // Per-section size.
    let mut doc = load_test_doc();
    let count = make_section_run_less(&mut doc, &nid, text);
    assert!(doc.set_section_font_size(&nid, 0, 17.5));
    let runs = &doc.mindmap.nodes[&nid].sections[0].text_runs;
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].start, runs[0].end), (0, count));
    assert_eq!(runs[0].size_pt, 17.5);

    // Per-section family.
    let mut doc = load_test_doc();
    let count = make_section_run_less(&mut doc, &nid, text);
    assert!(doc.set_section_font_family(&nid, 0, Some("DejaVu Sans Mono")));
    let runs = &doc.mindmap.nodes[&nid].sections[0].text_runs;
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].start, runs[0].end), (0, count));
    assert_eq!(runs[0].font, "DejaVu Sans Mono");
}

/// A section with **no text** stays run-less, and the setters say
/// so by returning `false`.
///
/// `text_run_ops` requires `start < end` and panics in debug
/// builds on a degenerate run, so authoring `TextRun { start: 0,
/// end: 0 }` here would be a crash waiting on the next slice or
/// splice — the "create the default run" answer is only correct
/// where there is text for it to span.
///
/// Fails when: the emptiness guard in `write_every_section_run`
/// goes. Paired with the test above so "returns false" cannot be
/// satisfied by a setter that declines everything.
#[test]
fn test_uniform_font_setters_leave_a_textless_section_run_less() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    make_section_run_less(&mut doc, &nid, "");
    doc.undo_stack.clear();
    doc.dirty = false;

    assert!(!doc.set_node_font_size(&nid, 33.0));
    assert!(!doc.set_node_font_family(&nid, Some("DejaVu Sans Mono")));
    assert!(!doc.set_section_font_size(&nid, 0, 17.5));
    assert!(!doc.set_section_font_family(&nid, 0, Some("DejaVu Sans Mono")));

    assert!(doc.mindmap.nodes[&nid].sections[0].text_runs.is_empty());
    assert!(
        doc.undo_stack.is_empty(),
        "a declined setter must push no undo entry"
    );
    assert!(!doc.dirty, "a declined setter must not dirty the document");
}

/// The write half of the pair refuses a textless section on its
/// own, not only because its callers ask first.
///
/// The *per-section* setters ask first: `section_runs_all_match`
/// reports a textless run-less section as nothing-to-do and they
/// return before the write. The whole-node pair does not — it asks
/// about the node, `sections.iter().all(..)`, and then writes every
/// section, so one section that needs a run carries its textless
/// siblings into `write_every_section_run` with it. That live path
/// is pinned by
/// `test_a_whole_node_setter_carries_a_textless_section_into_the_guard`;
/// this test is the same guard exercised directly, so a helper that
/// no caller happened to route into a zero-length write today would
/// still be caught. Without it, the push is
/// `TextRun { start: 0, end: 0 }`, and `text_run_ops` panics in
/// debug builds on the next slice or splice of it.
///
/// Fails when: the `count == 0` guard in
/// `write_every_section_run` goes. The text-bearing half of the
/// same call is the control — it must author a run, so the
/// refusal cannot be a helper that never writes at all.
#[test]
fn test_write_every_section_run_refuses_to_author_a_zero_length_run() {
    use super::nodes::write_every_section_run;
    use baumhard::mindmap::model::MindSection;

    let mut textless = MindSection::new_default(String::new(), Vec::new());
    write_every_section_run(&mut textless, |r| r.size_pt = 33.0);
    assert!(
        textless.text_runs.is_empty(),
        "no text means no range for a run to span"
    );

    let mut texted = MindSection::new_default("abc".to_string(), Vec::new());
    write_every_section_run(&mut texted, |r| r.size_pt = 33.0);
    assert_eq!(
        texted.text_runs.len(),
        1,
        "control: the same call on a text-bearing section must author a run"
    );
    assert_eq!((texted.text_runs[0].start, texted.text_runs[0].end), (0, 3));
    assert_eq!(texted.text_runs[0].size_pt, 33.0);
}

/// **The whole-node setters do reach the guard**, on a node whose
/// sections disagree about whether there is anything to do.
///
/// `set_node_font_size` decides once for the node — `already =
/// sections.iter().all(section_runs_all_match)` — and then writes
/// *every* section. A node carrying one text-bearing run-less
/// section and one textless one therefore hands the textless one to
/// `write_every_section_run` even though `section_runs_all_match`
/// had reported it as nothing-to-do, and only the emptiness guard
/// keeps a zero-length run off it. Nothing else in the suite drives
/// a caller into that branch, which is how it was described as
/// unreachable.
///
/// Fails when: the `count == 0` guard in `write_every_section_run`
/// goes — `sections[1]` then gains `TextRun { start: 0, end: 0 }`,
/// the shape `text_run_ops` panics on. `sections[0]` is asserted in
/// the same call as the control: the setter must still author the
/// run it was asked for, so "no run on the textless section" cannot
/// be a setter that wrote nothing at all.
#[test]
fn test_a_whole_node_setter_carries_a_textless_section_into_the_guard() {
    use baumhard::mindmap::model::MindSection;

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let count = make_section_run_less(&mut doc, &nid, "run-less");
    doc.mindmap
        .nodes
        .get_mut(&nid)
        .expect("node")
        .sections
        .push(MindSection::new_default(String::new(), Vec::new()));

    let sections = &doc.mindmap.nodes[&nid].sections;
    assert_eq!(sections.len(), 2, "the fixture needs both section shapes");
    assert!(
        sections[0].text_runs.is_empty() && sections[1].text_runs.is_empty(),
        "both sections start run-less, or a later assertion reads a pre-existing run"
    );

    assert!(
        doc.set_node_font_size(&nid, 33.0),
        "the text-bearing section needs the write, so the node is not already at 33"
    );

    let sections = &doc.mindmap.nodes[&nid].sections;
    assert_eq!(
        sections[0].text_runs.len(),
        1,
        "control: the text-bearing section is written"
    );
    assert_eq!(
        (sections[0].text_runs[0].start, sections[0].text_runs[0].end),
        (0, count)
    );
    assert_eq!(sections[0].text_runs[0].size_pt, 33.0);
    assert!(
        sections[1].text_runs.is_empty(),
        "the textless section reached `write_every_section_run` and must come back \
         run-less: a zero-length run is what `text_run_ops` panics on"
    );
}

/// The authored run is undoable in one step, like every other
/// write through the style envelope.
///
/// Fails when: the run is authored outside the envelope's
/// snapshot, which would leave undo restoring a section that
/// still carries it. The pre-assertions pin the run-less start
/// state, so "run-less after undo" is a restoration rather than a
/// state that was never left.
#[test]
fn test_authoring_a_run_onto_a_run_less_section_undoes_in_one_step() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    make_section_run_less(&mut doc, &nid, "undo me");
    doc.undo_stack.clear();

    assert!(doc.set_node_font_size(&nid, 33.0));
    assert_eq!(doc.undo_stack.len(), 1, "one entry for one setter call");
    assert_eq!(doc.mindmap.nodes[&nid].sections[0].text_runs.len(), 1);

    doc.undo();
    assert!(
        doc.mindmap.nodes[&nid].sections[0].text_runs.is_empty(),
        "undo must restore the section to the run-less shape it was loaded in"
    );
    assert_eq!(doc.mindmap.nodes[&nid].sections[0].text, "undo me");
}

/// The uniform setter and its range sibling now agree on a
/// run-less section: `font size=N` over the whole text and
/// `font size=N range=0..n` produce the same runs.
///
/// This is the disagreement the fix closes — the range path filled
/// its gap from `default_text_run` while the uniform path declined
/// — so the assertion is a direct comparison of the two results
/// rather than a restatement of either one's expected shape.
#[test]
fn test_uniform_and_range_font_size_agree_on_a_run_less_section() {
    let text = "agree on me";

    let mut uniform = load_test_doc();
    let nid = first_testament_node_id(&uniform);
    let count = make_section_run_less(&mut uniform, &nid, text);
    assert!(uniform.set_section_font_size(&nid, 0, 19.0));

    let mut ranged = load_test_doc();
    make_section_run_less(&mut ranged, &nid, text);
    assert!(ranged.set_section_font_size_range(&nid, 0, 0, count, 19.0));

    assert_eq!(
        uniform.mindmap.nodes[&nid].sections[0].text_runs, ranged.mindmap.nodes[&nid].sections[0].text_runs,
        "the whole-text uniform setter and the whole-text range setter must author \
         the same run"
    );
}
