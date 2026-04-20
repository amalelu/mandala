//! Small-scale 2D geometry helpers: rotation around a pivot,
//! epsilon-aware float comparisons, and pixel-space ordering.

use glam::{Mat3, Vec2};

/// Rotate `a` clockwise by `degrees` around `pivot`, returning the
/// transformed point. Uses `glam::Mat3::from_rotation_z` internally;
/// O(1).
pub fn clockwise_rotation_around_pivot(a: Vec2, pivot: Vec2, degrees: f32) -> Vec2 {
   let translated = a - pivot;
   let radians = -degrees.to_radians();
   let rotation = Mat3::from_rotation_z(radians);
   rotation.transform_point2(translated) + pivot
}

const ERROR_TOLERANCE_ALMOST_EQUAL: f32 = 1e-5;

/// `|a - b| <= 1e-5`. The baumhard-wide epsilon for "close enough"
/// between two `f32`s.
pub fn almost_equal(a: f32, b: f32) -> bool {
   (a - b).abs() <= ERROR_TOLERANCE_ALMOST_EQUAL
}

/// Logical inverse of [`almost_equal`]. Named "pretty" because it
/// treats within-epsilon pairs as equal rather than fighting with raw
/// `!=` comparisons at float boundaries.
pub fn pretty_inequal(a: f32, b: f32) -> bool {
   !almost_equal(a, b)
}

/// Pixel-reading-order `>=` on `(x, y)` pairs: y-dominant, x as
/// tie-breaker, using [`almost_equal`] for the equality test.
pub fn pixel_greater_or_equal(a_greater_or: (f32, f32), equal_b: (f32, f32)) -> bool {
   pixel_greater_than(a_greater_or, equal_b) || (
      almost_equal(a_greater_or.0, equal_b.0) && almost_equal(a_greater_or.1, equal_b.1))
}

/// Pixel-reading-order `>` on `(x, y)` pairs: if the y components are
/// almost-equal, compare x; otherwise compare y. Matches how a cursor
/// walks a page of glyphs.
pub fn pixel_greater_than(a_greater: (f32, f32), than_b: (f32, f32)) -> bool {
   if almost_equal(a_greater.1, than_b.1) {
      a_greater.0 > than_b.0
   } else {
      a_greater.1 > than_b.1
   }
}

/// Pixel-reading-order `<=` on `(x, y)` pairs. Mirror of
/// [`pixel_greater_or_equal`].
pub fn pixel_less_or_equal(a_less_or: (f32, f32), equal_b: (f32, f32)) -> bool {
   pixel_lesser_than(a_less_or, equal_b) || (
      almost_equal(a_less_or.0, equal_b.0) && almost_equal(a_less_or.1, equal_b.1))
}

/// Pixel-reading-order `<` on `(x, y)` pairs. Mirror of
/// [`pixel_greater_than`].
pub fn pixel_lesser_than(a_lesser: (f32, f32), than_b: (f32, f32)) -> bool {
   if almost_equal(a_lesser.1, than_b.1) {
      a_lesser.0 < than_b.0
   } else {
      a_lesser.1 < than_b.1
   }
}

/// Area of the rectangle whose width / height are the x / y
/// components of `vec`. O(1).
pub fn vec2_area(vec: Vec2) -> f32 {
   vec.x * vec.y
}

/// Component-wise [`pretty_inequal`] on two vectors: true if either
/// component pair is outside the `almost_equal` tolerance.
pub fn pretty_inequal_vec2(vec1: Vec2, vec2: Vec2) -> bool {
   pretty_inequal(vec1.x, vec2.x) || pretty_inequal(vec1.y, vec2.y)
}

/// Component-wise [`almost_equal`] on two vectors: true iff both
/// component pairs are within tolerance.
pub fn almost_equal_vec2(vec1: Vec2, vec2: Vec2) -> bool {
   almost_equal(vec1.x, vec2.x) && almost_equal(vec1.y, vec2.y)
}
