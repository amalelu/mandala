//! Tests for [`crate::gfx_structs::tree::Tree`] — arena operations,
//! mutations, events, and subscriber dispatch (§T1).

use crossbeam_channel::unbounded;
use glam::Vec2;
use indextree::{NodeId};
use lazy_static::lazy_static;
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{Arc, Mutex};
use strum::IntoEnumIterator;

use crate::font::fonts;
use crate::font::fonts::AppFont;
use crate::font::fonts::AppFont::{AliceInWonderland, Evilz};
use crate::font::fonts::AppFont::AlphaMusicMan;
use crate::font::fonts::AppFont::AppleTea;
use crate::font::fonts::AppFont::Casanova;
use crate::font::fonts::AppFont::DenseLetters;
use crate::font::fonts::AppFont::NorseBold;
use crate::gfx_structs::area::{
    DeltaGlyphArea, GlyphArea, GlyphAreaCommand, GlyphAreaCommandType, GlyphAreaField,
};
use crate::gfx_structs::element::{GfxElement, GfxElementField};
use crate::gfx_structs::model::{
    GlyphComponent, GlyphLine, GlyphModel, GlyphModelCommand, GlyphModelCommandType,
    GlyphModelField,
};
use crate::gfx_structs::mutator::Instruction::RepeatWhile;
use crate::gfx_structs::mutator::{
    GfxMutator, GlyphTreeEvent, GlyphTreeEventInstance, Mutation,
};
use crate::gfx_structs::tree::{BranchChannel, EventSubscriber, MutatorTree, Tree};
use crate::gfx_structs::tree_walker::walk_tree_from;
use crate::util::color::{add_rgba, Color, FloatRgba};

use crate::core::primitives::{
    Applicable, ApplyOperation, ColorFontRegion, ColorFontRegionField, ColorFontRegions, Range,
};
use crate::gfx_structs::predicate::{Comparator, Predicate};
use crate::gfx_structs::util::regions::RegionParams;
use crate::util::geometry;
use crate::util::ordered_vec2::OrderedVec2;

/// The tests are written in a non-test-annotated function and then wrapped by an annotated test function
/// So that they can be reused for benchmarking

#[test]
pub fn test_basics_solo_mutation() {
    basics_solo_mutation();
}

pub fn basics_solo_mutation() {
    // This is necessary to initialize lazy statics
    fonts::init();
    let mut element = GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "test",
            1.0,
            10.0,
            Vec2::new(10.0, 10.0),
            Vec2::new(10.0, 10.0),
        ),
        0,
        0,
    );

    let color_font_region = ColorFontRegion::new(
        Range::new(0, 1),
        Some(DenseLetters),
        Some([0.0, 0.0, 0.0, 1.0]),
    );

    element
        .glyph_area_mut()
        .unwrap()
        .regions
        .submit_region(color_font_region.clone());

    assert_element_delta(
        &element,
        vec![
            GlyphAreaField::Text("test".to_string()),
            GlyphAreaField::scale(1.0),
            GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![
                color_font_region.clone()
            ])),
            GlyphAreaField::position(10.0, 10.0),
        ],
    );

    let mutator = GfxMutator::new(
        Mutation::area_delta(DeltaGlyphArea::new(vec![
            GlyphAreaField::Operation(ApplyOperation::Add),
            GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![
                ColorFontRegion::new(Range::new(0, 1), Some(AppleTea), None),
            ])),
            GlyphAreaField::position(5.0, 5.0),
        ])),
        0,
    );

    mutator.apply_to(&mut element);

    assert_element_delta(
        &element,
        vec![
            GlyphAreaField::Text("test".to_string()),
            GlyphAreaField::scale(1.0),
            GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![
                ColorFontRegion::new(Range::new(0, 1), Some(AppleTea), Some([0.0, 0.0, 0.0, 1.0])),
            ])),
            GlyphAreaField::position(15.0, 15.0),
        ],
    );

    element.rotate(Vec2::new(5.0, 5.0), 60.0);

    let expect =
        geometry::clockwise_rotation_around_pivot(Vec2::new(15.0, 15.0), Vec2::new(5.0, 5.0), 60.0);
    assert_element_delta(
        &element,
        vec![
            GlyphAreaField::Text("test".to_string()),
            GlyphAreaField::scale(1.0),
            GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![
                ColorFontRegion::new(Range::new(0, 1), Some(AppleTea), Some([0.0, 0.0, 0.0, 1.0])),
            ])),
            GlyphAreaField::position(expect.x, expect.y),
        ],
    );
}

lazy_static!(
             // a command to test on the left side, and the expected change on the right side
              pub static ref AREA_COMMANDS: Vec<(GlyphAreaCommand, Vec<(GfxElementField, ApplyOperation)>)> =
       vec![
             (GlyphAreaCommand::NudgeDown(1.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::position(0.0, 1.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::NudgeDown(-2.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::position(0.0, -2.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::NudgeUp(1.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::position(0.0, -1.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::NudgeUp(-2.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::position(0.0, 2.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::GrowFont(5.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::scale(5.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::ShrinkFont(5.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::scale(-5.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::NudgeLeft(-11.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::position(11.0, 0.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::NudgeLeft(10.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::position(-10.0, 0.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::NudgeRight(-100.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::position(-100.0, 0.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::NudgeRight(101.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::position(101.0, 0.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::ShrinkLineHeight(10.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::line_height(-10.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::ShrinkLineHeight(-10.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::line_height(10.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::GrowLineHeight(10.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::line_height(10.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::GrowLineHeight(-10.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::line_height(-10.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::ShrinkFont(-10.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::scale(10.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::ShrinkFont(10.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::scale(-10.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::GrowFont(10.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::scale(10.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::GrowFont(-10.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::scale(-10.0)), ApplyOperation::Add)]),
             (GlyphAreaCommand::SetBounds(10.0, 100.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::bounds(10.0, 100.0)), ApplyOperation::Assign)]),
             (GlyphAreaCommand::SetLineHeight(3.0),
                vec![(GfxElementField::GlyphArea(GlyphAreaField::line_height(3.0)), ApplyOperation::Assign)]),
             (GlyphAreaCommand::MoveTo(500.0, 500.0),
                vec![(GfxElementField::GlyphArea(
                GlyphAreaField::position(500.0, 500.0)), ApplyOperation::Assign)]),
             (GlyphAreaCommand::SetRegionColor(Range::new(0,0),[25.0, 25.0, 25.0, 25.0]),
                vec![(GfxElementField::Region(Range::new(0,0),
                ColorFontRegionField::Color([25.0, 25.0, 25.0, 25.0])), ApplyOperation::Assign)]),
             (GlyphAreaCommand::SetRegionFont(Range::new(0,0), AppFont::Skullphabet),
                vec![(GfxElementField::Region(Range::new(0,0),
                ColorFontRegionField::Font(AppFont::Skullphabet)), ApplyOperation::Assign)]),
             (GlyphAreaCommand::PopBack(3),
                vec![(GfxElementField::GlyphArea(
                GlyphAreaField::Text("piz".to_string())), ApplyOperation::Assign)]),
             (GlyphAreaCommand::PopFront(3),
                vec![(GfxElementField::GlyphArea(
                GlyphAreaField::Text("za🍕".to_string())), ApplyOperation::Assign)]),
             (GlyphAreaCommand::SetFontSize(20.0),
                vec![(GfxElementField::GlyphArea(
                GlyphAreaField::scale(20.0)), ApplyOperation::Assign)]),
             (GlyphAreaCommand::DeleteColorFontRegion(Range::new(200, 300)),
                vec![(GfxElementField::Region( Range::new(200, 300), ColorFontRegionField::This), ApplyOperation::Delete)]),
             (GlyphAreaCommand::ChangeRegionRange( Range::new(0, 1), Range::new(200,300) ),
                vec![(GfxElementField::Region( Range::new(0, 1),
                 ColorFontRegionField::Range( Range::new(200,300) )), ApplyOperation::Assign)]),
       ];

   pub static ref MODEL_COMMANDS: Vec<(GlyphModelCommand, Vec<(GfxElementField, ApplyOperation)>)> =
           vec![
               (GlyphModelCommand::MoveTo(10.0, 10.0),
                   vec![(GfxElementField::GlyphModel(GlyphModelField::position(10.0, 10.0)), ApplyOperation::Assign)]),
               (GlyphModelCommand::NudgeDown(200.0),
                   vec![(GfxElementField::GlyphModel(GlyphModelField::position(0.0, 200.0)), ApplyOperation::Add)]),
               (GlyphModelCommand::NudgeDown(200.0),
                   vec![(GfxElementField::GlyphModel(GlyphModelField::position(0.0, 200.0)), ApplyOperation::Assign)]),

               // this is the operation to be tested
               (GlyphModelCommand::RudeInsert {
                           line_num: 0,
                           at_idx: 0,
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(GlyphModelField::GlyphLine(0,
                               GlyphLine::new_with(GlyphComponent::text("hello", AlphaMusicMan, Color::black())))),
                           ApplyOperation::Assign)]),

               // this is the operation to be tested
               (GlyphModelCommand::RudeInsert {
                           line_num: 0,
                           at_idx: 1,
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(
                   GlyphModelField::GlyphLine(0, GlyphLine::new_with_vec(vec![
                       GlyphComponent::space(1),
                       GlyphComponent::text("hello", AlphaMusicMan, Color::black())], false))),
                   ApplyOperation::Assign)]),

               // this is the operation to be tested
               (GlyphModelCommand::RudeInsert {
                           line_num: 0,
                           at_idx: 2, // This does not go out of bounds
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(
                   GlyphModelField::GlyphLine(0, GlyphLine::new_with_vec(vec![
                       GlyphComponent::space(2),
                       GlyphComponent::text("hello", AlphaMusicMan, Color::black())], false))),
                   ApplyOperation::Assign)]),

               // this is the operation to be tested
               (GlyphModelCommand::RudeInsert {
                           line_num: 0,
                           at_idx: 3, // This index goes out of bounds, so additional space(s) must be added
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(
                   // This requires the code to notice that the left-boundary is already a space component
                   // and thus expand it rather than add an extra space component
                   GlyphModelField::GlyphLine(0, GlyphLine::new_with_vec(vec![
                       GlyphComponent::space(3),
                       GlyphComponent::text("hello", AlphaMusicMan, Color::black())], false))),
                   ApplyOperation::Assign)]),

               // this is the operation to be tested
               (GlyphModelCommand::RudeInsert {
                           line_num: 1, // This line doesn't already exist
                           at_idx: 0, // This obviously doesn't go out of bounds
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(
                   GlyphModelField::GlyphLine(1, GlyphLine::new_with_vec(vec![
                       GlyphComponent::text("hello", AlphaMusicMan, Color::black())], false))),
                   ApplyOperation::Assign)]),

               // this is the operation to be tested
               (GlyphModelCommand::RudeInsert {
                           line_num: 2, // This line doesn't already exist
                           at_idx: 5, // The index goes out of bounds, so additional space(s) must be added
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(
                   GlyphModelField::GlyphLine(2, GlyphLine::new_with_vec(vec![
                       GlyphComponent::space(5),
                       GlyphComponent::text("hello", AlphaMusicMan, Color::black())], false))),
                   ApplyOperation::Assign)]),

                           // this is the operation to be tested
               (GlyphModelCommand::PoliteInsert {
                           line_num: 0,
                           at_idx: 0, // Inserting here pushes the two initial spaces forward
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(
                   GlyphModelField::GlyphLine(0, GlyphLine::new_with_vec(vec![
                       GlyphComponent::text("hello", AlphaMusicMan, Color::black()),
                       GlyphComponent::space(2)], false))),
                   ApplyOperation::Assign)]),

               // this is the operation to be tested
               (GlyphModelCommand::PoliteInsert {
                           line_num: 0,
                           at_idx: 1, // The reference begins with 2 spaces, we're inserting between them
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(
                   GlyphModelField::GlyphLine(0, GlyphLine::new_with_vec(vec![
                       GlyphComponent::space(1),
                       GlyphComponent::text("hello", AlphaMusicMan, Color::black()),
                       GlyphComponent::space(1)], false))),
                   ApplyOperation::Assign)]),

               // this is the operation to be tested
               (GlyphModelCommand::PoliteInsert {
                           line_num: 0,
                           at_idx: 10, // This is out of bounds
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(
                   GlyphModelField::GlyphLine(0, GlyphLine::new_with_vec(vec![
                       GlyphComponent::space(10),
                       GlyphComponent::text("hello", AlphaMusicMan, Color::black())], false))),
                   ApplyOperation::Assign)]),

               // this is the operation to be tested
               (GlyphModelCommand::PoliteInsert {
                           line_num: 2, // This line doesn't exist
                           at_idx: 0, // This is out of bounds
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(
                   GlyphModelField::GlyphLine(2, GlyphLine::new_with_vec(vec![
                       GlyphComponent::text("hello", AlphaMusicMan, Color::black())], false))),
                   ApplyOperation::Assign)]),

               // this is the operation to be tested
               (GlyphModelCommand::PoliteInsert {
                           line_num: 10, // This line doesn't exist
                           at_idx: 10, // This is out of bounds
                           component: GlyphComponent::text("hello", AlphaMusicMan, Color::black())},
                   // This is the expected result
                   vec![(GfxElementField::GlyphModel(
                   GlyphModelField::GlyphLine(10, GlyphLine::new_with_vec(vec![
                       GlyphComponent::space(10),
                       GlyphComponent::text("hello", AlphaMusicMan, Color::black())], false))),
                   ApplyOperation::Assign)]),
];
);

#[test]
fn test_model_block_commands() {
    model_block_commands();
}

pub fn model_block_commands() {
    fonts::init();

    let mut reference_model = GfxElement::new_model_non_indexed_with_id(GlyphModel::new(), 0, 0);
    reference_model
        .glyph_model_mut()
        .unwrap()
        .add_line(GlyphLine::new_with(GlyphComponent::space(2)));

    let mut command_type_set: FxHashSet<GlyphModelCommandType> = FxHashSet::default();
    for command_expect_set in MODEL_COMMANDS.clone() {
        let mut my_model = reference_model.clone();
        {
            // Apply the command to be tested to our model
            command_type_set.insert(command_expect_set.0.variant());
            command_expect_set
                .0
                .apply_to(my_model.glyph_model_mut().unwrap());
        }
        // The expected values are applied to our reference
        for (element_field, apply_operation) in command_expect_set.1 {
            match element_field {
                GfxElementField::GlyphModel(model_field) => match model_field {
                    GlyphModelField::GlyphMatrix(matrix) => {
                        let mut my_reference = reference_model.clone();
                        apply_operation.apply(
                            &mut my_reference.glyph_model_mut().unwrap().glyph_matrix,
                            matrix,
                        );
                        assert_eq!(my_reference, my_model);
                    }
                    GlyphModelField::GlyphLine(num, line) => {
                        let mut my_reference = reference_model.clone();
                        apply_operation.apply(
                            &mut my_reference.glyph_model_mut().unwrap().glyph_matrix[num],
                            line,
                        );
                        assert_eq!(my_reference, my_model);
                    }
                    GlyphModelField::GlyphLines(lines) => {
                        let mut my_matrix =
                            reference_model.glyph_model().unwrap().glyph_matrix.clone();
                        for (num, line) in lines {
                            apply_operation.apply(&mut my_matrix[num], line);
                            assert_eq!(
                                my_matrix[num],
                                my_model.glyph_model().unwrap().glyph_matrix[num]
                            );
                        }
                        assert_eq!(my_matrix, my_model.glyph_model().unwrap().glyph_matrix);
                    }
                    GlyphModelField::Layer(layer) => {
                        let mut my_reference_layer = reference_model.glyph_model().unwrap().layer;
                        apply_operation.apply(&mut my_reference_layer, layer);
                        assert_eq!(my_reference_layer, my_model.glyph_model().unwrap().layer);
                    }
                    GlyphModelField::Position(vec) => {
                        let mut my_x = reference_model.glyph_model().unwrap().position.x;
                        let mut my_y = reference_model.glyph_model().unwrap().position.y;
                        apply_operation.apply(&mut my_x, vec.x);
                        apply_operation.apply(&mut my_y, vec.y);
                        assert_eq!(my_x, my_model.glyph_model().unwrap().position.x);
                        assert_eq!(my_y, my_model.glyph_model().unwrap().position.y);
                    }
                    _ => {}
                },
                GfxElementField::Channel(_channel) => {}
                GfxElementField::Id(_id) => {}
                _ => {}
            }
        }
    }
}

#[test]
pub fn test_area_block_commands() {
    area_block_commands();
}

pub fn area_block_commands() {
    // This is necessary to initialize lazy statics
    fonts::init();
    let mut reference_block = GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "pizza🍕",
            1.0,
            10.0,
            Vec2::new(500.0, 500.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        0,
    );
    reference_block
        .glyph_area_mut()
        .unwrap()
        .regions
        .submit_region(ColorFontRegion::new(
            Range::new(0, 1),
            Some(DenseLetters),
            None,
        ));
    reference_block
        .glyph_area_mut()
        .unwrap()
        .regions
        .submit_region(ColorFontRegion::new(
            Range::new(200, 300),
            Some(AppFont::African),
            None,
        ));

    let mut command_type_set: FxHashSet<GlyphAreaCommandType> = FxHashSet::default();

    for command_expect_set in AREA_COMMANDS.clone() {
        let mut my_block = reference_block.clone();
        {
            command_type_set.insert(command_expect_set.0.variant());
            command_expect_set
                .0
                .apply_to(my_block.glyph_area_mut().unwrap());
        }
        for (element_field, apply_operation) in command_expect_set.1 {
            match apply_operation {
                ApplyOperation::Add => {
                    match element_field {
                        GfxElementField::GlyphArea(field) => match field {
                            GlyphAreaField::Text(text) => {
                                let my_text = my_block.glyph_area().unwrap().text.as_str();
                                let reference_text =
                                    reference_block.glyph_area().unwrap().text.as_str();
                                assert_eq!(my_text, reference_text.to_owned() + text.as_str())
                            }
                            GlyphAreaField::Scale(scale) => {
                                assert_eq!(
                                    scale + reference_block.glyph_area().unwrap().scale,
                                    my_block.glyph_area().unwrap().scale
                                )
                            }
                            GlyphAreaField::LineHeight(line_height) => {
                                assert_eq!(
                                    line_height + reference_block.glyph_area().unwrap().line_height,
                                    my_block.glyph_area().unwrap().line_height
                                )
                            }
                            GlyphAreaField::Position(vec) => {
                                assert_eq!(
                                    vec.x + reference_block.glyph_area().unwrap().position.x,
                                    my_block.glyph_area().unwrap().position.x
                                );

                                assert_eq!(
                                    vec.y + reference_block.glyph_area().unwrap().position.y,
                                    my_block.glyph_area().unwrap().position.y
                                );
                            }
                            GlyphAreaField::Bounds(vec) => {
                                assert_eq!(
                                   vec.x + reference_block.glyph_area().unwrap().render_bounds.x,
                                   my_block.glyph_area().unwrap().render_bounds.x
                                );

                                assert_eq!(
                                   vec.y + reference_block.glyph_area().unwrap().render_bounds.y,
                                   my_block.glyph_area().unwrap().render_bounds.y
                                );
                            }
                            GlyphAreaField::ColorFontRegions(_) => {
                                panic!("Not supported, use GfxElementField::Region instead, dummy (you are/have always been enough)")
                            }
                            GlyphAreaField::Outline(_) => {
                                // Outline mutations aren't expressible
                                // through the legacy `GlyphAreaCommand`
                                // surface — they only flow through
                                // `DeltaGlyphArea` (see the dedicated
                                // tests in `area_tests::do_outline_*`).
                            }
                            GlyphAreaField::Shape(_) => {
                                // Same story as Outline — shape lives
                                // on `DeltaGlyphArea`; see
                                // `shape::tests` in gfx_structs/shape.rs.
                            }
                            GlyphAreaField::ZoomVisibility(_) => {
                                // Window lives on `DeltaGlyphArea`;
                                // see `zoom_visibility_tests.rs`.
                            }
                            GlyphAreaField::Operation(_) => {}
                        },
                        // This range is the expected target
                        GfxElementField::Region(range, field) => {
                            match field {
                                // This range is the value that is expected to have been added
                                ColorFontRegionField::Range(add_range) => {
                                    let new_range = Range::new(
                                        range.start + add_range.start,
                                        range.end + add_range.end,
                                    );
                                    let ref_range = reference_block
                                        .glyph_area()
                                        .unwrap()
                                        .regions
                                        .hard_get(range)
                                        .range;

                                    assert_eq!(
                                        Range::new(
                                            ref_range.start + add_range.start,
                                            ref_range.end + add_range.end,
                                        ),
                                        new_range
                                    );

                                    let my_range = my_block
                                        .glyph_area()
                                        .unwrap()
                                        .regions
                                        .hard_get(new_range)
                                        .range;
                                    assert_eq!(my_range, new_range)
                                }
                                ColorFontRegionField::Font(_) => {
                                    panic!(
                                        "Add operation on font?? Do you also just mix random \
                              chemicals for fun, just to see what happens?"
                                    );
                                }
                                ColorFontRegionField::Color(color) => {
                                    assert_eq!(
                                        add_rgba(
                                            &reference_block.color_at_region(range).unwrap(),
                                            &color
                                        ),
                                        my_block.color_at_region(range).unwrap()
                                    );
                                }
                                ColorFontRegionField::This => {
                                    panic!("Unsupported operation: Add on ColorFontRegionField::This -> Use Assign");
                                }
                            }
                        }
                        GfxElementField::Channel(chan) => {
                            assert_eq!(my_block.channel(), reference_block.channel() + chan);
                        }
                        GfxElementField::Id(_) => {}
                        GfxElementField::GlyphModel(_) => {
                            todo!()
                        }
                        GfxElementField::Flag(_) => {}
                    }
                }
                ApplyOperation::Assign => match element_field {
                    GfxElementField::GlyphArea(field) => match field {
                        GlyphAreaField::Text(text) => {
                            assert_eq!(my_block.glyph_area().unwrap().text, text);
                        }
                        GlyphAreaField::Scale(scale) => {
                            assert_eq!(my_block.glyph_area().unwrap().scale, scale);
                        }
                        GlyphAreaField::LineHeight(line_height) => {
                            assert_eq!(my_block.glyph_area().unwrap().line_height, line_height);
                        }
                        GlyphAreaField::Position(vec) => {
                            assert_eq!(my_block.glyph_area().unwrap().position.x, vec.x);
                            assert_eq!(my_block.glyph_area().unwrap().position.y, vec.y);
                        }
                        GlyphAreaField::Bounds(vec) => {
                            assert_eq!(my_block.glyph_area().unwrap().render_bounds.x, vec.x);
                            assert_eq!(my_block.glyph_area().unwrap().render_bounds.y, vec.y);
                        }
                        GlyphAreaField::ColorFontRegions(_) => {
                            panic!("unsupported.");
                        }
                        GlyphAreaField::Outline(_) => {
                            // See the matching arm in the Add branch
                            // above — outline lives on `DeltaGlyphArea`.
                        }
                        GlyphAreaField::Shape(_) => {
                            // Same story as Outline — see the
                            // `Add` branch arm above.
                        }
                        GlyphAreaField::ZoomVisibility(_) => {
                            // Same story — see the `Add` branch.
                        }
                        GlyphAreaField::Operation(_) => {}
                    },
                    GfxElementField::Region(range, color_font_region_field) => {
                        match color_font_region_field {
                            ColorFontRegionField::Range(new_range) => {
                                assert!(my_block
                                    .glyph_area()
                                    .unwrap()
                                    .regions
                                    .get(new_range)
                                    .is_some());
                                assert!(my_block
                                    .glyph_area()
                                    .unwrap()
                                    .regions
                                    .get(range)
                                    .is_none());
                            }
                            ColorFontRegionField::Font(font) => {
                                assert_eq!(
                                    font,
                                    my_block
                                        .glyph_area()
                                        .unwrap()
                                        .regions
                                        .hard_get(range)
                                        .font
                                        .unwrap()
                                );
                            }
                            ColorFontRegionField::Color(color) => {
                                assert_eq!(
                                    color,
                                    my_block
                                        .glyph_area()
                                        .unwrap()
                                        .regions
                                        .hard_get(range)
                                        .color
                                        .unwrap()
                                );
                            }
                            ColorFontRegionField::This => {
                                assert!(my_block
                                    .glyph_area()
                                    .unwrap()
                                    .regions
                                    .get(range)
                                    .is_some());
                                assert!(reference_block
                                    .glyph_area()
                                    .unwrap()
                                    .regions
                                    .get(range)
                                    .is_none());
                            }
                        }
                    }
                    GfxElementField::Channel(chan) => {
                        assert_eq!(chan, my_block.channel());
                    }
                    GfxElementField::Id(_) => {
                        todo!()
                    }
                    GfxElementField::GlyphModel(_) => {
                        todo!()
                    }
                    GfxElementField::Flag(_) => {}
                },
                ApplyOperation::Subtract => {
                    match element_field {
                        GfxElementField::GlyphArea(field) => match field {
                            GlyphAreaField::Text(_text) => {
                                panic!(
                                    "Subtract operation on text? \
                              God bless you."
                                );
                            }
                            GlyphAreaField::Scale(scale) => {
                                assert_eq!(
                                    scale - reference_block.glyph_area().unwrap().scale,
                                    my_block.glyph_area().unwrap().scale
                                )
                            }
                            GlyphAreaField::LineHeight(line_height) => {
                                assert_eq!(
                                    line_height - reference_block.glyph_area().unwrap().line_height,
                                    my_block.glyph_area().unwrap().line_height
                                )
                            }
                            GlyphAreaField::Position(vec) => {
                                assert_eq!(
                                    vec.x - reference_block.glyph_area().unwrap().position.x,
                                    my_block.glyph_area().unwrap().position.x
                                );

                                assert_eq!(
                                    vec.y - reference_block.glyph_area().unwrap().position.y,
                                    my_block.glyph_area().unwrap().position.y
                                );
                            }
                            GlyphAreaField::Bounds(vec) => {
                                assert_eq!(
                                   vec.x - reference_block.glyph_area().unwrap().render_bounds.x,
                                   my_block.glyph_area().unwrap().render_bounds.x
                                );

                                assert_eq!(
                                   vec.y - reference_block.glyph_area().unwrap().render_bounds.y,
                                   my_block.glyph_area().unwrap().render_bounds.y
                                );
                            }
                            GlyphAreaField::ColorFontRegions(_) => {
                                panic!("Not supported, use GfxElementField::Region instead, idiot (God loves you)")
                            }
                            GlyphAreaField::Outline(_) => {
                                // See `do_outline_*` in area_tests.rs.
                            }
                            GlyphAreaField::Shape(_) => {
                                // Shape lives on `DeltaGlyphArea`
                                // only; see `shape::tests`.
                            }
                            GlyphAreaField::ZoomVisibility(_) => {
                                // Window lives on `DeltaGlyphArea`;
                                // see `zoom_visibility_tests.rs`.
                            }
                            GlyphAreaField::Operation(_) => {}
                        },
                        // This range is the expected target
                        GfxElementField::Region(range, field) => {
                            match field {
                                // This range is the value that is expected to have been added
                                ColorFontRegionField::Range(add_range) => {
                                    let new_range = Range::new(
                                        range.start - add_range.start,
                                        range.end - add_range.end,
                                    );

                                    let ref_range = reference_block
                                        .glyph_area()
                                        .unwrap()
                                        .regions
                                        .hard_get(range)
                                        .range;

                                    assert_eq!(
                                        Range::new(
                                            ref_range.start - add_range.start,
                                            ref_range.end - add_range.end,
                                        ),
                                        new_range
                                    );

                                    let my_range = my_block
                                        .glyph_area()
                                        .unwrap()
                                        .regions
                                        .hard_get(new_range)
                                        .range;
                                    assert_eq!(my_range, new_range)
                                }
                                ColorFontRegionField::Font(_font) => {
                                    panic!("Subtract operation on font? You have strayed [far] from the path.");
                                }
                                ColorFontRegionField::Color(color) => {
                                    assert_eq!(
                                        add_rgba(
                                            &reference_block.color_at_region(range).unwrap(),
                                            &color
                                        ),
                                        my_block.color_at_region(range).unwrap()
                                    );
                                }
                                ColorFontRegionField::This => {
                                    panic!("Unsupported operation: Subtract on ColorFontRegionField::This -> Use Delete");
                                }
                            }
                        }
                        GfxElementField::Channel(chan) => {
                            assert_eq!(my_block.channel(), reference_block.channel() - chan);
                        }
                        GfxElementField::Id(_) => {}
                        GfxElementField::GlyphModel(_) => {
                            todo!()
                        }
                        GfxElementField::Flag(_) => {}
                    }
                }
                ApplyOperation::Multiply => {
                    match element_field {
                        GfxElementField::GlyphArea(field) => match field {
                            GlyphAreaField::Text(_) => {
                                panic!("Multiply on text? Do you not understand that actions have consequences?");
                            }
                            GlyphAreaField::Scale(scale) => {
                                assert_eq!(
                                    scale * reference_block.glyph_area().unwrap().scale,
                                    my_block.glyph_area().unwrap().scale
                                )
                            }
                            GlyphAreaField::LineHeight(line_height) => {
                                assert_eq!(
                                    line_height * reference_block.glyph_area().unwrap().line_height,
                                    my_block.glyph_area().unwrap().line_height
                                )
                            }
                            GlyphAreaField::Position(vec) => {
                                assert_eq!(
                                    vec.x * reference_block.glyph_area().unwrap().position.x,
                                    my_block.glyph_area().unwrap().position.x
                                );

                                assert_eq!(
                                    vec.y * reference_block.glyph_area().unwrap().position.y,
                                    my_block.glyph_area().unwrap().position.y
                                );
                            }
                            GlyphAreaField::Bounds(vec) => {
                                assert_eq!(
                                   vec.x * reference_block.glyph_area().unwrap().render_bounds.x,
                                   my_block.glyph_area().unwrap().render_bounds.x
                                );

                                assert_eq!(
                                   vec.y * reference_block.glyph_area().unwrap().render_bounds.y,
                                   my_block.glyph_area().unwrap().render_bounds.y
                                );
                            }
                            GlyphAreaField::ColorFontRegions(_) => {
                                panic!("Not supported, use GfxElementField::Region instead, dummy (you are forgiven)")
                            }
                            GlyphAreaField::Outline(_) => {
                                // See `do_outline_*` in area_tests.rs.
                            }
                            GlyphAreaField::Shape(_) => {
                                // Shape lives on `DeltaGlyphArea`
                                // only; see `shape::tests`.
                            }
                            GlyphAreaField::ZoomVisibility(_) => {
                                // Window lives on `DeltaGlyphArea`;
                                // see `zoom_visibility_tests.rs`.
                            }
                            GlyphAreaField::Operation(_) => {}
                        },
                        // This range is the expected target
                        GfxElementField::Region(range, field) => {
                            match field {
                                // This range is the value that is expected to have been added
                                ColorFontRegionField::Range(add_range) => {
                                    let new_range = Range::new(
                                        range.start * add_range.start,
                                        range.end * add_range.end,
                                    );
                                    let ref_range = reference_block
                                        .glyph_area()
                                        .unwrap()
                                        .regions
                                        .hard_get(range)
                                        .range;

                                    assert_eq!(
                                        Range::new(
                                            ref_range.start * add_range.start,
                                            ref_range.end * add_range.end,
                                        ),
                                        new_range
                                    );

                                    let my_range = my_block
                                        .glyph_area()
                                        .unwrap()
                                        .regions
                                        .hard_get(new_range)
                                        .range;
                                    assert_eq!(my_range, new_range)
                                }
                                ColorFontRegionField::Font(_font) => {
                                    panic!("Add operation on font? My friend, there is darkness within you.");
                                }
                                ColorFontRegionField::Color(color) => {
                                    assert_eq!(
                                        add_rgba(
                                            &reference_block.color_at_region(range).unwrap(),
                                            &color
                                        ),
                                        my_block.color_at_region(range).unwrap()
                                    );
                                }
                                ColorFontRegionField::This => {
                                    panic!("You just love to tempt Fate don't you?");
                                }
                            }
                        }
                        GfxElementField::Channel(chan) => {
                            assert_eq!(my_block.channel(), reference_block.channel() * chan);
                        }
                        GfxElementField::Id(_) => {}
                        GfxElementField::GlyphModel(_) => {
                            todo!()
                        }
                        GfxElementField::Flag(_) => {}
                    }
                }
                ApplyOperation::Noop => {}
                ApplyOperation::Delete => match element_field {
                    GfxElementField::GlyphArea(section) => match section {
                        GlyphAreaField::Text(_) => {}
                        GlyphAreaField::Scale(scale) => {
                            assert_eq!(my_block.glyph_area().unwrap().scale, scale);
                        }
                        GlyphAreaField::LineHeight(line_height) => {
                            assert_eq!(my_block.glyph_area().unwrap().line_height, line_height);
                        }
                        GlyphAreaField::Position(vec) => {
                            assert_eq!(my_block.glyph_area().unwrap().position.x, vec.x);
                            assert_eq!(my_block.glyph_area().unwrap().position.y, vec.y);
                        }
                        GlyphAreaField::Bounds(vec) => {
                            assert_eq!(my_block.glyph_area().unwrap().render_bounds.x, vec.x);
                            assert_eq!(my_block.glyph_area().unwrap().render_bounds.y, vec.y);
                        }
                        GlyphAreaField::ColorFontRegions(_) => {
                            panic!("Use GfxElementField::Region(_,_) instead, my sweet lord");
                        }
                        GlyphAreaField::Outline(_) => {
                            // See `do_outline_*` in area_tests.rs.
                        }
                        GlyphAreaField::Shape(_) => {
                            // See `shape::tests`.
                        }
                        GlyphAreaField::ZoomVisibility(_) => {
                            // See `zoom_visibility_tests.rs`.
                        }
                        GlyphAreaField::Operation(_) => {}
                    },
                    GfxElementField::Region(range, field) => match field {
                        ColorFontRegionField::Range(_range) => {
                            panic!("Unsupported operation. Don't think that we don't have our eyes on you.");
                        }
                        ColorFontRegionField::Font(_) => {
                            assert!(my_block
                                .glyph_area()
                                .unwrap()
                                .regions
                                .hard_get(range)
                                .font
                                .is_none());
                            assert!(reference_block
                                .glyph_area()
                                .unwrap()
                                .regions
                                .hard_get(range)
                                .font
                                .is_some());
                        }
                        ColorFontRegionField::Color(_) => {
                            assert!(my_block
                                .glyph_area()
                                .unwrap()
                                .regions
                                .hard_get(range)
                                .color
                                .is_none());
                            assert!(reference_block
                                .glyph_area()
                                .unwrap()
                                .regions
                                .hard_get(range)
                                .color
                                .is_some());
                        }
                        ColorFontRegionField::This => {
                            assert!(my_block.glyph_area().unwrap().regions.get(range).is_none());
                            assert!(reference_block
                                .glyph_area()
                                .unwrap()
                                .regions
                                .get(range)
                                .is_some());
                        }
                    },
                    GfxElementField::Channel(_) => {}
                    GfxElementField::Id(_) => {}
                    GfxElementField::GlyphModel(_) => {}
                    GfxElementField::Flag(_) => {}
                },
            }
        }
    }
    for command_type in GlyphAreaCommandType::iter() {
        assert!(
            command_type_set.contains(&command_type),
            "The type {} was not tested",
            command_type
        );
    }
}

#[inline(always)]
fn incr(i: &mut usize) -> usize {
    let u = *i;
    *i += 1;
    u
}

lazy_static!(
    //                          event, timestamp, target_id, ancestry sequence
    pub static ref EVENTS: Vec<(GlyphTreeEvent, usize, usize, &'static str)> = vec![
      (GlyphTreeEvent::NoopEvent(0), 0, 0, "root"), //gen0
      (GlyphTreeEvent::NoopEvent(1), 0, 1, "0"), //gen1
      (GlyphTreeEvent::NoopEvent(2), 1, 2, "0"),
      (GlyphTreeEvent::NoopEvent(3), 0, 3, "0,1"), //gen2
      (GlyphTreeEvent::NoopEvent(4), 1, 4, "0,1"),
      (GlyphTreeEvent::NoopEvent(5), 0, 5, "0,2"),
      (GlyphTreeEvent::NoopEvent(6), 1, 6, "0,2"),
      (GlyphTreeEvent::NoopEvent(7), 0, 7, "0,1,3"), //gen3
      (GlyphTreeEvent::NoopEvent(8), 0, 8, "0,1,3,7"), //gen4
      (GlyphTreeEvent::NoopEvent(9), 0, 9, "0,1,3,7,8"), //gen5
      (GlyphTreeEvent::NoopEvent(10), 0, 10, "0,1,3,7,8,9"), //gen6
   ];
    /*        o (0)
             /  \
            o    o (1)
           / \  / \
           o o  o o (2)
           |
           o (3)
           |
           o (4)
           |
           o (5)
           |
           o (6)
    */
);

// Test-only formatter kept as a helper seam for future
// ancestry-assertion tests; see the already-landed ancestry
// tests above for the convention it plugs into.
#[allow(dead_code)]
#[inline(always)]
fn ancestry_seq(sequence: &[usize]) -> String {
    sequence
        .iter()
        .map(|num| num.to_string())
        .collect::<Vec<String>>()
        .join(",")
}

#[test]
pub fn test_event_propagation_complex_symmetric() {
    event_propagation_complex_symmetric();
}

pub fn event_propagation_complex_symmetric() {
    // This is necessary to initialize lazy statics
    fonts::init();
    let (mock_sender, _mock_receiver) = unbounded();
    let region_params = Arc::new(RegionParams::new(10, (1000, 1000)));
    let mut model: Tree<GfxElement, GfxMutator> = Tree::new(region_params, mock_sender);
    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();

    let mut model_index: FxHashMap<String, NodeId> = FxHashMap::default();
    let mut mutator_index: FxHashMap<String, NodeId> = FxHashMap::default();

    let results_index = Arc::new(Mutex::new(FxHashMap::<usize, GlyphTreeEvent>::default()));

    let subscriber: EventSubscriber = {
        let results_index = Arc::clone(&results_index);
        Arc::new(Mutex::new(
            move |gfx_element: &mut GfxElement, event: GlyphTreeEventInstance| {
                // Example logic for handling the event
                println!(
                    "Event received: {} on node with id {}",
                    event.event_type,
                    gfx_element.unique_id()
                );
                // Safely access results_index within the closure
                let results_index = results_index.lock().unwrap();
                let expected_event = results_index.get(&gfx_element.unique_id()).unwrap().clone();
                println!(
                    "Expected event is: {:?}, actual event is: {:?}",
                    expected_event, event.event_type
                );
                assert_eq!(expected_event, event.event_type);
                // Modify gfx_element based on the event here
            },
        ))
    };

    for entry in EVENTS.clone() {
        {
            let mut results_index = results_index.lock().unwrap();
            results_index.insert(entry.2, entry.0.clone());
        }

        let current_node_address = entry.3.to_string() + "," + &entry.2.to_string();
        // Create the target node
        let mut model_node = GfxElement::new_void_with_id(entry.1, entry.2);
        model_node.subscribers_mut().push(subscriber.clone());
        let model_node_id = model.arena.new_node(model_node);

        // Create the mutator node
        let mutator_node = GfxMutator::new(
            Mutation::Event(GlyphTreeEventInstance::new(entry.0.clone(), 0)),
            entry.1,
        );
        let mutator_node_id = mutator.arena.new_node(mutator_node);

        // Join to ancestor
        let ancestor_path = entry.3.to_string();
        if ancestor_path == "root" {
            model_index.insert("0".to_string(), model_node_id);
            mutator_index.insert("0".to_string(), mutator_node_id);
        } else {
            model_index
                .get(&ancestor_path)
                .unwrap()
                .append(model_node_id, &mut model.arena);
            model_index.insert(current_node_address.clone(), model_node_id);
            mutator_index
                .get(&ancestor_path)
                .unwrap()
                .append(mutator_node_id, &mut mutator.arena);
            mutator_index.insert(current_node_address.clone(), mutator_node_id);
        }
    }
    // Now both trees are completed, we can apply mutator to the target
    walk_tree_from(
        &mut model,
        &mut mutator,
        model_index.get("0").unwrap().clone(),
        mutator_index.get("0").unwrap().clone(),
    );
}

#[test]
pub fn test_event_propagation() {
    // This is mainly a smoke test
    event_propagation_simple();
}

pub fn event_propagation_simple() {
    // This is necessary to initialize lazy statics
    fonts::init();
    let mut model: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let mut id_head: usize = 0;
    let mut model_root = GfxElement::new_void_with_id(0, incr(&mut id_head));
    let mut model_baby = GfxElement::new_void_with_id(0, incr(&mut id_head));

    let subscriber: EventSubscriber = Arc::new(Mutex::new(
        |gfx_element: &mut GfxElement, event: GlyphTreeEventInstance| {
            // Example logic for handling the event
            println!(
                "Event received: {} on node with id {}",
                event.event_type,
                gfx_element.unique_id()
            );
            // Modify gfx_element based on the event here
        },
    ));

    model_root.subscribers_mut().push(subscriber.clone());
    model_baby.subscribers_mut().push(subscriber);

    let model_root_id = model.arena.new_node(model_root);
    let model_baby_id = model.arena.new_node(model_baby);
    model_root_id.append(model_baby_id, &mut model.arena);

    let event = GfxMutator::new(
        Mutation::Event(GlyphTreeEventInstance {
            event_type: GlyphTreeEvent::AppEvent,
            event_time_millis: 0,
        }),
        0,
    );

    let event_baby = GfxMutator::new(
        Mutation::Event(GlyphTreeEventInstance {
            event_type: GlyphTreeEvent::KillEvent,
            event_time_millis: 0,
        }),
        0,
    );

    let event_id = mutator.arena.new_node(event);
    let event_baby_id = mutator.arena.new_node(event_baby);
    event_id.append(event_baby_id, &mut mutator.arena);
    walk_tree_from(
        &mut model,
        &mut mutator,
        model_root_id,
        event_id,
    );
}

#[test]
pub fn test_complex_tree_mutation() {
    complex_tree_mutation();
}

pub fn complex_tree_mutation() {
    // This is necessary to initialize lazy statics
    fonts::init();
    let mut mutator_c: MutatorTree<GfxMutator> = MutatorTree::new();
    let mut model: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();

    // Create the tree-model nodes
    let root_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "root",
            1.0,
            10.0,
            Vec2::new(50.0, 50.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        0,
    ));

    let node_a_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "A",
            1.0,
            10.0,
            Vec2::new(30.0, 70.0),
            Vec2::new(500.0, 500.0),
        ),
        1,
        1,
    ));

    let node_b_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "B",
            1.0,
            10.0,
            Vec2::new(70.0, 70.0),
            Vec2::new(500.0, 500.0),
        ),
        2,
        2,
    ));

    let node_c_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "C",
            1.0,
            10.0,
            Vec2::new(50.0, 70.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        3,
    ));

    let node_aa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Aa",
            1.0,
            10.0,
            Vec2::new(20.0, 80.0),
            Vec2::new(500.0, 500.0),
        ),
        1,
        4,
    ));

    let node_ab_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Ab",
            1.0,
            10.0,
            Vec2::new(40.0, 80.0),
            Vec2::new(500.0, 500.0),
        ),
        2,
        5,
    ));

    let node_aaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Aaa",
            1.0,
            10.0,
            Vec2::new(20.0, 90.0),
            Vec2::new(500.0, 500.0),
        ),
        1,
        6,
    ));

    let node_aba_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Aba",
            1.0,
            10.0,
            Vec2::new(40.0, 90.0),
            Vec2::new(500.0, 500.0),
        ),
        2,
        7,
    ));

    let node_aaaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Aaaa",
            1.0,
            10.0,
            Vec2::new(20.0, 100.0),
            Vec2::new(500.0, 500.0),
        ),
        1,
        8,
    ));

    let node_abaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Abaa",
            1.0,
            10.0,
            Vec2::new(40.0, 100.0),
            Vec2::new(500.0, 500.0),
        ),
        2,
        9,
    ));

    let node_aaaaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Aaaaa",
            1.0,
            10.0,
            Vec2::new(20.0, 110.0),
            Vec2::new(500.0, 500.0),
        ),
        1,
        10,
    ));

    let node_abaaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Abaaa",
            1.0,
            10.0,
            Vec2::new(40.0, 110.0),
            Vec2::new(500.0, 500.0),
        ),
        2,
        11,
    ));

    let node_ca_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Ca",
            1.0,
            10.0,
            Vec2::new(50.0, 80.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        12,
    ));

    let node_caa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Caa",
            1.0,
            10.0,
            Vec2::new(50.0, 90.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        13,
    ));

    let node_caaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Caaa",
            1.0,
            10.0,
            Vec2::new(50.0, 100.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        14,
    ));

    let node_caaaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Caaaa",
            1.0,
            10.0,
            Vec2::new(50.0, 110.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        15,
    ));

    let node_caaaaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Caaaaa",
            1.0,
            10.0,
            Vec2::new(50.0, 120.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        16,
    ));

    let node_caaaaaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Caaaaaa",
            1.0,
            10.0,
            Vec2::new(50.0, 130.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        17,
    ));

    let node_ba_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Ba",
            1.0,
            10.0,
            Vec2::new(60.0, 80.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        18,
    ));

    let node_bb_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Bb",
            1.0,
            10.0,
            Vec2::new(80.0, 80.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        19,
    ));

    let node_baa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Baa",
            1.0,
            10.0,
            Vec2::new(60.0, 90.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        20,
    ));

    let node_bba_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Bba",
            1.0,
            10.0,
            Vec2::new(80.0, 90.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        21,
    ));

    let node_baaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Baaa",
            1.0,
            10.0,
            Vec2::new(60.0, 100.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        22,
    ));

    let node_bbaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Bbaa",
            1.0,
            10.0,
            Vec2::new(80.0, 100.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        23,
    ));

    let node_baaaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Baaaa",
            1.0,
            10.0,
            Vec2::new(60.0, 110.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        24,
    ));

    let node_bbaaa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Bbaaa",
            1.0,
            10.0,
            Vec2::new(80.0, 110.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        25,
    ));

    // Set the relationships
    root_id.append(node_c_id, &mut model.arena);
    root_id.append(node_a_id, &mut model.arena);
    root_id.append(node_b_id, &mut model.arena);

    node_a_id.append(node_aa_id, &mut model.arena);
    node_a_id.append(node_ab_id, &mut model.arena);
    node_aa_id.append(node_aaa_id, &mut model.arena);
    node_aaa_id.append(node_aaaa_id, &mut model.arena);
    node_aaaa_id.append(node_aaaaa_id, &mut model.arena);
    node_ab_id.append(node_aba_id, &mut model.arena);
    node_aba_id.append(node_abaa_id, &mut model.arena);
    node_abaa_id.append(node_abaaa_id, &mut model.arena);

    node_b_id.append(node_ba_id, &mut model.arena);
    node_b_id.append(node_bb_id, &mut model.arena);
    node_ba_id.append(node_baa_id, &mut model.arena);
    node_baa_id.append(node_baaa_id, &mut model.arena);
    node_baaa_id.append(node_baaaa_id, &mut model.arena);
    node_bb_id.append(node_bba_id, &mut model.arena);
    node_bba_id.append(node_bbaa_id, &mut model.arena);
    node_bbaa_id.append(node_bbaaa_id, &mut model.arena);

    node_c_id.append(node_ca_id, &mut model.arena);
    node_ca_id.append(node_caa_id, &mut model.arena);
    node_caa_id.append(node_caaa_id, &mut model.arena);
    node_caaa_id.append(node_caaaa_id, &mut model.arena);
    node_caaaa_id.append(node_caaaaa_id, &mut model.arena);
    node_caaaaa_id.append(node_caaaaaa_id, &mut model.arena);

    let mut c_predicate = Predicate::new();
    c_predicate
        .fields
        .push((GfxElementField::Channel(0), Comparator::equals()));
    // Create mutator-tree
    let c_instruction = mutator_c.arena.new_node(GfxMutator::new_instruction(RepeatWhile(c_predicate)));

    let mutator_child_c = mutator_c.arena.new_node(GfxMutator::new_macro(
        vec![
            Mutation::area_command(GlyphAreaCommand::SetRegionFont(
                Range::new(0, 1),
                AppFont::CarnivaleeFreakshow,
            )),
            Mutation::area_command(GlyphAreaCommand::GrowFont(20.0)),
            Mutation::area_command(GlyphAreaCommand::NudgeDown(200.0)),
        ],
        0,
    ));

    c_instruction.append(mutator_child_c, &mut mutator_c.arena);

    // Apply mutation
    walk_tree_from(&mut model, &mut mutator_c, root_id, c_instruction);
    // The root node should not be mutated, because by specification it is the children of the instruction-node that
    // should be applied to the children of the target node. If we want to also mutate the root, then we can do that
    // a few ways. For example a mutator can be set in the instruction node that specifically targets the root
    // But also, you could restructure the trees, for example such that the real root has an abstract parent that
    // can be the "target" for the mutation, and the "real root" will then be targetted as a child of
    // the "abstract parent". But in this case we are testing that the root remains untouched
    let root = model.get(root_id).unwrap();
    assert_eq!(root.get().position(), Vec2::new(50.0, 50.0));
    assert_eq!(root.get().glyph_area().unwrap().scale, 1.0);
    assert_eq!(root.get().glyph_area().unwrap().regions.num_regions(), 0);

    let caaaaaa = model.get(node_caaaaaa_id).unwrap();
    assert_eq!(caaaaaa.get().position(), Vec2::new(50.0, 330.0));
    assert_eq!(
        caaaaaa
            .get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::CarnivaleeFreakshow
    );
    assert_eq!(caaaaaa.get().glyph_area().unwrap().scale, 21.0);

    let caaaaa = model.get(node_caaaaa_id).unwrap();
    assert_eq!(caaaaa.get().position(), Vec2::new(50.0, 320.0));
    assert_eq!(
        caaaaa
            .get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::CarnivaleeFreakshow
    );
    assert_eq!(caaaaa.get().glyph_area().unwrap().scale, 21.0);

    let caaaa = model.get(node_caaaa_id).unwrap();
    assert_eq!(caaaa.get().position(), Vec2::new(50.0, 310.0));
    assert_eq!(
        caaaa
            .get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::CarnivaleeFreakshow
    );
    assert_eq!(caaaa.get().glyph_area().unwrap().scale, 21.0);

    let caaa = model.get(node_caaa_id).unwrap();
    assert_eq!(caaa.get().position(), Vec2::new(50.0, 300.0));
    assert_eq!(
        caaa.get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::CarnivaleeFreakshow
    );
    assert_eq!(caaa.get().glyph_area().unwrap().scale, 21.0);

    let caa = model.get(node_caa_id).unwrap();
    assert_eq!(caa.get().position(), Vec2::new(50.0, 290.0));
    assert_eq!(
        caa.get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::CarnivaleeFreakshow
    );
    assert_eq!(caa.get().glyph_area().unwrap().scale, 21.0);

    let ca = model.get(node_ca_id).unwrap();
    assert_eq!(ca.get().position(), Vec2::new(50.0, 280.0));
    assert_eq!(
        ca.get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::CarnivaleeFreakshow
    );
    assert_eq!(ca.get().glyph_area().unwrap().scale, 21.0);

    let c = model.get(node_c_id).unwrap();
    assert_eq!(c.get().position(), Vec2::new(50.0, 270.0));
    assert_eq!(
        c.get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::CarnivaleeFreakshow
    );
    assert_eq!(c.get().glyph_area().unwrap().scale, 21.0);

    let a = model.get(node_a_id).unwrap();
    assert_eq!(a.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(a.get().position(), Vec2::new(30.0, 70.0));
    assert_eq!(a.get().glyph_area().unwrap().scale, 1.0);

    let aa = model.get(node_aa_id).unwrap();
    assert_eq!(aa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(aa.get().position(), Vec2::new(20.0, 80.0));
    assert_eq!(aa.get().glyph_area().unwrap().scale, 1.0);

    let aaa = model.get(node_aaa_id).unwrap();
    assert_eq!(aaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(aaa.get().position(), Vec2::new(20.0, 90.0));
    assert_eq!(aaa.get().glyph_area().unwrap().scale, 1.0);

    let aaaa = model.get(node_aaaa_id).unwrap();
    assert_eq!(aaaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(aaaa.get().position(), Vec2::new(20.0, 100.0));
    assert_eq!(aaaa.get().glyph_area().unwrap().scale, 1.0);

    let aaaaa = model.get(node_aaaaa_id).unwrap();
    assert_eq!(aaaaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(aaaaa.get().position(), Vec2::new(20.0, 110.0));
    assert_eq!(aaaaa.get().glyph_area().unwrap().scale, 1.0);

    let ab = model.get(node_ab_id).unwrap();
    assert_eq!(ab.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(ab.get().position(), Vec2::new(40.0, 80.0));
    assert_eq!(ab.get().glyph_area().unwrap().scale, 1.0);

    let aba = model.get(node_aba_id).unwrap();
    assert_eq!(aba.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(aba.get().position(), Vec2::new(40.0, 90.0));
    assert_eq!(aba.get().glyph_area().unwrap().scale, 1.0);

    let abaa = model.get(node_abaa_id).unwrap();
    assert_eq!(abaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(abaa.get().position(), Vec2::new(40.0, 100.0));
    assert_eq!(abaa.get().glyph_area().unwrap().scale, 1.0);

    let abaaa = model.get(node_abaaa_id).unwrap();
    assert_eq!(abaaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(abaaa.get().position(), Vec2::new(40.0, 110.0));
    assert_eq!(abaaa.get().glyph_area().unwrap().scale, 1.0);

    let b = model.get(node_b_id).unwrap();
    assert_eq!(b.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(b.get().position(), Vec2::new(70.0, 70.0));
    assert_eq!(b.get().glyph_area().unwrap().scale, 1.0);

    let ba = model.get(node_ba_id).unwrap();
    assert_eq!(ba.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(ba.get().position(), Vec2::new(60.0, 80.0));
    assert_eq!(ba.get().glyph_area().unwrap().scale, 1.0);

    let baa = model.get(node_baa_id).unwrap();
    assert_eq!(baa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(baa.get().position(), Vec2::new(60.0, 90.0));
    assert_eq!(baa.get().glyph_area().unwrap().scale, 1.0);

    let baaa = model.get(node_baaa_id).unwrap();
    assert_eq!(baaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(baaa.get().position(), Vec2::new(60.0, 100.0));
    assert_eq!(baaa.get().glyph_area().unwrap().scale, 1.0);

    let baaaa = model.get(node_baaaa_id).unwrap();
    assert_eq!(baaaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(baaaa.get().position(), Vec2::new(60.0, 110.0));
    assert_eq!(baaaa.get().glyph_area().unwrap().scale, 1.0);

    let bb = model.get(node_bb_id).unwrap();
    assert_eq!(bb.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(bb.get().position(), Vec2::new(80.0, 80.0));
    assert_eq!(bb.get().glyph_area().unwrap().scale, 1.0);

    let bba = model.get(node_bba_id).unwrap();
    assert_eq!(bba.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(bba.get().position(), Vec2::new(80.0, 90.0));
    assert_eq!(bba.get().glyph_area().unwrap().scale, 1.0);

    let bbaa = model.get(node_bbaa_id).unwrap();
    assert_eq!(bbaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(bbaa.get().position(), Vec2::new(80.0, 100.0));
    assert_eq!(bbaa.get().glyph_area().unwrap().scale, 1.0);

    let bbaaa = model.get(node_bbaaa_id).unwrap();
    assert_eq!(bbaaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(bbaaa.get().position(), Vec2::new(80.0, 110.0));
    assert_eq!(bbaaa.get().glyph_area().unwrap().scale, 1.0);

    let mut mutator_a: MutatorTree<GfxMutator> = MutatorTree::new();
    let mut a_predicate = Predicate::new();
    a_predicate
        .fields
        .push((GfxElementField::Channel(1), Comparator::equals()));

    let a_instruction = mutator_a.arena.new_node(GfxMutator::new_instruction(RepeatWhile(
        a_predicate,
    )));

    let mutator_child_a = mutator_a.arena.new_node(GfxMutator::new_macro(
        vec![
            Mutation::area_command(GlyphAreaCommand::SetRegionFont(
                Range::new(0, 1),
                AppFont::Norse,
            )),
            Mutation::area_command(GlyphAreaCommand::GrowFont(10.0)),
            Mutation::area_command(GlyphAreaCommand::NudgeDown(100.0)),
        ],
        1,
    ));

    a_instruction.append(mutator_child_a, &mut mutator_a.arena);
    assert_eq!(mutator_a.get(a_instruction).is_some(), true);

    walk_tree_from(&mut model, &mut mutator_a, root_id, a_instruction);

    let a = model.get(node_a_id).unwrap();
    assert_eq!(
        a.get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::Norse
    );

    assert_eq!(
        a.get().glyph_area().unwrap().position,
        OrderedVec2::new_f32(30.0, 170.0)
    );
    assert_eq!(a.get().glyph_area().unwrap().scale, 11.0);

    let aa = model.get(node_aa_id).unwrap();
    assert_eq!(
        aa.get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::Norse
    );

    assert_eq!(
        aa.get().glyph_area().unwrap().position,
        OrderedVec2::new_f32(20.0, 180.0)
    );
    assert_eq!(aa.get().glyph_area().unwrap().scale, 11.0);

    let aaa = model.get(node_aaa_id).unwrap();
    assert_eq!(
        aaa.get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::Norse
    );
    assert_eq!(
        aaa.get().glyph_area().unwrap().position,
        OrderedVec2::new_f32(20.0, 190.0)
    );
    assert_eq!(aaa.get().glyph_area().unwrap().scale, 11.0);

    let aaaa = model.get(node_aaaa_id).unwrap();
    assert_eq!(
        aaaa.get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::Norse
    );

    assert_eq!(
        aaaa.get().glyph_area().unwrap().position,
        OrderedVec2::new_f32(20.0, 200.0)
    );
    assert_eq!(aaaa.get().glyph_area().unwrap().scale, 11.0);

    let aaaaa = model.get(node_aaaaa_id).unwrap();
    assert_eq!(
        aaaaa
            .get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppFont::Norse
    );

    assert_eq!(
        aaaaa.get().glyph_area().unwrap().position,
        OrderedVec2::new_f32(20.0, 210.0)
    );
    assert_eq!(aaaaa.get().glyph_area().unwrap().scale, 11.0);

    let ab = model.get(node_ab_id).unwrap();
    assert_eq!(ab.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(ab.get().position(), Vec2::new(40.0, 80.0));
    assert_eq!(ab.get().glyph_area().unwrap().scale, 1.0);

    let aba = model.get(node_aba_id).unwrap();
    assert_eq!(aba.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(aba.get().position(), Vec2::new(40.0, 90.0));
    assert_eq!(aba.get().glyph_area().unwrap().scale, 1.0);

    let abaa = model.get(node_abaa_id).unwrap();
    assert_eq!(abaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(abaa.get().position(), Vec2::new(40.0, 100.0));
    assert_eq!(abaa.get().glyph_area().unwrap().scale, 1.0);

    let abaaa = model.get(node_abaaa_id).unwrap();
    assert_eq!(abaaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(abaaa.get().position(), Vec2::new(40.0, 110.0));
    assert_eq!(abaaa.get().glyph_area().unwrap().scale, 1.0);

    let b = model.get(node_b_id).unwrap();
    assert_eq!(b.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(b.get().position(), Vec2::new(70.0, 70.0));
    assert_eq!(b.get().glyph_area().unwrap().scale, 1.0);

    let ba = model.get(node_ba_id).unwrap();
    assert_eq!(ba.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(ba.get().position(), Vec2::new(60.0, 80.0));
    assert_eq!(ba.get().glyph_area().unwrap().scale, 1.0);

    let baa = model.get(node_baa_id).unwrap();
    assert_eq!(baa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(baa.get().position(), Vec2::new(60.0, 90.0));
    assert_eq!(baa.get().glyph_area().unwrap().scale, 1.0);

    let baaa = model.get(node_baaa_id).unwrap();
    assert_eq!(baaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(baaa.get().position(), Vec2::new(60.0, 100.0));
    assert_eq!(baaa.get().glyph_area().unwrap().scale, 1.0);

    let baaaa = model.get(node_baaaa_id).unwrap();
    assert_eq!(baaaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(baaaa.get().position(), Vec2::new(60.0, 110.0));
    assert_eq!(baaaa.get().glyph_area().unwrap().scale, 1.0);

    let bb = model.get(node_bb_id).unwrap();
    assert_eq!(bb.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(bb.get().position(), Vec2::new(80.0, 80.0));
    assert_eq!(bb.get().glyph_area().unwrap().scale, 1.0);

    let bba = model.get(node_bba_id).unwrap();
    assert_eq!(bba.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(bba.get().position(), Vec2::new(80.0, 90.0));
    assert_eq!(bba.get().glyph_area().unwrap().scale, 1.0);

    let bbaa = model.get(node_bbaa_id).unwrap();
    assert_eq!(bbaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(bbaa.get().position(), Vec2::new(80.0, 100.0));
    assert_eq!(bbaa.get().glyph_area().unwrap().scale, 1.0);

    let bbaaa = model.get(node_bbaaa_id).unwrap();
    assert_eq!(bbaaa.get().glyph_area().unwrap().regions.num_regions(), 0);
    assert_eq!(bbaaa.get().position(), Vec2::new(80.0, 110.0));
    assert_eq!(bbaaa.get().glyph_area().unwrap().scale, 1.0);
}

#[test]
pub fn test_simple_tree_mutation() {
    simple_tree_mutation();
}

pub fn simple_tree_mutation() {
    // This is necessary to initialize lazy statics
    fonts::init();
    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let mut model: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();

    // Create the tree-model nodes
    let root_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "root",
            1.0,
            10.0,
            Vec2::new(50.0, 50.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        0,
    ));

    let node_a_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "A",
            1.0,
            10.0,
            Vec2::new(30.0, 70.0),
            Vec2::new(500.0, 500.0),
        ),
        1,
        1,
    ));

    let node_b_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "B",
            1.0,
            10.0,
            Vec2::new(70.0, 70.0),
            Vec2::new(500.0, 500.0),
        ),
        2,
        2,
    ));

    let node_c_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "C",
            1.0,
            10.0,
            Vec2::new(50.0, 70.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        3,
    ));

    let node_aa_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Aa",
            1.0,
            10.0,
            Vec2::new(20.0, 80.0),
            Vec2::new(500.0, 500.0),
        ),
        1,
        4,
    ));

    let node_ab_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Ab",
            1.0,
            10.0,
            Vec2::new(40.0, 80.0),
            Vec2::new(500.0, 500.0),
        ),
        1,
        5,
    ));

    let node_ca_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Ca",
            1.0,
            10.0,
            Vec2::new(50.0, 80.0),
            Vec2::new(500.0, 500.0),
        ),
        0,
        6,
    ));

    let node_ba_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Ba",
            1.0,
            10.0,
            Vec2::new(60.0, 80.0),
            Vec2::new(500.0, 500.0),
        ),
        1,
        7,
    ));

    let node_bb_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "Bb",
            1.0,
            10.0,
            Vec2::new(80.0, 80.0),
            Vec2::new(500.0, 500.0),
        ),
        2,
        8,
    ));

    // Set the relationships
    root_id.append(node_c_id, &mut model.arena);
    root_id.append(node_a_id, &mut model.arena);
    root_id.append(node_b_id, &mut model.arena);

    node_a_id.append(node_aa_id, &mut model.arena);
    node_a_id.append(node_ab_id, &mut model.arena);

    node_b_id.append(node_ba_id, &mut model.arena);
    node_b_id.append(node_bb_id, &mut model.arena);

    node_c_id.append(node_ca_id, &mut model.arena);

    // Create mutator-tree
    let mutator_root = mutator.arena.new_node(GfxMutator::new(
        Mutation::area_delta(DeltaGlyphArea::new(vec![
            GlyphAreaField::Operation(ApplyOperation::Add),
            GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![
                ColorFontRegion::new(Range::new(0, 1), Some(AppleTea), None),
            ])),
        ])),
        0,
    ));

    let mutator_a = mutator.arena.new_node(GfxMutator::new(
        Mutation::area_delta(make_mutator(
            Some(AliceInWonderland),
            None,
            Vec2::new(-1.0, -5.0),
        )),
        1,
    ));

    let mutator_b = mutator.arena.new_node(GfxMutator::new(
        Mutation::area_delta(make_mutator(Some(NorseBold), None, Vec2::new(-1.0, -5.0))),
        2,
    ));

    let mutator_c = mutator.arena.new_node(GfxMutator::new(
        Mutation::area_delta(make_mutator(
            Some(AliceInWonderland),
            None,
            Vec2::new(0.0, -4.0),
        )),
        0,
    ));

    let mutator_ca = mutator.arena.new_node(GfxMutator::new(
        Mutation::area_delta(make_mutator(Some(Casanova), None, Vec2::new(0.0, -4.0))),
        0,
    ));

    let mutator_ba = mutator.arena.new_node(GfxMutator::new(
        Mutation::area_delta(make_mutator(
            Some(Evilz),
            None,
            Vec2::new(-25.0, 25.0),
        )),
        1,
    ));

    let mutator_bb = mutator.arena.new_node(GfxMutator::new(
        Mutation::area_delta(make_mutator(Some(Evilz), None, Vec2::new(25.0, 25.0))),
        2,
    ));

    // Set mutator relationships
    mutator_root.append(mutator_c, &mut mutator.arena);
    mutator_root.append(mutator_a, &mut mutator.arena);
    mutator_root.append(mutator_b, &mut mutator.arena);

    mutator_b.append(mutator_ba, &mut mutator.arena);
    mutator_b.append(mutator_bb, &mut mutator.arena);

    mutator_c.append(mutator_ca, &mut mutator.arena);

    // Do mutation
    walk_tree_from(&mut model, &mut mutator, root_id, mutator_root);

    let root = model.get(root_id).unwrap();
    assert_eq!(root.get().position(), Vec2::new(50.0, 50.0));
    assert_eq!(
        root.get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AppleTea
    );

    let a = model.get(node_a_id).unwrap();
    assert_eq!(a.get().position(), Vec2::new(29.0, 65.0));
    assert_eq!(
        a.get()
            .glyph_area()
            .unwrap()
            .regions
            .hard_get(Range::new(0, 1))
            .font
            .unwrap(),
        AliceInWonderland
    );

    let b = model.get(node_b_id).unwrap();
    assert_element_delta(
        b.get(),
        vec![
            GlyphAreaField::position(69.0, 65.0),
            GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![
                ColorFontRegion::new(Range::new(0, 1), Some(NorseBold), None),
            ])),
        ],
    );

    let ba = model.get(node_ba_id).unwrap();
    assert_element_delta(
        ba.get(),
        vec![
            GlyphAreaField::position(35.0, 105.0),
            GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![
                ColorFontRegion::new(Range::new(0, 1), Some(Evilz), None),
            ])),
        ],
    );

    let bb = model.get(node_bb_id).unwrap();
    assert_element_delta(
        bb.get(),
        vec![
            GlyphAreaField::position(105.0, 105.0),
            GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![
                ColorFontRegion::new(Range::new(0, 1), Some(Evilz), None),
            ])),
        ],
    );

    let ca = model.get(node_ca_id).unwrap();
    assert_element_delta(
        ca.get(),
        vec![
            GlyphAreaField::position(50.0, 76.0),
            GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![
                ColorFontRegion::new(Range::new(0, 1), Some(Casanova), None),
            ])),
        ],
    );

    let c = model.get(node_c_id).unwrap();
    assert_element_delta(
        c.get(),
        vec![
            GlyphAreaField::position(50.0, 66.0),
            GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![
                ColorFontRegion::new(Range::new(0, 1), Some(AliceInWonderland), None),
            ])),
        ],
    );
}

fn make_mutator(font: Option<AppFont>, color: Option<FloatRgba>, position: Vec2) -> DeltaGlyphArea {
    DeltaGlyphArea::new(vec![
        GlyphAreaField::Operation(ApplyOperation::Add),
        GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![ColorFontRegion::new(
            Range::new(0, 1),
            font,
            color,
        )])),
        GlyphAreaField::position(position.x, position.y),
    ])
}

fn assert_element_delta(element: &GfxElement, fields: Vec<GlyphAreaField>) {
    for field in fields {
        match field {
            GlyphAreaField::Text(x) => {
                assert_eq!(element.glyph_area().unwrap().text, x);
            }
            GlyphAreaField::Scale(x) => {
                assert_eq!(element.glyph_area().unwrap().scale, x);
            }
            GlyphAreaField::Position(vec) => {
                assert_eq!(element.glyph_area().unwrap().position, vec);
            }
            GlyphAreaField::Bounds(vec) => {
                assert_eq!(element.glyph_area().unwrap().render_bounds, vec);
            }
            GlyphAreaField::ColorFontRegions(regions) => {
                assert_eq!(element.glyph_area().unwrap().regions, regions);
            }
            GlyphAreaField::LineHeight(height) => {
                assert_eq!(element.glyph_area().unwrap().line_height, height);
            }
            GlyphAreaField::Outline(outline) => {
                assert_eq!(element.glyph_area().unwrap().outline, outline);
            }
            GlyphAreaField::Shape(shape) => {
                assert_eq!(element.glyph_area().unwrap().shape, shape);
            }
            GlyphAreaField::ZoomVisibility(window) => {
                assert_eq!(element.glyph_area().unwrap().zoom_visibility, window);
            }
            GlyphAreaField::Operation(_) => {}
        }
    }
}
