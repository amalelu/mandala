// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::util::color`] and
//! [`crate::util::color_conversion`] — the `Color` wrapping
//! arithmetic and every string-to-color path the scene builders
//! resolve per element: `resolve_var` and `hex_to_rgba_safe` run
//! for every text run, border, connection, and background color on
//! every scene build (§T1, §B7).
//!
//! Follows the `do_*()` / `test_*()` split from §T2.2: every `do_*`
//! body is benchmarkable from `benches/test_bench.rs`. The
//! quarter-stepped exhaustive byte sweep near the bottom is a plain
//! `#[test]` on purpose — its runtime is the 16.7-million-case
//! enumeration rather than the primitive under test, which is §B8
//! opt-out class 3.

use crate::util::color::Color;
use crate::util::color_conversion::{
    from_hex, hex_to_color, hex_to_hsv_safe, hex_to_rgba, hex_to_rgba_safe, hex_with_alpha_scaled,
    hsv_to_hex, hsv_to_rgb, is_valid_hex_color, is_var_ref, parse_var_name, resolve_var, rgb_to_hsv,
    rgba_to_hex,
};
use lazy_static::lazy_static;
use std::collections::HashMap;

/// Byte-quad → float-quad, the conversion the deleted `rgba!`
/// literal macro used to spell. Written out here rather than as a
/// crate macro because the controls below are the only callers left
/// and a test control should be readable at its use site.
const fn bytes_to_rgba(rgba: [u8; 4]) -> [f32; 4] {
    [
        rgba[0] as f32 / 255.0,
        rgba[1] as f32 / 255.0,
        rgba[2] as f32 / 255.0,
        rgba[3] as f32 / 255.0,
    ]
}

#[test]
pub fn test_from_hex() {
    do_from_hex();
}

/// Controls are byte quads rather than a second hex parse: a control
/// derived from the same parser under test proves only that the
/// parser agrees with itself.
pub fn do_from_hex() {
    let rgba = from_hex(&["f7b267", "f79d65", "f4845f", "f27059", "f25c54"]);
    let controls = [
        bytes_to_rgba([0xf7, 0xb2, 0x67, 255]),
        bytes_to_rgba([0xf7, 0x9d, 0x65, 255]),
        bytes_to_rgba([0xf4, 0x84, 0x5f, 255]),
        bytes_to_rgba([0xf2, 0x70, 0x59, 255]),
        bytes_to_rgba([0xf2, 0x5c, 0x54, 255]),
    ];
    assert_eq!(rgba.len(), controls.len());
    for (i, control) in controls.iter().enumerate() {
        assert_eq!(rgba.get(i).unwrap(), control, "entry {i}");
    }
}

lazy_static! {
    pub static ref CONTROL_1: [f32; 4] = bytes_to_rgba([0x05, 0x63, 0x8f, 255]);
    pub static ref CONTROL_2: [f32; 4] = bytes_to_rgba([0xdd, 0xbf, 0xfd, 255]);
    pub static ref CONTROL_3: [f32; 4] = bytes_to_rgba([0xba, 0x08, 0x4f, 255]);
    pub static ref CONTROL_4: [f32; 4] = bytes_to_rgba([0xfb, 0xa2, 0xc6, 255]);
    pub static ref RGBA_COLORS: Vec<[f32; 4]> = from_hex(&["#05638f", "ddbffd", "#ba084f", "#fba2c6"]);
}

#[test]
fn test_from_hex_lazy_static() {
    do_from_hex_lazy_static();
}

pub fn do_from_hex_lazy_static() {
    assert_eq!(RGBA_COLORS.len(), 4);
    assert_eq!(RGBA_COLORS.first().unwrap(), &CONTROL_1.clone());
    assert_eq!(RGBA_COLORS.get(1).unwrap(), &CONTROL_2.clone());
    assert_eq!(RGBA_COLORS.get(2).unwrap(), &CONTROL_3.clone());
    assert_eq!(RGBA_COLORS.get(3).unwrap(), &CONTROL_4.clone());
}

#[test]
fn test_from_hex_garbage_falls_back_to_black() {
    do_from_hex_garbage_falls_back_to_black();
}

/// Regression: bad hex strings must degrade to the fallback instead
/// of crashing. The valid entry in the middle ensures surrounding
/// items still parse correctly. The sentinel fallback `[0.42, …]`
/// distinguishes "returned the fallback" from "hardcoded black".
pub fn do_from_hex_garbage_falls_back_to_black() {
    // from_hex uses opaque-black as fallback internally.
    let rgba = from_hex(&["zzzzzz", "ff0000", "not-a-color", ""]);
    assert_eq!(rgba.len(), 4);
    assert_eq!(rgba[0], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(rgba[1], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(rgba[2], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(rgba[3], [0.0, 0.0, 0.0, 1.0]);
    // Use a sentinel fallback to prove hex_to_rgba_safe actually
    // returns the caller's fallback rather than a hardcoded value.
    let sentinel = [0.42, 0.42, 0.42, 0.42];
    assert_eq!(hex_to_rgba_safe("garbage", sentinel), sentinel);
    assert_eq!(hex_to_rgba_safe("", sentinel), sentinel);
}

#[test]
fn test_hex_to_rgba_three_digit() {
    do_hex_to_rgba_three_digit();
}

/// `#f0a` expands per-nibble to `#ff00aa` with alpha pinned to 1.0.
/// Locks the canonical CSS-style 3-digit shorthand expansion.
pub fn do_hex_to_rgba_three_digit() {
    let parsed = hex_to_rgba("#f0a").unwrap();
    assert_eq!(parsed, [1.0, 0.0, 170.0 / 255.0, 1.0]);
    // Same string, no leading `#`.
    assert_eq!(hex_to_rgba("f0a").unwrap(), parsed);
}

#[test]
fn test_hex_to_rgba_four_digit() {
    do_hex_to_rgba_four_digit();
}

/// `#f0a8` expands per-nibble to `#ff00aa88` — alpha lifted from
/// the fourth nibble rather than pinned to 1.0.
pub fn do_hex_to_rgba_four_digit() {
    let parsed = hex_to_rgba("#f0a8").unwrap();
    assert_eq!(parsed, [1.0, 0.0, 170.0 / 255.0, 136.0 / 255.0]);
}

#[test]
fn test_hex_to_rgba_six_digit() {
    do_hex_to_rgba_six_digit();
}

/// `#05638f` is the same fixture used elsewhere in this file
/// (matches `hex!("#05638f")` and `rgba!([5, 99, 143, 255])`).
/// Six-digit form pins alpha to 1.0.
pub fn do_hex_to_rgba_six_digit() {
    let parsed = hex_to_rgba("#05638f").unwrap();
    assert_eq!(parsed, [5.0 / 255.0, 99.0 / 255.0, 143.0 / 255.0, 1.0]);
}

#[test]
fn test_hex_to_rgba_eight_digit() {
    do_hex_to_rgba_eight_digit();
}

/// Eight-digit form carries an explicit alpha byte. Tests an
/// unambiguous half-alpha (`80` = 128) so the channel can't be
/// confused with a default 255.
pub fn do_hex_to_rgba_eight_digit() {
    let parsed = hex_to_rgba("#05638f80").unwrap();
    assert_eq!(parsed, [5.0 / 255.0, 99.0 / 255.0, 143.0 / 255.0, 128.0 / 255.0]);
}

#[test]
fn test_hex_to_rgba_rejects_invalid_length() {
    do_hex_to_rgba_rejects_invalid_length();
}

/// 1, 2, 5, 7, 9 digit and empty inputs are not accepted lengths.
/// Each must round-trip to `None` so the fallible API contract holds.
pub fn do_hex_to_rgba_rejects_invalid_length() {
    assert!(hex_to_rgba("").is_none());
    assert!(hex_to_rgba("#").is_none());
    assert!(hex_to_rgba("#f").is_none());
    assert!(hex_to_rgba("#ff").is_none());
    assert!(hex_to_rgba("#ffff5").is_none());
    assert!(hex_to_rgba("#ffff55a").is_none());
    assert!(hex_to_rgba("#ffff55aa1").is_none());
}

#[test]
fn test_hex_to_rgba_rejects_non_hex_char() {
    do_hex_to_rgba_rejects_non_hex_char();
}

/// A non-hex byte anywhere in the body fails the parse — both
/// in the short-form path (length 3/4) and the byte-pair path
/// (length 6/8).
pub fn do_hex_to_rgba_rejects_non_hex_char() {
    assert!(hex_to_rgba("#zzz").is_none());
    assert!(hex_to_rgba("#zzzz").is_none());
    assert!(hex_to_rgba("#gg0000").is_none());
    assert!(hex_to_rgba("#ff00ZZ").is_none());
    assert!(hex_to_rgba("#deadbeef!").is_none()); // length-mismatch wins
    assert!(hex_to_rgba("#ff00 0a").is_none()); // embedded space
}

#[test]
fn test_is_valid_hex_color_matches_hex_parser() {
    do_is_valid_hex_color_matches_hex_parser();
}

pub fn do_is_valid_hex_color_matches_hex_parser() {
    for valid in ["#abc", "abc", "#abcd", "abcd", "#aabbcc", "aabbcc", "#aabbccdd"] {
        assert!(is_valid_hex_color(valid), "{valid:?} should be accepted");
    }
    for invalid in ["", "#", "#ab", "#abcde", "#abcdefghi", "#zzzzzz", "var(--accent)"] {
        assert!(!is_valid_hex_color(invalid), "{invalid:?} should be rejected");
    }
}

#[test]
fn test_parse_var_name_tolerates_inner_whitespace() {
    do_parse_var_name_tolerates_inner_whitespace();
}

pub fn do_parse_var_name_tolerates_inner_whitespace() {
    assert_eq!(parse_var_name("var(--accent)"), Some("--accent"));
    assert_eq!(parse_var_name(" var( --bg ) "), Some("--bg"));
    assert!(is_var_ref("var( --fg )"));
}

#[test]
fn test_parse_var_name_rejects_malformed_refs() {
    do_parse_var_name_rejects_malformed_refs();
}

pub fn do_parse_var_name_rejects_malformed_refs() {
    for invalid in [
        "var(--)",
        "var(accent)",
        "var(--accent)extra",
        "var(--foo(bar))",
        "var(--accent",
        "#abcdef",
    ] {
        assert_eq!(parse_var_name(invalid), None, "{invalid:?} should be rejected");
        assert!(!is_var_ref(invalid), "{invalid:?} should not be a var ref");
    }
}

fn vars(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn test_resolve_var_hit() {
    do_resolve_var_hit();
}

pub fn do_resolve_var_hit() {
    let v = vars(&[("--bg", "#111111")]);
    assert_eq!(resolve_var("var(--bg)", &v), "#111111");
}

#[test]
fn test_resolve_var_miss_returns_raw() {
    do_resolve_var_miss_returns_raw();
}

pub fn do_resolve_var_miss_returns_raw() {
    let v = vars(&[("--bg", "#111111")]);
    assert_eq!(resolve_var("var(--missing)", &v), "var(--missing)");
}

#[test]
fn test_resolve_var_plain_hex_passes_through() {
    do_resolve_var_plain_hex_passes_through();
}

pub fn do_resolve_var_plain_hex_passes_through() {
    let v = vars(&[("--bg", "#111111")]);
    assert_eq!(resolve_var("#ff00aa", &v), "#ff00aa");
}

#[test]
fn test_resolve_var_malformed_passes_through() {
    do_resolve_var_malformed_passes_through();
}

pub fn do_resolve_var_malformed_passes_through() {
    let v = vars(&[("--bg", "#111111")]);
    // Missing closing paren — treat as raw
    assert_eq!(resolve_var("var(--bg", &v), "var(--bg");
}

#[test]
fn test_resolve_var_tolerates_whitespace_inside() {
    do_resolve_var_tolerates_whitespace_inside();
}

pub fn do_resolve_var_tolerates_whitespace_inside() {
    let v = vars(&[("--bg", "#abc123")]);
    assert_eq!(resolve_var("var( --bg )", &v), "#abc123");
}

#[test]
fn test_resolve_var_single_level_no_recursion() {
    do_resolve_var_single_level_no_recursion();
}

pub fn do_resolve_var_single_level_no_recursion() {
    // A variable whose value is itself a var(...) reference is NOT
    // dereferenced further in v1 — returned verbatim.
    let v = vars(&[("--primary", "var(--secondary)"), ("--secondary", "#abcdef")]);
    assert_eq!(resolve_var("var(--primary)", &v), "var(--secondary)");
}

#[test]
fn test_hex_to_rgba_safe_good_input() {
    do_hex_to_rgba_safe_good_input();
}

pub fn do_hex_to_rgba_safe_good_input() {
    let got = hex_to_rgba_safe("#ff0000", [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(got[0], 1.0);
    assert_eq!(got[1], 0.0);
    assert_eq!(got[2], 0.0);
    assert_eq!(got[3], 1.0);
}

#[test]
fn test_hex_to_rgba_safe_garbage_returns_fallback() {
    do_hex_to_rgba_safe_garbage_returns_fallback();
}

pub fn do_hex_to_rgba_safe_garbage_returns_fallback() {
    let fb = [0.5, 0.5, 0.5, 1.0];
    assert_eq!(hex_to_rgba_safe("not-a-color", fb), fb);
    assert_eq!(hex_to_rgba_safe("var(--bgg)", fb), fb);
    assert_eq!(hex_to_rgba_safe("#xyz", fb), fb);
    assert_eq!(hex_to_rgba_safe("", fb), fb);
}

#[test]
fn test_hex_with_alpha_scaled_halves_opaque_input() {
    do_hex_with_alpha_scaled_halves_opaque_input();
}

/// Round-trip: a 6-char `#RRGGBB` with alpha=1.0 multiplied by
/// 0.5 lands at `#RRGGBB80` (alpha=128/255 ≈ 0.502). Locks the
/// dimming pass's per-element use case (Plan §B.2 — inactive
/// nodes render at 50% alpha when NodeEdit is open).
pub fn do_hex_with_alpha_scaled_halves_opaque_input() {
    let got = hex_with_alpha_scaled("#ff0000", 0.5);
    // `rgba_to_hex` emits 8-char form whenever alpha < 1.0.
    assert_eq!(got, "#ff000080");
}

#[test]
fn test_hex_with_alpha_scaled_factor_one_round_trips() {
    do_hex_with_alpha_scaled_factor_one_round_trips();
}

/// Factor 1.0 leaves 6-char `#RRGGBB` opaque hex unchanged
/// (the no-op fast path the Default-mode caller hits).
pub fn do_hex_with_alpha_scaled_factor_one_round_trips() {
    // `#ff8800` is exactly representable in 8-bit channels;
    // round-trip through parse → re-format should be byte-equal.
    assert_eq!(hex_with_alpha_scaled("#ff8800", 1.0), "#ff8800");
}

#[test]
fn test_hex_with_alpha_scaled_factor_zero_zeros_alpha() {
    do_hex_with_alpha_scaled_factor_zero_zeros_alpha();
}

/// Factor 0.0 zeros the alpha channel — the resulting hex
/// shows alpha=00 (fully transparent).
pub fn do_hex_with_alpha_scaled_factor_zero_zeros_alpha() {
    let got = hex_with_alpha_scaled("#abcdef", 0.0);
    assert_eq!(got, "#abcdef00");
}

#[test]
fn test_hex_with_alpha_scaled_factor_above_one_clamps_to_full_alpha() {
    do_hex_with_alpha_scaled_factor_above_one_clamps_to_full_alpha();
}

/// Factor > 1.0 clamps alpha to 1.0 — protects against a
/// caller passing a misordered (multiplier, factor) pair from
/// promoting an already-saturated alpha into nonsense.
pub fn do_hex_with_alpha_scaled_factor_above_one_clamps_to_full_alpha() {
    // Start from `#80aabbcc` (alpha=0xcc / 255 ≈ 0.8) × 10.0
    // would naively land at 8.0; expect clamp to 1.0 → 6-char form.
    let got = hex_with_alpha_scaled("#aabbccdd", 10.0);
    assert_eq!(got, "#aabbcc");
}

#[test]
fn test_hex_with_alpha_scaled_preserves_rgb_on_8_char_input() {
    do_hex_with_alpha_scaled_preserves_rgb_on_8_char_input();
}

/// Existing 8-char `#RRGGBBAA` input — only the alpha mutates,
/// RGB channels round-trip byte-equal.
pub fn do_hex_with_alpha_scaled_preserves_rgb_on_8_char_input() {
    // `#10204080` × 0.5 → alpha 0x80 / 2 = 0x40
    let got = hex_with_alpha_scaled("#10204080", 0.5);
    assert_eq!(got, "#10204040");
}

#[test]
fn test_hex_with_alpha_scaled_parse_failure_passes_through() {
    do_hex_with_alpha_scaled_parse_failure_passes_through();
}

/// Parse failure (malformed hex) returns the input verbatim.
/// Same forgiving posture as `hex_to_rgba_safe` — the dimming
/// pass shouldn't crash a frame over a typo in a theme variable.
pub fn do_hex_with_alpha_scaled_parse_failure_passes_through() {
    assert_eq!(hex_with_alpha_scaled("not-a-color", 0.5), "not-a-color");
    assert_eq!(hex_with_alpha_scaled("var(--bg)", 0.5), "var(--bg)");
    assert_eq!(hex_with_alpha_scaled("", 0.5), "");
    assert_eq!(hex_with_alpha_scaled("#xyz", 0.5), "#xyz");
}

#[test]
fn test_hex_with_alpha_scaled_composes() {
    do_hex_with_alpha_scaled_composes();
}

/// Composition pin: applying factor 0.5 twice yields factor
/// 0.25 — exercises the parse → multiply → re-serialize round
/// trip stability under repeated application.
pub fn do_hex_with_alpha_scaled_composes() {
    let once = hex_with_alpha_scaled("#ffffff", 0.5);
    // 0xff × 0.5 = 0x7f.5 → rounded to 0x80 (`convert_f32_to_u8`
    // uses .round()).
    assert_eq!(once, "#ffffff80");
    let twice = hex_with_alpha_scaled(&once, 0.5);
    // 0x80 × 0.5 = 0x40 exactly.
    assert_eq!(twice, "#ffffff40");
}

#[test]
fn test_hex_to_rgba_safe_with_alpha() {
    do_hex_to_rgba_safe_with_alpha();
}

pub fn do_hex_to_rgba_safe_with_alpha() {
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
// border, connection, and background color on every scene build.
// Any panic here crashes the WASM renderer; any regression from O(1)
// HashMap lookup to linear scan here would be invisible to a smoke
// test but visible in the frame budget.
// -----------------------------------------------------------------

#[test]
fn test_hex_to_rgba_safe_no_panic_on_malformed_batch() {
    do_hex_to_rgba_safe_no_panic_on_malformed_batch();
}

/// A single pathological theme-variable typo must never crash the
/// renderer. Iterate over a batch of malformed inputs and assert
/// every call returns the fallback without panicking.
///
/// Note: `#123` and `#1234` are valid CSS shorthand (`#rgb` and
/// `#rgba`) and are handled by the parser, so they're not
/// listed here. Only genuinely broken inputs are exercised.
pub fn do_hex_to_rgba_safe_no_panic_on_malformed_batch() {
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
        assert_eq!(got, fb, "malformed input {:?} should return fallback", bad);
    }
}

#[test]
fn test_hex_to_rgba_safe_short_hex_expands_each_nibble() {
    do_hex_to_rgba_safe_short_hex_expands_each_nibble();
}

/// CSS-style short hex (`#rgb` and `#rgba`) must parse by
/// doubling each nibble, so `#abc` → `#aabbcc`.
pub fn do_hex_to_rgba_safe_short_hex_expands_each_nibble() {
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

#[test]
fn test_hex_to_rgba_safe_accepts_valid_6_and_8_char_both_cases() {
    do_hex_to_rgba_safe_accepts_valid_6_and_8_char_both_cases();
}

/// Valid 6-char and 8-char hex — with and without the `#` prefix,
/// upper and lower case — must all parse. Happy-path guard.
pub fn do_hex_to_rgba_safe_accepts_valid_6_and_8_char_both_cases() {
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

#[test]
fn test_resolve_var_large_theme_map_zero_copy_passthrough() {
    do_resolve_var_large_theme_map_zero_copy_passthrough();
}

/// `resolve_var` over a large theme map must stay correct and must
/// return a pointer-equal slice on passthrough (zero-copy
/// invariant). This catches a regression from `&str` to `String`
/// return type, which would silently invalidate the zero-alloc
/// property every scene build relies on.
pub fn do_resolve_var_large_theme_map_zero_copy_passthrough() {
    let mut map = HashMap::with_capacity(1000);
    for i in 0..1000 {
        map.insert(format!("--k{}", i), format!("#0000{:02x}", i & 0xff));
    }
    // Hit case — known variable.
    let got = resolve_var("var(--k500)", &map);
    assert!(got.starts_with('#'), "expected hex value, got {:?}", got);
    // Miss case — pointer-equal slice returned (zero-copy).
    let raw = "not-a-var-reference";
    let out = resolve_var(raw, &map);
    assert_eq!(
        out.as_ptr(),
        raw.as_ptr(),
        "passthrough should be zero-copy (same pointer)"
    );
    // Unknown var reference passes through as the original slice.
    let unknown = "var(--no-such-key)";
    let out_unknown = resolve_var(unknown, &map);
    assert_eq!(
        out_unknown.as_ptr(),
        unknown.as_ptr(),
        "unknown var() should pass through zero-copy"
    );
}

#[test]
fn test_resolve_var_passthrough_on_unknown_is_verbatim() {
    do_resolve_var_passthrough_on_unknown_is_verbatim();
}

/// An unknown `var(--x)` reference must return the raw string, NOT
/// silently substitute anything. Explicit guard against a future
/// "helpful" fallback-to-black or similar behavior that would mask
/// theme typos.
pub fn do_resolve_var_passthrough_on_unknown_is_verbatim() {
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
fn test_hsv_to_rgb_primaries() {
    do_hsv_to_rgb_primaries();
}

pub fn do_hsv_to_rgb_primaries() {
    assert!(rgb_close(hsv_to_rgb(0.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
    assert!(rgb_close(hsv_to_rgb(120.0, 1.0, 1.0), [0.0, 1.0, 0.0]));
    assert!(rgb_close(hsv_to_rgb(240.0, 1.0, 1.0), [0.0, 0.0, 1.0]));
    assert!(rgb_close(hsv_to_rgb(60.0, 1.0, 1.0), [1.0, 1.0, 0.0]));
    assert!(rgb_close(hsv_to_rgb(180.0, 1.0, 1.0), [0.0, 1.0, 1.0]));
    assert!(rgb_close(hsv_to_rgb(300.0, 1.0, 1.0), [1.0, 0.0, 1.0]));
}

#[test]
fn test_hsv_to_rgb_grayscale_ignores_hue() {
    do_hsv_to_rgb_grayscale_ignores_hue();
}

pub fn do_hsv_to_rgb_grayscale_ignores_hue() {
    // s = 0 ⇒ achromatic; hue is irrelevant.
    assert!(rgb_close(hsv_to_rgb(0.0, 0.0, 0.5), [0.5, 0.5, 0.5]));
    assert!(rgb_close(hsv_to_rgb(200.0, 0.0, 0.5), [0.5, 0.5, 0.5]));
    assert!(rgb_close(hsv_to_rgb(0.0, 0.0, 0.0), [0.0, 0.0, 0.0]));
    assert!(rgb_close(hsv_to_rgb(0.0, 0.0, 1.0), [1.0, 1.0, 1.0]));
}

#[test]
fn test_hsv_to_rgb_wraps_hue() {
    do_hsv_to_rgb_wraps_hue();
}

pub fn do_hsv_to_rgb_wraps_hue() {
    // hsv_to_rgb should wrap negative and > 360 hues via rem_euclid.
    assert!(rgb_close(hsv_to_rgb(360.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
    assert!(rgb_close(hsv_to_rgb(-360.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
    assert!(rgb_close(hsv_to_rgb(720.0, 1.0, 1.0), [1.0, 0.0, 0.0]));
}

#[test]
fn test_rgb_to_hsv_primaries() {
    do_rgb_to_hsv_primaries();
}

pub fn do_rgb_to_hsv_primaries() {
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
fn test_hsv_hex_roundtrip_named_colors() {
    do_hsv_hex_roundtrip_named_colors();
}

pub fn do_hsv_hex_roundtrip_named_colors() {
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
            assert!(
                (got_hsv.0 - expected_hsv.0).abs() < 1e-2,
                "hue for {} expected {}, got {}",
                hex,
                expected_hsv.0,
                got_hsv.0
            );
        }
        assert!((got_hsv.1 - expected_hsv.1).abs() < 1e-3, "sat for {}", hex);
        assert!((got_hsv.2 - expected_hsv.2).abs() < 1e-3, "val for {}", hex);
        // Round-trip through hsv_to_hex.
        let back = hsv_to_hex(got_hsv.0, got_hsv.1, got_hsv.2);
        assert_eq!(back, *hex, "round-trip mismatch for {}", hex);
    }
}

#[test]
fn test_hex_to_hsv_safe_rejects_garbage() {
    do_hex_to_hsv_safe_rejects_garbage();
}

pub fn do_hex_to_hsv_safe_rejects_garbage() {
    assert_eq!(hex_to_hsv_safe("not-a-color"), None);
    assert_eq!(hex_to_hsv_safe(""), None);
    assert_eq!(hex_to_hsv_safe("#xyz"), None);
    assert_eq!(hex_to_hsv_safe("var(--x)"), None);
}

#[test]
fn test_hsv_to_hex_emits_six_char_format() {
    do_hsv_to_hex_emits_six_char_format();
}

pub fn do_hsv_to_hex_emits_six_char_format() {
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

#[test]
fn test_rgba_to_hex_drops_alpha_only_when_saturated_opaque() {
    do_rgba_to_hex_drops_alpha_only_when_saturated_opaque();
}

/// `rgba_to_hex` round-trips opaque RGBA into the same
/// `#RRGGBB` shape `MindEdge.color` and `TextRun.color`
/// stash. Drops alpha at α=1.0; emits the eight-char form
/// otherwise so transparent picks survive a round-trip.
pub fn do_rgba_to_hex_drops_alpha_only_when_saturated_opaque() {
    // Pure red, opaque.
    let s = rgba_to_hex([1.0, 0.0, 0.0, 1.0]);
    assert_eq!(s, "#ff0000");

    // Pure red, semi-transparent — alpha must be encoded.
    let s = rgba_to_hex([1.0, 0.0, 0.0, 0.5]);
    assert_eq!(s, "#ff000080");

    // Round-trip stability: hex → rgba → hex must match.
    let original = "#3366cc";
    let parsed = hex_to_rgba_safe(original, [0.0; 4]);
    let back = rgba_to_hex(parsed);
    assert_eq!(back, original);
}

#[test]
fn test_color_add_wraps_per_channel_modulo_256() {
    do_color_add_wraps_per_channel_modulo_256();
}

/// `Color + Color` wraps modulo 256 per channel — the
/// procedural-palette use case the wrapping policy exists
/// for. Locks the contract against any future drift to
/// saturating semantics.
pub fn do_color_add_wraps_per_channel_modulo_256() {
    let a = Color::new_u8(&[200, 100, 50, 255]);
    let b = Color::new_u8(&[100, 200, 50, 1]);
    let r = a + b;
    // 200+100=300 → 44, 100+200=300 → 44, 50+50=100, 255+1=256 → 0.
    assert_eq!(r[0], 44);
    assert_eq!(r[1], 44);
    assert_eq!(r[2], 100);
    assert_eq!(r[3], 0);
}

#[test]
fn test_color_sub_wraps_underflow_modulo_256() {
    do_color_sub_wraps_underflow_modulo_256();
}

/// `Color - Color` underflow wraps modulo 256.
pub fn do_color_sub_wraps_underflow_modulo_256() {
    let a = Color::new_u8(&[10, 0, 100, 255]);
    let b = Color::new_u8(&[20, 1, 100, 0]);
    let r = a - b;
    // 10-20=-10 → 246, 0-1=-1 → 255, 100-100=0, 255-0=255.
    assert_eq!(r[0], 246);
    assert_eq!(r[1], 255);
    assert_eq!(r[2], 0);
    assert_eq!(r[3], 255);
}

#[test]
fn test_color_mul_wraps_overflow_modulo_256() {
    do_color_mul_wraps_overflow_modulo_256();
}

/// `Color * Color` overflow wraps modulo 256.
pub fn do_color_mul_wraps_overflow_modulo_256() {
    let a = Color::new_u8(&[16, 4, 0, 255]);
    let b = Color::new_u8(&[16, 64, 200, 1]);
    let r = a * b;
    // 16*16=256 → 0, 4*64=256 → 0, 0*200=0, 255*1=255.
    assert_eq!(r[0], 0);
    assert_eq!(r[1], 0);
    assert_eq!(r[2], 0);
    assert_eq!(r[3], 255);
}

#[test]
fn test_color_div_per_channel() {
    do_color_div_per_channel();
}

/// `Color / Color` uses `u8::wrapping_div`. Division by
/// zero panics in `wrapping_div` on debug builds and
/// returns `0` in release; this test exercises only the
/// non-zero divisor path that consumers actually use.
pub fn do_color_div_per_channel() {
    let a = Color::new_u8(&[200, 100, 50, 255]);
    let b = Color::new_u8(&[2, 4, 5, 255]);
    let r = a / b;
    assert_eq!(r[0], 100);
    assert_eq!(r[1], 25);
    assert_eq!(r[2], 10);
    assert_eq!(r[3], 1);
}

/// `Color::to_float` and `Color::new_f32` are inverses within
/// rounding slack of `1.0/255.0` — a regression to the prior
/// integer-division body would collapse every non-saturated
/// channel to `0.0` and fail this immediately. Cycles every
/// fourth byte value so subnormal mid-range channels (where
/// the bug lived) get exercised.
#[test]
fn test_color_to_float_round_trips_through_new_f32() {
    for r in (0u8..=255).step_by(4) {
        for g in (0u8..=255).step_by(4) {
            for b in (0u8..=255).step_by(4) {
                for a in (0u8..=255).step_by(4) {
                    let original = Color::new_u8(&[r, g, b, a]);
                    let floats = original.to_float();
                    for (i, channel) in floats.iter().enumerate() {
                        assert!(
                            (*channel - original[i] as f32 / 255.0).abs() < f32::EPSILON,
                            "channel {i} of {original:?} → {channel} drifted",
                        );
                    }
                    let back = Color::new_f32(&floats);
                    // Slack of 1 byte covers the round-trip's
                    // mul-by-255 + round; in practice every
                    // channel returns to itself bit-exact, but
                    // the `<= 1` guard documents the contract.
                    for i in 0..4 {
                        assert!(
                            (back[i] as i16 - original[i] as i16).abs() <= 1,
                            "channel {i} of {original:?} round-tripped to {back:?}",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn test_color_to_float_does_not_collapse_mid_range_channels() {
    do_color_to_float_does_not_collapse_mid_range_channels();
}

/// Mid-range channels — the silent victims of the prior
/// integer-division `to_float` — must produce non-zero floats.
/// Direct regression guard: `[128, 200, 50, 255]` previously
/// returned `[0, 0, 0, 1]` and rendered as black.
pub fn do_color_to_float_does_not_collapse_mid_range_channels() {
    let mid = Color::new_u8(&[128, 200, 50, 255]);
    let f = mid.to_float();
    assert!(f[0] > 0.49 && f[0] < 0.51, "red 128/255 ≈ 0.502");
    assert!(f[1] > 0.78 && f[1] < 0.79, "green 200/255 ≈ 0.784");
    assert!(f[2] > 0.19 && f[2] < 0.20, "blue 50/255 ≈ 0.196");
    assert_eq!(f[3], 1.0);
}

#[test]
fn test_color_new_f32_to_float_round_trips_within_one_byte() {
    do_color_new_f32_to_float_round_trips_within_one_byte();
}

/// `Color::new_f32` then `to_float` round-trips within
/// `1.0/255.0` — the rounding slack of 8-bit quantisation.
/// Property-style sweep across the unit interval (4096 sample
/// points per channel = 64⁴ ≈ 16M combinations is too slow;
/// step by 0.05 = 21⁴ ≈ 200k combinations runs in well under
/// a second). Pins the §6.1 invariant that f32-source colors
/// (sliders, picker outputs, theme variables) round-trip
/// cleanly through the 8-bit storage.
pub fn do_color_new_f32_to_float_round_trips_within_one_byte() {
    let slack = 1.0 / 255.0 + f32::EPSILON;
    let mut steps = Vec::new();
    let mut x = 0.0f32;
    while x <= 1.0 {
        steps.push(x);
        x += 0.05;
    }
    steps.push(1.0);
    for &r in &steps {
        for &g in &steps {
            for &b in &steps {
                for &a in &steps {
                    let c = Color::new_f32(&[r, g, b, a]);
                    let back = c.to_float();
                    for (i, original) in [r, g, b, a].iter().enumerate() {
                        let drift = (back[i] - *original).abs();
                        assert!(
                            drift <= slack,
                            "channel {i} of {:?}: {} → {} drifted {}",
                            [r, g, b, a],
                            original,
                            back[i],
                            drift,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn test_hex_to_color_parses_bytes_at_compile_time() {
    do_hex_to_color_parses_bytes_at_compile_time();
}

/// `hex_to_color` is the project's only hex parser and it evaluates
/// in `const` position. `CONST_PARSED` is the second half of that
/// claim: initialising a `const` from the function stops compiling
/// the day it stops being const-evaluable, which is how the
/// compile-time palette constants that rest on it
/// (`mindmap::SELECTION_HIGHLIGHT`) fail loudly rather than quietly.
///
/// The expectations are byte quads written out by hand, not a second
/// call to [`hex_to_rgba`]: that function is now this one plus a
/// division, so a control derived from it would only show the parser
/// agreeing with itself.
///
/// The input that makes the `###f0a` assertion fail — and only that
/// one of the last three; the two `is_none` claims below it stay true
/// under the mutation, and an `assert_eq!` aborts at the first failure
/// regardless — is the hand-rolled `#`-scan replacing
/// `trim_start_matches('#')`. A scan that stopped after one `#` reads
/// `##f0a` as a five-character body and rejects a string the previous
/// parser accepted, which for a file format is a silent narrowing.
pub fn do_hex_to_color_parses_bytes_at_compile_time() {
    const CONST_PARSED: Color = match hex_to_color("#05638f80") {
        Some(color) => color,
        None => panic!("a well-formed literal must parse"),
    };
    assert_eq!(CONST_PARSED.rgba, [5, 99, 143, 128]);

    assert_eq!(
        hex_to_color("#f0a").map(|c| c.rgba),
        Some([0xff, 0x00, 0xaa, 255])
    );
    assert_eq!(
        hex_to_color("#f0a8").map(|c| c.rgba),
        Some([0xff, 0x00, 0xaa, 0x88])
    );
    assert_eq!(hex_to_color("05638f").map(|c| c.rgba), Some([5, 99, 143, 255]));
    assert_eq!(
        hex_to_color("###f0a").map(|c| c.rgba),
        Some([0xff, 0x00, 0xaa, 255])
    );
    assert!(hex_to_color("###").is_none(), "a body of nothing is not a color");
    assert!(hex_to_color("#gg0000").is_none(), "`g` is not a hex digit");
}

#[test]
fn test_color_with_alpha_replaces_only_the_alpha_channel() {
    do_color_with_alpha_replaces_only_the_alpha_channel();
}

/// [`Color::with_alpha`] is the `const`, value-returning sibling of
/// `Color::set_alpha`, so a translucent form of a color *constant*
/// stays a constant. Both halves are checked: the RGB channels
/// survive untouched, and the result matches what the `&mut`
/// sibling produces from the same source — two separately written
/// implementations, so the agreement is evidence rather than a
/// mirror.
///
/// The input that makes it fail is a `with_alpha` that rebuilds the
/// quad from `opacity` in the wrong slot; `[5, 99, 143]` has three
/// distinct channels so any transposition shows.
pub fn do_color_with_alpha_replaces_only_the_alpha_channel() {
    const OPAQUE: Color = Color {
        rgba: [5, 99, 143, 255],
    };
    const TRANSLUCENT: Color = OPAQUE.with_alpha(200);
    assert_eq!(TRANSLUCENT.rgba, [5, 99, 143, 200]);
    assert_eq!(
        OPAQUE.rgba,
        [5, 99, 143, 255],
        "the source is taken by value, not mutated"
    );

    let mut via_setter = OPAQUE;
    via_setter.set_alpha(200);
    assert_eq!(via_setter.rgba, TRANSLUCENT.rgba);
}

// Both halves follow `util::rust_source`'s own gate — it reads files
// off the filesystem, which wasm32 does not have — the same reason
// `util::tests::rust_source_tests` is gated as a whole module. This
// file's other bodies are target-independent, so the gate is on the
// pair rather than on the file.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_every_hex_entry_point_is_downstream_of_the_one_parser() {
    do_every_hex_entry_point_is_downstream_of_the_one_parser();
}

/// [`hex_to_color`]'s claim to be the project's only hex-color parser,
/// derived from the source instead of enumerated in prose.
///
/// The enumeration it replaces was wrong on the commit that added it:
/// it listed five downstream entry points and missed
/// [`is_valid_hex_color`], a sixth `pub` function that takes a hex
/// string. A list a reader is asked to trust is exactly the thing that
/// goes stale, and this one went stale immediately.
///
/// So: read both hex-bearing modules' production code, collect every
/// `pub` function whose *name* says it takes a hex string, and grow a
/// downstream set outward from `hex_to_color` until it stops growing.
/// Every collected name must land in that set.
///
/// The input that makes it fail is a new (or rewritten) hex entry
/// point that walks the bytes itself instead of delegating — the exact
/// drift the claim exists to forbid — and it fails naming that
/// function. Planting `pub fn hex_to_rgba_direct(c: &str) -> Option<[f32; 4]> { let _ = c.as_bytes(); None }`
/// in `color_conversion.rs` turns it red; a body that calls
/// `hex_to_rgba` instead leaves it green.
///
/// Scoped through [`production_code`](crate::util::rust_source::production_code),
/// so the many `hex_to_rgba` mentions in these modules' doc comments
/// do not stand in for a call, and the test modules below them are cut
/// away before the scan starts.
///
/// `rgba_to_hex` and `hsv_to_hex` are the two deliberate non-members:
/// both are *inverses*, formatting a hex string rather than reading
/// one, so neither has any business being downstream of a parser.
/// They are listed one by one rather than filtered by name shape,
/// because "the name ends in `_to_hex`" is a weaker rule than "these
/// two functions, for this reason" — and a filter by shape is how a
/// real parser slips out of the scan under a name nobody vetted.
///
/// **The bound, which is that same hole one step earlier.** Collection
/// filters on `name.contains("hex")`, so a hex parser under a name
/// that does not say "hex" is never collected and this test is green
/// on it. Measured: a `pub fn parse_color_literal(c: &str) ->
/// Option<[f32; 4]>` that walks `c.as_bytes()` itself, planted in
/// `color_conversion.rs`, leaves this test passing. The scope stated
/// above — "every `pub` function whose *name* says it takes a hex
/// string" — is therefore exact rather than hedged, but it is a
/// *name* rule on the collection side while the paragraph above
/// refuses a *name* rule on the exemption side. Closing it means
/// classifying by body ("indexes `as_bytes()` and returns a color"),
/// which decides a different question and would need its own controls;
/// until someone writes that, what this test derives is "no hex-named
/// entry point bypasses the one parser", and a color function that
/// hides its subject in its name is left to review.
#[cfg(not(target_arch = "wasm32"))]
pub fn do_every_hex_entry_point_is_downstream_of_the_one_parser() {
    use crate::util::rust_source::{braced_block_after, production_code};

    const FILES: &[&str] = &[
        "lib/baumhard/src/util/color_conversion.rs",
        "lib/baumhard/src/font/hex.rs",
    ];
    const NOT_A_PARSER: &[&str] = &["rgba_to_hex", "hsv_to_hex"];
    const ROOT: &str = "hex_to_color";

    // Every `pub …fn NAME(` in `code`, in declaration order.
    fn pub_fn_names(code: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (at, _) in code.match_indices("pub") {
            // Whole word: `public_thing` is not `pub`.
            let mut rest = &code[at + 3..];
            if rest.starts_with('(') {
                // `pub(crate)`, `pub(in path)`, …
                let Some(close) = rest.find(')') else { continue };
                rest = &rest[close + 1..];
            } else if !rest.starts_with(char::is_whitespace) {
                continue;
            }
            rest = rest.trim_start();
            for keyword in ["const ", "unsafe ", "extern "] {
                if let Some(tail) = rest.strip_prefix(keyword) {
                    rest = tail.trim_start();
                }
            }
            let Some(tail) = rest.strip_prefix("fn ") else { continue };
            let name: String = tail
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.push(name);
            }
        }
        out
    }

    // (name, body) for every hex-named `pub` function across both files.
    let mut entries: Vec<(String, String)> = Vec::new();
    for path in FILES {
        let code = production_code(path);
        assert!(
            !code.trim().is_empty(),
            "{path} produced no production code — the scan below would pass vacuously"
        );
        for name in pub_fn_names(&code) {
            if !name.contains("hex") || NOT_A_PARSER.contains(&name.as_str()) {
                continue;
            }
            let body = braced_block_after(&code, &format!("fn {name}("))
                .unwrap_or_else(|| panic!("{path} declares `{name}` with no body"));
            entries.push((name, body.to_string()));
        }
    }
    assert!(
        entries.iter().any(|(name, _)| name == ROOT),
        "the scan must find `{ROOT}` itself, or it is not reading the parser's own module"
    );
    assert!(
        entries.len() >= 7,
        "expected at least the parser and its six downstream entry points; found {:?}",
        entries.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );

    // Grow outward from the root: a function is downstream when its
    // body names something already known to be.
    let mut downstream: Vec<&str> = vec![ROOT];
    loop {
        let before = downstream.len();
        for (name, body) in &entries {
            if downstream.contains(&name.as_str()) {
                continue;
            }
            if downstream.iter().any(|known| body.contains(known)) {
                downstream.push(name);
            }
        }
        if downstream.len() == before {
            break;
        }
    }

    for (name, _) in &entries {
        assert!(
            downstream.contains(&name.as_str()),
            "`{name}` takes a hex string but never reaches `{ROOT}` — either route it \
             through the one parser, or, if it does not parse hex at all, name it in \
             NOT_A_PARSER with the reason"
        );
    }
}
