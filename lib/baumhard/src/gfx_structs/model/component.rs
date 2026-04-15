//! `GlyphComponent` + `GlyphComponentField` — the leaf of the glyph
//! model hierarchy: one contiguous run of text sharing a font and a
//! colour. A `GlyphLine` is a `Vec<GlyphComponent>`; a `GlyphMatrix`
//! is a `Vec<GlyphLine>`; a `GlyphModel` wraps the matrix.

use crate::font::fonts::AppFont;
use crate::util::color::{Color, FloatRgba};
use crate::util::grapheme_chad::{
    count_grapheme_clusters, delete_back_unicode, delete_front_unicode, split_off_graphemes,
};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::ops::{Add, AddAssign, MulAssign};

/// Each variant represents a field in GlyphComponent, used for mutators
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum GlyphComponentField {
    Text(String),
    Font(AppFont),
    Color(FloatRgba),
}

#[derive(Serialize, Debug, Eq, PartialEq, Deserialize, Clone)]
pub struct GlyphComponent {
    pub text: String,
    pub font: AppFont,
    pub color: Color,
}

impl Hash for GlyphComponent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.font.hash(state);
        self.color.rgba.hash(state);
    }
}

impl MulAssign for GlyphComponent {
    fn mul_assign(&mut self, rhs: Self) {
        if !rhs.text.is_empty() {
            self.text = rhs.text.clone();
        }
        if rhs.font != AppFont::Any {
            self.font = rhs.font;
        }
        // Colors
        let mut result = self.color[0].wrapping_mul(rhs.color[0]);
        self.color[0] = result;
        result = self.color[1].wrapping_mul(rhs.color[1]);
        self.color[1] = result;
        result = self.color[2].wrapping_mul(rhs.color[2]);
        self.color[2] = result;
        result = self.color[3].wrapping_mul(rhs.color[3]);
        self.color[3] = result;
    }
}

impl AddAssign for GlyphComponent {
    fn add_assign(&mut self, rhs: Self) {
        if !rhs.text.is_empty() {
            self.text = self.text.clone() + &*rhs.text;
        }
        if self.font == AppFont::Any {
            self.font = rhs.font;
        }
        let mut result = self.color[0].wrapping_add(rhs.color[0]);
        self.color[0] = result;
        result = self.color[1].wrapping_add(rhs.color[1]);
        self.color[1] = result;
        result = self.color[2].wrapping_add(rhs.color[2]);
        self.color[2] = result;
        result = self.color[3].wrapping_add(rhs.color[3]);
        self.color[3] = result;
    }
}

impl Add for GlyphComponent {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        let mut output = self.clone();
        output += rhs;
        output
    }
}

impl GlyphComponent {
    pub fn text(text: &str, font: AppFont, color: Color) -> Self {
        GlyphComponent {
            text: text.to_string(),
            font,
            color,
        }
    }

    pub fn space(n: usize) -> Self {
        GlyphComponent {
            text: " ".repeat(n),
            font: AppFont::Any,
            color: Color::invisible(),
        }
    }

    pub fn split_off(&mut self, at_idx: usize) -> Self {
        let split_str = split_off_graphemes(&mut self.text, at_idx);
        GlyphComponent {
            text: split_str,
            font: self.font,
            color: self.color,
        }
    }

    pub fn space_front(&mut self, n: usize) {
        self.pad_front(" ", n);
    }

    pub fn space_back(&mut self, n: usize) {
        self.pad_back(" ", n);
    }

    pub fn pad_front(&mut self, pad: &str, n: usize) {
        let padding = pad.repeat(n);
        self.text.insert_str(0, &padding);
    }

    pub fn pad_back(&mut self, pad: &str, n: usize) {
        let padding = pad.repeat(n);
        self.text.push_str(&padding);
    }

    pub fn contains_non_space(&self) -> bool {
        self.text.chars().any(|c| !c.is_whitespace())
    }

    pub fn index_of_first_non_space_char(&self) -> Option<usize> {
        self.text
            .chars()
            .enumerate()
            .find(|&(_, c)| !c.is_whitespace())
            .map(|(i, _)| i)
    }

    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    /// Returns number of grapheme clusters
    pub fn length(&self) -> usize {
        count_grapheme_clusters(&self.text)
    }

    /// This works on unicode grapheme clusters
    pub fn discard_front(&mut self, num: usize) {
        delete_front_unicode(&mut self.text, num);
    }

    /// This works on unicode grapheme clusters
    pub fn discard_back(&mut self, num: usize) {
        delete_back_unicode(&mut self.text, num);
    }
}
