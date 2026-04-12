use std::collections::HashMap;
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

pub fn convert_f32_to_u8(color: &[f32; 4]) -> [u8; 4] {
    let mut u8_color = [0u8; 4];
    for (i, &float_val) in color.iter().enumerate() {
        // Convert the f32 value to u8 by scaling up to 255
        // Using `saturating_mul` to ensure it doesn't overflow the u8 range
        u8_color[i] = (float_val * 255.0).round() as u8;
    }
    u8_color
}

/// Resolve a (possibly `var(--name)`) color reference against a set of
/// theme variables. If `raw` looks like `var(--something)` and the
/// corresponding entry exists in `vars`, return the variable's value;
/// otherwise return `raw` unchanged. This is deliberately forgiving:
/// unknown variables, malformed references, and plain hex strings all
/// pass through so downstream hex parsing can decide what to do.
///
/// Only one level of indirection is resolved — if a variable's value is
/// itself a `var(--other)` reference, it is returned verbatim and not
/// dereferenced further. That's a deliberate v1 simplification.
pub fn resolve_var<'a>(raw: &'a str, vars: &'a HashMap<String, String>) -> &'a str {
    let trimmed = raw.trim();
    if !trimmed.starts_with("var(") || !trimmed.ends_with(')') {
        return raw;
    }
    let inner = trimmed["var(".len()..trimmed.len() - 1].trim();
    match vars.get(inner) {
        Some(value) => value.as_str(),
        None => raw,
    }
}

/// Parse a hex color string into an `[f32; 4]` RGBA quad, returning
/// `fallback` on any parse failure. Accepts 3, 4, 6, or 8 hex chars
/// with an optional leading `#`. Intended for render-time color
/// resolution paths that should never crash the app over a typo in a
/// theme variable.
pub fn hex_to_rgba_safe(color: &str, fallback: [f32; 4]) -> [f32; 4] {
    let color = color.trim_start_matches('#');
    let length = color.len();
    // CSS color hex accepts both short (`#rgb`, `#rgba`) and long
    // (`#rrggbb`, `#rrggbbaa`) forms. For short forms, each nibble is
    // doubled ("a" → "aa") to produce the full 8-bit component. Any
    // other length is a typo and returns the caller's fallback.
    if length != 3 && length != 4 && length != 6 && length != 8 {
        return fallback;
    }

    fn nibble(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }

    let bytes = color.as_bytes();
    let mut rgba = fallback;
    if length == 3 || length == 4 {
        for i in 0..length {
            let n = match nibble(bytes[i]) {
                Some(v) => v,
                None => return fallback,
            };
            // Double the nibble: "a" → "aa" = (n << 4) | n.
            rgba[i] = ((n << 4) | n) as f32 / 255.0;
        }
        if length == 3 {
            rgba[3] = 1.0;
        }
    } else {
        for i in 0..(length / 2) {
            let hi = match nibble(bytes[i * 2]) {
                Some(v) => v,
                None => return fallback,
            };
            let lo = match nibble(bytes[i * 2 + 1]) {
                Some(v) => v,
                None => return fallback,
            };
            rgba[i] = ((hi << 4) | lo) as f32 / 255.0;
        }
        if length == 6 {
            rgba[3] = 1.0;
        }
    }
    rgba
}

/// Convert HSV → RGB, all components normalized to `[0, 1]`. `h` is in
/// degrees (`[0, 360)`); values outside that range are wrapped via
/// `rem_euclid`. Saturation and value are clamped to `[0, 1]`.
///
/// Used by the glyph-wheel color picker to paint each hue-ring slot,
/// sat/val bar cell, and central preview glyph at the current HSV
/// coordinates. Kept on `color.rs` next to the other hex/rgba helpers
/// because the picker shouldn't re-implement color math that could
/// drift from the canonical path.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let c = v * s;
    let hh = h / 60.0;
    let x = c * (1.0 - (hh.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match hh as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r1 + m, g1 + m, b1 + m]
}

/// Convert RGB → HSV. Inputs in `[0, 1]`; output `(h_deg, s, v)` with
/// `h_deg` in `[0, 360)`. For achromatic inputs (max == min) hue is
/// reported as `0.0` — arbitrary but deterministic so round-trips are
/// stable. Saturation is `0` when `max == 0`.
pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { delta / max };
    (h, s, max)
}

/// Convert HSV → `#RRGGBB` hex string (no alpha). Canonical path for
/// the color picker commit: quantize an HSV triple into the same hex
/// string shape stored in `MindEdge.color` / `PortalPair.color`.
pub fn hsv_to_hex(h: f32, s: f32, v: f32) -> String {
    let [r, g, b] = hsv_to_rgb(h, s, v);
    let u = convert_f32_to_u8(&[r, g, b, 1.0]);
    format!("#{:02x}{:02x}{:02x}", u[0], u[1], u[2])
}

/// Parse a hex color string into HSV, returning `None` on any parse
/// failure. Delegates to `hex_to_rgba_safe` with a sentinel fallback
/// whose alpha channel we can't collide with (negative alpha), then
/// converts. Used to seed the color picker's HSV state from the
/// target's current color at open time.
pub fn hex_to_hsv_safe(hex: &str) -> Option<(f32, f32, f32)> {
    // Sentinel with an out-of-range marker in the RED channel so we
    // can detect the "parse failed" case without a second round-trip.
    const SENTINEL: [f32; 4] = [-1.0, -1.0, -1.0, -1.0];
    let rgba = hex_to_rgba_safe(hex, SENTINEL);
    if rgba[0] < 0.0 {
        return None;
    }
    Some(rgb_to_hsv(rgba[0], rgba[1], rgba[2]))
}

/// Parse a slice of hex color strings into rgba quads. Bad strings
/// fall back to opaque black via `hex_to_rgba_safe`.
pub fn from_hex(colors: &[&str]) -> Vec<[f32; 4]> {
    let mut rgba_colors: Vec<[f32; 4]> = Vec::with_capacity(colors.len());
    for color in colors.iter() {
        rgba_colors.push(hex_to_rgba_safe(color, [0.0, 0.0, 0.0, 1.0]));
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

#[cfg(test)]
mod tests {
    use super::*;

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
