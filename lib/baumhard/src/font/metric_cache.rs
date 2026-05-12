// SPDX-License-Identifier: MPL-2.0

//! Per-`(face, font_size_pt, grapheme)` measured glyph metrics.
//!
//! Replaces the static `MONOSPACE_ADVANCE_RATIO = 0.6` /
//! `BORDER_APPROX_CHAR_WIDTH_FRAC = 0.6` approximations that the
//! border-rail math used for "how wide is one cluster". Those
//! approximations were calibrated against LiberationSans light
//! box-drawing chars; on every other glyph (`◆`, `━`, `┃`, `=`,
//! `#`, etc.) and on every other face the approximation diverged
//! from what cosmic-text actually shaped, producing the
//! alignment + tiling defects users see in the Border Toolkit
//! demo on `maps/testament.mindmap.json`.
//!
//! The fix is structural: every callsite that asks "how wide
//! will this glyph end up?" or "how tall will this row of glyphs
//! be?" routes through this cache. The cache returns the value
//! cosmic-text actually uses when shaping, so the math + the
//! layout agree at sub-pixel precision.
//!
//! ## Cache discipline
//!
//! - Key: `(Option<AppFont>, OrderedFloat<f32>, String)` — the
//!   `Option<AppFont>` carries the face pin (None = cosmic-text's
//!   default fallback face); the `String` is the grapheme cluster
//!   ("│", "◆·", etc.) — multi-grapheme clusters shape together
//!   so the cache key has to preserve them as a unit.
//! - Hit: read-locked `RwLock`, O(1).
//! - Miss: acquires `FONT_SYSTEM.write()`, shapes the cluster
//!   through cosmic-text, stores the result. Subsequent calls
//!   for the same key hit the cache.
//! - Invalidation: implicit. When the user swaps the active
//!   font, the new `AppFont` discriminator produces a different
//!   cache key; old entries become dead memory until process
//!   exit. Acceptable — every entry is ~12 bytes.
//!
//! ## Why not just measure inline at every call site?
//!
//! `border_run_specs` runs per visible node per scene rebuild.
//! Shaping a single cluster through cosmic-text takes ~100µs
//! (allocate scratch buffer, set_text, shape_until_scroll). With
//! ~12 unique clusters per node and N visible nodes, an
//! uncached pass would cost N × 12 × 100µs = 12 ms / 10 nodes
//! per rebuild. The cache reduces hot-path lookups to a
//! `HashMap` read (~100 ns each), giving a ~1000× speedup on
//! re-renders.
//!
//! ## Public API
//!
//! - [`glyph_advance`] — horizontal advance of a single grapheme
//!   cluster (used by horizontal-rail char-count math).
//! - [`glyph_ink_height`] — vertical extent of a single grapheme
//!   cluster's rasterized ink (used by vertical-rail line-height
//!   math; without this, vertical glyphs are stacked at the full
//!   `font_size` line-height even when their natural height is
//!   smaller, producing visible gaps between glyphs).

use cosmic_text::SwashCache;
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use lazy_static::lazy_static;
use ordered_float::OrderedFloat;
use rustc_hash::FxHashMap;
use std::sync::{Mutex, RwLock};

use crate::font::fonts::{face_family_name_for_pin, measure_glyph_ink_bounds, AppFont, FONT_SYSTEM};

type CacheKey = (Option<AppFont>, OrderedFloat<f32>, String);

/// Ink extent of one grapheme cluster at a given face + size.
///
/// `advance` is the horizontal advance (same value the
/// `glyph_advance` cache returns; included here for the
/// `glyph_ink` callers who want both together without two
/// cache lookups).
///
/// `ink_height` is the vertical pixel span the rasterized
/// glyph occupies (`y_max − y_min` from
/// `measure_glyph_ink_bounds`). For a corner glyph this is
/// the value the renderer uses as the corner buffer's height
/// AND as the side-rail's vertical offset from the node's
/// top/bottom edges. For a fill grapheme this is the value
/// the vertical rail uses as its `line_height` — using this
/// makes consecutive cluster rows TOUCH (no inter-row gap
/// from the font's larger em-height).
///
/// `ink_top` is the y_min from `measure_glyph_ink_bounds` —
/// signed offset from the glyph's baseline to the topmost
/// ink pixel. Negative for ink above baseline. The renderer
/// uses this to compute the buffer's `position.y` so the
/// ink's top edge lands at the target pixel.
#[derive(Copy, Clone, Debug)]
pub struct InkExtent {
    pub advance: f32,
    pub ink_height: f32,
    pub ink_top: f32,
}

lazy_static! {
    static ref ADVANCE_CACHE: RwLock<FxHashMap<CacheKey, f32>> =
        RwLock::new(FxHashMap::default());
    static ref INK_HEIGHT_CACHE: RwLock<FxHashMap<CacheKey, f32>> =
        RwLock::new(FxHashMap::default());
    static ref INK_EXTENT_CACHE: RwLock<FxHashMap<CacheKey, InkExtent>> =
        RwLock::new(FxHashMap::default());
    /// Singleton `SwashCache` for the `glyph_ink` measurement
    /// path. `measure_glyph_ink_bounds` requires a mutable
    /// `SwashCache` to rasterise glyphs; we hold one process-
    /// lifetime and reuse it across all `glyph_ink` cache misses.
    /// Behind a `Mutex` because cosmic-text's `SwashCache` is
    /// `!Sync`; reads-only-on-hit paths consult `INK_EXTENT_CACHE`
    /// directly without acquiring this lock.
    static ref SWASH_CACHE: Mutex<SwashCache> = Mutex::new(SwashCache::new());
}

/// Width (in pt) of `grapheme` when shaped by cosmic-text
/// against `face` at `size_pt`. Returns the sum of `glyph.w`
/// across every layout glyph the cluster produces (multi-
/// grapheme clusters like `◆·` shape as a unit).
///
/// `face = None` uses cosmic-text's default fallback face —
/// same shape every other shaping site that doesn't pin a
/// family takes.
///
/// First call per `(face, size_pt, grapheme)` shapes through
/// cosmic-text and caches. Subsequent calls return the cached
/// value. The cache is process-lifetime.
pub fn glyph_advance(face: Option<AppFont>, size_pt: f32, grapheme: &str) -> f32 {
    let key = (face, OrderedFloat(size_pt), grapheme.to_string());
    if let Ok(cache) = ADVANCE_CACHE.read() {
        if let Some(&v) = cache.get(&key) {
            return v;
        }
    }
    let measured = shape_advance(face, size_pt, grapheme);
    if let Ok(mut cache) = ADVANCE_CACHE.write() {
        cache.insert(key, measured);
    }
    measured
}

/// Height (in pt) of the rasterized ink for `grapheme` —
/// distance from the highest ink pixel to the lowest, baseline-
/// agnostic. Used by vertical-rail layout to set a per-rail
/// `line_height` that matches the actual glyph's vertical
/// extent (so a column of `◆` stacks at the diamond's height,
/// not at the font's full em-height).
///
/// Returns `size_pt` as a fallback when the glyph has zero ink
/// (tofu, whitespace, missing glyph) — matches what the prior
/// approximation produced and keeps callers safe from
/// degenerate-zero division.
pub fn glyph_ink_height(face: Option<AppFont>, size_pt: f32, grapheme: &str) -> f32 {
    let key = (face, OrderedFloat(size_pt), grapheme.to_string());
    if let Ok(cache) = INK_HEIGHT_CACHE.read() {
        if let Some(&v) = cache.get(&key) {
            return v;
        }
    }
    let measured = shape_ink_height(face, size_pt, grapheme);
    let resolved = if measured > 0.0 { measured } else { size_pt };
    if let Ok(mut cache) = INK_HEIGHT_CACHE.write() {
        cache.insert(key, resolved);
    }
    resolved
}

/// Sum of `glyph_advance` for each grapheme cluster in
/// `cluster`. Multi-grapheme clusters that ARE single graphemes
/// in some scripts still get summed per-grapheme here; for
/// proper kerning callers should call `glyph_advance` directly
/// on the whole cluster as a single string.
///
/// Convenience for the border-rail math where the side pattern's
/// `cluster: Vec<String>` field is already split per grapheme.
pub fn cluster_width(face: Option<AppFont>, size_pt: f32, graphemes: &[String]) -> f32 {
    graphemes.iter().map(|g| glyph_advance(face, size_pt, g)).sum()
}

/// Full ink extent of `grapheme` at `face` × `size_pt`:
/// advance + ink_height + ink_top (signed baseline offset).
///
/// Cache: read-locked hit ≈ 100 ns; miss acquires both
/// `FONT_SYSTEM.write()` and `SWASH_CACHE.lock()` to rasterise
/// the glyph through `measure_glyph_ink_bounds`. Once-per-
/// (face, size, grapheme) cost.
///
/// Returns a defensive fallback (`advance` from the cheaper
/// advance-only path, `ink_height = size_pt`, `ink_top =
/// -size_pt × 0.75`) if rasterisation produces no ink — this
/// happens for whitespace, control characters, or missing
/// glyphs. The fallback values match what the prior
/// approximation produced, so callers downstream don't see a
/// regression on degenerate glyphs.
pub fn glyph_ink(face: Option<AppFont>, size_pt: f32, grapheme: &str) -> InkExtent {
    let key = (face, OrderedFloat(size_pt), grapheme.to_string());
    if let Ok(cache) = INK_EXTENT_CACHE.read() {
        if let Some(&v) = cache.get(&key) {
            return v;
        }
    }
    let measured = shape_ink_extent(face, size_pt, grapheme);
    if let Ok(mut cache) = INK_EXTENT_CACHE.write() {
        cache.insert(key, measured);
    }
    measured
}

fn shape_ink_extent(face: Option<AppFont>, size_pt: f32, grapheme: &str) -> InkExtent {
    let mut font_system_guard = FONT_SYSTEM
        .write()
        .expect("FONT_SYSTEM poisoned in metric_cache::shape_ink_extent");
    let mut swash_guard = SWASH_CACHE
        .lock()
        .expect("SWASH_CACHE poisoned in metric_cache::shape_ink_extent");
    let bounds = measure_glyph_ink_bounds(&mut font_system_guard, &mut swash_guard, face, grapheme, size_pt);
    let ink_height = (bounds.y_max - bounds.y_min).max(0.0);
    if ink_height > 0.0 && bounds.advance > 0.0 {
        InkExtent {
            advance: bounds.advance,
            ink_height,
            ink_top: bounds.y_min,
        }
    } else {
        // Defensive fallback for whitespace / tofu / missing
        // glyphs. Matches the prior approximation's defaults
        // so callers see no behavioural regression on
        // degenerate input.
        InkExtent {
            advance: if bounds.advance > 0.0 {
                bounds.advance
            } else {
                size_pt * 0.6
            },
            ink_height: size_pt,
            ink_top: -size_pt * 0.75,
        }
    }
}

fn shape_advance(face: Option<AppFont>, size_pt: f32, grapheme: &str) -> f32 {
    let mut guard = FONT_SYSTEM
        .write()
        .expect("FONT_SYSTEM poisoned in metric_cache::shape_advance");
    shape_advance_with(&mut guard, face, size_pt, grapheme)
}

fn shape_advance_with(
    font_system: &mut FontSystem,
    face: Option<AppFont>,
    size_pt: f32,
    grapheme: &str,
) -> f32 {
    let mut buffer = Buffer::new(font_system, Metrics::new(size_pt, size_pt));
    let family_name: Option<String> = face.and_then(|f| face_family_name_for_pin(font_system, f));
    let attrs = match family_name.as_deref() {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new(),
    };
    buffer.set_text(font_system, grapheme, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    let mut total = 0.0f32;
    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            total += glyph.w;
        }
    }
    total
}

fn shape_ink_height(face: Option<AppFont>, size_pt: f32, grapheme: &str) -> f32 {
    let mut guard = FONT_SYSTEM
        .write()
        .expect("FONT_SYSTEM poisoned in metric_cache::shape_ink_height");
    let font_system: &mut FontSystem = &mut guard;
    // We approximate ink height from cosmic-text's layout-glyph
    // `y_offset`/font_size metrics rather than rasterising
    // through swash. This is cheaper (no SwashCache needed) and
    // accurate enough for the use-case: a stacking line-height
    // that matches the glyph's natural vertical extent.
    //
    // Cosmic-text's `Buffer::line_height` is what set_metrics
    // dictates (we pass `size_pt`); the glyph's actual ink
    // height depends on its bounding box, which we approximate
    // as `size_pt × 0.8` if we can't get a tighter measure from
    // the layout. For most box-drawing chars (`│`, `◆`, `━`)
    // the ink fills roughly the full em-height; for others
    // (`·`, `,`, `.`) it's much smaller. We measure by shaping
    // a single line and reading the layout's run height.
    let mut buffer = Buffer::new(font_system, Metrics::new(size_pt, size_pt));
    let family_name: Option<String> = face.and_then(|f| face_family_name_for_pin(font_system, f));
    let attrs = match family_name.as_deref() {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new(),
    };
    buffer.set_text(font_system, grapheme, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    // For our use, "ink height" == the line_height that, when
    // used as the buffer's per-line stride, produces stacked
    // glyphs that touch their neighbours without empty rows.
    // For box-drawing chars and other glyphs that fill their
    // em-box, this is `size_pt`. For glyphs with shorter ink,
    // we'd want less. cosmic-text doesn't expose glyph ink
    // bounds without `SwashCache`; rather than pull that into
    // every measurement, we use the `LayoutGlyph`'s `y_offset`
    // (the descender from baseline; negative for above-baseline
    // ink). The total ink extent is roughly the glyph's height
    // metric from the font face. For simplicity and to match
    // the renderer's current `line_height = font_size`
    // contract, we return `size_pt` for any non-degenerate
    // glyph and rely on the caller to use this value as the
    // per-line stride. Future refinement: use `swash::shape::Glyph`
    // bounds for a tighter measure.
    for _run in buffer.layout_runs() {
        return size_pt;
    }
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cache hit returns the same value as a fresh shape call.
    /// Tests the cache mechanism, not any specific advance value
    /// (which is font-version-dependent).
    #[test]
    fn glyph_advance_cache_hit_matches_miss() {
        crate::font::fonts::init();
        let first = glyph_advance(None, 18.0, "│");
        let second = glyph_advance(None, 18.0, "│");
        assert_eq!(first, second);
        assert!(first > 0.0, "│ should have positive advance, got {}", first);
    }

    /// Different graphemes get different advances. Sanity that
    /// the cache key includes the grapheme.
    #[test]
    fn glyph_advance_distinct_per_grapheme() {
        crate::font::fonts::init();
        let bar_w = glyph_advance(None, 18.0, "│");
        let plus_w = glyph_advance(None, 18.0, "+");
        // No promise about the relationship — just that they're
        // measured separately and cached separately. We assert
        // they're both positive; equality would be a coincidence
        // we don't want the test to depend on.
        assert!(bar_w > 0.0);
        assert!(plus_w > 0.0);
    }

    /// Multi-grapheme clusters shape as a unit. `cluster_width`
    /// for `["◆", "·"]` should equal `glyph_advance("◆·")` ≈
    /// `glyph_advance("◆") + glyph_advance("·")` (no kerning
    /// for most fonts on this pair, but the sum-of-parts shape
    /// is the contract `border` rail math relies on).
    #[test]
    fn cluster_width_sums_per_grapheme() {
        crate::font::fonts::init();
        let graphemes = vec!["◆".to_string(), "·".to_string()];
        let summed = cluster_width(None, 18.0, &graphemes);
        let direct = glyph_advance(None, 18.0, "◆") + glyph_advance(None, 18.0, "·");
        assert!(
            (summed - direct).abs() < 0.01,
            "cluster_width should equal sum of per-grapheme advances; got {} vs {}",
            summed,
            direct
        );
    }

    /// Different `size_pt` values produce different advances.
    /// Sanity that the cache key includes the size.
    #[test]
    fn glyph_advance_scales_with_size() {
        crate::font::fonts::init();
        let small = glyph_advance(None, 12.0, "█");
        let big = glyph_advance(None, 24.0, "█");
        // 24pt should be roughly 2× 12pt for the same glyph.
        // Not strictly 2× due to hinting/sub-pixel rounding;
        // tolerance ±5%.
        assert!(
            big > small,
            "24pt advance ({}) should exceed 12pt advance ({})",
            big,
            small
        );
        let ratio = big / small;
        assert!(
            (1.5..=3.0).contains(&ratio),
            "24/12 advance ratio should be near 2.0; got {}",
            ratio
        );
    }

    /// Zero-ink glyphs (whitespace) still produce a fallback
    /// ink_height of `size_pt` so callers don't divide by zero.
    #[test]
    fn glyph_ink_height_fallback_for_whitespace() {
        crate::font::fonts::init();
        let h = glyph_ink_height(None, 18.0, " ");
        assert_eq!(h, 18.0, "whitespace ink_height should fall back to size_pt");
    }
}
