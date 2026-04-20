//! Hashable 2D float vector — the wrapper baumhard uses wherever
//! `glam::Vec2` would sit in a `HashMap` key, a `BTreeSet`, or the
//! mutation system's keyed collections. `glam::Vec2` lacks `Hash` /
//! `Eq` because `f32` is not totally ordered; `OrderedVec2` wraps
//! each axis in `ordered_float::OrderedFloat` to make those traits
//! available without giving up component-wise arithmetic.

use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub, SubAssign};
use glam::Vec2;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

/// Hashable, `Eq`-able 2D float vector. Wraps each component in
/// [`OrderedFloat`] so instances can live inside hash maps, `BTreeSet`s,
/// and the mutation system's keyed collections — `glam::Vec2` lacks
/// `Hash` / `Eq` because `f32` is not totally-ordered. Arithmetic ops
/// are component-wise; no NaN handling beyond what `OrderedFloat`
/// provides.
#[derive(Clone, Copy, Hash, Eq, Debug, Serialize, Deserialize)]
pub struct OrderedVec2 {
    pub x: OrderedFloat<f32>,
    pub y: OrderedFloat<f32>,
}

impl PartialEq for OrderedVec2 {
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x && self.y == other.y
    }
}

impl OrderedVec2 {
    /// Construct from a `glam::Vec2`. O(1).
    pub fn from_vec2(vec2: Vec2) -> Self {
        Self::new_f32(vec2.x, vec2.y)
    }

    /// Construct from already-wrapped [`OrderedFloat`] components.
    /// O(1).
    pub fn new(x: OrderedFloat<f32>, y: OrderedFloat<f32>) -> Self {
        OrderedVec2 {
            x: OrderedFloat::from(x),
            y: OrderedFloat::from(y),
        }
    }

    /// Construct from raw `f32` components. O(1).
    pub fn new_f32(x: f32, y: f32) -> Self {
        OrderedVec2 {
            x: OrderedFloat::from(x),
            y: OrderedFloat::from(y),
        }
    }

    /// Unwrap the x component as `f32`.
    pub fn x(&self) -> f32 {
        self.x.0
    }

    /// Unwrap the y component as `f32`.
    pub fn y(&self) -> f32 {
        self.y.0
    }

    /// Convert to `glam::Vec2`. O(1).
    pub fn to_vec2(&self) -> Vec2 {
        Vec2::new(self.x.0, self.y.0)
    }

    /// Convert to a `(x, y)` `f32` tuple. O(1).
    pub fn to_pair(&self) -> (f32, f32) {
        (self.x.0, self.y.0)
    }
}

impl SubAssign for OrderedVec2 {
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
    }
}

impl AddAssign for OrderedVec2 {
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl MulAssign for OrderedVec2 {
    fn mul_assign(&mut self, rhs: Self) {
        self.x *= rhs.x;
        self.y *= rhs.y;
    }
}

impl DivAssign for OrderedVec2 {
    fn div_assign(&mut self, rhs: Self) {
        self.x /= rhs.x;
        self.y /= rhs.y;
    }
}

impl Add for OrderedVec2 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        OrderedVec2::new_f32(self.x.0 + rhs.x.0, self.y.0 + rhs.y.0)
    }
}

impl Sub for OrderedVec2 {
    type Output = OrderedVec2;

    fn sub(self, rhs: Self) -> Self::Output {
        OrderedVec2::new_f32(self.x.0 - rhs.x.0, self.y.0 - rhs.y.0)
    }
}

impl Mul for OrderedVec2 {
    type Output = OrderedVec2;

    fn mul(self, rhs: Self) -> Self::Output {
        OrderedVec2::new_f32(self.x.0 * rhs.x.0, self.y.0 * rhs.y.0)
    }
}

impl Div for OrderedVec2 {
    type Output = OrderedVec2;

    fn div(self, rhs: Self) -> Self::Output {
        OrderedVec2::new_f32(self.x.0 / rhs.x.0, self.y.0 / rhs.y.0)
    }
}
