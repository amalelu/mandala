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
//! - Key: two levels. The outer map is keyed `(Option<AppFont>,
//!   OrderedFloat<f32>)` — the face pin (None = cosmic-text's
//!   default fallback face) and the size in points, both `Copy`.
//!   The inner map is keyed by the grapheme cluster ("│", "◆·",
//!   etc.), which multi-grapheme clusters preserve as a unit
//!   because they shape together.
//!
//!   **The split is what lets a lookup probe with a `&str`.** A
//!   single tuple key carrying a `String` cannot be borrowed from
//!   `(face, size, &str)` — `HashMap::get` needs `K: Borrow<Q>`,
//!   and no `Q` borrows out of a tuple — so the flat shape this
//!   replaced built an owned `String` before *every* probe, on hits
//!   as well as misses, at each of the four entry points. Splitting
//!   the key moves the only borrowable component into a map of its
//!   own, where `&str` is exactly what `get` takes. The two levels
//!   carry the same equivalence classes as the flat key: nothing
//!   merges and nothing splits, so no measurement changes.
//! - Hit: read-locked `RwLock`, O(1); no `FONT_SYSTEM` access, and
//!   no allocation.
//! - Miss (unlocked caller): acquires the `FONT_SYSTEM` write
//!   guard through `acquire_font_system_write` — the timeout-
//!   guarded helper, never a raw `.write()` (CONVENTIONS §B5) —
//!   shapes the cluster through cosmic-text, stores the result.
//!   Subsequent calls for the same key hit the cache.
//! - Miss (caller already holding the guard): use the
//!   `*_with(&mut FontSystem, ...)` variants, which shape through
//!   the passed guard instead of acquiring a second one. The
//!   renderer's border-rebuild loop holds the write guard across
//!   the whole loop and MUST use these — a nested same-thread
//!   acquire is a guaranteed deadlock. This mirrors the
//!   `measure_glyph_ink_bounds` / `measure_text_block_unbounded`
//!   composable design in `font/fonts.rs`.
//! - Invalidation: implicit. When the user swaps the active
//!   font, the new `AppFont` discriminator produces a different
//!   outer key; the previous face's inner map becomes dead memory
//!   until process exit.
//!
//!   What bounds that is the input rather than a byte count. An
//!   entry exists only for a `(face, size, cluster)` triple
//!   something actually asked to measure, so the dead set is at
//!   most what the session already measured — and a session cannot
//!   ask for a face it cannot select or a cluster it has not
//!   rendered. This bullet used to close with "every entry is ~12
//!   bytes", which is wrong twice over: 12 bytes is the size of
//!   [`InkExtent`](crate::font::metric_cache::InkExtent) alone, so
//!   it counted neither the `String` key (whose text is on the
//!   heap) nor the table's per-slot overhead, and it did not
//!   describe an `ADVANCE_CACHE` entry at all.
//!
//! ## Why not just measure inline at every call site?
//!
//! `border_run_specs` runs per visible node per scene rebuild and
//! asks for roughly a dozen clusters per node. Measuring one
//! cluster means allocating a cosmic-text `Buffer`, setting its
//! text, running `shape_until_scroll` and walking the layout runs
//! — and for [`glyph_ink`](crate::font::metric_cache::glyph_ink),
//! rasterizing the glyph through a `SwashCache` — all of it while
//! holding the `FONT_SYSTEM` **write** guard, the lock the whole
//! renderer serializes on
//! (§B5). A hit is a hash lookup under a **read** guard: it takes
//! no write lock, reaches no font system, and allocates nothing.
//!
//! **The lock round-trip per lookup is untouched by this**, and the
//! issue that motivated the key change (#36 item 3) names it in the
//! same breath as the allocation. A hit still acquires the read guard
//! once per call, so an N-cluster border pattern going through
//! [`cluster_width`](crate::font::metric_cache::cluster_width) is
//! still N acquires. Removing that needs a batching entry point that
//! holds one guard across a run of clusters — a new surface rather
//! than a key change — and it has not been built.
//!
//! That difference is stated in operations rather than in times on
//! purpose. This paragraph used to carry three numbers — "~100µs"
//! to shape, "~100 ns" per hit, and a "~1000× speedup" derived
//! from them — and `lib/baumhard/CONVENTIONS.md` §B7 requires a
//! main-against-main control row for any of the three. There was
//! none, so they are gone rather than restated.
//!
//! ## Public API
//!
//! - [`glyph_advance`](crate::font::metric_cache::glyph_advance) —
//!   horizontal advance of a single grapheme cluster (used by
//!   horizontal-rail char-count math).
//! - [`glyph_ink`](crate::font::metric_cache::glyph_ink) — the full
//!   ink extent (height plus top offset) of a single cluster, which
//!   is what the vertical-rail line-height math actually reads.

use cosmic_text::SwashCache;
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use lazy_static::lazy_static;
use ordered_float::OrderedFloat;
use rustc_hash::FxHashMap;
use std::sync::{Mutex, RwLock};

use crate::font::fonts::{
    acquire_font_system_write, ensure_warm, face_family_name_for_pin, measure_glyph_ink_bounds, AppFont,
};

/// Outer cache key: the face pin plus the size in points. `Copy`,
/// so building one to probe with costs nothing on the heap.
///
/// `OrderedFloat` rather than the raw bits, so the size folds into
/// buckets exactly as it did under the flat key — `-0.0` with `0.0`,
/// and every `NaN` together.
type SizedFace = (Option<AppFont>, OrderedFloat<f32>);

/// A measured-metric cache: `(face, size) -> cluster -> value`.
///
/// The nesting is not organization, it is the probe. See the module
/// header's "Cache discipline" for why one flat tuple key cannot be
/// looked up without allocating and this can.
type MetricCache<V> = FxHashMap<SizedFace, FxHashMap<String, V>>;

/// Read `cache` for an already-measured value.
///
/// Takes the cluster as `&str` and never owns it: the outer key is
/// two `Copy` fields and the inner lookup goes through
/// `HashMap<String, V>`'s `Borrow<str>`. `None` on a miss **and** on
/// a poisoned lock — a caller that cannot read the cache re-measures
/// rather than failing, which is the same posture the code this
/// replaced took.
///
/// Cost: one read-guard acquire and two hash lookups. No allocation.
fn probe<V: Copy>(
    cache: &RwLock<MetricCache<V>>,
    face: Option<AppFont>,
    size_pt: f32,
    grapheme: &str,
) -> Option<V> {
    let guard = cache.read().ok()?;
    guard.get(&(face, OrderedFloat(size_pt)))?.get(grapheme).copied()
}

/// Record a freshly measured value.
///
/// This is the one place a `String` is built from `grapheme`, and it
/// runs once per `(face, size, cluster)` triple for the process's
/// lifetime. A poisoned lock drops the value silently: the next
/// caller measures it again.
///
/// Cost: one write-guard acquire, two hash lookups, and one `String`
/// allocation on the first store for a cluster.
fn store<V>(cache: &RwLock<MetricCache<V>>, face: Option<AppFont>, size_pt: f32, grapheme: &str, value: V) {
    if let Ok(mut guard) = cache.write() {
        guard
            .entry((face, OrderedFloat(size_pt)))
            .or_default()
            .insert(grapheme.to_string(), value);
    }
}

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
    static ref ADVANCE_CACHE: RwLock<MetricCache<f32>> = RwLock::new(FxHashMap::default());
    static ref INK_EXTENT_CACHE: RwLock<MetricCache<InkExtent>> = RwLock::new(FxHashMap::default());
    /// Singleton `SwashCache` for the `glyph_ink` measurement
    /// path. `measure_glyph_ink_bounds` requires a mutable
    /// `SwashCache` to rasterize glyphs; we hold one process-
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
    // Fast path: a cache hit needs no `FONT_SYSTEM` access at all,
    // and probes with the borrowed cluster rather than an owned copy.
    if let Some(v) = probe(&ADVANCE_CACHE, face, size_pt, grapheme) {
        return v;
    }
    // Miss: warm the font lazy-statics BEFORE acquiring so a
    // `Some(face)` pin lookup under the guard can't re-enter
    // `load_fonts` (see `ensure_warm`), then acquire through the
    // helper (the lock is private; there is no raw `.write()`) and
    // shape. A caller that already holds the guard panics here at once,
    // naming both sites, rather than waiting on itself.
    ensure_warm();
    let mut guard = acquire_font_system_write("metric_cache::glyph_advance");
    glyph_advance_with(&mut guard, face, size_pt, grapheme)
}

/// [`glyph_advance`] for callers that already hold the `FONT_SYSTEM`
/// write guard (the renderer's border-rebuild loop). Shapes any
/// cache miss through the passed `font_system` instead of acquiring
/// a second guard — the composable design §B5 and the
/// `measure_glyph_ink_bounds` primitive share.
pub fn glyph_advance_with(
    font_system: &mut FontSystem,
    face: Option<AppFont>,
    size_pt: f32,
    grapheme: &str,
) -> f32 {
    if let Some(v) = probe(&ADVANCE_CACHE, face, size_pt, grapheme) {
        return v;
    }
    let measured = shape_advance_with(font_system, face, size_pt, grapheme);
    store(&ADVANCE_CACHE, face, size_pt, grapheme, measured);
    measured
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
/// Cache: a hit is two hash lookups under the read guard, with no
/// `FONT_SYSTEM` access and no allocation. A miss acquires the
/// `FONT_SYSTEM` write guard (via `acquire_font_system_write`) plus
/// `SWASH_CACHE.lock()` to rasterize the glyph through
/// `measure_glyph_ink_bounds`. Once-per-(face, size, grapheme) cost.
/// Callers already holding the guard must use [`glyph_ink_with`].
///
/// Returns a defensive fallback (`advance` from the cheaper
/// advance-only path, `ink_height = size_pt`, `ink_top =
/// -size_pt × 0.75`) if rasterization produces no ink — this
/// happens for whitespace, control characters, or missing
/// glyphs. The fallback values match what the prior
/// approximation produced, so callers downstream don't see a
/// regression on degenerate glyphs.
pub fn glyph_ink(face: Option<AppFont>, size_pt: f32, grapheme: &str) -> InkExtent {
    // Fast path: a cache hit needs no `FONT_SYSTEM` access, and
    // probes with the borrowed cluster rather than an owned copy.
    if let Some(v) = probe(&INK_EXTENT_CACHE, face, size_pt, grapheme) {
        return v;
    }
    // Warm before acquiring (see `ensure_warm`) so a `Some(face)` pin
    // lookup under the guard can't re-enter `load_fonts`.
    ensure_warm();
    let mut guard = acquire_font_system_write("metric_cache::glyph_ink");
    glyph_ink_with(&mut guard, face, size_pt, grapheme)
}

/// [`glyph_ink`] for callers that already hold the `FONT_SYSTEM`
/// write guard (the renderer's border-rebuild loop). Rasterises any
/// cache miss through the passed `font_system` — plus its own
/// `SWASH_CACHE.lock()`, which is a distinct lock and so re-entrancy-
/// safe — instead of acquiring a second `FONT_SYSTEM` guard.
pub fn glyph_ink_with(
    font_system: &mut FontSystem,
    face: Option<AppFont>,
    size_pt: f32,
    grapheme: &str,
) -> InkExtent {
    if let Some(v) = probe(&INK_EXTENT_CACHE, face, size_pt, grapheme) {
        return v;
    }
    let measured = shape_ink_extent_with(font_system, face, size_pt, grapheme);
    store(&INK_EXTENT_CACHE, face, size_pt, grapheme, measured);
    measured
}

fn shape_ink_extent_with(
    font_system: &mut FontSystem,
    face: Option<AppFont>,
    size_pt: f32,
    grapheme: &str,
) -> InkExtent {
    let mut swash_guard = SWASH_CACHE
        .lock()
        .expect("SWASH_CACHE poisoned in metric_cache::shape_ink_extent");
    let bounds = measure_glyph_ink_bounds(font_system, &mut swash_guard, face, grapheme, size_pt);
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
        // so callers see no behavioral regression on
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
    buffer.set_text(grapheme, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    let mut total = 0.0f32;
    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            total += glyph.w;
        }
    }
    total
}
