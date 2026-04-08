use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub, SubAssign};
use glam::Vec2;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

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
    pub fn from_vec2(vec2: Vec2) -> Self {
        Self::new_f32(vec2.x, vec2.y)
    }

    pub fn new(x: OrderedFloat<f32>, y: OrderedFloat<f32>) -> Self {
        OrderedVec2 {
            x: OrderedFloat::from(x),
            y: OrderedFloat::from(y),
        }
    }

    pub fn new_f32(x: f32, y: f32) -> Self {
        OrderedVec2 {
            x: OrderedFloat::from(x),
            y: OrderedFloat::from(y),
        }
    }

    pub fn x(&self) -> f32 {
        self.x.0
    }

    pub fn y(&self) -> f32 {
        self.y.0
    }

    pub fn to_vec2(&self) -> Vec2 {
        Vec2::new(self.x.0, self.y.0)
    }

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
