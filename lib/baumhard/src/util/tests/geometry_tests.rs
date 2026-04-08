use glam::Vec2;
use crate::util::geometry::{almost_equal, almost_equal_vec2, clockwise_rotation_around_pivot, pixel_greater_or_equal,
                            pixel_greater_than, pixel_less_or_equal, pixel_lesser_than};

#[test]
fn test_90_deg_rotation() {
    do_90_deg_rotation();
}

pub fn do_90_deg_rotation() {
    let point = Vec2::new(1.0, 0.0);
    let pivot = Vec2::new(0.0, 0.0);
    let rotated = clockwise_rotation_around_pivot(point, pivot, 90.0);
    let expected = Vec2::new(0.0, -1.0);
    assert!(almost_equal_vec2(rotated, expected));
}

#[test]
fn test_180_deg_rotation() {
    do_180_deg_rotation();
}

pub fn do_180_deg_rotation() {
    let point = Vec2::new(1.0, 0.0);
    let pivot = Vec2::new(0.0, 0.0);
    let rotated = clockwise_rotation_around_pivot(point, pivot, 180.0);
    let expected = Vec2::new(-1.0, 0.0);
    assert!(almost_equal_vec2(rotated, expected));
}

#[test]
fn test_non_origin_pivot_rotation() {
    do_non_origin_pivot_rotation();
}

pub fn do_non_origin_pivot_rotation() {
    let point = Vec2::new(2.0, 2.0);
    let pivot = Vec2::new(1.0, 1.0);
    let rotated = clockwise_rotation_around_pivot(point, pivot, 90.0);
    let expected = Vec2::new(2.0, 0.0);
    assert_eq!(rotated, expected);
}

#[test]
fn test_0_deg_rotation() {
    do_0_deg_rotation();
}

pub fn do_0_deg_rotation() {
    let point = Vec2::new(1.0, 0.0);
    let pivot = Vec2::new(0.0, 0.0);
    let rotated = clockwise_rotation_around_pivot(point, pivot, 0.0);
    assert_eq!(rotated, point);
}

#[test]
fn test_pixel_functions() {
    do_pixel_functions();
}

pub fn do_pixel_functions() {
    assert!(pixel_greater_than((100.0, 100.0), (200.0, 90.0)));
    assert!(!pixel_greater_than((100.0, 100.0), (200.0, 110.0)));
    assert!(pixel_greater_than((105.0, 100.0), (100.0, 100.0)));
    assert!(pixel_greater_than((101.0, 100.0), (100.0, 100.0)));
    assert!(!pixel_greater_than((100.0, 100.0), (100.0, 100.0)));
    assert!(pixel_greater_or_equal((100.0, 100.0), (100.0, 100.0)));
    assert!(!pixel_greater_or_equal((100.0, 100.0), (100.0, 101.0)));
    assert!(pixel_greater_or_equal((100.0, 105.0), (100.0, 101.0)));
    assert!(pixel_greater_or_equal((100.0, 105.0), (100.0, 105.0)));
    assert!(pixel_greater_or_equal((101.0, 105.0), (100.0, 105.0)));
    assert!(!pixel_greater_or_equal((101.0, 105.0), (102.0, 105.0)));
    assert!(!pixel_lesser_than((100.0, 100.0), (100.0, 100.0)));
    assert!(!pixel_lesser_than((100.0, 100.0), (200.0, 99.0)));
    assert!(pixel_lesser_than((100.0, 100.0), (200.0, 100.0)));
    assert!(pixel_lesser_than((100.0, 100.0), (100.0, 101.0)));
    assert!(pixel_lesser_than((200.0, 10.0), (100.0, 101.0)));
    assert!(pixel_less_or_equal((200.0, 10.0), (100.0, 101.0)));
    assert!(pixel_less_or_equal((100.0, 100.0), (100.0, 100.0)));
    assert!(!pixel_less_or_equal((101.0, 100.0), (100.0, 100.0)));
    assert!(!pixel_less_or_equal((100.0, 101.0), (100.0, 100.0)));
    assert!(pixel_less_or_equal((100.0, 100.0), (101.0, 100.0)));
    assert!(pixel_less_or_equal((100.0, 100.0), (100.0, 101.0)));
}

#[test]
fn test_almost_equal() {
    do_almost_equal();
}

pub fn do_almost_equal() {
    // Test positive cases
    assert!(almost_equal(0.000001f32, 0.000002f32));
    assert!(almost_equal(1.000001f32, 1.000002f32));
    assert!(almost_equal(-1.000001f32, -1.000002f32));

    // Test negative cases
    assert!(!almost_equal(0.1f32, 0.2f32));
    assert!(!almost_equal(1.1f32, 1.2f32));
    assert!(!almost_equal(-1.1f32, -1.2f32));
    assert!(!almost_equal(95.0, 105.0));
    assert!(!almost_equal(105.0, 95.0));
}
#[test]
fn test_almost_equal_vec2() {
    do_almost_equal_vec2();
}

pub fn do_almost_equal_vec2() {
    // Test positive cases
    assert!(almost_equal_vec2(
        Vec2::new(0.000001f32, 0.000003f32),
        Vec2::new(0.000002f32, 0.000004f32)
    ));

    assert!(almost_equal_vec2(
        Vec2::new(1.000001f32, 1.000003f32),
        Vec2::new(1.000002f32, 1.000004f32)
    ));

    assert!(almost_equal_vec2(
        Vec2::new(-1.000001f32, -1.000003f32),
        Vec2::new(-1.000002f32, -1.000004f32)
    ));

    // Test negative cases
    assert!(!almost_equal_vec2(
        Vec2::new(0.1f32, 0.2f32),
        Vec2::new(0.2f32, 0.3f32)
    ));

    assert!(!almost_equal_vec2(
        Vec2::new(95.0, 150.0),
        Vec2::new(105.0, 150.0)
    ));

    assert!(!almost_equal_vec2(
        Vec2::new(1.1f32, 1.2f32),
        Vec2::new(1.2f32, 1.3f32)
    ));

    assert!(!almost_equal_vec2(
        Vec2::new(-1.1f32, -1.2f32),
        Vec2::new(-1.2f32, -1.3f32)
    ));
}
