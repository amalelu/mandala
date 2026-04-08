use glam::{Mat3, Vec2};

pub fn clockwise_rotation_around_pivot(a: Vec2, pivot: Vec2, degrees: f32) -> Vec2 {
   // Translate to origin
   let translated = a - pivot;

   // Rotation matrix
   let radians = -degrees.to_radians();
   let rotation = Mat3::from_rotation_z(radians);

   // Rotate around origin and translate back
   let result = rotation.transform_point2(translated) + pivot;

   result
}

const ERROR_TOLERANCE_ALMOST_EQUAL: f32 = 1e-5;

pub fn almost_equal(a: f32, b: f32) -> bool {
   (a - b).abs() <= ERROR_TOLERANCE_ALMOST_EQUAL
}

pub fn pretty_inequal(a: f32, b: f32) -> bool {
   !almost_equal(a, b)
}

pub fn pixel_greater_or_equal(a_greater_or: (f32, f32), equal_b: (f32, f32)) -> bool {
   pixel_greater_than(a_greater_or, equal_b) || (
      almost_equal(a_greater_or.0, equal_b.0) && almost_equal(a_greater_or.1, equal_b.1))
}

pub fn pixel_greater_than(a_greater: (f32, f32), than_b: (f32, f32)) -> bool {
   if almost_equal(a_greater.1, than_b.1) {
      a_greater.0 > than_b.0
   } else {
      a_greater.1 > than_b.1
   }
}

pub fn pixel_less_or_equal(a_less_or: (f32, f32), equal_b: (f32, f32)) -> bool {
   pixel_lesser_than(a_less_or, equal_b) || (
      almost_equal(a_less_or.0, equal_b.0) && almost_equal(a_less_or.1, equal_b.1))
}

pub fn pixel_lesser_than(a_lesser: (f32, f32), than_b: (f32, f32)) -> bool {
   if almost_equal(a_lesser.1, than_b.1) {
      a_lesser.0 < than_b.0
   } else {
      a_lesser.1 < than_b.1
   }
}

pub fn vec2_area(vec: Vec2) -> f32 {
   vec.x * vec.y
}

pub fn pretty_inequal_vec2(vec1: Vec2, vec2: Vec2) -> bool {
   pretty_inequal(vec1.x, vec2.x) || pretty_inequal(vec1.y, vec2.y)
}

pub fn almost_equal_vec2(vec1: Vec2, vec2: Vec2) -> bool {
   almost_equal(vec1.x, vec2.x) && almost_equal(vec1.y, vec2.y)
}
