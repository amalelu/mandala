// SPDX-License-Identifier: MPL-2.0

//! Colour-space conversion utilities — hex / RGB / HSV.
//!
//! These helpers are the single source of truth for "how do we turn a
//! hex string from a `.mindmap.json` canvas into a float quad" and
//! "how do we convert between HSV and RGB for the color picker."
//! Keeping them next to the `Color` struct (in `super::color`) makes
//! the colour-math vocabulary a two-file neighbourhood where a new
//! session can find everything without guessing.

use std::collections::HashMap;

use super::color::{FloatRgba, ALPHA_IDX};

/// Convert `[f32; 4]` RGBA (each component in `[0, 1]`) to `[u8; 4]`
/// by scaling to 255 and rounding. Clamped by the `as u8` cast (values
/// > 1.0 saturate to 255).
pub fn convert_f32_to_u8(color: &[f32; 4]) -> [u8; 4] {
    let mut u8_color = [0u8; 4];
    for (i, &float_val) in color.iter().enumerate() {
        u8_color[i] = (float_val * 255.0).round() as u8;
    }
    u8_color
}

/// Convert `[u8; 4]` RGBA channels to `[f32; 4]` in `[0.0, 1.0]` —
/// the inverse of [`convert_f32_to_u8`]. Single source of truth so
/// every byte→float quantisation in the project lands on the same
/// `c as f32 / 255.0` arithmetic.
pub fn convert_u8_to_f32(color: &[u8; 4]) -> [f32; 4] {
    [
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
        color[3] as f32 / 255.0,
    ]
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
    match parse_var_name(raw).and_then(|name| vars.get(name)) {
        Some(value) => value.as_str(),
        None => raw,
    }
}

/// Parse a CSS-style theme variable reference and return the model
/// key, including its leading `--` (for example `var(--bg)` returns
/// `--bg`). Whitespace inside the `var(...)` wrapper is tolerated to
/// match [`resolve_var`]; malformed references return `None`.
pub fn parse_var_name(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if !trimmed.starts_with("var(") || !trimmed.ends_with(')') {
        return None;
    }
    let inner = trimmed["var(".len()..trimmed.len() - 1].trim();
    if inner.len() <= 2 || !inner.starts_with("--") || inner.contains(|c: char| c == '(' || c == ')') {
        return None;
    }
    Some(inner)
}

/// Return whether `raw` is a well-formed `var(--name)` color
/// reference under Baumhard's canonical parser.
pub fn is_var_ref(raw: &str) -> bool {
    parse_var_name(raw).is_some()
}

/// Return whether `raw` is one of the hex color literal forms accepted
/// by [`hex_to_rgba`]: 3, 4, 6, or 8 hex chars with an optional
/// leading `#`.
pub fn is_valid_hex_color(raw: &str) -> bool {
    hex_to_rgba(raw).is_some()
}

/// Parse a hex color string into an `[f32; 4]` RGBA quad, returning
/// `None` on any parse failure. Accepts 3, 4, 6, or 8 hex chars
/// with an optional leading `#`. The fallible cousin of
/// [`hex_to_rgba_safe`]; reach for this when the caller wants to
/// reject malformed strings rather than substitute a default.
pub fn hex_to_rgba(color: &str) -> Option<[f32; 4]> {
    let color = color.trim_start_matches('#');
    let length = color.len();
    if length != 3 && length != 4 && length != 6 && length != 8 {
        return None;
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
    let mut rgba = [0.0f32; 4];
    if length == 3 || length == 4 {
        for i in 0..length {
            let n = nibble(bytes[i])?;
            rgba[i] = ((n << 4) | n) as f32 / 255.0;
        }
        if length == 3 {
            rgba[3] = 1.0;
        }
    } else {
        for i in 0..(length / 2) {
            let hi = nibble(bytes[i * 2])?;
            let lo = nibble(bytes[i * 2 + 1])?;
            rgba[i] = ((hi << 4) | lo) as f32 / 255.0;
        }
        if length == 6 {
            rgba[3] = 1.0;
        }
    }
    Some(rgba)
}

/// Parse a hex color string into an `[f32; 4]` RGBA quad, returning
/// `fallback` on any parse failure. Accepts 3, 4, 6, or 8 hex chars
/// with an optional leading `#`. Intended for render-time color
/// resolution paths that should never crash the app over a typo in a
/// theme variable.
pub fn hex_to_rgba_safe(color: &str, fallback: [f32; 4]) -> [f32; 4] {
    hex_to_rgba(color).unwrap_or(fallback)
}

/// Convert HSV → RGB, all components normalized to `[0, 1]`. `h` is in
/// degrees (`[0, 360)`); values outside that range are wrapped via
/// `rem_euclid`. Saturation and value are clamped to `[0, 1]`.
///
/// Used by the glyph-wheel color picker to paint each hue-ring slot,
/// sat/val bar cell, and central preview glyph at the current HSV
/// coordinates. Kept next to the other hex/rgba helpers because the
/// picker shouldn't re-implement color math that could drift from the
/// canonical path.
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
/// string shape stored in `MindEdge.color`.
pub fn hsv_to_hex(h: f32, s: f32, v: f32) -> String {
    let [r, g, b] = hsv_to_rgb(h, s, v);
    let u = convert_f32_to_u8(&[r, g, b, 1.0]);
    format!("#{:02x}{:02x}{:02x}", u[0], u[1], u[2])
}

/// Convert a `FloatRgba` quad into a `#RRGGBB` (or `#RRGGBBAA`)
/// hex string in the same lowercase shape stored in
/// `MindEdge.color` and `TextRun.color`. Drops the alpha
/// channel only when α >= 1.0 (saturated opaque); otherwise
/// emits the eight-char form so model-side state survives the
/// round-trip.
///
/// Forward conversion through `convert_f32_to_u8`, identical to
/// `hsv_to_hex`'s shape — every model-side `#RRGGBB` site uses
/// the same quantisation path. Used by the section-mutation
/// reverse converter (`region_to_text_run`) where a tree-side
/// `ColorFontRegion.color: FloatRgba` rolls back into the
/// model's `TextRun.color: String`.
///
/// Lossy on `var(--name)` references — by construction. The
/// forward path resolves variables before shaping; this reverse
/// path can't recover the original variable name without an
/// additional reverse-lookup table. Document this limit at the
/// reverse-converter call site so authors know what survives a
/// custom-mutation round-trip.
pub fn rgba_to_hex(rgba: FloatRgba) -> String {
    let u = convert_f32_to_u8(&rgba);
    if rgba[3] >= 1.0 {
        format!("#{:02x}{:02x}{:02x}", u[0], u[1], u[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", u[0], u[1], u[2], u[3])
    }
}

/// Multiply a hex color string's alpha channel by `factor`,
/// returning the rebuilt `#RRGGBB` / `#RRGGBBAA` hex string. RGB
/// channels stay untouched; the alpha channel is clamped to
/// `[0.0, 1.0]` after multiplication. Tolerates a leading `#` and
/// the same 3 / 4 / 6 / 8 hex-char shapes [`hex_to_rgba`] accepts;
/// returns the input string verbatim on parse failure so a typo in
/// a theme variable stays visible at full opacity rather than
/// disappearing into transparent black.
///
/// Used by the inactive-node dimming pass to render every node's
/// chrome at half alpha while one node is active in NodeEdit mode —
/// the per-element color resolution downstream of this is unchanged
/// (it sees the dimmed hex string just like any other color).
pub fn hex_with_alpha_scaled(hex: &str, factor: f32) -> String {
    let Some(mut rgba) = hex_to_rgba(hex) else {
        return hex.to_string();
    };
    rgba[ALPHA_IDX] = (rgba[ALPHA_IDX] * factor).clamp(0.0, 1.0);
    rgba_to_hex(rgba)
}

/// Parse a hex color string into HSV, returning `None` on any parse
/// failure. Delegates to `hex_to_rgba_safe` with a sentinel fallback
/// whose alpha channel we can't collide with (negative alpha), then
/// converts. Used to seed the color picker's HSV state from the
/// target's current color at open time.
pub fn hex_to_hsv_safe(hex: &str) -> Option<(f32, f32, f32)> {
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

/// Component-wise RGBA addition. No clamping — callers that need
/// values in `[0, 1]` must clamp afterward.
pub fn add_rgba(a: &FloatRgba, b: &FloatRgba) -> [f32; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}
