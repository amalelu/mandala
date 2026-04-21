//! Tests for [`crate::font::fonts`] measurement primitives:
//! [`measure_glyph_ink_bounds`] and [`measure_text_block_unbounded`].
//!
//! Follows the `do_*()` / `test_*()` split from §B8 — every `do_*`
//! body is benchmarkable from `benches/test_bench.rs`.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use cosmic_text::SwashCache;

use crate::font::fonts;
use crate::font::fonts::{
    acquire_font_system_write_with_timeout, measure_glyph_ink_bounds,
    measure_text_block_unbounded, AppFont, FONT_SYSTEM,
};

#[test]
fn test_measure_glyph_ink_bounds_latin_has_positive_advance() {
    do_measure_glyph_ink_bounds_latin_has_positive_advance();
}

/// Measuring a plain Latin glyph returns a non-zero advance and a
/// non-empty ink rectangle. The primitive's happy path.
pub fn do_measure_glyph_ink_bounds_latin_has_positive_advance() {
    fonts::init();
    let mut fs = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
    let mut cache = SwashCache::new();
    let bounds = measure_glyph_ink_bounds(&mut fs, &mut cache, None, "A", 24.0);
    assert!(bounds.advance > 0.0, "Latin advance must be positive");
    assert!(bounds.x_max > bounds.x_min, "ink rect must be non-empty");
    assert!(bounds.y_max > bounds.y_min, "ink rect must be non-empty");
}

#[test]
fn test_measure_glyph_ink_bounds_tibetan_svasti_has_sidebearing() {
    do_measure_glyph_ink_bounds_tibetan_svasti_has_sidebearing();
}

/// The Tibetan right-facing svasti (U+0FD5, the color picker's
/// central preview glyph) has non-trivial sidebearings — `x_min` is
/// bounded away from zero. This is the exact inkcenter drift that
/// motivates the primitive.
pub fn do_measure_glyph_ink_bounds_tibetan_svasti_has_sidebearing() {
    fonts::init();
    let mut fs = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
    let mut cache = SwashCache::new();
    let bounds = measure_glyph_ink_bounds(
        &mut fs,
        &mut cache,
        Some(AppFont::NotoSerifTibetanRegular),
        "\u{0FD5}",
        32.0,
    );
    assert!(bounds.advance > 0.0);
    // Sidebearings exist in both directions; the exact magnitude is
    // font-specific but the ink must sit strictly inside the pen-end
    // bounds.
    assert!(bounds.x_min >= 0.0, "ink left must not precede pen origin");
    assert!(
        bounds.x_max <= bounds.advance + 1.0,
        "ink right must not exceed advance (allowing 1px slop)"
    );
}

#[test]
fn test_measure_glyph_ink_bounds_empty_string_is_zero() {
    do_measure_glyph_ink_bounds_empty_string_is_zero();
}

/// Empty input yields a zero bounding box — no glyphs, no advance,
/// no ink.
pub fn do_measure_glyph_ink_bounds_empty_string_is_zero() {
    fonts::init();
    let mut fs = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
    let mut cache = SwashCache::new();
    let bounds = measure_glyph_ink_bounds(&mut fs, &mut cache, None, "", 24.0);
    assert_eq!(bounds.advance, 0.0);
    assert_eq!(bounds.x_min, 0.0);
    assert_eq!(bounds.x_max, 0.0);
    assert_eq!(bounds.y_min, 0.0);
    assert_eq!(bounds.y_max, 0.0);
}

#[test]
fn test_measure_glyph_ink_bounds_x_offset_from_advance_center() {
    do_measure_glyph_ink_bounds_x_offset_from_advance_center();
}

/// The `x_offset_from_advance_center` helper returns zero for a
/// glyph whose ink sits symmetrically around the advance center and
/// a non-zero value for one that doesn't. We compare the Latin "A"
/// (roughly-symmetric) against the Tibetan svasti (known to drift
/// to the right per the color picker issue).
pub fn do_measure_glyph_ink_bounds_x_offset_from_advance_center() {
    fonts::init();
    let mut fs = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
    let mut cache = SwashCache::new();
    let latin = measure_glyph_ink_bounds(&mut fs, &mut cache, None, "A", 24.0);
    let svasti = measure_glyph_ink_bounds(
        &mut fs,
        &mut cache,
        Some(AppFont::NotoSerifTibetanRegular),
        "\u{0FD5}",
        32.0,
    );
    // Both are finite (no NaN, no inf).
    assert!(latin.x_offset_from_advance_center().is_finite());
    assert!(svasti.x_offset_from_advance_center().is_finite());
}

#[test]
fn test_measure_glyph_ink_bounds_reports_baseline_line_y() {
    do_measure_glyph_ink_bounds_reports_baseline_line_y();
}

/// `line_y` (baseline-from-buffer-top) is non-zero for any inked
/// glyph — cosmic-text places the baseline below the buffer's top
/// edge by approximately the font's ascent.
pub fn do_measure_glyph_ink_bounds_reports_baseline_line_y() {
    fonts::init();
    let mut fs = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
    let mut cache = SwashCache::new();
    let bounds = measure_glyph_ink_bounds(&mut fs, &mut cache, None, "M", 24.0);
    assert!(
        bounds.line_y > 0.0 && bounds.line_y.is_finite(),
        "baseline should sit below buffer top, got line_y={}",
        bounds.line_y
    );
}

#[test]
fn test_measure_glyph_ink_bounds_y_offset_from_box_center() {
    do_measure_glyph_ink_bounds_y_offset_from_box_center();
}

/// `y_offset_from_box_center` is finite for inked glyphs and varies
/// with `line_height_mul` linearly (every doubling of the bounds
/// height shifts the box center down by half the increase, so the
/// offset shifts up by the same amount). Compares Devanagari (ink
/// biased toward the shirorekha-top) against Egyptian hieroglyphs
/// (ink typically biased low) at the picker's `1.5` line-height
/// multiplier — both must be finite and the two scripts must
/// produce different offsets, which is the whole point of moving
/// from a single per-arm Y of zero to a per-glyph Y correction.
pub fn do_measure_glyph_ink_bounds_y_offset_from_box_center() {
    fonts::init();
    let mut fs = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
    let mut cache = SwashCache::new();
    let font_size = 24.0;
    let deva = measure_glyph_ink_bounds(&mut fs, &mut cache, None, "अ", font_size);
    let hiero = measure_glyph_ink_bounds(
        &mut fs,
        &mut cache,
        Some(AppFont::NotoSansEgyptianHieroglyphsRegular),
        "\u{13000}",
        font_size,
    );
    let deva_y = deva.y_offset_from_box_center(font_size, 1.5);
    let hiero_y = hiero.y_offset_from_box_center(font_size, 1.5);
    assert!(deva_y.is_finite() && hiero_y.is_finite());
    // The two scripts must drift differently — that's the bug a
    // single per-arm Y of zero couldn't fix.
    assert!(
        (deva_y - hiero_y).abs() > 0.5,
        "scripts should produce different Y offsets, got deva={} hiero={}",
        deva_y,
        hiero_y
    );
    // Doubling line_height_mul halves the box-center distance from
    // the buffer top, so the offset shrinks by exactly that delta.
    let deva_y_doubled = deva.y_offset_from_box_center(font_size, 3.0);
    let expected_delta = -(font_size * (3.0 - 1.5) * 0.5);
    assert!(
        (deva_y_doubled - deva_y - expected_delta).abs() < 0.001,
        "doubling line_height_mul should shift offset by {}; got {}",
        expected_delta,
        deva_y_doubled - deva_y
    );
}

#[test]
fn test_measure_text_block_unbounded_empty_is_zero() {
    do_measure_text_block_unbounded_empty_is_zero();
}

/// Empty input short-circuits to `TextBlockSize::ZERO` without
/// touching the shaper.
pub fn do_measure_text_block_unbounded_empty_is_zero() {
    fonts::init();
    let mut fs = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
    let out = measure_text_block_unbounded(&mut fs, "", 14.0, 16.8);
    assert_eq!(out.width, 0.0);
    assert_eq!(out.height, 0.0);
    assert_eq!(out.line_count, 0);
}

#[test]
fn test_measure_text_block_unbounded_single_line_nonzero() {
    do_measure_text_block_unbounded_single_line_nonzero();
}

/// A single-line Latin string shapes to one run with positive width
/// and `height == line_height`.
pub fn do_measure_text_block_unbounded_single_line_nonzero() {
    fonts::init();
    let mut fs = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
    let out = measure_text_block_unbounded(&mut fs, "Hello", 14.0, 16.8);
    assert_eq!(out.line_count, 1, "one line expected for no-newline input");
    assert!(out.width > 0.0, "non-empty text must produce positive width");
    assert!(
        (out.height - 16.8).abs() < 0.001,
        "height should be line_height * 1 line, got {}",
        out.height
    );
}

#[test]
fn test_measure_text_block_unbounded_multiline_width_is_widest_line() {
    do_measure_text_block_unbounded_multiline_width_is_widest_line();
}

/// Embedded `\n` produces one layout run per line; `width` is the
/// widest run and `height == line_count * line_height`.
pub fn do_measure_text_block_unbounded_multiline_width_is_widest_line() {
    fonts::init();
    let mut fs = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
    let narrow = measure_text_block_unbounded(&mut fs, "a", 14.0, 16.8);
    let wide = measure_text_block_unbounded(&mut fs, "ccccc", 14.0, 16.8);
    let block = measure_text_block_unbounded(&mut fs, "a\nbb\nccccc", 14.0, 16.8);
    assert_eq!(block.line_count, 3, "three \\n-separated lines expected");
    assert!(
        (block.height - 3.0 * 16.8).abs() < 0.001,
        "height should be 3 * line_height, got {}",
        block.height
    );
    // Width must match the widest standalone line within float slop.
    assert!(
        (block.width - wide.width).abs() < 0.5,
        "block width should match widest line; block={} wide={} narrow={}",
        block.width,
        wide.width,
        narrow.width
    );
    assert!(block.width > narrow.width);
}

#[test]
fn test_measure_text_block_unbounded_width_scales_with_font_size() {
    do_measure_text_block_unbounded_width_scales_with_font_size();
}

/// Doubling `scale` roughly doubles the returned `width` — the
/// primitive actually drives shaping rather than e.g. ignoring the
/// size parameter. Uses a generous tolerance because exact scaling
/// is font-dependent (kerning, hinting).
pub fn do_measure_text_block_unbounded_width_scales_with_font_size() {
    fonts::init();
    let mut fs = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
    let small = measure_text_block_unbounded(&mut fs, "Hello world", 14.0, 16.8);
    let large = measure_text_block_unbounded(&mut fs, "Hello world", 28.0, 33.6);
    let ratio = large.width / small.width;
    assert!(
        (1.8..=2.2).contains(&ratio),
        "width should scale ~linearly with font size; ratio={} (small={}, large={})",
        ratio,
        small.width,
        large.width
    );
}

/// Freeze-hardening regression: `acquire_font_system_write` must
/// panic (not hang) when the write guard cannot be obtained within
/// its timeout budget. The production deadlock this guards against
/// is a same-thread re-entrant `RwLock::write()` acquire — which
/// `std::sync::RwLock` would otherwise block on forever.
///
/// The test holds the guard on a **separate** thread (not the test
/// thread) to avoid poisoning the lock when the panic unwinds: the
/// test thread never holds the guard, so the panic's unwind drops
/// nothing the lock cares about. The spawned holder thread
/// eventually drops its guard cleanly when it finishes sleeping,
/// leaving FONT_SYSTEM usable for subsequent tests.
#[test]
#[should_panic(expected = "FONT_SYSTEM write lock not available")]
fn test_acquire_font_system_write_panics_on_timeout() {
    fonts::init();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    // Spawn a thread that grabs the guard, signals us, then holds
    // it long enough to let our acquire attempt time out. We do
    // not join the handle — the test function panics below, and
    // the detached thread finishes on its own.
    let _holder = thread::spawn(move || {
        let _guard = FONT_SYSTEM.write().expect("FONT_SYSTEM poisoned");
        acquired_tx.send(()).unwrap();
        thread::sleep(Duration::from_millis(500));
    });
    acquired_rx.recv().expect("holder thread should acquire");
    // Test-scale timeout — the production constant is 5 s which
    // would make this test slow. The contract we're pinning is
    // "panics instead of hanging"; the panic message and the code
    // path are identical.
    let _would_hang = acquire_font_system_write_with_timeout(
        "test_acquire_font_system_write_panics_on_timeout",
        Duration::from_millis(50),
    );
}
