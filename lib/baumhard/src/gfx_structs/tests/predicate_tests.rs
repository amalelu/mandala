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

use crate::core::primitives::{ColorFontRegionField, Range};
use crate::gfx_structs::area::{GlyphArea, GlyphAreaField};
use crate::gfx_structs::element::{GfxElement, GfxElementField};
use crate::gfx_structs::model::{GlyphLine, GlyphMatrix, GlyphModelField};
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

// ── Predicate degrade paths (§9: interactive-path panics → false) ───
//
// The matcher rejects seven malformed field-type × comparator
// pairings. Each used to panic; per §9 each now returns `false` and
// logs a warning. The tests below pin that posture so a future
// refactor that re-introduces a panic breaks loudly at test time.

#[test]
fn test_predicate_text_with_greater_than_degrades_to_false() {
    do_predicate_text_with_greater_than_degrades_to_false();
}

/// `GlyphAreaField::Text` only defines equality — ordering against
/// a text payload has no meaning. A predicate pairing the two must
/// return `false` rather than panic. O(1).
pub fn do_predicate_text_with_greater_than_degrades_to_false() {
    let pred = Predicate {
        fields: vec![(
            GfxElementField::GlyphArea(GlyphAreaField::Text("x".to_string())),
            Comparator::greater(),
        )],
        always_match: false,
    };
    let area = GlyphArea::new_with_str(
        "y",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 10.0),
    );
    let elem = GfxElement::new_area_non_indexed(area, 0);
    assert!(
        !pred.test(&elem),
        "Text + GreaterThan is malformed; predicate must degrade to false",
    );
}

#[test]
fn test_predicate_region_this_with_equals_degrades_to_false() {
    do_predicate_region_this_with_equals_degrades_to_false();
}

/// `ColorFontRegionField::This` is a no-payload marker used by
/// `Exists`-style probes; pairing it with `Equals` is malformed
/// input. The matcher must degrade to `false`. O(1).
pub fn do_predicate_region_this_with_equals_degrades_to_false() {
    let range = Range::new(0, 1);
    let pred = Predicate {
        fields: vec![(
            GfxElementField::Region(range, ColorFontRegionField::This),
            Comparator::equals(),
        )],
        always_match: false,
    };
    // Need a GlyphArea element with a region at [0,1) so the
    // matcher reaches the inner match arm rather than early-outing.
    let mut area = GlyphArea::new_with_str(
        "ab",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 10.0),
    );
    area.regions.submit_region(crate::core::primitives::ColorFontRegion::new(
        range, None, None,
    ));
    let elem = GfxElement::new_area_non_indexed(area, 0);
    assert!(
        !pred.test(&elem),
        "Equals on ColorFontRegionField::This is malformed; must degrade to false",
    );
}

#[test]
fn test_predicate_region_font_with_greater_than_degrades_to_false() {
    do_predicate_region_font_with_greater_than_degrades_to_false();
}

/// Only `ColorFontRegionField::Range` has an ordering. `Font` is
/// opaque and cannot be compared with `<` / `>`; the matcher must
/// degrade to `false`. O(1).
pub fn do_predicate_region_font_with_greater_than_degrades_to_false() {
    let range = Range::new(0, 1);
    let pred = Predicate {
        fields: vec![(
            GfxElementField::Region(
                range,
                ColorFontRegionField::Font(crate::font::fonts::AppFont::Any),
            ),
            Comparator::greater(),
        )],
        always_match: false,
    };
    let mut area = GlyphArea::new_with_str(
        "ab",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 10.0),
    );
    area.regions.submit_region(crate::core::primitives::ColorFontRegion::new(
        range, None, None,
    ));
    let elem = GfxElement::new_area_non_indexed(area, 0);
    assert!(
        !pred.test(&elem),
        "GreaterThan on ColorFontRegionField::Font must degrade to false",
    );
}

#[test]
fn test_predicate_region_color_with_less_than_degrades_to_false() {
    do_predicate_region_color_with_less_than_degrades_to_false();
}

/// Colours are opaque too — same degrade posture as font. O(1).
pub fn do_predicate_region_color_with_less_than_degrades_to_false() {
    let range = Range::new(0, 1);
    let pred = Predicate {
        fields: vec![(
            GfxElementField::Region(
                range,
                ColorFontRegionField::Color([1.0, 0.0, 0.0, 1.0]),
            ),
            Comparator::less(),
        )],
        always_match: false,
    };
    let mut area = GlyphArea::new_with_str(
        "ab",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 10.0),
    );
    area.regions.submit_region(crate::core::primitives::ColorFontRegion::new(
        range, None, None,
    ));
    let elem = GfxElement::new_area_non_indexed(area, 0);
    assert!(
        !pred.test(&elem),
        "LessThan on ColorFontRegionField::Color must degrade to false",
    );
}

#[test]
fn test_predicate_glyph_lines_with_equals_degrades_to_false() {
    do_predicate_glyph_lines_with_equals_degrades_to_false();
}

/// `GlyphLines(Vec<(usize, GlyphLine)>)` is a count-based field
/// used for line-count ordering. Equality has no meaning for it;
/// callers needing an equality test should use `GlyphMatrix` or
/// `GlyphLine` instead. O(1) on the degrade path.
pub fn do_predicate_glyph_lines_with_equals_degrades_to_false() {
    let pred = Predicate {
        fields: vec![(
            GfxElementField::GlyphModel(GlyphModelField::GlyphLines(vec![
                (0, GlyphLine::new()),
            ])),
            Comparator::equals(),
        )],
        always_match: false,
    };
    let elem = GfxElement::new_model_blank(0, 0);
    assert!(
        !pred.test(&elem),
        "Equals on GlyphLines is malformed; must degrade to false",
    );
}

#[test]
fn test_predicate_glyph_matrix_with_greater_than_degrades_to_false() {
    do_predicate_glyph_matrix_with_greater_than_degrades_to_false();
}

/// `GlyphMatrix` is a structured payload; it has no ordering. Only
/// equality works. Use `GlyphLines(n)` with `GreaterThan` for line-
/// count ordering. Degrade to `false` rather than panic. O(1).
pub fn do_predicate_glyph_matrix_with_greater_than_degrades_to_false() {
    let pred = Predicate {
        fields: vec![(
            GfxElementField::GlyphModel(GlyphModelField::GlyphMatrix(GlyphMatrix::new())),
            Comparator::greater(),
        )],
        always_match: false,
    };
    let elem = GfxElement::new_model_blank(0, 0);
    assert!(
        !pred.test(&elem),
        "GreaterThan on GlyphMatrix must degrade to false",
    );
}

#[test]
fn test_predicate_glyph_matrix_with_less_than_degrades_to_false() {
    do_predicate_glyph_matrix_with_less_than_degrades_to_false();
}

/// Mirror of the `GreaterThan` case above. O(1) on the degrade path.
pub fn do_predicate_glyph_matrix_with_less_than_degrades_to_false() {
    let pred = Predicate {
        fields: vec![(
            GfxElementField::GlyphModel(GlyphModelField::GlyphMatrix(GlyphMatrix::new())),
            Comparator::less(),
        )],
        always_match: false,
    };
    let elem = GfxElement::new_model_blank(0, 0);
    assert!(
        !pred.test(&elem),
        "LessThan on GlyphMatrix must degrade to false",
    );
}

#[test]
fn test_predicate_glyph_area_field_on_void_degrades_to_false() {
    do_predicate_glyph_area_field_on_void_degrades_to_false();
}

/// A predicate targeting a `GlyphArea` sub-field applied to a
/// `Void` element cannot match — the element has no `GlyphArea` to
/// inspect. The matcher returns `false` rather than panicking on
/// the missing payload. O(1).
pub fn do_predicate_glyph_area_field_on_void_degrades_to_false() {
    let pred = Predicate {
        fields: vec![(
            GfxElementField::GlyphArea(GlyphAreaField::scale(14.0)),
            Comparator::equals(),
        )],
        always_match: false,
    };
    let void_elem = GfxElement::new_void_with_id(0, 0);
    assert!(
        !pred.test(&void_elem),
        "GlyphAreaField predicate must degrade on elements without a GlyphArea",
    );
}
