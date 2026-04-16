//! Tests for [`crate::gfx_structs::camera::Camera2D`] — viewport math
//! fundamentals (§T1).
//!
//! Camera is platform-shared logic that must behave identically on
//! native and WASM. These tests cover the canvas↔screen coordinate
//! round-trip, zoom-at-cursor invariant, pan, fit-to-bounds (including
//! degenerate inputs), and `CameraMutation` dispatch.
//!
//! Follows the `do_*()` / `test_*()` split from §T2.2: every public
//! body is benchmarkable from `benches/test_bench.rs`.

use glam::Vec2;

use crate::gfx_structs::camera::{Camera2D, CameraMutation};
use crate::util::geometry::almost_equal_vec2;

// ── round-trip ──────────────────────────────────────────────────────

#[test]
fn test_canvas_to_screen_round_trips() {
    do_canvas_to_screen_round_trips();
}

/// `canvas_to_screen` → `screen_to_canvas` gives back the original
/// point within floating-point epsilon, across several camera states.
pub fn do_canvas_to_screen_round_trips() {
    let configs: Vec<(Vec2, f32)> = vec![
        (Vec2::ZERO, 1.0),
        (Vec2::new(100.0, -200.0), 1.5),
        (Vec2::new(-500.0, 300.0), 0.25),
        (Vec2::new(0.0, 0.0), 3.0),
        (Vec2::new(120.0, -80.0), 0.5),
    ];
    let test_points = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(50.0, 75.0),
        Vec2::new(-300.0, 400.0),
        Vec2::new(1000.0, -1000.0),
    ];

    for (pos, zoom) in &configs {
        let mut cam = Camera2D::new(1920, 1080);
        cam.position = *pos;
        cam.zoom = *zoom;

        for pt in &test_points {
            let screen = cam.canvas_to_screen(*pt);
            let back = cam.screen_to_canvas(screen);
            assert!(
                almost_equal_vec2(back, *pt),
                "Round-trip failed for camera pos={pos}, zoom={zoom}, point={pt}: got {back}"
            );
        }
    }
}

// ── identity at default ─────────────────────────────────────────────

#[test]
fn test_screen_to_canvas_identity_at_default() {
    do_screen_to_canvas_identity_at_default();
}

/// With default camera (position=0, zoom=1), screen coords equal
/// canvas coords offset by the viewport center — i.e.
/// `screen_to_canvas(screen_center) == Vec2::ZERO`.
pub fn do_screen_to_canvas_identity_at_default() {
    let cam = Camera2D::new(800, 600);
    let screen_center = Vec2::new(400.0, 300.0);

    // The viewport center maps to canvas origin.
    let canvas = cam.screen_to_canvas(screen_center);
    assert!(
        almost_equal_vec2(canvas, Vec2::ZERO),
        "Expected canvas origin at screen center, got {canvas}"
    );

    // An arbitrary screen point maps to (screen - center) in canvas.
    let screen_pt = Vec2::new(500.0, 350.0);
    let expected_canvas = screen_pt - screen_center; // (100, 50)
    let actual_canvas = cam.screen_to_canvas(screen_pt);
    assert!(
        almost_equal_vec2(actual_canvas, expected_canvas),
        "Expected {expected_canvas}, got {actual_canvas}"
    );
}

// ── zoom-at preserves point under cursor ────────────────────────────

#[test]
fn test_zoom_at_preserves_point_under_cursor() {
    do_zoom_at_preserves_point_under_cursor();
}

/// After `zoom_at(cursor, factor)`, the canvas point that was under
/// the cursor before the zoom is still under the cursor after.
pub fn do_zoom_at_preserves_point_under_cursor() {
    let factors = [1.5_f32, 2.0, 0.5, 0.8, 3.5];
    let cursors = [
        Vec2::new(200.0, 150.0),
        Vec2::new(0.0, 0.0),
        Vec2::new(799.0, 599.0),
        Vec2::new(400.0, 300.0),
    ];

    for factor in &factors {
        for cursor in &cursors {
            let mut cam = Camera2D::new(800, 600);
            cam.position = Vec2::new(50.0, -30.0);
            cam.zoom = 1.2;

            let canvas_before = cam.screen_to_canvas(*cursor);
            cam.zoom_at(*cursor, *factor);
            let canvas_after = cam.screen_to_canvas(*cursor);

            assert!(
                almost_equal_vec2(canvas_before, canvas_after),
                "zoom_at({cursor}, {factor}) moved canvas point under cursor: \
                 {canvas_before} → {canvas_after}"
            );
        }
    }
}

// ── pan shifts viewport ─────────────────────────────────────────────

#[test]
fn test_pan_shifts_viewport() {
    do_pan_shifts_viewport();
}

/// `pan(dx, dy)` moves the camera offset by `-delta / zoom` in canvas
/// space (dragging right moves the view right, so the camera position
/// moves left).
pub fn do_pan_shifts_viewport() {
    let mut cam = Camera2D::new(800, 600);
    cam.zoom = 2.0;
    let pos_before = cam.position;

    let delta = Vec2::new(100.0, -50.0);
    cam.pan(delta);

    let expected = pos_before - delta / cam.zoom;
    assert!(
        almost_equal_vec2(cam.position, expected),
        "Expected position {expected}, got {}",
        cam.position
    );
}

// ── fit_to_bounds with a single element ─────────────────────────────

#[test]
fn test_fit_to_bounds_with_single_element() {
    do_fit_to_bounds_with_single_element();
}

/// `fit_camera_to_bounds` with a small bounding box produces a zoom
/// level and offset that centre the content and scale it to fill the
/// usable viewport area.
pub fn do_fit_to_bounds_with_single_element() {
    let mut cam = Camera2D::new(800, 600);
    let min = Vec2::new(90.0, 190.0);
    let max = Vec2::new(110.0, 210.0);
    let padding = 0.05_f32;

    cam.fit_to_bounds(min, max, padding);

    // Camera position should be at the centre of the bounding box.
    let expected_center = (min + max) / 2.0; // (100, 200)
    assert!(
        almost_equal_vec2(cam.position, expected_center),
        "Expected position {expected_center}, got {}",
        cam.position
    );

    // The zoom should scale the 20x20 box into the usable viewport
    // area. Usable width = 800 * 0.9 = 720, usable height = 600 * 0.9
    // = 540. Zoom = min(720/20, 540/20) = 27.0, but clamped to
    // MAX_ZOOM.
    assert_eq!(cam.zoom, Camera2D::MAX_ZOOM);
}

// ── fit_to_bounds with empty/zero-size box ──────────────────────────

#[test]
fn test_fit_to_bounds_empty() {
    do_fit_to_bounds_empty();
}

/// Fitting to an empty (zero-size) bounding box must not panic and
/// should produce a sane camera state — position centred on the
/// degenerate point, zoom unchanged (the `if canvas_size > 0` guard
/// skips the zoom calculation).
pub fn do_fit_to_bounds_empty() {
    let mut cam = Camera2D::new(800, 600);
    let original_zoom = cam.zoom;

    let point = Vec2::new(42.0, -17.0);
    cam.fit_to_bounds(point, point, 0.1);

    // Position should centre on the single point.
    assert!(
        almost_equal_vec2(cam.position, point),
        "Expected position {point}, got {}",
        cam.position
    );

    // Zoom should be unchanged — the zero-size guard leaves it alone.
    assert_eq!(cam.zoom, original_zoom);
}

// ── CameraMutation::Pan ─────────────────────────────────────────────

#[test]
fn test_camera_mutation_pan() {
    do_camera_mutation_pan();
}

/// Applying `CameraMutation::Pan { screen_delta }` via
/// `apply_mutation` shifts the camera position by
/// `-screen_delta / zoom`, identical to calling `pan()` directly.
pub fn do_camera_mutation_pan() {
    let mut cam = Camera2D::new(800, 600);
    cam.position = Vec2::new(10.0, 20.0);
    cam.zoom = 1.5;
    let pos_before = cam.position;

    let delta = Vec2::new(30.0, -45.0);
    cam.apply_mutation(&CameraMutation::Pan { screen_delta: delta });

    let expected = pos_before - delta / 1.5;
    assert!(
        almost_equal_vec2(cam.position, expected),
        "Pan mutation: expected {expected}, got {}",
        cam.position
    );
}

// ── CameraMutation::ZoomAt ──────────────────────────────────────────

#[test]
fn test_camera_mutation_zoom() {
    do_camera_mutation_zoom();
}

/// Applying `CameraMutation::ZoomAt` via `apply_mutation` adjusts
/// the zoom level and preserves the canvas point under the focus.
pub fn do_camera_mutation_zoom() {
    let mut cam = Camera2D::new(800, 600);
    cam.position = Vec2::new(50.0, -30.0);
    cam.zoom = 1.0;

    let focus = Vec2::new(300.0, 200.0);
    let canvas_before = cam.screen_to_canvas(focus);

    cam.apply_mutation(&CameraMutation::ZoomAt {
        screen_focus: focus,
        factor: 2.0,
    });

    // Zoom should have doubled.
    assert!(
        (cam.zoom - 2.0).abs() < 1e-6,
        "Expected zoom 2.0, got {}",
        cam.zoom
    );

    // The canvas point under the focus should be unchanged.
    let canvas_after = cam.screen_to_canvas(focus);
    assert!(
        almost_equal_vec2(canvas_before, canvas_after),
        "ZoomAt mutation moved focus point: {canvas_before} → {canvas_after}"
    );
}
