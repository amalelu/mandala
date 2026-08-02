// SPDX-License-Identifier: MPL-2.0

//! Predicate language that steers walker traversal — the condition
//! expressions inside `Instruction::RepeatWhile` and its siblings.
//! A `Predicate` pairs a `Comparator` with a field selector drawn
//! from the `GfxElement` / `GlyphArea` / `GlyphModel` field enums,
//! so a mutator can express things like "repeat on every child
//! whose text starts empty" without hand-coding a walker. Pure
//! data + O(1) evaluations; serde-serializable so the mutator DSL
//! can persist predicates verbatim.

use crate::core::primitives::{ColorFontRegionField, Flaggable};
use crate::gfx_structs::area::GlyphAreaField;
use crate::gfx_structs::area::GlyphAreaField::{Bounds, ColorFontRegions, LineHeight, Scale, Text};
use crate::gfx_structs::element::GfxElementField::{Channel, GlyphArea, GlyphModel, Id, Region};
use crate::gfx_structs::element::{GfxElement, GfxElementField};
use crate::gfx_structs::model::GlyphModelField;
use crate::gfx_structs::model::GlyphModelField::{GlyphLine, GlyphLines, GlyphMatrix, Layer};
use crate::gfx_structs::predicate::Comparator::{Equals, Exists, GreaterThan, LessThan};
use crate::gfx_structs::tree::BranchChannel;
use crate::util::geometry::{
    almost_equal, almost_equal_vec2, pixel_greater_than, pixel_lesser_than, vec2_area,
};
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// A comparison operator used by [`Predicate`] to test a single field of a
/// [`GfxElement`] against a reference value.
///
/// Each variant wraps a `bool` *negation flag*: when `false` the comparison
/// is applied as-is; when `true` the result is inverted. This lets a single
/// enum express both a comparator and its logical complement (e.g. `==` and
/// `!=`) without doubling the variant count.
///
/// Costs: all comparisons are O(1); floating-point equality delegates to
/// [`crate::util::geometry::almost_equal`] to absorb rounding.
///
/// **Negation flag worked example.** A
/// `Predicate { fields: [(Flag(SectionRoot), Equals(false))], … }`
/// matches every element whose `SectionRoot` flag is **set** —
/// `Equals(false)` is the un-negated `==`, so the predicate
/// passes when `is_set == true`. The negation form
/// `Equals(true)` reads as `!=`, so it passes when `is_set == false`
/// (every element *without* the flag). The two interpretations
/// are easy to flip in your head — keep this example pinned at
/// the call site if you write a new field arm.
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum Comparator {
    /// Equality test. `Equals(false)` means `==`; `Equals(true)` means `!=`.
    Equals(bool),
    /// Existence test. `Exists(false)` returns `true` unconditionally (the
    /// field is present); `Exists(true)` returns `false` (the field must
    /// *not* exist). Primarily used for optional sub-fields like region
    /// font/color.
    Exists(bool),
    /// Strict greater-than. `GreaterThan(false)` means `a > b`;
    /// `GreaterThan(true)` means `a <= b` (the negation).
    GreaterThan(bool),
    /// Strict less-than. `LessThan(false)` means `a < b`;
    /// `LessThan(true)` means `a >= b` (the negation).
    LessThan(bool),
}

impl Comparator {
    /// Construct an equality comparator (`==`). O(1), no allocation.
    pub fn equals() -> Self {
        Equals(false)
    }

    /// Construct a not-equal comparator (`!=`). O(1), no allocation.
    pub fn not_equals() -> Self {
        Equals(true)
    }

    /// Construct an existence comparator — always returns `true`. O(1).
    pub fn exists() -> Self {
        Exists(false)
    }

    /// Construct a non-existence comparator — always returns `false`. O(1).
    pub fn not_exists() -> Self {
        Exists(true)
    }

    /// Construct a strict greater-than comparator (`>`). O(1).
    pub fn greater() -> Self {
        GreaterThan(false)
    }

    /// Construct a less-or-equal comparator (`<=`), the negation of
    /// greater-than. O(1).
    pub fn less_or_equal() -> Self {
        GreaterThan(true)
    }

    /// Construct a strict less-than comparator (`<`). O(1).
    pub fn less() -> Self {
        LessThan(false)
    }

    /// Construct a greater-or-equal comparator (`>=`), the negation of
    /// less-than. O(1).
    pub fn greater_or_equal() -> Self {
        LessThan(true)
    }

    /// Compare two `f32` values using this comparator's semantics.
    ///
    /// * `a` — the element-side value (left operand).
    /// * `b` — the reference value from the predicate field (right operand).
    ///
    /// Equality uses [`crate::util::geometry::almost_equal`] to absorb
    /// floating-point rounding. `Exists` ignores both inputs and returns
    /// the existence flag directly. O(1), no allocation.
    pub fn compare_f32(&self, a: f32, b: f32) -> bool {
        match self {
            Equals(negation) => almost_equal(a, b) != *negation,
            GreaterThan(negation) => (a > b) != *negation,
            LessThan(negation) => (a < b) != *negation,
            Exists(negation) => !negation,
        }
    }
}

/// A condition that can be tested against a [`GfxElement`] to decide
/// whether a mutation or query should apply to it.
///
/// A `Predicate` holds a list of `(GfxElementField, Comparator)` pairs.
/// [`Predicate::test`] walks the list and returns `true` when the first
/// matching field comparison succeeds, or `false` when no field matches.
/// The special `always_match` flag short-circuits the walk and
/// unconditionally returns `true` — used by `TargetScope::Descendants`
/// to blanket-apply mutations.
///
/// Costs: `test()` is O(n) in `fields.len()`, but typical predicates
/// carry one or two fields so the cost is effectively O(1). No
/// allocation on the test path.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Predicate {
    /// The field/comparator pairs to evaluate against a candidate element.
    /// Evaluation stops at the first pair whose field matches the element's
    /// variant — remaining pairs are not consulted.
    pub fields: Vec<(GfxElementField, Comparator)>,
    /// When true, this predicate matches any element regardless of fields.
    /// Used by TargetScope::Descendants to apply mutations to all descendants.
    #[serde(default)]
    pub always_match: bool,
}
impl Predicate {
    /// Create an empty predicate that matches nothing (no fields, no
    /// `always_match`). O(1), one empty `Vec` allocation.
    pub fn new() -> Self {
        Predicate {
            fields: vec![],
            always_match: false,
        }
    }

    /// Create a predicate that matches every element unconditionally.
    /// O(1), one empty `Vec` allocation.
    pub fn always_true() -> Self {
        Predicate {
            fields: vec![],
            always_match: true,
        }
    }

    /// Test whether `element` satisfies this predicate.
    ///
    /// * Returns `true` immediately if `always_match` is set.
    /// * Otherwise walks `fields` and returns the result of the first
    ///   field whose variant matches a property of `element`.
    /// * Returns `false` if no field matches.
    ///
    /// Costs: O(n) in `fields.len()`, no allocation.
    pub fn test(&self, element: &GfxElement) -> bool {
        if self.always_match {
            return true;
        }
        for (field, comparator) in &self.fields {
            if let Some(verdict) = evaluate_field(element, field, comparator) {
                return verdict;
            }
        }
        false
    }
}

/// Evaluate one `(field, comparator)` pair against `element`.
///
/// Returns `Some(true)` / `Some(false)` when the pair decides the
/// predicate's verdict, or `None` when the pair is inapplicable
/// (e.g. a `GlyphAreaField::Outline` axis that the predicate
/// language deliberately doesn't expose, or a `Region` axis whose
/// `region_id` isn't present on the element). The outer loop in
/// [`Predicate::test`] continues to the next field on `None` and
/// returns `false` if every field falls through.
fn evaluate_field(element: &GfxElement, field: &GfxElementField, comparator: &Comparator) -> Option<bool> {
    match field {
        GlyphArea(section) => evaluate_glyph_area_field(element, section, comparator),
        Channel(channel) => Some(match comparator {
            Equals(negation) => (element.channel() == *channel) != *negation,
            GreaterThan(negation) => (element.channel() > *channel) != *negation,
            LessThan(negation) => (element.channel() < *channel) != *negation,
            Exists(negation) => !negation,
        }),
        Region(region, color_font_region_field) => {
            // Region predicates only make sense on elements that
            // carry a `GlyphArea`. Degrade to non-match (not
            // fall-through) for other element kinds (§9).
            let Some(area) = element.glyph_area() else {
                return Some(false);
            };
            // Region missing on this element — fall through to next field.
            let target = area.regions.get(*region).copied()?;
            Some(evaluate_region_match(
                &target,
                color_font_region_field,
                comparator,
            ))
        }
        Id(id) => Some(match comparator {
            Equals(negation) => (*id == element.unique_id()) != *negation,
            GreaterThan(negation) => (element.unique_id() > *id) != *negation,
            LessThan(negation) => (element.unique_id() < *id) != *negation,
            Exists(negation) => !negation,
        }),
        GlyphModel(model_field) => {
            // Missing `glyph_model()` decides the predicate as
            // false — matches the original `return false` at the
            // tail of the old `GlyphModel` arm. `?` would
            // fall-through to the next field instead, which
            // would diverge for multi-field predicates.
            let Some(target_model) = element.glyph_model() else {
                return Some(false);
            };
            Some(evaluate_glyph_model_match(target_model, model_field, comparator))
        }
        GfxElementField::Flag(flag) => {
            let is_set = element.flag_is_set(*flag);
            Some(match comparator {
                // `Equals(false)` ⇒ match when the flag's presence
                // equals the inferred reference (`true`);
                // `Equals(true)` ⇒ inverted (the flag is absent).
                // `Exists` flips on the negation flag, mirroring
                // the GlyphAreaField / GlyphModelField shape.
                Equals(negation) => is_set != *negation,
                // `Exists` tests field presence, not value. A flag is
                // always a present field on the element; the negation
                // flag alone decides the result, matching Channel/Id.
                Exists(negation) => !negation,
                // Ordering on a presence bit is not meaningful —
                // degrade to non-match rather than panic (§9).
                _ => {
                    log::warn!(
                        "predicate: unsupported Comparator {:?} on Flag({:?}) \
                         — treating as non-match",
                        comparator,
                        flag,
                    );
                    false
                }
            })
        }
    }
}

fn evaluate_glyph_area_field(
    element: &GfxElement,
    section: &GlyphAreaField,
    comparator: &Comparator,
) -> Option<bool> {
    match section {
        Text(text) => Some(match comparator {
            Equals(negation) => element
                .glyph_area()
                .map(|area| (area.text == *text) != *negation)
                .unwrap_or(false),
            // Text is an unordered payload — only `Equals` is
            // meaningful. Pairing it with `<`/`>`/`Exists` is
            // malformed; degrade to non-match rather than panic.
            _ => {
                log::warn!(
                    "predicate: unsupported Comparator {:?} on GlyphArea::Text \
                     — treating as non-match",
                    comparator,
                );
                false
            }
        }),
        Scale(scale) => {
            // Missing area decides the predicate as false (matches
            // the original `let Some(area) = ... else { return false; };`).
            let Some(area) = element.glyph_area() else {
                return Some(false);
            };
            Some(comparator.compare_f32(area.scale.0, scale.0))
        }
        LineHeight(line_height) => {
            let Some(area) = element.glyph_area() else {
                return Some(false);
            };
            Some(comparator.compare_f32(area.line_height.0, line_height.0))
        }
        GlyphAreaField::Position(vec) => Some(match comparator {
            Equals(negation) => almost_equal_vec2(element.position(), vec.to_vec2()) != *negation,
            GreaterThan(negation) => {
                let element_pos = element.position().to_array();
                pixel_greater_than((element_pos[0], element_pos[1]), vec.to_pair()) != *negation
            }
            LessThan(negation) => {
                let element_pos = element.position().to_array();
                pixel_lesser_than((element_pos[0], element_pos[1]), vec.to_pair()) != *negation
            }
            Exists(negation) => !negation,
        }),
        Bounds(vec) => Some(match comparator {
            Equals(negation) => element
                .glyph_area()
                .map(|area| almost_equal_vec2(area.render_bounds.to_vec2(), vec.to_vec2()) != *negation)
                .unwrap_or(false),
            GreaterThan(negation) => element
                .glyph_area()
                .map(|area| (vec2_area(area.render_bounds.to_vec2()) > vec2_area(vec.to_vec2())) != *negation)
                .unwrap_or(false),
            LessThan(negation) => element
                .glyph_area()
                .map(|area| (vec2_area(area.render_bounds.to_vec2()) < vec2_area(vec.to_vec2())) != *negation)
                .unwrap_or(false),
            Exists(negation) => !negation,
        }),
        // Not predicate axes — fall through to the next field.
        ColorFontRegions(_)
        | GlyphAreaField::Outline(_)
        | GlyphAreaField::Shape(_)
        | GlyphAreaField::ZoomVisibility(_)
        | GlyphAreaField::Operation(_) => None,
    }
}

fn evaluate_region_match(
    target: &crate::core::primitives::ColorFontRegion,
    color_font_region_field: &ColorFontRegionField,
    comparator: &Comparator,
) -> bool {
    match comparator {
        Equals(negation) => match color_font_region_field {
            ColorFontRegionField::Range(range) => (*range == target.range) != *negation,
            ColorFontRegionField::Font(font) => target
                .font
                .map(|target_font| (*font == target_font) != *negation)
                .unwrap_or(false),
            ColorFontRegionField::Color(color) => target
                .color
                .map(|target_color| (*color == target_color) != *negation)
                .unwrap_or(false),
            // `This` is a no-payload marker used for `Exists`-style
            // probes; `Equals(This)` is malformed input.
            ColorFontRegionField::This => {
                log::warn!(
                    "predicate: Equals on ColorFontRegionField::This has no \
                     meaning — treating as non-match",
                );
                false
            }
        },
        GreaterThan(negation) => match color_font_region_field {
            ColorFontRegionField::Range(range) => (target.range > *range) != *negation,
            // Only `Range` has an ordering; font / color / this are opaque.
            _ => {
                log::warn!(
                    "predicate: GreaterThan on non-Range \
                     ColorFontRegionField {:?} — treating as non-match",
                    color_font_region_field,
                );
                false
            }
        },
        LessThan(negation) => match color_font_region_field {
            ColorFontRegionField::Range(range) => (target.range < *range) != *negation,
            _ => {
                log::warn!(
                    "predicate: LessThan on non-Range \
                     ColorFontRegionField {:?} — treating as non-match",
                    color_font_region_field,
                );
                false
            }
        },
        Exists(negation) => match color_font_region_field {
            ColorFontRegionField::Range(_) => !negation,
            ColorFontRegionField::Font(_) => target.font.is_some() != *negation,
            ColorFontRegionField::Color(_) => target.color.is_some() != *negation,
            ColorFontRegionField::This => !negation,
        },
    }
}

fn evaluate_glyph_model_match(
    target_model: &crate::gfx_structs::model::GlyphModel,
    model_field: &GlyphModelField,
    comparator: &Comparator,
) -> bool {
    match comparator {
        Equals(negation) => match model_field {
            GlyphMatrix(matrix) => (*matrix == target_model.glyph_matrix) != *negation,
            GlyphLine(line_num, line) => target_model
                .glyph_matrix
                .get(*line_num)
                .map(|our_line| (our_line == line) != *negation)
                .unwrap_or(false),
            // `GlyphLines` is a count-based field — use
            // `GreaterThan` / `LessThan` against it, or
            // `GlyphMatrix` / `GlyphLine` for equality.
            GlyphLines(_) => {
                log::warn!(
                    "predicate: Equals on GlyphLines (count-only field) \
                     — use GlyphMatrix or GlyphLine for equality",
                );
                false
            }
            Layer(layer) => (*layer == target_model.layer) != *negation,
            GlyphModelField::Position(vec) => (target_model.position == *vec) != *negation,
            GlyphModelField::Operation(_) => false,
        },
        GreaterThan(negation) => match model_field {
            // A matrix is a structured payload; only `Equals` is
            // defined for it. Use `GlyphLines(n)` with
            // `GreaterThan` for line-count ordering.
            GlyphMatrix(_) => {
                log::warn!(
                    "predicate: GreaterThan on GlyphMatrix \
                     (structured payload) — use GlyphLines for count \
                     ordering",
                );
                false
            }
            GlyphLine(line_num, line) => target_model
                .glyph_matrix
                .get(*line_num)
                .map(|our_line| (our_line.length() > line.length()) != *negation)
                .unwrap_or(false),
            GlyphLines(lines) => (target_model.glyph_matrix.matrix.len() > lines.len()) != *negation,
            Layer(layer) => (target_model.layer > *layer) != *negation,
            GlyphModelField::Position(vec) => {
                (target_model.position.to_vec2().distance(Vec2::new(0.0, 0.0))
                    > vec.to_vec2().distance(Vec2::new(0.0, 0.0)))
                    != *negation
            }
            GlyphModelField::Operation(_) => false,
        },
        LessThan(negation) => match model_field {
            GlyphMatrix(_) => {
                log::warn!(
                    "predicate: LessThan on GlyphMatrix \
                     (structured payload) — use GlyphLines for count \
                     ordering",
                );
                false
            }
            GlyphLine(line_num, line) => target_model
                .glyph_matrix
                .get(*line_num)
                .map(|our_line| (our_line.length() < line.length()) != *negation)
                .unwrap_or(false),
            GlyphLines(lines) => (target_model.glyph_matrix.matrix.len() < lines.len()) != *negation,
            Layer(layer) => (target_model.layer < *layer) != *negation,
            GlyphModelField::Position(vec) => {
                (target_model.position.to_vec2().distance(Vec2::new(0.0, 0.0))
                    < vec.to_vec2().distance(Vec2::new(0.0, 0.0)))
                    != *negation
            }
            GlyphModelField::Operation(_) => false,
        },
        Exists(negation) => !negation,
    }
}
