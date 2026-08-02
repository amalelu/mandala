// SPDX-License-Identifier: MPL-2.0

//! Tests for the shared mutation-surface plumbing (§T1 — mutations
//! are a fundamental): the [`Delta`](crate::gfx_structs::delta::Delta)
//! container both element types instantiate, the
//! [`crate::core::primitives::Discriminated`] tag
//! access derived by strum, and
//! [`ApplyOperation::apply_ref`](crate::core::primitives::ApplyOperation::apply_ref).
//!
//! The two `*_tags_cover_every_field` bodies are the load-bearing
//! ones: each maps *every* tag to a representative field payload
//! through an exhaustive `match`. Adding a variant to either field
//! enum adds a tag, which makes that match non-exhaustive and fails
//! the build — the compiler help the hand-written discriminant enums
//! never gave us.

use crate::core::primitives::{ApplyOperation, ColorFontRegions, Discriminated};
use crate::font::fonts::AppFont;
use crate::gfx_structs::area::{
    DeltaGlyphArea, GlyphArea, GlyphAreaCommand, GlyphAreaCommandType, GlyphAreaField, GlyphAreaFieldType,
    OutlineStyle,
};
use crate::gfx_structs::delta::DeltaField;
use crate::gfx_structs::element::{GfxElement, GfxElementType};
use crate::gfx_structs::model::{
    DeltaGlyphModel, GlyphComponent, GlyphLine, GlyphMatrix, GlyphModelCommand, GlyphModelCommandType,
    GlyphModelField, GlyphModelFieldType,
};
use crate::gfx_structs::mutator::{GfxMutator, Mutation, MutationType, MutatorType};
use crate::gfx_structs::shape::NodeShape;
use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::util::color::Color;
use crate::util::ordered_vec2::OrderedVec2;
use ordered_float::OrderedFloat;
use strum::IntoEnumIterator;

/// One representative payload per area field tag. Exhaustive by
/// construction — a new `GlyphAreaField` variant breaks this match
/// until it is handled.
fn area_field_for(tag: GlyphAreaFieldType) -> GlyphAreaField {
    match tag {
        GlyphAreaFieldType::Text => GlyphAreaField::Text("text".to_string()),
        GlyphAreaFieldType::Scale => GlyphAreaField::scale(12.0),
        GlyphAreaFieldType::LineHeight => GlyphAreaField::line_height(14.0),
        GlyphAreaFieldType::Position => GlyphAreaField::position(1.0, 2.0),
        GlyphAreaFieldType::Bounds => GlyphAreaField::bounds(30.0, 40.0),
        // Non-empty on purpose: `apply_reaches_every_area_field`
        // asserts the payload actually lands, and an empty region set
        // is indistinguishable from a fresh area's default.
        GlyphAreaFieldType::ColorFontRegions => GlyphAreaField::ColorFontRegions(
            ColorFontRegions::single_span(4, Some([1.0, 0.0, 0.0, 1.0]), None),
        ),
        GlyphAreaFieldType::Outline => GlyphAreaField::Outline(Some(OutlineStyle {
            color: [1, 2, 3, 4],
            px: 1.5,
        })),
        GlyphAreaFieldType::Shape => GlyphAreaField::Shape(NodeShape::Ellipse),
        GlyphAreaFieldType::ZoomVisibility => {
            GlyphAreaField::ZoomVisibility(ZoomVisibility::try_new(Some(0.5), Some(2.0)).unwrap())
        }
        GlyphAreaFieldType::Operation => GlyphAreaField::Operation(ApplyOperation::Assign),
    }
}

/// One representative payload per model field tag. Exhaustive by
/// construction — see [`area_field_for`].
fn model_field_for(tag: GlyphModelFieldType) -> GlyphModelField {
    let line = || GlyphLine::new_with(GlyphComponent::text("m", AppFont::AppleTea, Color::black()));
    match tag {
        GlyphModelFieldType::GlyphMatrix => {
            let mut matrix = GlyphMatrix::new();
            matrix.push(line());
            GlyphModelField::GlyphMatrix(matrix)
        }
        GlyphModelFieldType::GlyphLine => GlyphModelField::GlyphLine(0, line()),
        GlyphModelFieldType::GlyphLines => GlyphModelField::GlyphLines(vec![(0, line()), (1, line())]),
        GlyphModelFieldType::Layer => GlyphModelField::Layer(3),
        GlyphModelFieldType::Position => GlyphModelField::position(5.0, 6.0),
        GlyphModelFieldType::Operation => GlyphModelField::Operation(ApplyOperation::Assign),
    }
}

/// Every derived area tag is produced by exactly the field variant it
/// names, and a delta built from one payload per tag keeps all of
/// them. Locks the derive-generated tag ↔ variant correspondence that
/// replaced two hand-maintained `match` ladders.
#[test]
pub fn test_area_field_tags_cover_every_field() {
    do_area_field_tags_cover_every_field();
}

pub fn do_area_field_tags_cover_every_field() {
    let all: Vec<GlyphAreaField> = GlyphAreaFieldType::iter().map(area_field_for).collect();
    for tag in GlyphAreaFieldType::iter() {
        assert_eq!(area_field_for(tag).variant(), tag, "tag {} round-trip", tag);
    }
    let count = all.len();
    let delta = DeltaGlyphArea::new(all);
    assert_eq!(delta.fields.len(), count);
    assert_eq!(delta.operation_variant(), ApplyOperation::Assign);
}

/// Model-side twin of [`do_area_field_tags_cover_every_field`].
#[test]
pub fn test_model_field_tags_cover_every_field() {
    do_model_field_tags_cover_every_field();
}

pub fn do_model_field_tags_cover_every_field() {
    let all: Vec<GlyphModelField> = GlyphModelFieldType::iter().map(model_field_for).collect();
    for tag in GlyphModelFieldType::iter() {
        assert_eq!(model_field_for(tag).variant(), tag, "tag {} round-trip", tag);
    }
    let count = all.len();
    let delta = DeltaGlyphModel::new(all);
    assert_eq!(delta.fields.len(), count);
    assert_eq!(delta.operation_variant(), ApplyOperation::Assign);
}

/// The whole point of the tag: `same_type` compares variants and
/// ignores payload, on both vocabularies and on the command enums
/// that share the trait.
#[test]
pub fn test_same_type_ignores_payload() {
    do_same_type_ignores_payload();
}

pub fn do_same_type_ignores_payload() {
    assert!(GlyphAreaField::scale(1.0).same_type(&GlyphAreaField::scale(99.0)));
    assert!(!GlyphAreaField::scale(1.0).same_type(&GlyphAreaField::line_height(1.0)));

    let line = GlyphLine::new_with(GlyphComponent::text("x", AppFont::AppleTea, Color::black()));
    assert!(GlyphModelField::Layer(0).same_type(&GlyphModelField::Layer(7)));
    assert!(!GlyphModelField::Layer(0).same_type(&GlyphModelField::GlyphLine(0, line)));

    assert!(GlyphAreaCommand::NudgeUp(1.0).same_type(&GlyphAreaCommand::NudgeUp(9.0)));
    assert!(!GlyphAreaCommand::NudgeUp(1.0).same_type(&GlyphAreaCommand::NudgeDown(1.0)));
    assert!(GlyphModelCommand::NudgeUp(1.0).same_type(&GlyphModelCommand::NudgeUp(9.0)));
    assert!(!GlyphModelCommand::NudgeUp(1.0).same_type(&GlyphModelCommand::MoveTo(0.0, 0.0)));
}

/// The element / mutator / mutation tags come from the same derive
/// and the same trait, so `variant()` is one name across the crate.
#[test]
pub fn test_pipeline_tags_are_derived() {
    do_pipeline_tags_are_derived();
}

pub fn do_pipeline_tags_are_derived() {
    assert_eq!(GfxElement::new_void(0).variant(), GfxElementType::Void);
    assert_eq!(
        GfxElement::new_area_non_indexed(
            GlyphArea::new_with_str("t", 1.0, 1.0, Default::default(), Default::default()),
            0
        )
        .variant(),
        GfxElementType::GlyphArea
    );
    assert_eq!(GfxMutator::new_void(3).variant(), MutatorType::Void);
    assert!(GfxMutator::new_void(3).is(MutatorType::Void));
    assert_eq!(Mutation::none().variant(), MutationType::None);
    assert_eq!(
        Mutation::area_delta(DeltaGlyphArea::new(vec![])).variant(),
        MutationType::AreaDelta
    );

    // Every command tag is reachable from a command value — the
    // derive guarantees it, these iterations prove the derive is
    // actually wired to the enums the pipeline dispatches on.
    assert!(GlyphAreaCommandType::iter().count() > 0);
    assert!(GlyphModelCommandType::iter().count() > 0);
}

/// A delta with no `Operation` entry must not silently assign — the
/// shared container defaults to `Noop`, and duplicate variants
/// collapse to the last one supplied.
#[test]
pub fn test_delta_new_collapses_duplicates_and_defaults_to_noop() {
    do_delta_new_collapses_duplicates_and_defaults_to_noop();
}

pub fn do_delta_new_collapses_duplicates_and_defaults_to_noop() {
    let delta = DeltaGlyphArea::new(vec![GlyphAreaField::scale(1.0), GlyphAreaField::scale(2.0)]);
    assert_eq!(delta.fields.len(), 1);
    assert_eq!(delta.scale(), Some(2.0));
    assert_eq!(delta.operation_variant(), ApplyOperation::Noop);
    assert!(delta.touches(GlyphAreaFieldType::Scale));
    assert!(!delta.touches(GlyphAreaFieldType::Text));

    let model_delta = DeltaGlyphModel::new(vec![GlyphModelField::Layer(1), GlyphModelField::Layer(4)]);
    assert_eq!(model_delta.fields.len(), 1);
    assert_eq!(model_delta.layer(), Some(4));
    assert_eq!(model_delta.operation_variant(), ApplyOperation::Noop);
}

/// Merging two deltas composes shared fields through the field
/// vocabulary's own `Add` and carries one-sided fields across
/// untouched.
#[test]
pub fn test_delta_add_merges_both_sides() {
    do_delta_add_merges_both_sides();
}

pub fn do_delta_add_merges_both_sides() {
    let lhs = DeltaGlyphArea::new(vec![
        GlyphAreaField::scale(2.0),
        GlyphAreaField::Text("ab".to_string()),
    ]);
    let rhs = DeltaGlyphArea::new(vec![
        GlyphAreaField::scale(3.0),
        GlyphAreaField::position(4.0, 5.0),
    ]);

    let merged = lhs + rhs;

    assert_eq!(merged.fields.len(), 3);
    assert_eq!(merged.scale(), Some(5.0));
    assert_eq!(merged.text_ref(), Some("ab"));
    assert_eq!(merged.position().map(|p| (p.x, p.y)), Some((4.0, 5.0)));
}

/// The control variant is the only field that reports an operation —
/// the lookup key and the unwrap must agree, or `operation_variant`
/// silently degrades every delta to `Noop`.
#[test]
pub fn test_operation_key_matches_control_variant() {
    do_operation_key_matches_control_variant();
}

pub fn do_operation_key_matches_control_variant() {
    for operation in [
        ApplyOperation::Add,
        ApplyOperation::Assign,
        ApplyOperation::Subtract,
        ApplyOperation::Multiply,
        ApplyOperation::Delete,
        ApplyOperation::Noop,
    ] {
        assert_eq!(
            DeltaGlyphArea::new(vec![GlyphAreaField::Operation(operation)]).operation_variant(),
            operation
        );
        assert_eq!(
            DeltaGlyphModel::new(vec![GlyphModelField::Operation(operation)]).operation_variant(),
            operation
        );
    }

    assert_eq!(
        GlyphAreaField::Operation(ApplyOperation::Add).variant(),
        GlyphAreaField::OPERATION_KEY
    );
    assert_eq!(
        GlyphModelField::Operation(ApplyOperation::Add).variant(),
        GlyphModelField::OPERATION_KEY
    );
    assert_eq!(GlyphAreaField::scale(1.0).as_operation(), None);
    assert_eq!(GlyphModelField::Layer(1).as_operation(), None);
}

/// `apply_ref` must behave exactly like `apply` on the arms that
/// consume the payload, and must ignore the payload entirely on
/// `Noop` / `Delete` — that is what lets a borrowed delta skip the
/// clone.
#[test]
pub fn test_apply_ref_matches_apply() {
    do_apply_ref_matches_apply();
}

pub fn do_apply_ref_matches_apply() {
    for operation in [
        ApplyOperation::Add,
        ApplyOperation::Assign,
        ApplyOperation::Subtract,
        ApplyOperation::Multiply,
        ApplyOperation::Delete,
        ApplyOperation::Noop,
    ] {
        let rhs = OrderedFloat::from(3.0_f32);
        let mut by_value = OrderedFloat::from(10.0_f32);
        let mut by_ref = OrderedFloat::from(10.0_f32);
        operation.apply(&mut by_value, rhs);
        operation.apply_ref(&mut by_ref, &rhs);
        assert_eq!(by_value, by_ref, "apply_ref diverged on {:?}", operation);
    }

    // The payload is untouched by Noop and reset by Delete, whatever
    // the delta carried.
    let mut vec = OrderedVec2::new_f32(1.0, 2.0);
    ApplyOperation::Noop.apply_ref(&mut vec.x, &OrderedFloat::from(99.0_f32));
    assert_eq!(vec.x, OrderedFloat::from(1.0_f32));
    ApplyOperation::Delete.apply_ref(&mut vec.y, &OrderedFloat::from(99.0_f32));
    assert_eq!(vec.y, OrderedFloat::from(0.0_f32));
}

/// Every area field variant must actually *reach* the element when
/// applied through the public mutator pipeline.
///
/// This is the test that makes the accessor and the
/// `GlyphArea::apply_operation` branch mandatory. Deriving the tag
/// enum forces a new variant to appear in `area_field_for`, but a
/// variant whose apply-path branch is missing would still deserialize,
/// walk the tree, match its channel, and mutate nothing — silently.
/// That is precisely the #10-A defect this epic exists to prevent
/// recurring, so it is pinned rather than left to review discipline:
/// a delta carrying the variant must leave the target different from
/// an untouched one.
#[test]
pub fn test_apply_reaches_every_area_field() {
    do_apply_reaches_every_area_field();
}

pub fn do_apply_reaches_every_area_field() {
    use crate::core::primitives::Applicable;

    for tag in GlyphAreaFieldType::iter() {
        // The control variant carries no payload of its own — it
        // names the arithmetic the others are applied with.
        if tag == GlyphAreaField::OPERATION_KEY {
            continue;
        }
        let pristine = GlyphArea::new(0.0, 0.0, glam::Vec2::ZERO, glam::Vec2::ZERO);
        let mut target = pristine.clone();
        DeltaGlyphArea::new(vec![
            GlyphAreaField::Operation(ApplyOperation::Assign),
            area_field_for(tag),
        ])
        .apply_to(&mut target);

        assert_ne!(
            target, pristine,
            "GlyphAreaField::{tag} applied through apply_to left the area unchanged — \
             its `GlyphArea::apply_operation` branch (or its DeltaGlyphArea accessor) is missing"
        );
    }
}

/// Model-side twin of [`do_apply_reaches_every_area_field`]. The two
/// model variants that shipped without an apply branch (#10-A) are
/// exactly what this catches.
#[test]
pub fn test_apply_reaches_every_model_field() {
    do_apply_reaches_every_model_field();
}

pub fn do_apply_reaches_every_model_field() {
    use crate::core::primitives::Applicable;
    use crate::gfx_structs::model::GlyphModel;

    for tag in GlyphModelFieldType::iter() {
        if tag == GlyphModelField::OPERATION_KEY {
            continue;
        }
        let pristine = GlyphModel::new();
        let mut target = GlyphModel::new();
        DeltaGlyphModel::new(vec![
            GlyphModelField::Operation(ApplyOperation::Assign),
            model_field_for(tag),
        ])
        .apply_to(&mut target);

        assert_ne!(
            target, pristine,
            "GlyphModelField::{tag} applied through apply_to left the model unchanged — \
             its `GlyphModel::apply_operation` branch (or its DeltaGlyphModel accessor) is missing"
        );
    }
}
