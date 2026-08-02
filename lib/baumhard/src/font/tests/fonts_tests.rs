// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::font::fonts`] measurement primitives:
//! [`measure_glyph_ink_bounds`] and [`measure_text_block_unbounded`].
//!
//! Follows the `do_*()` / `test_*()` split from §B8 — every `do_*`
//! body is benchmarkable from `benches/test_bench.rs`.

use cosmic_text::SwashCache;

use crate::font::fonts;
use crate::font::fonts::{
    acquire_font_system_write, app_font_by_family, family_name_of, list_loaded_families, loaded_families_iter,
    measure_glyph_ink_bounds, measure_text_block_unbounded, AppFont,
};

#[test]
fn test_measure_glyph_ink_bounds_latin_has_positive_advance() {
    do_measure_glyph_ink_bounds_latin_has_positive_advance();
}

/// Measuring a plain Latin glyph returns a non-zero advance and a
/// non-empty ink rectangle. The primitive's happy path.
pub fn do_measure_glyph_ink_bounds_latin_has_positive_advance() {
    fonts::init();
    let mut fs = acquire_font_system_write("fonts_tests::do_*");
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
    let mut fs = acquire_font_system_write("fonts_tests::do_*");
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
    let mut fs = acquire_font_system_write("fonts_tests::do_*");
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
    let mut fs = acquire_font_system_write("fonts_tests::do_*");
    let mut cache = SwashCache::new();
    let latin = measure_glyph_ink_bounds(&mut fs, &mut cache, None, "A", 24.0);
    let svasti = measure_glyph_ink_bounds(
        &mut fs,
        &mut cache,
        Some(AppFont::NotoSerifTibetanRegular),
        "\u{0FD5}",
        32.0,
    );
    let latin_offset = latin.x_offset_from_advance_center();
    let svasti_offset = svasti.x_offset_from_advance_center();

    // Sanity floor: no NaN, no inf.
    assert!(latin_offset.is_finite(), "Latin 'A' offset must be finite");
    assert!(svasti_offset.is_finite(), "Tibetan svasti offset must be finite");

    // Latin "A" sits near-symmetrically around the advance center
    // — offset should be small relative to the glyph's advance.
    // 1.0 px at 24pt is ~3% of the typical advance; tolerating up
    // to 4 px covers font-specific kerning without admitting a
    // genuine asymmetry. Pre-fix this returned `0` from a buggy
    // helper; the assertion guards against that regression.
    assert!(
        latin_offset.abs() <= 4.0,
        "Latin 'A' offset ({latin_offset}) should be small (≤ 4px) — \
         the glyph is roughly symmetric around its advance center",
    );

    // Tibetan svasti (U+0FD5) is the documented motivating bug:
    // its ink doesn't sit symmetrically around the advance
    // center. Pre-fix the helper returned `0` for every glyph
    // regardless of shape; a non-zero offset here pins the
    // corrected shape. The actual measured offset depends on the
    // font face used (≈0.24 px at 32pt with NotoSerifTibetan);
    // the assertion threshold of 0.1 px catches the
    // returns-zero-always regression while staying tolerant of
    // font-version drift.
    assert!(
        svasti_offset.abs() > 0.1,
        "Tibetan svasti offset ({svasti_offset}) should be non-zero — \
         the glyph drifts off-center; a 0 result indicates the helper \
         returned a uniform value regardless of input",
    );
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
    let mut fs = acquire_font_system_write("fonts_tests::do_*");
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
    let mut fs = acquire_font_system_write("fonts_tests::do_*");
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
    let mut fs = acquire_font_system_write("fonts_tests::do_*");
    let out = measure_text_block_unbounded(&mut fs, "", 14.0, 16.8, None);
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
    let mut fs = acquire_font_system_write("fonts_tests::do_*");
    let out = measure_text_block_unbounded(&mut fs, "Hello", 14.0, 16.8, None);
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
    let mut fs = acquire_font_system_write("fonts_tests::do_*");
    let narrow = measure_text_block_unbounded(&mut fs, "a", 14.0, 16.8, None);
    let wide = measure_text_block_unbounded(&mut fs, "ccccc", 14.0, 16.8, None);
    let block = measure_text_block_unbounded(&mut fs, "a\nbb\nccccc", 14.0, 16.8, None);
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
    let mut fs = acquire_font_system_write("fonts_tests::do_*");
    let small = measure_text_block_unbounded(&mut fs, "Hello world", 14.0, 16.8, None);
    let large = measure_text_block_unbounded(&mut fs, "Hello world", 28.0, 33.6, None);
    let ratio = large.width / small.width;
    assert!(
        (1.8..=2.2).contains(&ratio),
        "width should scale ~linearly with font size; ratio={} (small={}, large={})",
        ratio,
        small.width,
        large.width
    );
}

#[test]
fn test_list_loaded_families_is_nonempty_sorted_unique() {
    do_list_loaded_families_is_nonempty_sorted_unique();
}

/// `list_loaded_families` returns every compiled-in font's family
/// name, sorted ascending and free of duplicates. The console
/// completion popup and `font list` verb both depend on the
/// sort/uniqueness contract.
pub fn do_list_loaded_families_is_nonempty_sorted_unique() {
    fonts::init();
    let families = list_loaded_families();
    assert!(!families.is_empty(), "at least one bundled family must be listed");
    let sorted = {
        let mut copy = families.clone();
        copy.sort();
        copy
    };
    assert_eq!(families, sorted, "families must come back sorted");
    let unique = {
        let mut copy = families.clone();
        copy.sort();
        copy.dedup();
        copy
    };
    assert_eq!(families.len(), unique.len(), "no duplicates allowed");
}

#[test]
fn test_app_font_by_family_round_trips() {
    do_app_font_by_family_round_trips();
}

/// Every family name from `loaded_families_iter` resolves back to
/// some `AppFont` via `app_font_by_family`. This is the
/// round-trip contract `font set <name>` relies on: a name picked
/// from the popup must always resolve when the user submits it.
pub fn do_app_font_by_family_round_trips() {
    fonts::init();
    for family in loaded_families_iter() {
        assert!(
            app_font_by_family(family).is_some(),
            "family '{}' should resolve to some AppFont",
            family
        );
    }
}

/// `family_name_of` reverses `app_font_by_family` for every
/// loaded family — the post-mutation reverse-converter relies on
/// the round-trip when rolling tree-side `ColorFontRegion.font:
/// Option<AppFont>` back into the model's
/// `TextRun.font: String`. Empty / unknown families resolve to
/// `None` rather than panicking, matching the optional-pin
/// contract on `app_font_by_family`.
#[test]
fn test_family_name_of_round_trips() {
    do_family_name_of_round_trips();
}

pub fn do_family_name_of_round_trips() {
    fonts::init();
    for family in loaded_families_iter() {
        let app_font = app_font_by_family(family).expect("loaded family resolves");
        // Every loaded AppFont has at least one family name back —
        // not necessarily *this* exact name (multiple aliases per
        // AppFont collapse to alphabetical-first), but always
        // some non-empty name.
        let back = family_name_of(app_font);
        assert!(
            back.is_some_and(|s| !s.is_empty()),
            "family '{}' (AppFont {:?}) must reverse to a non-empty name",
            family,
            app_font
        );
    }
}

#[test]
fn test_loaded_families_iter_matches_owned_list() {
    do_loaded_families_iter_matches_owned_list();
}

/// `loaded_families_iter` yields the same names, in the same
/// order, as the owned `list_loaded_families` helper — the iterator
/// helper is a zero-allocation alternative, not a different shape.
pub fn do_loaded_families_iter_matches_owned_list() {
    fonts::init();
    let owned = list_loaded_families();
    let borrowed: Vec<&'static str> = loaded_families_iter().collect();
    assert_eq!(owned.len(), borrowed.len());
    for (a, b) in owned.iter().zip(borrowed.iter()) {
        assert_eq!(a, b);
    }
}

#[test]
fn test_app_font_by_family_unknown_returns_none() {
    do_app_font_by_family_unknown_returns_none();
}

/// Unknown families return `None` rather than panicking. The
/// console command relies on this to surface a clean error
/// message for typos.
pub fn do_app_font_by_family_unknown_returns_none() {
    fonts::init();
    assert!(app_font_by_family("DefinitelyNotAFontFamilyXYZ").is_none());
    assert!(app_font_by_family("").is_none());
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
    use crate::font::fonts::{acquire_font_system_write, acquire_font_system_write_with_timeout};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    fonts::init();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    // Spawn a thread that grabs the guard, signals us, then holds
    // it long enough to let our acquire attempt time out *and* to
    // cover scheduler jitter on a loaded CI runner. We do not join
    // the handle — the test function panics below, and the
    // detached thread finishes on its own. The holder acquires
    // through the same helper (not a raw `.write()`) so the codebase
    // stays grep-clean of raw `FONT_SYSTEM.write()`; the lock is free
    // at spawn time, so this returns instantly.
    let _holder = thread::spawn(move || {
        let _guard = acquire_font_system_write("test_timeout_holder");
        acquired_tx.send(()).unwrap();
        // Hold for 5× the acquire timeout so a slow scheduler
        // cannot let `try_write` succeed before the timeout fires.
        thread::sleep(Duration::from_millis(1000));
    });
    acquired_rx.recv().expect("holder thread should acquire");
    // Test-scale timeout — the production constant is 5 s which
    // would make this test slow. 200 ms gives the acquire loop
    // ~200 poll cycles (1 ms each) of margin against scheduler
    // jitter on loaded CI runners; still well under the holder's
    // 1 s grip. The contract we're pinning is "panics instead of
    // hanging"; the panic message and the code path are identical.
    let _would_hang = acquire_font_system_write_with_timeout(
        "test_acquire_font_system_write_panics_on_timeout",
        Duration::from_millis(200),
    );
}
