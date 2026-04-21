//! Tests for [`crate::gfx_structs::zoom_visibility`] — inclusive
//! `[min, max]` containment, `None`-as-open-bound semantics, and the
//! `GlyphArea` mutator round-trip via `GlyphAreaField::ZoomVisibility`.
//!
//! Follows the `do_*()` / `test_*()` split from
//! [`TEST_CONVENTIONS.md §T2.2`]: the body lives in a `pub fn do_*()`
//! so the criterion bench harness can reuse it; a thin
//! `#[test] pub fn test_*()` wrapper exposes it to `cargo test`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use glam::f32::Vec2;

use crate::core::primitives::ApplyOperation;
use crate::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaField};
use crate::gfx_structs::zoom_visibility::ZoomVisibility;

/// The default value — both bounds `None` — advertises itself as
/// default via `is_default()`. This is the gate
/// `#[serde(skip_serializing_if = "ZoomVisibility::is_default")]`
/// relies on; without it, every historical `GlyphArea` would start
/// emitting a new JSON key and break byte-identical roundtrip of
/// pre-existing fixtures.
#[test]
pub fn test_default_is_unbounded() {
    do_default_is_unbounded();
}

pub fn do_default_is_unbounded() {
    let v = ZoomVisibility::default();
    assert!(v.min.is_none());
    assert!(v.max.is_none());
    assert!(v.is_default());
    assert_eq!(v, ZoomVisibility::unbounded());
}

/// Unbounded windows render at every zoom the camera can reach, from
/// `MIN_ZOOM` through `MAX_ZOOM` (camera.rs). Pins the "default means
/// always visible" contract that existing maps depend on.
#[test]
pub fn test_unbounded_contains_full_camera_range() {
    do_unbounded_contains_full_camera_range();
}

pub fn do_unbounded_contains_full_camera_range() {
    let v = ZoomVisibility::unbounded();
    // Samples span the camera's clamped range.
    for z in [0.05_f32, 0.5, 1.0, 2.5, 5.0] {
        assert!(v.contains(z), "unbounded should contain {z}");
    }
    // And well beyond, so a future camera retune doesn't silently
    // break the contract.
    assert!(v.contains(0.0));
    assert!(v.contains(100.0));
}

/// `min`-only window: renders at `min` (inclusive) and above; the
/// "zoom in for detail" half of the Google-Maps-style layering
/// use case.
#[test]
pub fn test_min_only_is_inclusive() {
    do_min_only_is_inclusive();
}

pub fn do_min_only_is_inclusive() {
    let v = ZoomVisibility { min: Some(1.5), max: None };
    assert!(!v.contains(1.0));
    assert!(!v.contains(1.4999));
    assert!(v.contains(1.5), "min should be inclusive");
    assert!(v.contains(1.5001));
    assert!(v.contains(5.0));
}

/// `max`-only window: renders at `max` (inclusive) and below; the
/// "zoom out for landmark" half of the layering use case.
#[test]
pub fn test_max_only_is_inclusive() {
    do_max_only_is_inclusive();
}

pub fn do_max_only_is_inclusive() {
    let v = ZoomVisibility { min: None, max: Some(0.5) };
    assert!(v.contains(0.05));
    assert!(v.contains(0.4999));
    assert!(v.contains(0.5), "max should be inclusive");
    assert!(!v.contains(0.5001));
    assert!(!v.contains(1.0));
}

/// Closed window: both bounds set. Renders only inside the
/// inclusive band.
#[test]
pub fn test_closed_window_renders_inside_band() {
    do_closed_window_renders_inside_band();
}

pub fn do_closed_window_renders_inside_band() {
    let v = ZoomVisibility { min: Some(0.5), max: Some(2.0) };
    assert!(!v.contains(0.25));
    assert!(v.contains(0.5));
    assert!(v.contains(1.0));
    assert!(v.contains(2.0));
    assert!(!v.contains(2.5));
}

/// Degenerate `min == max`: the band collapses to a single point;
/// `contains` still returns `true` at that point (inclusive on
/// both sides). Documents the corner so a future reader doesn't
/// "fix" it into a half-open interval.
#[test]
pub fn test_single_point_band_is_inclusive() {
    do_single_point_band_is_inclusive();
}

pub fn do_single_point_band_is_inclusive() {
    let v = ZoomVisibility { min: Some(1.0), max: Some(1.0) };
    assert!(v.contains(1.0));
    assert!(!v.contains(0.9999));
    assert!(!v.contains(1.0001));
}

/// Inverted band (`min > max`): `contains` still returns a
/// well-defined boolean (always `false`) rather than panicking.
/// The verifier catches this at load-time — see the `maptool
/// verify` check in commit 5 — but the render-loop path must not
/// panic per `CODE_CONVENTIONS.md` §9.
#[test]
pub fn test_inverted_band_never_contains() {
    do_inverted_band_never_contains();
}

pub fn do_inverted_band_never_contains() {
    let v = ZoomVisibility { min: Some(2.0), max: Some(0.5) };
    for z in [0.05_f32, 0.5, 1.0, 2.0, 5.0] {
        assert!(!v.contains(z));
    }
}

/// `DeltaGlyphArea` round-trip: an `Assign` delta writes a window
/// onto a previously-unbounded area; a follow-up `Assign` with
/// unbounded clears it. Pins the mutator-surface contract that
/// custom mutations depend on for zoom-triggered LOD transitions.
#[test]
pub fn test_zoom_visibility_assign_round_trip() {
    do_zoom_visibility_assign_round_trip();
}

pub fn do_zoom_visibility_assign_round_trip() {
    let mut area = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );
    assert!(area.zoom_visibility.is_default());

    let window = ZoomVisibility { min: Some(1.0), max: Some(2.5) };
    let delta_set = DeltaGlyphArea::new(vec![
        GlyphAreaField::ZoomVisibility(window),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    area.apply_operation(&delta_set);
    assert_eq!(area.zoom_visibility, window);

    let delta_clear = DeltaGlyphArea::new(vec![
        GlyphAreaField::ZoomVisibility(ZoomVisibility::unbounded()),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    area.apply_operation(&delta_clear);
    assert!(area.zoom_visibility.is_default());
}

/// `Subtract` resets the window to unbounded regardless of payload
/// — the "remove the zoom gate" semantic. Matches the Shape
/// precedent (Subtract resets to Rectangle) rather than Outline
/// (Subtract clears to None) because the default value here is
/// itself "no gate".
#[test]
pub fn test_zoom_visibility_subtract_resets_to_unbounded() {
    do_zoom_visibility_subtract_resets_to_unbounded();
}

pub fn do_zoom_visibility_subtract_resets_to_unbounded() {
    let mut area = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );
    area.zoom_visibility = ZoomVisibility { min: Some(1.0), max: Some(2.0) };

    // Payload is non-default but Subtract ignores it.
    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::ZoomVisibility(ZoomVisibility { min: Some(0.1), max: None }),
        GlyphAreaField::Operation(ApplyOperation::Subtract),
    ]);
    area.apply_operation(&delta);
    assert!(area.zoom_visibility.is_default());
}

/// Additive merge picks the rhs — windows don't compose, same
/// last-writer-wins posture as `Outline` and `Shape`.
#[test]
pub fn test_zoom_visibility_field_add_picks_rhs() {
    do_zoom_visibility_field_add_picks_rhs();
}

pub fn do_zoom_visibility_field_add_picks_rhs() {
    let lhs = GlyphAreaField::ZoomVisibility(ZoomVisibility { min: Some(0.5), max: None });
    let rhs = GlyphAreaField::ZoomVisibility(ZoomVisibility { min: None, max: Some(2.0) });
    let combined = lhs + rhs.clone();
    assert_eq!(combined, rhs);
}

/// Hash discrimination: two areas identical apart from
/// `zoom_visibility` hash to different values. Dirty-set
/// machinery downstream keys on `GlyphArea` hashing, so without
/// this a window-only change would be invisible to the renderer's
/// "does this buffer need reshaping?" check.
#[test]
pub fn test_zoom_visibility_changes_hash() {
    do_zoom_visibility_changes_hash();
}

pub fn do_zoom_visibility_changes_hash() {
    let area_a = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );
    let mut area_b = area_a.clone();
    area_b.zoom_visibility = ZoomVisibility { min: Some(1.0), max: Some(2.0) };

    let mut h_a = DefaultHasher::new();
    area_a.hash(&mut h_a);
    let mut h_b = DefaultHasher::new();
    area_b.hash(&mut h_b);
    assert_ne!(
        h_a.finish(),
        h_b.finish(),
        "zoom_visibility difference must change GlyphArea hash"
    );
}

/// Serde: the default value serializes to `{}` (both bounds
/// skipped) and a default-only `GlyphArea` doesn't mention
/// `zoom_visibility` in its JSON. This is what keeps existing
/// fixtures byte-identical on roundtrip (`CODE_CONVENTIONS.md`
/// §10 — no backwards-compat shims means the old shape is also
/// the new shape for unchanged data).
#[test]
pub fn test_zoom_visibility_default_is_skipped_in_json() {
    do_zoom_visibility_default_is_skipped_in_json();
}

pub fn do_zoom_visibility_default_is_skipped_in_json() {
    let area = GlyphArea::new_with_str(
        "hi",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0, 10.0),
    );
    let json = serde_json::to_string(&area).expect("serialize");
    assert!(
        !json.contains("zoom_visibility"),
        "default zoom_visibility should be omitted, got: {json}"
    );

    // And a non-default window does show up.
    let mut gated = area.clone();
    gated.zoom_visibility = ZoomVisibility { min: Some(1.0), max: None };
    let gated_json = serde_json::to_string(&gated).expect("serialize");
    assert!(gated_json.contains("zoom_visibility"));
    assert!(gated_json.contains("\"min\":1.0"));
}
