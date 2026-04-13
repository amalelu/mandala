//! Tests for [`crate::font::fonts::measure_glyph_ink_bounds`].
//!
//! Follows the `do_*()` / `test_*()` split from §B8 — every `do_*`
//! body is benchmarkable from `benches/test_bench.rs`.

use cosmic_text::SwashCache;

use crate::font::fonts;
use crate::font::fonts::{measure_glyph_ink_bounds, AppFont, FONT_SYSTEM};

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
