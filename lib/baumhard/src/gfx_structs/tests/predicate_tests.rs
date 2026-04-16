//! Tests for [`crate::gfx_structs::predicate`] — comparator and
//! predicate fundamentals (§T1).
//!
//! Covers every [`Comparator`] variant via `compare_f32`, and the
//! two key [`Predicate`] contracts: `always_true()` matches any
//! element, and a field-bearing predicate matches or rejects based
//! on the element's value.
//!
//! Follows the `do_*()` / `test_*()` split from §T2.2: every public
//! body is benchmarkable from `benches/test_bench.rs`.

use glam::Vec2;

use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::{GfxElement, GfxElementField};
use crate::gfx_structs::predicate::{Comparator, Predicate};

// ── Comparator::Equals (==) ────────────────────────────────────────

#[test]
fn test_comparator_equal_f32() {
    do_comparator_equal_f32();
}

/// `Comparator::equals()` delegates to `almost_equal`, so identical
/// and nearly-identical values return `true`; clearly distinct values
/// return `false`. O(1) per call.
pub fn do_comparator_equal_f32() {
    let cmp = Comparator::equals();

    // Equal values.
    assert!(cmp.compare_f32(1.0, 1.0));
    assert!(cmp.compare_f32(0.0, 0.0));
    assert!(cmp.compare_f32(-5.5, -5.5));

    // Unequal values.
    assert!(!cmp.compare_f32(1.0, 2.0));
    assert!(!cmp.compare_f32(-1.0, 1.0));
    assert!(!cmp.compare_f32(0.0, 100.0));
}

// ── Comparator::Equals negated (!=) ────────────────────────────────

#[test]
fn test_comparator_not_equal_f32() {
    do_comparator_not_equal_f32();
}

/// `Comparator::not_equals()` inverts the equality result: equal
/// values return `false`, distinct values return `true`. O(1).
pub fn do_comparator_not_equal_f32() {
    let cmp = Comparator::not_equals();

    // Equal values — should be false (negated equality).
    assert!(!cmp.compare_f32(1.0, 1.0));
    assert!(!cmp.compare_f32(0.0, 0.0));

    // Unequal values — should be true.
    assert!(cmp.compare_f32(1.0, 2.0));
    assert!(cmp.compare_f32(-1.0, 1.0));
}

// ── Comparator::LessThan (<) ───────────────────────────────────────

#[test]
fn test_comparator_less_than_f32() {
    do_comparator_less_than_f32();
}

/// `Comparator::less()` is strict less-than: `a < b`. O(1).
pub fn do_comparator_less_than_f32() {
    let cmp = Comparator::less();

    // a < b → true.
    assert!(cmp.compare_f32(1.0, 2.0));
    assert!(cmp.compare_f32(-10.0, 0.0));

    // a == b → false.
    assert!(!cmp.compare_f32(5.0, 5.0));

    // a > b → false.
    assert!(!cmp.compare_f32(3.0, 1.0));
}

// ── Comparator::GreaterThan (>) ────────────────────────────────────

#[test]
fn test_comparator_greater_than_f32() {
    do_comparator_greater_than_f32();
}

/// `Comparator::greater()` is strict greater-than: `a > b`. O(1).
pub fn do_comparator_greater_than_f32() {
    let cmp = Comparator::greater();

    // a > b → true.
    assert!(cmp.compare_f32(2.0, 1.0));
    assert!(cmp.compare_f32(0.0, -10.0));

    // a == b → false.
    assert!(!cmp.compare_f32(5.0, 5.0));

    // a < b → false.
    assert!(!cmp.compare_f32(1.0, 3.0));
}

// ── Comparator::LessThan negated (>=) ──────────────────────────────

#[test]
fn test_comparator_greater_equal_f32() {
    do_comparator_greater_equal_f32();
}

/// `Comparator::greater_or_equal()` is the negation of strict
/// less-than: `!(a < b)`, equivalent to `a >= b`. O(1).
pub fn do_comparator_greater_equal_f32() {
    let cmp = Comparator::greater_or_equal();

    // a > b → true (not less).
    assert!(cmp.compare_f32(3.0, 1.0));

    // a == b → true (not less).
    assert!(cmp.compare_f32(5.0, 5.0));

    // a < b → false.
    assert!(!cmp.compare_f32(1.0, 3.0));
}

// ── Comparator::GreaterThan negated (<=) ───────────────────────────

#[test]
fn test_comparator_less_equal_f32() {
    do_comparator_less_equal_f32();
}

/// `Comparator::less_or_equal()` is the negation of strict
/// greater-than: `!(a > b)`, equivalent to `a <= b`. O(1).
pub fn do_comparator_less_equal_f32() {
    let cmp = Comparator::less_or_equal();

    // a < b → true (not greater).
    assert!(cmp.compare_f32(1.0, 3.0));

    // a == b → true (not greater).
    assert!(cmp.compare_f32(5.0, 5.0));

    // a > b → false.
    assert!(!cmp.compare_f32(3.0, 1.0));
}

// ── Predicate::always_true ─────────────────────────────────────────

#[test]
fn test_predicate_always_true_matches_anything() {
    do_predicate_always_true_matches_anything();
}

/// `Predicate::always_true()` matches every element variant —
/// `GlyphArea`, `GlyphModel`, and `Void` — without inspecting any
/// field. O(1), no allocation on the test path.
pub fn do_predicate_always_true_matches_anything() {
    let pred = Predicate::always_true();

    // A GlyphArea element.
    let area = GlyphArea::new(16.0, 1.2, Vec2::new(10.0, 20.0), Vec2::new(100.0, 50.0));
    let area_elem = GfxElement::new_area_non_indexed(area, 0);
    assert!(pred.test(&area_elem));

    // A Void element.
    let void_elem = GfxElement::new_void(0);
    assert!(pred.test(&void_elem));

    // A blank GlyphModel element.
    let model_elem = GfxElement::new_model_blank(0, 0);
    assert!(pred.test(&model_elem));
}

// ── Predicate::test with a field-bearing predicate ─────────────────

#[test]
fn test_predicate_matches_field_value() {
    do_predicate_matches_field_value();
}

/// A predicate targeting `GfxElementField::Id` with `Comparator::equals()`
/// matches an element whose `unique_id` equals the reference value and
/// rejects one whose `unique_id` differs. O(1) per call.
pub fn do_predicate_matches_field_value() {
    let target_id: usize = 42;
    let pred = Predicate {
        fields: vec![(GfxElementField::Id(target_id), Comparator::equals())],
        always_match: false,
    };

    // Element with matching id.
    let matching = GfxElement::new_void_with_id(0, 42);
    assert!(pred.test(&matching), "Predicate should match element with id 42");

    // Element with different id.
    let non_matching = GfxElement::new_void_with_id(0, 99);
    assert!(!pred.test(&non_matching), "Predicate should reject element with id 99");
}
