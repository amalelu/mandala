//! Double-click detection + already-editing guard tests. The
//! predicates under test (`is_double_click`, the guard
//! predicate embedded in the MouseInput handler) are pure
//! cursor / time math, so exercising them here keeps the
//! winit event loop out of the test scaffold.

use super::*;

// -----------------------------------------------------------------
// Double-click detection
// -----------------------------------------------------------------

#[test]
fn test_double_click_same_target_within_window_fires() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: Some("node-a".to_string()),
    };
    assert!(is_double_click(
        &prev,
        1100.0,
        (101.0, 100.0),
        &Some("node-a".to_string()),
    ));
}

#[test]
fn test_double_click_different_targets_does_not_fire() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: Some("node-a".to_string()),
    };
    assert!(!is_double_click(
        &prev,
        1100.0,
        (100.0, 100.0),
        &Some("node-b".to_string()),
    ));
}

#[test]
fn test_double_click_too_far_apart_does_not_fire() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: None,
    };
    // Distance = sqrt(20² + 0²) = 20px → dist² = 400, threshold = 256.
    assert!(!is_double_click(&prev, 1100.0, (120.0, 100.0), &None));
}

#[test]
fn test_double_click_expired_does_not_fire() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: None,
    };
    assert!(!is_double_click(&prev, 1500.0, (100.0, 100.0), &None));
}

#[test]
fn test_double_click_empty_space_both_misses_fires() {
    // Both clicks landed on no node — valid double-click for
    // the "create orphan" gesture.
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (50.0, 50.0),
        hit: None,
    };
    assert!(is_double_click(&prev, 1150.0, (52.0, 51.0), &None));
}

#[test]
fn test_double_click_exact_boundary_does_not_fire() {
    // At exactly DOUBLE_CLICK_MS elapsed, should NOT fire (uses >= threshold).
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: None,
    };
    assert!(!is_double_click(&prev, 1400.0, (100.0, 100.0), &None));
}

#[test]
fn test_double_click_just_under_boundary_fires() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: None,
    };
    assert!(is_double_click(&prev, 1399.0, (100.0, 100.0), &None));
}

// -----------------------------------------------------------------
// "is_double_click + already_editing_same_target" guard semantics
// -----------------------------------------------------------------
//
// The bug report was: double-clicking inside an already-open
// editor on the same node silently discards the transient buffer
// because the Pressed path re-opens the editor, clobbering the
// in-progress buffer. The fix guards the dispatch with a check
// that re-opens are skipped if the editor is already on that
// target. We verify the guard predicate here; the actual event
// loop wiring is manually verified via `cargo run`.

#[test]
fn test_double_click_guard_skips_same_target_when_editor_open() {
    let editor = TextEditState::Open {
        node_id: "node-A".to_string(),
        buffer: "in progress".to_string(),
        cursor_grapheme_pos: 11,
        buffer_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        original_text: String::new(),
        original_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
    };
    let hit = Some("node-A".to_string());
    let already_editing = editor
        .node_id()
        .map(|id| hit.as_deref() == Some(id))
        .unwrap_or(false);
    assert!(already_editing, "guard must fire for same target");
}

#[test]
fn test_double_click_guard_allows_different_target_when_editor_open() {
    let editor = TextEditState::Open {
        node_id: "node-A".to_string(),
        buffer: "in progress".to_string(),
        cursor_grapheme_pos: 11,
        buffer_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        original_text: String::new(),
        original_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
    };
    let hit = Some("node-B".to_string());
    let already_editing = editor
        .node_id()
        .map(|id| hit.as_deref() == Some(id))
        .unwrap_or(false);
    assert!(!already_editing, "guard must NOT fire for different target");
}

#[test]
fn test_double_click_guard_allows_when_editor_closed() {
    let editor = TextEditState::Closed;
    let hit = Some("node-A".to_string());
    let already_editing = editor
        .node_id()
        .map(|id| hit.as_deref() == Some(id))
        .unwrap_or(false);
    assert!(!already_editing, "guard must NOT fire when editor is closed");
}
