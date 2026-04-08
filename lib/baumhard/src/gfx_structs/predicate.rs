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

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum Comparator {
    Equals(bool),
    Exists(bool),
    GreaterThan(bool),
    LessThan(bool),
}

impl Comparator {
    pub fn equals() -> Self {
        Equals(false)
    }

    pub fn not_equals() -> Self {
        Equals(true)
    }

    pub fn exists() -> Self {
        Exists(false)
    }

    pub fn not_exists() -> Self {
        Exists(true)
    }

    pub fn greater() -> Self {
        GreaterThan(false)
    }

    pub fn less_or_equal() -> Self {
        GreaterThan(true)
    }

    pub fn less() -> Self {
        LessThan(false)
    }

    pub fn greater_or_equal() -> Self {
        LessThan(true)
    }

    pub fn compare_f32(&self, a: f32, b: f32) -> bool {
        match self {
            Equals(negation) => almost_equal(a, b) != *negation,
            GreaterThan(negation) => (a > b) != *negation,
            LessThan(negation) => (a < b) != *negation,
            Exists(negation) => !negation,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Predicate {
    pub fields: Vec<(GfxElementField, Comparator)>,
}
// TODO: I want to reduce the complexity of this, there is a better design here, I just need to analyze it
impl Predicate {
    pub fn new() -> Self {
        Predicate { fields: vec![] }
    }

    pub fn test(&self, element: &GfxElement) -> bool {
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
                    ColorFontRegions(_) => {} //todo

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
                                GlyphModelField::Operation(_) => false, //todo: what's up here
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
                                    GlyphModelField::Operation(_) => false, //todo
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
                                    GlyphModelField::Operation(_) => false, //todo
                                }
                            }
                            Exists(negation) => !negation,
                        };
                    }
                    return false;
                }
                GfxElementField::Flag(flag) => {} //todo
            }
        }
        false
    }
}
