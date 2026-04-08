use std::ops::{Add, Div, Index, IndexMut, Mul, Sub};
use serde::{Deserialize, Serialize};

#[macro_export]
macro_rules! rgba {
    ([$r:expr, $g:expr, $b:expr, $a:expr]) => {{
        [
            ($r as f32) / 255.0,
            ($g as f32) / 255.0,
            ($b as f32) / 255.0,
            ($a as f32) / 255.0,
        ]
    }};
}

#[macro_export]
macro_rules! rgb {
    ([$r:expr, $g:expr, $b:expr]) => {{
        [
            ($r as f32) / 255.0,
            ($g as f32) / 255.0,
            ($b as f32) / 255.0,
            1.0,
        ]
    }};
}

#[macro_export]
macro_rules! hex {
    ($color:expr) => {{
        let color = $color.trim_start_matches('#');
        let length = color.len();
        let rgb_iter = (0..length)
            .step_by(2)
            .map(|i| u8::from_str_radix(&color[i..i + 2], 16).unwrap_or(0));

        let mut rgba = [0.0; 4];
        for (i, c) in rgb_iter.enumerate() {
            rgba[i] = c as f32 / 255.0;
        }

        if length == 6 {
            rgba[3] = 1.0;
        }

        rgba
    }};
}

fn hex_char_to_value(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => panic!("Invalid character in color code"),
    }
}

pub fn convert_f32_to_u8(color: &[f32; 4]) -> [u8; 4] {
    let mut u8_color = [0u8; 4];
    for (i, &float_val) in color.iter().enumerate() {
        // Convert the f32 value to u8 by scaling up to 255
        // Using `saturating_mul` to ensure it doesn't overflow the u8 range
        u8_color[i] = (float_val * 255.0).round() as u8;
    }
    u8_color
}

pub fn hex_to_rgba(color: &str) -> [f32; 4] {
    let color = color.trim_start_matches('#');
    let length = color.len();

    if length == 6 || length == 8 {
        let mut rgba = [0.0; 4];
        let mut byte_iter = color.bytes();

        for i in 0..(length / 2) {
            let high_nibble = hex_char_to_value(byte_iter.next().unwrap()) << 4;
            let low_nibble = hex_char_to_value(byte_iter.next().unwrap());
            rgba[i] = (high_nibble | low_nibble) as f32 / 255.0;
        }

        if length == 6 {
            rgba[3] = 1.0;
        }

        rgba
    } else {
        panic!("Invalid color length, expected 6 or 8 characters");
    }
}

pub fn from_hex(colors: &[&str]) -> Vec<[f32; 4]> {
    let mut rgba_colors: Vec<[f32; 4]> = Vec::with_capacity(colors.len());
    for color in colors.iter() {
        rgba_colors.push(hex_to_rgba(color));
    }
    rgba_colors
}

pub fn add_rgba(a: &FloatRgba, b: &FloatRgba) -> [f32; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

pub type FloatRgba = [f32; 4];
pub type Rgba = [u8; 4];
pub type Palette = Vec<FloatRgba>;

pub const ALPHA_IDX: usize = 3;
pub const BLUE_IDX: usize = 2;
pub const GREEN_IDX: usize = 1;
pub const RED_IDX: usize = 0;
pub const VAL_MAX: u8 = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub rgba: Rgba,
}

impl Div for Color {
    type Output = Color;

    fn div(self, rhs: Self) -> Self::Output {
        let mut result = self[0].wrapping_div(rhs[0]);
        let mut output = [0; 4];
        output[0] = result;
        result = self[1].wrapping_div(rhs[1]);
        output[1] = result;
        result = self[2].wrapping_div(rhs[2]);
        output[2] = result;
        result = self[3].wrapping_div(rhs[3]);
        output[3] = result;
        Color::new_u8(&output)
    }
}

impl Mul for Color {
    type Output = Color;

    fn mul(self, rhs: Self) -> Self::Output {
        let mut result = self[0].wrapping_mul(rhs[0]);
        let mut output = [0; 4];
        output[0] = result;
        result = self[1].wrapping_mul(rhs[1]);
        output[1] = result;
        result = self[2].wrapping_mul(rhs[2]);
        output[2] = result;
        result = self[3].wrapping_mul(rhs[3]);
        output[3] = result;
        Color::new_u8(&output)
    }
}

impl Sub for Color {
    type Output = Color;

    fn sub(self, rhs: Self) -> Self::Output {
        let mut result = self[0].wrapping_sub(rhs[0]);
        let mut output = [0; 4];
        output[0] = result;
        result = self[1].wrapping_sub(rhs[1]);
        output[1] = result;
        result = self[2].wrapping_sub(rhs[2]);
        output[2] = result;
        result = self[3].wrapping_sub(rhs[3]);
        output[3] = result;
        Color::new_u8(&output)
    }
}

impl Add for Color {
    type Output = Color;

    fn add(self, rhs: Self) -> Self::Output {
        let mut result = self[0].wrapping_add(rhs[0]);
        let mut output = [0; 4];
        output[0] = result;
        result = self[1].wrapping_add(rhs[1]);
        output[1] = result;
        result = self[2].wrapping_add(rhs[2]);
        output[2] = result;
        result = self[3].wrapping_add(rhs[3]);
        output[3] = result;
        Color::new_u8(&output)
    }
}

impl IndexMut<usize> for Color {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.rgba[index]
    }
}

impl Index<usize> for Color {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        &self.rgba[index]
    }
}

impl Color {
    pub fn black() -> Self {
        Color {
            rgba: [0, 0, 0, 255],
        }
    }

    pub fn invisible() -> Self {
        Color {
            rgba: [0, 0, 0, 0],
        }
    }

    pub fn white() -> Self {
        Color {
            rgba: [255, 255, 255, 255],
        }
    }

    pub fn new_u8(rgba: &Rgba) -> Self {
        Color { rgba: *rgba }
    }

    pub fn new_f32(float_rgba: &FloatRgba) -> Self {
        Color {
            rgba: convert_f32_to_u8(float_rgba),
        }
    }
    pub fn set_alpha(&mut self, opacity: u8) {
        self.rgba[ALPHA_IDX] = opacity;
    }

    pub fn to_float(&self) -> FloatRgba {
        [
            (self.rgba[RED_IDX] / VAL_MAX).into(),
            (self.rgba[GREEN_IDX] / VAL_MAX).into(),
            (self.rgba[BLUE_IDX] / VAL_MAX).into(),
            (self.rgba[ALPHA_IDX] / VAL_MAX).into(),
        ]
    }
}
