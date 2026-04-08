use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct HitBox {
    pub rectangles: Vec<BoundingRectangle>,
}

impl HitBox {
    pub fn new() -> Self {
        HitBox { rectangles: vec![] }
    }

    pub fn add(&mut self, rectangle: BoundingRectangle) {
        self.rectangles.push(rectangle)
    }

    pub fn copy_from(&mut self, other: &HitBox) {
        self.rectangles.extend_from_slice(&other.rectangles)
    }

    pub fn clear(&mut self) {
        self.rectangles.clear()
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
pub struct BoundingRectangle {
    pub delta_x: f32,
    pub delta_y: f32,
    pub length: f32,
    pub width: f32,
}

impl BoundingRectangle {
    pub fn at_origin(length: f32, width: f32) -> Self {
        BoundingRectangle {
            delta_x: 0.0,
            delta_y: 0.0,
            length,
            width,
        }
    }
}
