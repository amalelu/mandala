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

/// Non-panicking hex-to-rgba. Parses the same set of inputs as
/// `hex_to_rgba` (6 or 8 hex chars, optional leading `#`) but returns
/// `fallback` on any parse failure instead of panicking. Intended for
/// render-time color resolution paths that should never crash the app
/// over a typo in a theme variable.
pub fn hex_to_rgba_safe(color: &str, fallback: [f32; 4]) -> [f32; 4] {
    let color = color.trim_start_matches('#');
    let length = color.len();
    if length != 6 && length != 8 {
        return fallback;
    }
    let mut rgba = fallback;
    let bytes = color.as_bytes();
    for i in 0..(length / 2) {
        let hi = match bytes[i * 2] {
            b'0'..=b'9' => bytes[i * 2] - b'0',
            b'a'..=b'f' => bytes[i * 2] - b'a' + 10,
            b'A'..=b'F' => bytes[i * 2] - b'A' + 10,
            _ => return fallback,
        };
        let lo = match bytes[i * 2 + 1] {
            b'0'..=b'9' => bytes[i * 2 + 1] - b'0',
            b'a'..=b'f' => bytes[i * 2 + 1] - b'a' + 10,
            b'A'..=b'F' => bytes[i * 2 + 1] - b'A' + 10,
            _ => return fallback,
        };
        rgba[i] = ((hi << 4) | lo) as f32 / 255.0;
    }
    if length == 6 {
        rgba[3] = 1.0;
    }
    rgba
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
            "#1234",
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
}
