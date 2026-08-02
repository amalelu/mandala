// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::gfx_structs::area`] primitives.
//!
//! Per [`crate::gfx_structs::tests::tree_tests`], the legacy
//! [`crate::gfx_structs::area::GlyphAreaCommand`] surface is exercised
//! end-to-end by `test_area_block_commands`. This file covers the
//! field-level mutators (`DeltaGlyphArea`) for additions made after
//! that test was written — currently the [`crate::gfx_structs::area::OutlineStyle`]
//! halo primitive.
//!
//! Follows the `do_*()` / `test_*()` split from
//! [`TEST_CONVENTIONS.md §T2.2`]: the body lives in a `pub fn do_*()`
//! so the criterion bench harness can reuse it; a thin
//! `#[test] pub fn test_*()` wrapper exposes it to `cargo test`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use glam::f32::Vec2;

use crate::core::primitives::{Applicable, ApplyOperation, ColorFontRegions, Range};
use crate::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaCommand, GlyphAreaField, OutlineStyle};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::model::GlyphModel;
use crate::gfx_structs::mutator::Mutation;
use crate::gfx_structs::shape::NodeShape;
use crate::util::geometry::{almost_equal_vec2, clockwise_rotation_around_pivot};
use crate::util::ordered_vec2::OrderedVec2;

/// A halo style suitable for "add a 3 px black outline" — the
/// picker's default. Reused across the outline tests.
fn sample_outline() -> OutlineStyle {
    OutlineStyle {
        color: [0, 0, 0, 255],
        px: 3.0,
    }
}

/// Round-trip: a `DeltaGlyphArea` carrying `Some(outline)` under
/// `Assign` writes the halo onto a previously-bare area; a follow-up
/// delta carrying `None` clears it. Pins the on/off semantics that
/// the renderer's tree walker depends on.
#[test]
pub fn test_outline_assign_round_trip() {
    do_outline_assign_round_trip();
}

pub fn do_outline_assign_round_trip() {
    let mut area = GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));

    // Assign a halo via the field-based mutator surface.
    let outline = sample_outline();
    let delta_set = DeltaGlyphArea::new(vec![
        GlyphAreaField::Outline(Some(outline)),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    area.apply_operation(&delta_set);
    assert_eq!(area.outline, Some(outline), "Assign should set the halo");

    // Clear it via another Assign with `None`.
    let delta_clear = DeltaGlyphArea::new(vec![
        GlyphAreaField::Outline(None),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    area.apply_operation(&delta_clear);
    assert!(area.outline.is_none(), "Assign(None) should clear the halo");
}

/// `Subtract` clears the halo regardless of payload — the semantic is
/// "remove what's there". Distinct from `Assign(None)` only in that it
/// reads as a removal operation at the call site (the renderer can use
/// it as a deselection-style mutator).
#[test]
pub fn test_outline_subtract_clears() {
    do_outline_subtract_clears();
}

pub fn do_outline_subtract_clears() {
    let mut area = GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));
    area.outline = Some(sample_outline());

    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Outline(Some(sample_outline())),
        GlyphAreaField::Operation(ApplyOperation::Subtract),
    ]);
    area.apply_operation(&delta);
    assert!(
        area.outline.is_none(),
        "Subtract should clear regardless of payload"
    );
}

/// Hash discrimination: two areas that differ only in their
/// `outline` field hash to different values. Without this, dirty-set
/// machinery downstream (which keys on `GlyphArea` hashing) would
/// fail to detect halo changes.
#[test]
pub fn test_outline_changes_hash() {
    do_outline_changes_hash();
}

pub fn do_outline_changes_hash() {
    let mut area_a =
        GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));
    let mut area_b = area_a.clone();
    area_b.outline = Some(sample_outline());

    let mut h_a = DefaultHasher::new();
    area_a.hash(&mut h_a);
    let mut h_b = DefaultHasher::new();
    area_b.hash(&mut h_b);
    assert_ne!(
        h_a.finish(),
        h_b.finish(),
        "outline difference must change GlyphArea hash"
    );

    // And vice versa: same outline → same hash.
    area_a.outline = Some(sample_outline());
    let mut h_a2 = DefaultHasher::new();
    area_a.hash(&mut h_a2);
    let mut h_b2 = DefaultHasher::new();
    area_b.hash(&mut h_b2);
    assert_eq!(h_a2.finish(), h_b2.finish());
}

/// Additive merge: two `Outline` deltas combined via the
/// `GlyphAreaField::Add` impl yield the rhs (last-writer-wins).
/// Halos are on/off, not blendable — the canonical way to
/// "compose" two halos in a delta sequence is for the later one to
/// override the earlier.
#[test]
pub fn test_outline_field_add_picks_rhs() {
    do_outline_field_add_picks_rhs();
}

pub fn do_outline_field_add_picks_rhs() {
    let lhs = GlyphAreaField::Outline(Some(OutlineStyle {
        color: [0, 0, 0, 255],
        px: 1.0,
    }));
    let rhs = GlyphAreaField::Outline(Some(OutlineStyle {
        color: [255, 255, 255, 255],
        px: 5.0,
    }));
    let combined = lhs + rhs.clone();
    assert_eq!(combined, rhs, "Add on Outline should pick rhs");
}

/// Canonical stamp pattern: `offsets()` yields 8 entries, each at
/// distance `px` from the origin (cardinals + diagonals). Pins the
/// outline technique that the renderer's walker depends on — a
/// future change that drops to 4 samples or moves to an uneven
/// radius must be a conscious call, not an accident.
#[test]
pub fn test_outline_offsets_canonical_8_stamp() {
    do_outline_offsets_canonical_8_stamp();
}

pub fn do_outline_offsets_canonical_8_stamp() {
    let style = OutlineStyle {
        color: [0, 0, 0, 255],
        px: 3.0,
    };
    let offsets: Vec<(f32, f32)> = style.offsets().collect();
    assert_eq!(offsets.len(), 8, "canonical pattern is 8 stamps");
    for (dx, dy) in &offsets {
        let r = (dx * dx + dy * dy).sqrt();
        assert!(
            (r - 3.0).abs() < 1e-4,
            "stamp at ({dx}, {dy}) has radius {r}, expected 3.0"
        );
    }
    // All stamps distinct — no duplicates would mean a wasted
    // cosmic-text shape.
    for i in 0..offsets.len() {
        for j in (i + 1)..offsets.len() {
            assert!(
                (offsets[i].0 - offsets[j].0).abs() > 1e-4 || (offsets[i].1 - offsets[j].1).abs() > 1e-4,
                "stamps {i} and {j} are duplicates"
            );
        }
    }
}

/// Newly-constructed `GlyphArea`s default to `NodeShape::Rectangle`.
/// Locks in the backwards-compatible posture: every pre-existing
/// call site builds through `new_with_str` and must keep rendering
/// / hit-testing as an axis-aligned box unless it opts in.
#[test]
pub fn test_shape_default_is_rectangle() {
    do_shape_default_is_rectangle();
}

pub fn do_shape_default_is_rectangle() {
    let area = GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));
    assert_eq!(area.shape, NodeShape::Rectangle);
}

/// Round-trip: a `DeltaGlyphArea` carrying `Shape(Ellipse)` under
/// `Assign` rewrites the area's shape; a follow-up `Assign`
/// carrying `Rectangle` reverts it. Pins the "assign replaces"
/// semantics that mutation authors rely on for shape swaps.
#[test]
pub fn test_shape_assign_round_trip() {
    do_shape_assign_round_trip();
}

pub fn do_shape_assign_round_trip() {
    let mut area = GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));

    let delta_set = DeltaGlyphArea::new(vec![
        GlyphAreaField::Shape(NodeShape::Ellipse),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    area.apply_operation(&delta_set);
    assert_eq!(area.shape, NodeShape::Ellipse, "Assign should set the shape");

    let delta_revert = DeltaGlyphArea::new(vec![
        GlyphAreaField::Shape(NodeShape::Rectangle),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    area.apply_operation(&delta_revert);
    assert_eq!(
        area.shape,
        NodeShape::Rectangle,
        "Assign should replace the shape outright"
    );
}

/// `Subtract` resets the shape to `Rectangle` regardless of the
/// delta's payload — distinct from `Outline::Subtract` (which
/// clears to `None`) because shape has no "unset" state; the
/// natural "remove what's there" target is the default.
#[test]
pub fn test_shape_subtract_resets_to_rectangle() {
    do_shape_subtract_resets_to_rectangle();
}

pub fn do_shape_subtract_resets_to_rectangle() {
    let mut area = GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));
    area.shape = NodeShape::Ellipse;

    // Payload is `Ellipse` but `Subtract` ignores it — the
    // semantic is "remove the custom shape", which lands on
    // Rectangle regardless.
    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Shape(NodeShape::Ellipse),
        GlyphAreaField::Operation(ApplyOperation::Subtract),
    ]);
    area.apply_operation(&delta);
    assert_eq!(area.shape, NodeShape::Rectangle);
}

/// Hash discrimination: two areas identical apart from `shape`
/// hash to different values. Dirty-set machinery downstream keys
/// on `GlyphArea` hashing, so without this a shape-only change
/// would be invisible to the renderer's "does this buffer need
/// reshaping?" check.
#[test]
pub fn test_shape_changes_hash() {
    do_shape_changes_hash();
}

pub fn do_shape_changes_hash() {
    let area_rect = GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));
    let mut area_ellipse = area_rect.clone();
    area_ellipse.shape = NodeShape::Ellipse;

    let mut h_rect = DefaultHasher::new();
    area_rect.hash(&mut h_rect);
    let mut h_ellipse = DefaultHasher::new();
    area_ellipse.hash(&mut h_ellipse);
    assert_ne!(
        h_rect.finish(),
        h_ellipse.finish(),
        "shape difference must change GlyphArea hash"
    );
}

/// Additive merge: two `Shape` deltas combined via the
/// `GlyphAreaField::Add` impl yield the rhs — shapes don't
/// compose arithmetically, so last-writer-wins is the only
/// meaningful semantic (same posture as Outline above).
#[test]
pub fn test_shape_field_add_picks_rhs() {
    do_shape_field_add_picks_rhs();
}

pub fn do_shape_field_add_picks_rhs() {
    let lhs = GlyphAreaField::Shape(NodeShape::Rectangle);
    let rhs = GlyphAreaField::Shape(NodeShape::Ellipse);
    let combined = lhs + rhs.clone();
    assert_eq!(combined, rhs);
}
// ── Mutation-surface completeness: area commands / deltas (P1-10) ────

/// `GlyphAreaCommand::ChangeRegionRange` must not panic when the
/// requested source range is missing — interactive mutation paths
/// must survive stale JSON-authored ranges.
#[test]
pub fn test_change_region_range_missing_region_warns_and_leaves_area_intact() {
    do_change_region_range_missing_region_warns_and_leaves_area_intact();
}

pub fn do_change_region_range_missing_region_warns_and_leaves_area_intact() {
    let mut area = GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));
    area.regions = ColorFontRegions::single_span(5, Some([1.0, 0.0, 0.0, 1.0]), None);

    let before = area.regions.clone();
    let stale_current = Range::new(99, 105);
    let new_range = Range::new(0, 5);
    let command = GlyphAreaCommand::ChangeRegionRange(stale_current, new_range);

    command.apply_to(&mut area);

    assert_eq!(
        area.regions, before,
        "missing-range command must not mutate regions"
    );
}

/// `ApplyOperation::Delete` on the `Text` field clears the area's
/// text. Part of the per-field operation-table work for P1-10.
#[test]
pub fn test_delta_text_delete_clears_text() {
    do_delta_text_delete_clears_text();
}

pub fn do_delta_text_delete_clears_text() {
    let mut area = GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));

    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Text("ignored".to_string()),
        GlyphAreaField::Operation(ApplyOperation::Delete),
    ]);
    delta.apply_to(&mut area);

    assert_eq!(area.text, "");
}

/// `ApplyOperation::Delete` on the `ColorFontRegions` field clears
/// the area's region set. Part of the per-field operation-table work
/// for P1-10.
#[test]
pub fn test_delta_regions_delete_clears_regions() {
    do_delta_regions_delete_clears_regions();
}

pub fn do_delta_regions_delete_clears_regions() {
    let mut area = GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));
    area.regions = ColorFontRegions::single_span(5, Some([1.0, 0.0, 0.0, 1.0]), None);

    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::ColorFontRegions(ColorFontRegions::default()),
        GlyphAreaField::Operation(ApplyOperation::Delete),
    ]);
    delta.apply_to(&mut area);

    assert_eq!(area.regions.num_regions(), 0);
}

/// `GlyphArea::rotate` must rotate *around* the pivot, not merely
/// rotate the pivot-relative vector: the translate-back step used to
/// be missing, so every call teleported the area toward the origin.
///
/// Clockwise 90° takes `+x` onto `-y` in screen space (`+y` points
/// down), so a point ten units right of the pivot lands ten units
/// above it — anchored to the pivot rather than to the origin.
#[test]
pub fn test_area_rotate_moves_position_around_pivot() {
    do_area_rotate_moves_position_around_pivot();
}

pub fn do_area_rotate_moves_position_around_pivot() {
    let mut area =
        GlyphArea::new_with_str("rot", 14.0, 14.0, Vec2::new(110.0, 100.0), Vec2::new(100.0, 20.0));

    area.rotate(Vec2::new(100.0, 100.0), 90.0);

    assert!(almost_equal_vec2(area.position(), Vec2::new(100.0, 90.0)));
}

/// Rotating about the area's own position is the identity — the
/// bit-exact case the missing translate-back turned into "jump to the
/// origin".
#[test]
pub fn test_area_rotate_about_self_is_identity() {
    do_area_rotate_about_self_is_identity();
}

pub fn do_area_rotate_about_self_is_identity() {
    let start = Vec2::new(500.0, 500.0);
    let mut area = GlyphArea::new_with_str("rot", 14.0, 14.0, start, Vec2::new(100.0, 20.0));

    area.rotate(start, 137.0);

    assert!(almost_equal_vec2(area.position(), start));
}

/// The three `rotate` siblings — `GlyphArea`, `GlyphModel`, and the
/// `GfxElement` wrapper — must agree on unit (degrees) and direction
/// (clockwise). They did not: the area took radians counterclockwise
/// and dropped the pivot entirely. Pinning the agreement here means a
/// future divergence fails the suite rather than silently drifting one
/// element type away from the other two.
#[test]
pub fn test_area_rotate_matches_siblings() {
    do_area_rotate_matches_siblings();
}

pub fn do_area_rotate_matches_siblings() {
    let start = Vec2::new(37.0, -11.0);
    let pivot = Vec2::new(-4.0, 9.0);
    let degrees = 41.5;

    let mut area = GlyphArea::new_with_str("rot", 14.0, 14.0, start, Vec2::new(100.0, 20.0));
    area.rotate(pivot, degrees);

    let mut model = GlyphModel::new();
    model.position = OrderedVec2::from_vec2(start);
    model.rotate(&pivot, &degrees);

    let mut element = GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str("rot", 14.0, 14.0, start, Vec2::ZERO),
        0,
        0,
    );
    element.rotate(pivot, degrees);

    assert!(almost_equal_vec2(area.position(), model.position.to_vec2()));
    assert!(almost_equal_vec2(area.position(), element.position()));
    assert!(almost_equal_vec2(
        area.position(),
        clockwise_rotation_around_pivot(start, pivot, degrees)
    ));
}

/// The rotation is reachable from the mutation pipeline through
/// [`GlyphAreaCommand::Rotate`], the area-side twin of
/// `GlyphModelCommand::Rotate`. A primitive no mutator can reach is a
/// primitive nothing exercises — which is how the missing
/// translate-back survived unnoticed.
#[test]
pub fn test_area_rotate_command_applies() {
    do_area_rotate_command_applies();
}

pub fn do_area_rotate_command_applies() {
    let mut area = GlyphArea::new_with_str("rot", 14.0, 14.0, Vec2::new(10.0, 0.0), Vec2::new(100.0, 20.0));

    GlyphAreaCommand::Rotate {
        pivot: Vec2::ZERO,
        degrees: 90.0,
    }
    .apply_to(&mut area);

    assert!(almost_equal_vec2(area.position(), Vec2::new(0.0, -10.0)));
}

/// The exact JSON an author has to type for the new command, pinned
/// against the wire.
///
/// `Rotate` is the first `GlyphAreaCommand` variant carrying a
/// `Vec2`, and glam serializes `Vec2` as a **2-element sequence**
/// (`serialize_tuple_struct`; its visitor implements only
/// `visit_seq`), *not* as an `{ "x": …, "y": … }` map. Nothing else
/// in the suite touches this command's serialization, so a doc
/// example in the wrong shape would sail past every other test and
/// then fail at load time — and `MindMap.custom_mutations` is a
/// required-shape field, so one bad example takes the whole document
/// down with it. This test is the guard: it asserts the emitted
/// string, and then **reads the example out of `format/mutations.md`
/// itself** and parses that, so editing the doc back to the object
/// form fails here. A hard-copied literal would only have pinned the
/// test against itself.
#[test]
pub fn test_area_rotate_command_json_wire_shape() {
    do_area_rotate_command_json_wire_shape();
}

/// Pull the published `Rotate` example out of `format/mutations.md`.
///
/// The doc writes it as a single inline-code span on one line, so the
/// span between the first pair of backticks on the line that names
/// both `AreaCommand` and `Rotate` *is* the example. Panics with a
/// pointed message if the doc no longer contains it — a silent
/// fallback here would defeat the purpose of reading the file.
fn documented_rotate_example() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../format/mutations.md");
    let doc = std::fs::read_to_string(path).expect("format/mutations.md must be readable");
    for line in doc.lines() {
        if !line.contains("\"AreaCommand\"") || !line.contains("\"Rotate\"") {
            continue;
        }
        let mut spans = line.split('`');
        // `split` yields the text before the first backtick, then the
        // span itself.
        if let (Some(_), Some(example)) = (spans.next(), spans.next()) {
            if example.contains("\"Rotate\"") {
                return example.to_string();
            }
        }
    }
    panic!("format/mutations.md no longer publishes an inline `AreaCommand`/`Rotate` example");
}

pub fn do_area_rotate_command_json_wire_shape() {
    let command = Mutation::AreaCommand(Box::new(GlyphAreaCommand::Rotate {
        pivot: Vec2::new(1.0, 2.0),
        degrees: 90.0,
    }));

    let emitted = serde_json::to_string(&command).expect("Rotate must serialize");
    assert_eq!(
        emitted,
        r#"{"AreaCommand":{"Rotate":{"pivot":[1.0,2.0],"degrees":90.0}}}"#
    );

    // Read, do not restate: this is the byte sequence the doc actually
    // publishes today, whatever that is.
    let documented = documented_rotate_example();
    let parsed: Mutation = serde_json::from_str(&documented).unwrap_or_else(|e| {
        panic!("the example published in format/mutations.md must parse: {e}\n{documented}")
    });
    match parsed {
        Mutation::AreaCommand(command) => match *command {
            GlyphAreaCommand::Rotate { pivot, degrees } => {
                assert_eq!(pivot, Vec2::ZERO);
                assert_eq!(degrees, 90.0);
            }
            other => panic!("expected Rotate, got {:?}", other),
        },
        other => panic!("expected AreaCommand, got {:?}", other),
    }

    // The `{ "x": …, "y": … }` shape is *not* accepted by glam's
    // `Vec2` deserializer. Pinned so a future doc edit that reaches
    // for the intuitive-looking map shape fails here first.
    let map_shaped = r#"{"AreaCommand": { "Rotate": { "pivot": { "x": 0.0, "y": 0.0 }, "degrees": 90.0 } }}"#;
    assert!(serde_json::from_str::<Mutation>(map_shaped).is_err());
}
