// SPDX-License-Identifier: MPL-2.0

//! Core color type and arithmetic. The conversion utilities
//! (hex/RGB/HSV, theme variable resolution) live in the companion
//! `super::color_conversion` module, and are the single way a string
//! becomes a color:
//! [`hex_to_rgba_safe`](crate::util::color_conversion::hex_to_rgba_safe)
//! for the degrade-on-garbage posture the interactive paths need,
//! [`hex_to_rgba`](crate::util::color_conversion::hex_to_rgba) when
//! the caller wants to see the failure.

use serde::{Deserialize, Serialize};
use std::ops::{Add, Div, Index, IndexMut, Mul, Sub};

// Re-export every public item from color_conversion so existing
// `use baumhard::util::color::*` imports continue to resolve.
pub use super::color_conversion::*;

/// `[R, G, B, A]` in `[0.0, 1.0]` — the canvas-space color
/// representation consumed by the renderer. Plain array, zero
/// allocation, `Copy`.
pub type FloatRgba = [f32; 4];
/// `[R, G, B, A]` in `[0, 255]` — the byte-packed form used by
/// [`Color`] and by hex parsing. Plain array, zero allocation,
/// `Copy`.
pub type Rgba = [u8; 4];

/// Index of the alpha channel in an [`Rgba`] / [`FloatRgba`] quad.
pub const ALPHA_IDX: usize = 3;
/// Index of the blue channel in an [`Rgba`] / [`FloatRgba`] quad.
pub const BLUE_IDX: usize = 2;
/// Index of the green channel in an [`Rgba`] / [`FloatRgba`] quad.
pub const GREEN_IDX: usize = 1;
/// Index of the red channel in an [`Rgba`] / [`FloatRgba`] quad.
pub const RED_IDX: usize = 0;
/// Maximum value of a single [`Rgba`] channel (`255`, fully opaque /
/// saturated).
pub const VAL_MAX: u8 = 255;

/// Byte-packed RGBA color, the blessed in-memory color type in
/// baumhard. Wraps a `[u8; 4]` and implements the four wrapping
/// arithmetic traits ([`Add`], [`Sub`], [`Mul`], [`Div`]) plus
/// [`Index`] / [`IndexMut`] for channel access. `Copy`, zero
/// allocation, serde-serializable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    /// Raw `[R, G, B, A]` byte channels. Exposed `pub` so palette
    /// constants can be written as struct literals at compile time.
    pub rgba: Rgba,
}

impl Color {
    /// Apply a binary `u8 -> u8` op channel-wise across two
    /// `Color`s. Single source of truth for the four wrapping
    /// arithmetic impls (`Add`/`Sub`/`Mul`/`Div`) — each used to
    /// open-code an identical 4-channel loop differing only in
    /// the wrapping-method name. Inline; no heap; O(1).
    #[inline]
    fn channel_apply(self, rhs: Self, op: fn(u8, u8) -> u8) -> Self {
        Color::new_u8(&[
            op(self[0], rhs[0]),
            op(self[1], rhs[1]),
            op(self[2], rhs[2]),
            op(self[3], rhs[3]),
        ])
    }
}

/// Component-wise wrapping division of two [`Color`]s. Uses
/// `u8::wrapping_div` per channel. Wrapping was chosen over
/// saturating because color arithmetic in Baumhard is used for
/// procedural palette generation where wrap-around produces
/// artistically useful cycling; clamping would flatten the cycle.
impl Div for Color {
    type Output = Color;

    /// Divide each RGBA channel of `self` by the corresponding
    /// channel of `rhs` using wrapping semantics. O(1), no heap.
    fn div(self, rhs: Self) -> Self::Output {
        self.channel_apply(rhs, u8::wrapping_div)
    }
}

/// Component-wise wrapping multiplication of two [`Color`]s. Uses
/// `u8::wrapping_mul` — overflow wraps modulo 256. Wrapping was
/// chosen over saturating because color arithmetic in Baumhard is
/// used for procedural palette generation where wrap-around
/// produces artistically useful cycling; clamping would flatten the
/// cycle.
impl Mul for Color {
    type Output = Color;

    /// Multiply each RGBA channel of `self` by the corresponding
    /// channel of `rhs` using wrapping semantics. O(1), no heap.
    fn mul(self, rhs: Self) -> Self::Output {
        self.channel_apply(rhs, u8::wrapping_mul)
    }
}

/// Component-wise wrapping subtraction of two [`Color`]s. Uses
/// `u8::wrapping_sub` — underflow wraps modulo 256. Wrapping was
/// chosen over saturating because color arithmetic in Baumhard is
/// used for procedural palette generation where wrap-around
/// produces artistically useful cycling; clamping would flatten the
/// cycle.
impl Sub for Color {
    type Output = Color;

    /// Subtract each RGBA channel of `rhs` from the corresponding
    /// channel of `self` using wrapping semantics. O(1), no heap.
    fn sub(self, rhs: Self) -> Self::Output {
        self.channel_apply(rhs, u8::wrapping_sub)
    }
}

/// Component-wise wrapping addition of two [`Color`]s. Uses
/// `u8::wrapping_add` — overflow wraps modulo 256. Wrapping was
/// chosen over saturating because color arithmetic in Baumhard is
/// used for procedural palette generation where wrap-around
/// produces artistically useful cycling; clamping would flatten the
/// cycle.
impl Add for Color {
    type Output = Color;

    /// Add each RGBA channel of `rhs` to the corresponding channel
    /// of `self` using wrapping semantics. O(1), no heap.
    fn add(self, rhs: Self) -> Self::Output {
        self.channel_apply(rhs, u8::wrapping_add)
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
    /// Opaque black (`[0, 0, 0, 255]`). O(1), no heap.
    pub fn black() -> Self {
        Color { rgba: [0, 0, 0, 255] }
    }

    /// Fully transparent black (`[0, 0, 0, 0]`) — the "no fill"
    /// sentinel. O(1), no heap.
    pub fn invisible() -> Self {
        Color { rgba: [0, 0, 0, 0] }
    }

    /// Opaque white (`[255, 255, 255, 255]`). O(1), no heap.
    pub fn white() -> Self {
        Color {
            rgba: [255, 255, 255, 255],
        }
    }

    /// Construct a [`Color`] from a `[u8; 4]` RGBA quad. O(1), no
    /// conversion — the bytes are stored as-is.
    pub fn new_u8(rgba: &Rgba) -> Self {
        Color { rgba: *rgba }
    }

    /// Construct a [`Color`] from a `[f32; 4]` RGBA quad (each
    /// component in `[0.0, 1.0]`). Each channel is scaled to
    /// `[0, 255]` via [`convert_f32_to_u8`] with rounding. O(1), no
    /// heap.
    pub fn new_f32(float_rgba: &FloatRgba) -> Self {
        Color {
            rgba: convert_f32_to_u8(float_rgba),
        }
    }
    /// Overwrite the alpha channel with `opacity` (0 = transparent,
    /// 255 = opaque). RGB is unchanged. O(1), no heap.
    pub fn set_alpha(&mut self, opacity: u8) {
        self.rgba[ALPHA_IDX] = opacity;
    }

    /// Convert to [`FloatRgba`] by dividing each channel by 255.
    /// O(1), no heap. Inverse of [`Color::new_f32`] within rounding
    /// slack of `0.5/255.0` (the half-byte from `.round()` in the
    /// reverse direction). `const`, so a compile-time color constant
    /// reaches the renderer's float form without a second literal.
    pub const fn to_float(&self) -> FloatRgba {
        convert_u8_to_f32(&self.rgba)
    }

    /// This color with its alpha channel replaced by `opacity`
    /// (0 = transparent, 255 = opaque); RGB is unchanged. The
    /// value-returning, `const` sibling of [`Color::set_alpha`] —
    /// reach for it when the translucent form of a color constant
    /// is itself a constant. O(1), no heap.
    pub const fn with_alpha(self, opacity: u8) -> Self {
        let mut rgba = self.rgba;
        rgba[ALPHA_IDX] = opacity;
        Color { rgba }
    }
}
