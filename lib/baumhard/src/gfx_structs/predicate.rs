use crate::core::primitives::ColorFontRegionField;
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
        Predicate { fields: vec![], always_match: false }
    }

    /// Create a predicate that matches every element unconditionally.
    /// O(1), one empty `Vec` allocation.
    pub fn always_true() -> Self {
        Predicate { fields: vec![], always_match: true }
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
        for (element_field, comparator) in &self.fields {
            match element_field {
                GlyphArea(section) => match section {
                    Text(text) => {
                        return match comparator {
                            Equals(negation) => {
                                if let Some(area) = element.glyph_area() {
                                    (area.text == *text) != *negation
                                } else {
                                    false
                                }
                            }
                            _ => panic!("Unsupported Comparator for text"),
                        };
                    }
                    Scale(scale) => {
                        return comparator
                            .compare_f32(element.glyph_area().unwrap().scale.0, scale.0);
                    }
                    LineHeight(line_height) => {
                        let element_line_height = element.glyph_area().unwrap().line_height;
                        return comparator.compare_f32(element_line_height.0, line_height.0);
                    }
                    ColorFontRegions(_) => {} // not a predicate axis

                    GlyphAreaField::Position(vec) => {
                        return match comparator {
                            Equals(negation) => {
                                almost_equal_vec2(element.position(), vec.to_vec2()) != *negation
                            }
                            GreaterThan(negation) => {
                                let element_pos = element.position().to_array();
                                pixel_greater_than((element_pos[0], element_pos[1]), vec.to_pair())
                                    != *negation
                            }
                            LessThan(negation) => {
                                let element_pos = element.position().to_array();
                                pixel_lesser_than((element_pos[0], element_pos[1]), vec.to_pair())
                                    != *negation
                            }
                            Exists(negation) => !negation,
                        };
                    }
                    Bounds(vec) => {
                        return match comparator {
                            Equals(negation) => {
                                if let Some(area) = element.glyph_area() {
                                    almost_equal_vec2(area.render_bounds.to_vec2(), vec.to_vec2())
                                        != *negation
                                } else {
                                    false
                                }
                            }
                            GreaterThan(negation) => {
                                if let Some(area) = element.glyph_area() {
                                    (vec2_area(area.render_bounds.to_vec2()) > vec2_area(vec.to_vec2()))
                                        != *negation
                                } else {
                                    false
                                }
                            }
                            LessThan(negation) => {
                                return if let Some(area) = element.glyph_area() {
                                    (vec2_area(area.render_bounds.to_vec2()) < vec2_area(vec.to_vec2()))
                                        != *negation
                                } else {
                                    false
                                }
                            }
                            Exists(negation) => !negation,
                        };
                    }
                    GlyphAreaField::Outline(_) => {} // Halo state isn't a predicate axis.
                    GlyphAreaField::Operation(_) => {}
                },
                Channel(channel) => {
                    return match comparator {
                        Equals(negation) => (*channel == element.channel()) != *negation,
                        GreaterThan(negation) => (*channel > element.channel()) != *negation,
                        LessThan(negation) => (*channel < element.channel()) != *negation,
                        _ => false,
                    }
                }
                Region(region, color_font_region_field) => {
                    let target_range = element.glyph_area().unwrap().regions.get(*region);
                    if target_range.is_some() {
                        let target = *target_range.unwrap();
                        return match comparator {
                            Equals(negation) => match color_font_region_field {
                                ColorFontRegionField::Range(range) => {
                                    (*range == target.range) != *negation
                                }
                                ColorFontRegionField::Font(font) => {
                                    if let Some(target_font) = target.font {
                                        (*font == target_font) != *negation
                                    } else {
                                        false
                                    }
                                }
                                ColorFontRegionField::Color(color) => {
                                    if let Some(target_color) = target.color {
                                        (*color == target_color) != *negation
                                    } else {
                                        false
                                    }
                                }
                                ColorFontRegionField::This => panic!("Unsupported operation!"),
                            },
                            GreaterThan(negation) => match color_font_region_field {
                                ColorFontRegionField::Range(range) => {
                                    (target.range > *range) != *negation
                                }
                                _ => panic!("Unsupported operation on ColorFontRegionField"),
                            },
                            LessThan(negation) => match color_font_region_field {
                                ColorFontRegionField::Range(range) => {
                                    (target.range < *range) != *negation
                                }
                                _ => panic!("Unsupported operation on ColorFontRegionField"),
                            },
                            Exists(negation) => {
                                return match color_font_region_field {
                                    ColorFontRegionField::Range(_) => !negation,
                                    ColorFontRegionField::Font(_) => {
                                        target.font.is_some() != *negation
                                    }
                                    ColorFontRegionField::Color(_) => {
                                        target.color.is_some() != *negation
                                    }
                                    ColorFontRegionField::This => !negation,
                                }
                            }
                        };
                    }
                }
                Id(id) => {
                    return match comparator {
                        Equals(negation) => (*id == element.unique_id()) != *negation,
                        GreaterThan(negation) => (element.unique_id() > *id) != *negation,
                        LessThan(negation) => (element.unique_id() < *id) != *negation,
                        Exists(negation) => !negation,
                    }
                }
                GlyphModel(model_field) => {
                    if element.glyph_model().is_some() {
                        let target_model = element.glyph_model().unwrap();
                        return match comparator {
                            Equals(negation) => match model_field {
                                GlyphMatrix(matrix) => {
                                    (*matrix == target_model.glyph_matrix) != *negation
                                }
                                GlyphLine(line_num, line) => {
                                    // maybe she's born with it, maybe it's
                                    if let Some(our_line) = target_model.glyph_matrix.get(*line_num)
                                    {
                                        (our_line == line) != *negation
                                    } else {
                                        false
                                    }
                                }
                                GlyphLines(_) => {
                                    panic!("Unsupported operation: equality test on lines. Use GlyphMatrix or GlyphLine")
                                }
                                Layer(layer) => (*layer == target_model.layer) != *negation,
                                GlyphModelField::Position(vec) => {
                                    (target_model.position == *vec) != *negation
                                }
                                GlyphModelField::Operation(_) => false,
                            },
                            GreaterThan(negation) => {
                                match model_field {
                                    GlyphMatrix(_) => {
                                        panic!("Unsupported operation: GreaterThan test on glyph matrix")
                                    }
                                    GlyphLine(line_num, line) => {
                                        // maybe she's born with it, maybe it's
                                        if let Some(our_line) =
                                            target_model.glyph_matrix.get(*line_num)
                                        {
                                            (our_line.length() > line.length()) != *negation
                                        } else {
                                            false
                                        }
                                    }
                                    GlyphLines(lines) => {
                                        (lines.len() > target_model.glyph_matrix.matrix.len())
                                            != *negation
                                    }
                                    Layer(layer) => (*layer > target_model.layer) != *negation,
                                    GlyphModelField::Position(vec) => {
                                        (target_model
                                            .position
                                            .to_vec2()
                                            .distance(Vec2::new(0.0, 0.0))
                                            > vec.to_vec2().distance(Vec2::new(0.0, 0.0)))
                                            != *negation
                                    }
                                    GlyphModelField::Operation(_) => false,
                                }
                            }
                            LessThan(negation) => {
                                match model_field {
                                    GlyphMatrix(_) => panic!(
                                        "Unsupported operation: LessThan test on glyph matrix"
                                    ),
                                    GlyphLine(line_num, line) => {
                                        if let Some(our_line) =
                                            target_model.glyph_matrix.get(*line_num)
                                        {
                                            (our_line.length() < line.length()) != *negation
                                        } else {
                                            false
                                        }
                                    }
                                    GlyphLines(lines) => {
                                        (lines.len() < target_model.glyph_matrix.matrix.len())
                                            != *negation
                                    }

                                    Layer(layer) => *layer < target_model.layer,
                                    GlyphModelField::Position(vec) => {
                                        (target_model
                                            .position
                                            .to_vec2()
                                            .distance(Vec2::new(0.0, 0.0))
                                            < vec.to_vec2().distance(Vec2::new(0.0, 0.0)))
                                            != *negation
                                    }
                                    GlyphModelField::Operation(_) => false,
                                }
                            }
                            Exists(negation) => !negation,
                        };
                    }
                    return false;
                }
                GfxElementField::Flag(_flag) => {} // flag predicates not yet supported
            }
        }
        false
    }
}
