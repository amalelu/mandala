//! Core colour type and arithmetic, plus macros for compile-time
//! colour literals. The conversion utilities (hex/RGB/HSV, theme
//! variable resolution) live in the companion `super::color_conversion`
//! module.

use std::ops::{Add, Div, Index, IndexMut, Mul, Sub};
use serde::{Deserialize, Serialize};

// Re-export every public item from color_conversion so existing
// `use baumhard::util::color::*` imports continue to resolve.
pub use super::color_conversion::*;

/// Compile-time `[u8; 4]` → `[f32; 4]` RGBA literal, dividing each
/// channel by 255.0. Use in `const`-adjacent contexts where a
/// float-RGBA is wanted without a helper call; the runtime path is
/// [`super::color_conversion::rgba`].
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

/// Compile-time `[u8; 3]` → `[f32; 4]` RGB literal with alpha pinned
/// to 1.0. Mirrors [`rgba!`] for the common no-alpha case.
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

/// Parse a compile-time `#RRGGBB`-style hex string literal into
/// `[f32; 4]` RGBA (alpha defaults to 1.0). Tolerates leading `#`
/// and falls back to 0.0 per channel on unparseable hex digits so
/// the macro does not panic in `const`-like contexts.
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

/// `[R, G, B, A]` in `[0.0, 1.0]` — the canvas-space colour
/// representation consumed by the renderer. Plain array, zero
/// allocation, `Copy`.
pub type FloatRgba = [f32; 4];
/// `[R, G, B, A]` in `[0, 255]` — the byte-packed form used by
/// [`Color`] and by hex parsing. Plain array, zero allocation,
/// `Copy`.
pub type Rgba = [u8; 4];
/// Ordered list of float RGBA colours forming a named palette. Heap
/// allocation proportional to the number of entries; typically
/// constructed once at program start via `lazy_static`.
pub type Palette = Vec<FloatRgba>;

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

/// Byte-packed RGBA colour, the blessed in-memory colour type in
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

/// Component-wise wrapping division of two [`Color`]s. Uses
/// `u8::wrapping_div` per channel. Wrapping was chosen over
/// saturating because colour arithmetic in Baumhard is used for
/// procedural palette generation where wrap-around produces
/// artistically useful cycling; clamping would flatten the cycle.
impl Div for Color {
    type Output = Color;

    /// Divide each RGBA channel of `self` by the corresponding
    /// channel of `rhs` using wrapping semantics. O(1), no heap.
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

/// Component-wise wrapping multiplication of two [`Color`]s. Uses
/// `u8::wrapping_mul` — overflow wraps modulo 256. Wrapping was
/// chosen over saturating because colour arithmetic in Baumhard is
/// used for procedural palette generation where wrap-around
/// produces artistically useful cycling; clamping would flatten the
/// cycle.
impl Mul for Color {
    type Output = Color;

    /// Multiply each RGBA channel of `self` by the corresponding
    /// channel of `rhs` using wrapping semantics. O(1), no heap.
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

/// Component-wise wrapping subtraction of two [`Color`]s. Uses
/// `u8::wrapping_sub` — underflow wraps modulo 256. Wrapping was
/// chosen over saturating because colour arithmetic in Baumhard is
/// used for procedural palette generation where wrap-around
/// produces artistically useful cycling; clamping would flatten the
/// cycle.
impl Sub for Color {
    type Output = Color;

    /// Subtract each RGBA channel of `rhs` from the corresponding
    /// channel of `self` using wrapping semantics. O(1), no heap.
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

/// Component-wise wrapping addition of two [`Color`]s. Uses
/// `u8::wrapping_add` — overflow wraps modulo 256. Wrapping was
/// chosen over saturating because colour arithmetic in Baumhard is
/// used for procedural palette generation where wrap-around
/// produces artistically useful cycling; clamping would flatten the
/// cycle.
impl Add for Color {
    type Output = Color;

    /// Add each RGBA channel of `rhs` to the corresponding channel
    /// of `self` using wrapping semantics. O(1), no heap.
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
    /// Opaque black (`[0, 0, 0, 255]`). O(1), no heap.
    pub fn black() -> Self {
        Color {
            rgba: [0, 0, 0, 255],
        }
    }

    /// Fully transparent black (`[0, 0, 0, 0]`) — the "no fill"
    /// sentinel. O(1), no heap.
    pub fn invisible() -> Self {
        Color {
            rgba: [0, 0, 0, 0],
        }
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

    /// Convert to [`FloatRgba`] by integer-dividing each channel by
    /// [`VAL_MAX`]. O(1), no heap. Note: this uses `u8` integer
    /// division, so every non-max channel collapses to `0.0` — the
    /// lossy form is kept for the existing scaling call sites that
    /// rely on it.
    pub fn to_float(&self) -> FloatRgba {
        [
            (self.rgba[RED_IDX] / VAL_MAX).into(),
            (self.rgba[GREEN_IDX] / VAL_MAX).into(),
            (self.rgba[BLUE_IDX] / VAL_MAX).into(),
            (self.rgba[ALPHA_IDX] / VAL_MAX).into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn vars(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn resolve_var_hit() {
        let v = vars(&[("--bg", "#111111")]);
        assert_eq!(resolve_var("var(--bg)", &v), "#111111");
    }

    #[test]
    fn resolve_var_miss_returns_raw() {
        let v = vars(&[("--bg", "#111111")]);
        assert_eq!(resolve_var("var(--missing)", &v), "var(--missing)");
    }

    #[test]
    fn resolve_var_plain_hex_passes_through() {
        let v = vars(&[("--bg", "#111111")]);
        assert_eq!(resolve_var("#ff00aa", &v), "#ff00aa");
    }

    #[test]
    fn resolve_var_malformed_passes_through() {
        let v = vars(&[("--bg", "#111111")]);
        // Missing closing paren — treat as raw
        assert_eq!(resolve_var("var(--bg", &v), "var(--bg");
    }

    #[test]
    fn resolve_var_tolerates_whitespace_inside() {
        let v = vars(&[("--bg", "#abc123")]);
        assert_eq!(resolve_var("var( --bg )", &v), "#abc123");
    }

    #[test]
    fn resolve_var_single_level_no_recursion() {
        // A variable whose value is itself a var(...) reference is NOT
        // dereferenced further in v1 — returned verbatim.
        let v = vars(&[
            ("--primary", "var(--secondary)"),
            ("--secondary", "#abcdef"),
        ]);
        assert_eq!(resolve_var("var(--primary)", &v), "var(--secondary)");
    }

    #[test]
    fn hex_to_rgba_safe_good_input() {
        let got = hex_to_rgba_safe("#ff0000", [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(got[0], 1.0);
        assert_eq!(got[1], 0.0);
        assert_eq!(got[2], 0.0);
        assert_eq!(got[3], 1.0);
    }

    #[test]
    fn hex_to_rgba_safe_garbage_returns_fallback() {
        let fb = [0.5, 0.5, 0.5, 1.0];
        assert_eq!(hex_to_rgba_safe("not-a-color", fb), fb);
        assert_eq!(hex_to_rgba_safe("var(--bgg)", fb), fb);
        assert_eq!(hex_to_rgba_safe("#xyz", fb), fb);
        assert_eq!(hex_to_rgba_safe("", fb), fb);
    }

    #[test]
    fn hex_to_rgba_safe_with_alpha() {
        let got = hex_to_rgba_safe("#00ff0080", [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(got[0], 0.0);
        assert_eq!(got[1], 1.0);
        assert_eq!(got[2], 0.0);
        assert!((got[3] - 128.0 / 255.0).abs() < 1e-6);
    }

    // -----------------------------------------------------------------
    // Performance & robustness regression guards
    //
    // `resolve_var` and `hex_to_rgba_safe` are called for every text run,
    // border, connection, and background colour on every scene build.
    // Any panic here crashes the WASM renderer; any regression from O(1)
    // HashMap lookup to linear scan here would be invisible to a smoke
    // test but visible in the frame budget.
    // -----------------------------------------------------------------

    /// A single pathological theme-variable typo must never crash the
    /// renderer. Iterate over a batch of malformed inputs and assert
    /// every call returns the fallback without panicking.
    ///
    /// Note: `#123` and `#1234` are valid CSS shorthand (`#rgb` and
    /// `#rgba`) and are handled by the parser, so they're not
    /// listed here. Only genuinely broken inputs are exercised.
    #[test]
    fn hex_to_rgba_safe_no_panic_on_malformed_batch() {
        let fb = [0.25, 0.5, 0.75, 1.0];
        let long = "f".repeat(1024);
        let pathological: Vec<&str> = vec![
            "",
            "#",
            "##",
            "#g",
            "#12",
            "#12345",
            "#1234567",
            "#123456789",
            "var(--x)",
            "var(--missing)",
            "not-a-color",
            "rgb(255, 0, 0)",
            "#\u{1f308}\u{1f308}\u{1f308}",
            "\0\0\0",
            "   ",
            "\t\n\r",
            long.as_str(),
        ];
        for bad in &pathological {
            let got = hex_to_rgba_safe(bad, fb);
            assert_eq!(got, fb,
                "malformed input {:?} should return fallback", bad);
        }
    }

    /// CSS-style short hex (`#rgb` and `#rgba`) must parse by
    /// doubling each nibble, so `#abc` → `#aabbcc`.
    #[test]
    fn hex_to_rgba_safe_short_hex_expands_each_nibble() {
        let fb = [0.0, 0.0, 0.0, 0.0];
        // `#000` = opaque black — the common default in node styles.
        assert_eq!(hex_to_rgba_safe("#000", fb), [0.0, 0.0, 0.0, 1.0]);
        // `#fff` = opaque white.
        assert_eq!(hex_to_rgba_safe("#fff", fb), [1.0, 1.0, 1.0, 1.0]);
        // `#abc` → `#aabbcc` with alpha = 1.
        let got = hex_to_rgba_safe("#abc", fb);
        let expected_r = 0xaa as f32 / 255.0;
        let expected_g = 0xbb as f32 / 255.0;
        let expected_b = 0xcc as f32 / 255.0;
        assert!((got[0] - expected_r).abs() < 1e-6);
        assert!((got[1] - expected_g).abs() < 1e-6);
        assert!((got[2] - expected_b).abs() < 1e-6);
        assert_eq!(got[3], 1.0);
        // `#abcd` → `#aabbccdd` with alpha derived from the 4th nibble.
        let got = hex_to_rgba_safe("#abcd", fb);
        let expected_a = 0xdd as f32 / 255.0;
        assert!((got[3] - expected_a).abs() < 1e-6);
        // `#0000` → fully transparent black (the "no fill" sentinel).
        assert_eq!(hex_to_rgba_safe("#0000", fb), [0.0, 0.0, 0.0, 0.0]);
    }

    /// Valid 6-char and 8-char hex — with and without the `#` prefix,
    /// upper and lower case — must all parse. Happy-path guard.
    #[test]
    fn hex_to_rgba_safe_accepts_valid_6_and_8_char_both_cases() {
        let fb = [0.0, 0.0, 0.0, 0.0];
        let with_hash = hex_to_rgba_safe("#ff0000", fb);
        let without_hash = hex_to_rgba_safe("ff0000", fb);
        let upper = hex_to_rgba_safe("FF0000", fb);
        assert_eq!(with_hash, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(without_hash, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(upper, [1.0, 0.0, 0.0, 1.0]);
        // 8-char form carries alpha through.
        let with_alpha = hex_to_rgba_safe("#00ff00ff", fb);
        assert_eq!(with_alpha, [0.0, 1.0, 0.0, 1.0]);
        let half_alpha = hex_to_rgba_safe("00ff0080", fb);
        assert!((half_alpha[3] - 128.0 / 255.0).abs() < 1e-6);
    }

    /// `resolve_var` over a large theme map must stay correct and must
    /// return a pointer-equal slice on passthrough (zero-copy
    /// invariant). This catches a regression from `&str` to `String`
    /// return type, which would silently invalidate the zero-alloc
    /// property every scene build relies on.
    #[test]
    fn resolve_var_large_theme_map_zero_copy_passthrough() {
        let mut map = HashMap::with_capacity(1000);
        for i in 0..1000 {
            map.insert(format!("--k{}", i), format!("#0000{:02x}", i & 0xff));
        }
        // Hit case — known variable.
        let got = resolve_var("var(--k500)", &map);
        assert!(got.starts_with('#'),
            "expected hex value, got {:?}", got);
        // Miss case — pointer-equal slice returned (zero-copy).
        let raw = "not-a-var-reference";
        let out = resolve_var(raw, &map);
        assert_eq!(out.as_ptr(), raw.as_ptr(),
            "passthrough should be zero-copy (same pointer)");
        // Unknown var reference passes through as the original slice.
        let unknown = "var(--no-such-key)";
        let out_unknown = resolve_var(unknown, &map);
        assert_eq!(out_unknown.as_ptr(), unknown.as_ptr(),
            "unknown var() should pass through zero-copy");
    }

    /// An unknown `var(--x)` reference must return the raw string, NOT
    /// silently substitute anything. Explicit guard against a future
    /// "helpful" fallback-to-black or similar behaviour that would mask
    /// theme typos.
    #[test]
    fn resolve_var_passthrough_on_unknown_is_verbatim() {
        let map: HashMap<String, String> = HashMap::new();
        assert_eq!(resolve_var("var(--nope)", &map), "var(--nope)");
        let map_with_other = vars(&[("--other", "#ffffff")]);
        assert_eq!(resolve_var("var(--nope)", &map_with_other), "var(--nope)");
    }

    // -----------------------------------------------------------------
    // HSV helpers — used by the glyph-wheel color picker.
    // -----------------------------------------------------------------

    fn rgb_close(a: [f32; 3], b: [f32; 3]) -> bool {
        (a[0] - b[0]).abs() < 1.0 / 255.0
            && (a[1] - b[1]).abs() < 1.0 / 255.0
            && (a[2] - b[2]).abs() < 1.0 / 255.0
    }

    #[test]
    fn hsv_to_rgb_primaries() {
        assert!(rgb_close(hsv_to_rgb(0.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
        assert!(rgb_close(hsv_to_rgb(120.0, 1.0, 1.0), [0.0, 1.0, 0.0]));
        assert!(rgb_close(hsv_to_rgb(240.0, 1.0, 1.0), [0.0, 0.0, 1.0]));
        assert!(rgb_close(hsv_to_rgb(60.0, 1.0, 1.0), [1.0, 1.0, 0.0]));
        assert!(rgb_close(hsv_to_rgb(180.0, 1.0, 1.0), [0.0, 1.0, 1.0]));
        assert!(rgb_close(hsv_to_rgb(300.0, 1.0, 1.0), [1.0, 0.0, 1.0]));
    }

    #[test]
    fn hsv_to_rgb_grayscale_ignores_hue() {
        // s = 0 ⇒ achromatic; hue is irrelevant.
        assert!(rgb_close(hsv_to_rgb(0.0, 0.0, 0.5), [0.5, 0.5, 0.5]));
        assert!(rgb_close(hsv_to_rgb(200.0, 0.0, 0.5), [0.5, 0.5, 0.5]));
        assert!(rgb_close(hsv_to_rgb(0.0, 0.0, 0.0), [0.0, 0.0, 0.0]));
        assert!(rgb_close(hsv_to_rgb(0.0, 0.0, 1.0), [1.0, 1.0, 1.0]));
    }

    #[test]
    fn hsv_to_rgb_wraps_hue() {
        // hsv_to_rgb should wrap negative and > 360 hues via rem_euclid.
        assert!(rgb_close(hsv_to_rgb(360.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
        assert!(rgb_close(hsv_to_rgb(-360.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
        assert!(rgb_close(hsv_to_rgb(720.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
    }

    #[test]
    fn rgb_to_hsv_primaries() {
        let (h, s, v) = rgb_to_hsv(1.0, 0.0, 0.0);
        assert!((h - 0.0).abs() < 1e-3);
        assert!((s - 1.0).abs() < 1e-6);
        assert!((v - 1.0).abs() < 1e-6);
        let (h, s, v) = rgb_to_hsv(0.0, 1.0, 0.0);
        assert!((h - 120.0).abs() < 1e-3);
        assert!((s - 1.0).abs() < 1e-6);
        assert!((v - 1.0).abs() < 1e-6);
        let (h, s, v) = rgb_to_hsv(0.0, 0.0, 1.0);
        assert!((h - 240.0).abs() < 1e-3);
        assert!((s - 1.0).abs() < 1e-6);
        assert!((v - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hsv_hex_roundtrip_named_colors() {
        let cases: &[(&str, (f32, f32, f32))] = &[
            ("#ff0000", (0.0, 1.0, 1.0)),
            ("#00ff00", (120.0, 1.0, 1.0)),
            ("#0000ff", (240.0, 1.0, 1.0)),
            ("#ffff00", (60.0, 1.0, 1.0)),
            ("#00ffff", (180.0, 1.0, 1.0)),
            ("#ff00ff", (300.0, 1.0, 1.0)),
            ("#000000", (0.0, 0.0, 0.0)),
            ("#ffffff", (0.0, 0.0, 1.0)),
            ("#808080", (0.0, 0.0, 128.0 / 255.0)),
        ];
        for (hex, expected_hsv) in cases {
            let got_hsv = hex_to_hsv_safe(hex).unwrap();
            // Hue only meaningful when saturation > 0
            if expected_hsv.1 > 0.0 {
                assert!((got_hsv.0 - expected_hsv.0).abs() < 1e-2,
                    "hue for {} expected {}, got {}", hex, expected_hsv.0, got_hsv.0);
            }
            assert!((got_hsv.1 - expected_hsv.1).abs() < 1e-3,
                "sat for {}", hex);
            assert!((got_hsv.2 - expected_hsv.2).abs() < 1e-3,
                "val for {}", hex);
            // Round-trip through hsv_to_hex.
            let back = hsv_to_hex(got_hsv.0, got_hsv.1, got_hsv.2);
            assert_eq!(back, *hex, "round-trip mismatch for {}", hex);
        }
    }

    #[test]
    fn hex_to_hsv_safe_rejects_garbage() {
        assert_eq!(hex_to_hsv_safe("not-a-color"), None);
        assert_eq!(hex_to_hsv_safe(""), None);
        assert_eq!(hex_to_hsv_safe("#xyz"), None);
        assert_eq!(hex_to_hsv_safe("var(--x)"), None);
    }

    #[test]
    fn hsv_to_hex_emits_six_char_format() {
        let s = hsv_to_hex(0.0, 1.0, 1.0);
        assert_eq!(s, "#ff0000");
        assert_eq!(s.len(), 7);
        let s = hsv_to_hex(200.0, 0.5, 0.75);
        assert_eq!(s.len(), 7);
        assert!(s.starts_with('#'));
        // All lowercase hex.
        for c in s[1..].chars() {
            assert!(c.is_ascii_hexdigit());
            assert!(!c.is_ascii_uppercase());
        }
    }
}
