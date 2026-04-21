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

use crate::core::primitives::ApplyOperation;
use crate::gfx_structs::area::{
    DeltaGlyphArea, GlyphArea, GlyphAreaField, OutlineStyle,
};
use crate::gfx_structs::shape::NodeShape;

/// A halo style suitable for "add a 3 px black outline" — the
/// picker's default. Reused across the outline tests.
fn sample_outline() -> OutlineStyle {
    OutlineStyle { color: [0, 0, 0, 255], px: 3.0 }
}

/// Newly-constructed `GlyphArea`s default to no halo. A consumer
/// that never sets `outline` keeps the existing behavior — important
/// because every existing call site (mindmap nodes, console, palette,
/// borders) constructs through `new_with_str` and must not pay the
/// halo-shaping cost they didn't ask for.
#[test]
pub fn test_outline_default_is_none() {
    do_outline_default_is_none();
}

pub fn do_outline_default_is_none() {
    let area = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );
    assert!(area.outline.is_none(), "GlyphArea should default to no halo");
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
    let mut area = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );

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
    let mut area = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );
    area.outline = Some(sample_outline());

    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Outline(Some(sample_outline())),
        GlyphAreaField::Operation(ApplyOperation::Subtract),
    ]);
    area.apply_operation(&delta);
    assert!(area.outline.is_none(), "Subtract should clear regardless of payload");
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
    let mut area_a = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );
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
    let style = OutlineStyle { color: [0, 0, 0, 255], px: 3.0 };
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
                (offsets[i].0 - offsets[j].0).abs() > 1e-4
                    || (offsets[i].1 - offsets[j].1).abs() > 1e-4,
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
    let area = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );
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
    let mut area = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );

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
    let mut area = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );
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
    let area_rect = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );
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
