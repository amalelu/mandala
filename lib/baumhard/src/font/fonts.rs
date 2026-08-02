// SPDX-License-Identifier: MPL-2.0

//! Compiled-in font table, shared `FontSystem`, and cosmic-text
//! editor factories. The `AppFont` enum + `FONT_DATA` array are
//! emitted by `build.rs` at crate-compile time so the binary carries
//! every font it might need without touching the filesystem at run
//! time.

use std::sync::{Arc, OnceLock, RwLock, RwLockWriteGuard, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

use cosmic_text::fontdb::Source;
use cosmic_text::fontdb::ID;
use cosmic_text::FontSystem;
use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, SwashCache};
use lazy_static::lazy_static;
use log::debug;
use rand::seq::IteratorRandom;
use rustc_hash::FxHashMap;
use tinyvec::TinyVec;

// Serde derives are used by the generated AppFont enum below.
use serde::{Deserialize, Serialize};
// Build-time generated: defines `AppFont` and `FONT_DATA`. The
// naming and ordering rules the generator follows live in
// `crate::font::name_rules`, which `build.rs` shares by `include!`.
include!(concat!(env!("OUT_DIR"), "/generated_fonts_data.rs"));

fn load_font_sources() -> FxHashMap<AppFont, Source> {
    let mut map = FxHashMap::default();
    for a in FONT_DATA {
        map.insert(a.0, Source::Binary(Arc::new(a.1)));
    }
    return map;
}

/// Register every compiled-in font with [`FONT_SYSTEM`], returning
/// the `AppFont → fontdb ID` map callers use to resolve faces.
///
/// Acquires the `FONT_SYSTEM` **write** guard through
/// [`acquire_font_system_write`] (§B5: every write acquire routes
/// through the helper). A caller already holding any `FONT_SYSTEM`
/// guard on this thread makes the acquire time out and panic with a
/// site tag instead of hanging — but that never happens in practice
/// because [`init`] runs `load_fonts` at startup, before any guard
/// is held. Costs: one lock acquisition plus one `load_font_source`
/// call per entry in [`FONT_SOURCES`].
fn load_fonts() -> FxHashMap<AppFont, TinyVec<[ID; 8]>> {
    debug!("Waiting for font-system write lock");
    let mut font_system = acquire_font_system_write("load_fonts");
    let mut compiled_font_id_map = FxHashMap::default();
    do_for_all_sources(|x, source| {
        let font_id = font_system.db_mut().load_font_source(source.clone());
        debug!("loaded font {x:?}");
        compiled_font_id_map.insert(x, font_id);
    });
    drop(font_system);
    debug!("Released font-system lock.");
    return compiled_font_id_map;
}

lazy_static! {
    /// `AppFont → fontdb::Source` map built once from the compiled-in
    /// `FONT_DATA` byte arrays.
    pub static ref FONT_SOURCES: FxHashMap<AppFont, Source> = load_font_sources();
    /// Global cosmic-text `FontSystem`. Every cosmic-text operation
    /// (shaping, layout, measurement) goes through this single
    /// `RwLock`-guarded instance.
    pub static ref FONT_SYSTEM: RwLock<FontSystem> = RwLock::new(FontSystem::new());
    /// `AppFont → fontdb face IDs` map populated on first access by
    /// [`load_fonts`]. Read-only after initialization.
    pub static ref COMPILED_FONT_ID_MAP: FxHashMap<AppFont, TinyVec<[ID; 8]>> = load_fonts();
}

/// Force lazy initialization of [`COMPILED_FONT_ID_MAP`] — and, via
/// it, the one-time `FONT_SYSTEM` write-lock that registers every
/// compiled-in font — and the `FAMILY_INDEX` that
/// [`loaded_families_iter`] / [`app_font_by_family`] read.
/// Call once at program start before any shaping / measurement
/// path. Doing both eagerly here closes a latent re-entrant-read
/// risk: `FAMILY_INDEX`'s lazy build acquires `FONT_SYSTEM.read()`,
/// and `mindnode_to_glyph_area` indirectly calls
/// `app_font_by_family` from inside the tree builder; if a future
/// caller ever holds `FONT_SYSTEM.write()` while invoking the tree
/// builder, the lazy path would deadlock. Eager init at startup
/// makes that impossible.
pub fn init() {
    ensure_warm();
}

/// Force the font lazy-statics a `FONT_SYSTEM` write acquire depends
/// on to build **now** — `COMPILED_FONT_ID_MAP` (via `load_fonts`,
/// which itself takes the write guard) and `FAMILY_INDEX` (via
/// `build_family_index`, which takes the read guard). Library-callable
/// counterpart to the app-only [`init`]: a caller that is about to
/// hold the write guard and will reach `face_family_name_for_pin` /
/// `app_font_by_family` under it calls this **first**, so the lazy
/// build doesn't re-enter the same-thread lock. Idempotent — a no-op
/// once warm; both stores are `OnceLock`/`lazy_static`. §B5.
pub(crate) fn ensure_warm() {
    COMPILED_FONT_ID_MAP.capacity();
    FAMILY_INDEX.get_or_init(build_family_index);
}

/// Cached `(family_name, AppFont)` pairs, sorted by family name.
/// Built once on first access by [`loaded_families_iter`] /
/// [`app_font_by_family`] from [`COMPILED_FONT_ID_MAP`] +
/// [`FONT_SYSTEM`]. Trivially small — one entry per `AppFont` —
/// so the whole list is O(n) to scan even on the lookup path.
static FAMILY_INDEX: OnceLock<Vec<(String, AppFont)>> = OnceLock::new();

/// Build the `(family_name, AppFont)` index by walking every
/// face fontdb knows about. Holds the `FONT_SYSTEM` **read** lock
/// for the scope of the call.
///
/// Iteration is over `font_system.db().faces()` rather than over
/// our `COMPILED_FONT_ID_MAP`'s first id per `AppFont` so that:
/// 1. A single source that registers multiple faces (a TTC, or a
///    TTF whose name table exposes several subfamilies) contributes
///    every distinct family name, not just the one tied to the
///    first id;
/// 2. Faces whose `families[0]` is a localized name still surface
///    their English alias when fontdb knows about both — we walk
///    every entry in `face.families`, dedup by name, so a Devanagari
///    Noto font shows up under both its English label and its
///    Devanagari one.
///
/// The reverse lookup (`app_font_by_family`) maps each unique
/// family name to the first `AppFont` whose face advertises that
/// name. Faces fontdb knows about but that aren't compiled in (none
/// today; we own the database) map to `Any` so the cosmic-text
/// fallback picks them up. Sorting is alphabetical so a UI list is
/// stable.
fn build_family_index() -> Vec<(String, AppFont)> {
    // Force `COMPILED_FONT_ID_MAP`'s lazy-static init **before** we
    // grab the `FONT_SYSTEM` read lock. `load_fonts()` (the lazy
    // initializer) needs `FONT_SYSTEM.write()`, and a same-thread
    // read-then-write deadlocks the worker. `init()` documents this
    // ordering at the top of the module and explicitly does both
    // initializations in the right order, but tests / call paths
    // that reach `app_font_by_family` without going through
    // `init()` first (the cosmic-text tree builder, the inline text
    // editor, the console completion popup) would otherwise hit the
    // race. Touching `.capacity()` is enough to drive the
    // lazy_static — `load_fonts()` runs on this line, returns, and
    // releases its write guard before the read below acquires.
    COMPILED_FONT_ID_MAP.capacity();

    // Reverse map id → AppFont so we can attribute each face to a
    // compiled-in variant when one exists.
    let mut id_to_app_font: FxHashMap<ID, AppFont> =
        FxHashMap::with_capacity_and_hasher(COMPILED_FONT_ID_MAP.len(), Default::default());
    for (app_font, ids) in COMPILED_FONT_ID_MAP.iter() {
        for id in ids.iter() {
            id_to_app_font.insert(*id, *app_font);
        }
    }

    let font_system = FONT_SYSTEM
        .read()
        .expect("FONT_SYSTEM lock poisoned during family-index build");
    let mut seen: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
    let mut out: Vec<(String, AppFont)> = Vec::new();
    for face in font_system.db().faces() {
        if face.families.is_empty() {
            log::warn!(
                "build_family_index: face id {:?} has no family names; skipping",
                face.id
            );
            continue;
        }
        let attributed = match id_to_app_font.get(&face.id).copied() {
            Some(app_font) => app_font,
            None => {
                // The codebase owns the fontdb today (see `build.rs`
                // and the comment at the top of this file), so any
                // face we don't recognize comes from a future
                // system-fonts loader or a test that registered
                // fonts directly. Surface that at `debug` so it's
                // observable without spamming `warn`.
                log::debug!(
                    "build_family_index: face id {:?} ({:?}) is not in COMPILED_FONT_ID_MAP; \
                     attributing to AppFont::Any",
                    face.id,
                    face.families.first().map(|(n, _)| n.as_str()).unwrap_or("?"),
                );
                AppFont::Any
            }
        };
        for (family, _lang) in face.families.iter() {
            if family.is_empty() {
                continue;
            }
            if seen.insert(family.clone()) {
                out.push((family.clone(), attributed));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// All compiled-in font families, sorted ascending, as the
/// family-name strings the data model stores in `TextRun.font` and
/// `GlyphConnectionConfig.font`. Borrowing iterator — the per-call
/// allocation `list_loaded_families` did is gone, which matters on
/// the keystroke-hot completion path. The returned `&'static str`s
/// borrow from the `OnceLock`-cached index built by
/// `build_family_index`.
///
/// Costs: O(1) for the cache hit; O(n) one-time on first call.
pub fn loaded_families_iter() -> impl Iterator<Item = &'static str> {
    FAMILY_INDEX
        .get_or_init(build_family_index)
        .iter()
        .map(|(name, _)| name.as_str())
}

/// Materialize [`loaded_families_iter`] as `Vec<String>` — kept for
/// callers that need an owned list (tests, future external API
/// consumers). Allocates one `Vec<String>` per call.
pub fn list_loaded_families() -> Vec<String> {
    loaded_families_iter().map(str::to_string).collect()
}

/// Resolve a family-name string to the build-time `AppFont` enum
/// `crate::core::primitives::ColorFontRegion` stores in its `font`
/// slot. Exact-match (case-sensitive); fuzzy matching is the
/// caller's job — pre-filter via [`loaded_families_iter`] and
/// feed the chosen exact string back in.
///
/// Returns `None` for unknown families so callers can degrade
/// gracefully (e.g. fall back to the default font and surface a
/// warning).
pub fn app_font_by_family(name: &str) -> Option<AppFont> {
    FAMILY_INDEX
        .get_or_init(build_family_index)
        .iter()
        .find(|(family, _)| family == name)
        .map(|(_, app_font)| *app_font)
}

/// Reverse lookup: family name string for a given `AppFont`, or
/// `None` when the font isn't represented in the loaded
/// `FAMILY_INDEX`. Returns the *first* family name registered
/// for the variant (alphabetical-by-name, matching
/// `loaded_families_iter`'s order); when a single AppFont
/// surfaces multiple aliases (localized names, postscript names),
/// only the first is returned.
///
/// Used by the post-mutation reverse converter in
/// `document/custom.rs::region_to_text_run` — a tree-side
/// `ColorFontRegion.font: Option<AppFont>` rolls back into the
/// model's `TextRun.font: String` shape. `None` falls through to
/// the empty string at that site, matching the pre-section
/// behavior for runs without a family pin.
///
/// Costs: O(n) over `FAMILY_INDEX`, but `n` is the small set of
/// compiled-in families (~30 today), so the scan is cheaper than
/// a `HashMap` build. No allocation; the borrowed `&'static str`
/// borrows from the `OnceLock`-cached index that lives for the
/// process lifetime.
pub fn family_name_of(app_font: AppFont) -> Option<&'static str> {
    FAMILY_INDEX
        .get_or_init(build_family_index)
        .iter()
        .find(|(_, family_app_font)| *family_app_font == app_font)
        .map(|(name, _)| name.as_str())
}

/// Live fontdb lookup: return the face-canonical family name
/// for `app_font` by consulting the font_system database
/// directly. Returns the **first** family alias the face
/// declares (`face.families.first()`), which is what cosmic-text
/// expects when `Attrs::new().family(Family::Name(...))` is used
/// to pin a face for shaping. Does not allocate when the lookup
/// misses (`?` short-circuits).
///
/// Distinct from [`family_name_of`]: that helper consults the
/// `OnceLock`-cached [`FAMILY_INDEX`] (alphabetically-sorted, no
/// `font_system` needed) and is the right shape for the post-
/// mutation reverse converter. `face_family_name_for_pin` is the
/// shape every shaping path wants — same name string the face
/// stores natively, so the cosmic-text `Family::Name` match
/// can't drift.
///
/// Single helper for the three call sites
/// (`measure_glyph_ink_bounds`, `measure_text_block_unbounded`,
/// `font/attrs.rs::resolve_font_family`) that previously
/// hand-rolled the same `COMPILED_FONT_ID_MAP → face → families.first()`
/// chain. The four `?` short-circuits (missing AppFont, empty
/// id list, fontdb face miss, empty families list) all silently
/// fall back to the cosmic-text default — same monospace
/// fallback the shaper uses if the family pin fails to resolve,
/// so the floor we measure matches what the user will eventually
/// see. `build_family_index` warns and skips the same misses on
/// the index-build side; the asymmetry is deliberate (warn once
/// at build, degrade gracefully at every shape).
pub(crate) fn face_family_name_for_pin(
    font_system: &cosmic_text::FontSystem,
    app_font: AppFont,
) -> Option<String> {
    let ids = COMPILED_FONT_ID_MAP.get(&app_font)?;
    let face = font_system.db().face(*ids.first()?)?;
    Some(face.families.first()?.0.clone())
}

/// Wall-clock ceiling for a `FONT_SYSTEM` write acquisition. Mandala
/// is single-threaded (see `CLAUDE.md`), so in healthy operation the
/// lock is always free when a caller asks for it. Any wait longer
/// than a single frame means a re-entrancy bug — the same thread
/// already holds the guard and is trying to acquire it again, which
/// `std::sync::RwLock::write()` would otherwise block on forever.
/// 5 s is orders of magnitude beyond any legitimate frame budget and
/// conservative enough to never fire on a healthy system.
///
/// `pub(crate)` so the metric cache reuses the *same* budget for its
/// miss-path acquires rather than duplicating the literal — the
/// "same value" the two sites want is then compiler-enforced.
pub(crate) const FONT_SYSTEM_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Poll interval while waiting for the `FONT_SYSTEM` write guard.
/// Short enough that a lock release is noticed promptly; long enough
/// that a spinning thread doesn't waste CPU.
const FONT_SYSTEM_LOCK_POLL: Duration = Duration::from_millis(1);

/// Acquire the `FONT_SYSTEM` write guard with a bounded timeout,
/// panicking with `site` in the message on timeout or poison.
///
/// There are exactly **two sanctioned shapes** for taking the
/// `FONT_SYSTEM` write lock; a raw `FONT_SYSTEM.write()` is neither:
///
/// 1. **Blocking-with-timeout via this helper** — the default. Used
///    by every call site that must measure or shape and can afford
///    to wait a frame: `load_fonts`, the metric cache's miss path
///    (`metric_cache::glyph_*`), and the renderer's border /
///    handle / overlay rebuilds. A timeout here is a re-entrancy
///    bug (this thread already holds the guard); without the helper
///    that bug would hang the main thread forever, so instead we
///    panic with a stack trace pointing at the second acquisition.
/// 2. **Non-blocking `try_write` + frame-degrade** — the renderer's
///    interactive paths (`renderer::mod::rebuild_mode_status_overlay_if_needed`,
///    `rebuild_fps_overlay_if_needed`, `render::prepare_text_for_pass`)
///    take `FONT_SYSTEM.try_write()` directly and skip the frame on
///    `WouldBlock` rather than block or panic. This is legitimate:
///    a contended lock there means another pass is mid-shape, and
///    dropping one overlay frame is cheaper than stalling render.
///    These sites retry on the next `process()` cycle.
///
/// Guard-holding callers that need to measure a nested value must
/// thread their `&mut FontSystem` into a `*_with` variant (see
/// `metric_cache` / `border_run_specs_with`) rather than re-acquire.
///
/// `site` is a short static string naming the call site; it appears
/// in the panic message so the stack makes the culprit obvious even
/// in a stripped release build.
///
/// **Note on the busy-wait shape.** The plan flagged this as a
/// candidate for "replace with `try_write` + immediate panic on
/// contention." That works for production (single-threaded by
/// `CLAUDE.md` invariant) but breaks `cargo test`, which runs
/// integration tests in parallel and routinely sees legitimate
/// `WouldBlock` from sibling test threads. The busy-wait is the
/// minimal-friction shape that handles both contexts: production
/// hits `Ok(guard)` on the first iteration; test-thread contention
/// resolves within a handful of polls. The 5 s ceiling still
/// catches genuine deadlocks.
pub fn acquire_font_system_write(site: &'static str) -> RwLockWriteGuard<'static, FontSystem> {
    acquire_font_system_write_with_timeout(site, FONT_SYSTEM_LOCK_TIMEOUT)
}

/// Internal worker for [`acquire_font_system_write`] with a
/// caller-chosen timeout. Exposed (crate-visible) so tests can
/// exercise the timeout path without waiting the full production
/// 5-second budget.
pub fn acquire_font_system_write_with_timeout(
    site: &'static str,
    timeout: Duration,
) -> RwLockWriteGuard<'static, FontSystem> {
    let start = Instant::now();
    loop {
        match FONT_SYSTEM.try_write() {
            Ok(guard) => return guard,
            Err(TryLockError::Poisoned(_)) => {
                panic!(
                    "FONT_SYSTEM lock is poisoned (site: {site}). A prior \
                     holder panicked while holding the guard."
                );
            }
            Err(TryLockError::WouldBlock) => {
                if start.elapsed() >= timeout {
                    panic!(
                        "FONT_SYSTEM write lock not available after {:?} \
                         (site: {site}). In a single-threaded app this \
                         almost certainly means the current thread \
                         already holds the guard — look for a re-entrant \
                         call on the stack above.",
                        timeout
                    );
                }
                thread::sleep(FONT_SYSTEM_LOCK_POLL);
            }
        }
    }
}

/// Invoke `closure(app_font, source)` for every entry in
/// [`FONT_SOURCES`]. `Source` is cloned per call because cosmic-text
/// takes it by value when loading.
pub fn do_for_all_sources<F>(mut closure: F)
where
    F: FnMut(AppFont, Source),
{
    for (key, value) in &*FONT_SOURCES {
        closure(*key, value.clone());
    }
}

/// Clone out the `fontdb::Source` for a named compiled-in font.
/// Panics if `name` is not in [`FONT_SOURCES`].
pub fn get_font_source(name: &AppFont) -> Source {
    return FONT_SOURCES.get(name).unwrap().clone();
}

/// Pick a random compiled-in font source. **Test-only helper** —
/// production paths should pick fonts deterministically.
pub fn get_some_font() -> Source {
    let mut rng = rand::rng();
    return FONT_SOURCES.values().choose(&mut rng).unwrap().clone();
}

/// Opaque black. The default foreground color for newly-built
/// `AttrsList`s.
/// Ink bounding box of a shaped glyph string, measured at a specific
/// font size. Sibling of the `measure_max_glyph_advance` scalar
/// measurement (currently in the app-level renderer as pre-existing
/// debt per CODE_CONVENTIONS.md §1; tracked to move here on the way
/// past) — where advance measures just how wide the glyph pushes the
/// pen, ink bounds measure where the visible pixels actually land.
///
/// Consumers — today the color picker's crosshair arms and central
/// preview glyph — use this to compute ink-center-vs-advance-center
/// offsets so they can re-anchor positions that `Align::Center`
/// would otherwise center on the em-box. Without this correction
/// four scripts with four different sidebearings drift four
/// different directions off a shared visual center.
///
/// Coordinates:
/// - `x_min` / `x_max`: pen-relative pixels. `0.0` is the pen
///   origin; `advance` is the pen-end for the shaped string.
///   Sidebearings cause `x_min > 0.0` (left sidebearing) and
///   `x_max < advance` (right sidebearing).
/// - `y_min` / `y_max`: baseline-relative pixels, y-axis pointing
///   down (cosmic-text convention). Negative values sit above the
///   baseline; positive values (descenders) sit below.
/// - `advance`: sum of glyph advances across the shaped string.
/// - `line_y`: baseline-from-buffer-top in pixels at the measurement
///   font size — equals `cosmic_text::LayoutRun::line_y` for the run
///   that produced the ink. Combined with `y_center()` this gives
///   the ink center y inside a rendering box positioned at the
///   buffer's top.
#[derive(Clone, Copy, Debug, Default)]
pub struct InkBounds {
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
    pub advance: f32,
    pub line_y: f32,
}

impl InkBounds {
    /// Horizontal ink center (pen-relative, in pixels).
    pub fn x_center(&self) -> f32 {
        (self.x_min + self.x_max) * 0.5
    }

    /// Vertical ink center (baseline-relative, in pixels).
    pub fn y_center(&self) -> f32 {
        (self.y_min + self.y_max) * 0.5
    }

    /// Horizontal offset of the ink center from the advance center.
    /// Positive means the ink sits right-of the advance center;
    /// negative means left-of. A caller rendering with
    /// `Align::Center` and wanting the ink (not the em-box) to land
    /// at a target x must subtract this from that target.
    pub fn x_offset_from_advance_center(&self) -> f32 {
        self.x_center() - self.advance * 0.5
    }

    /// Vertical offset of the ink center from the rendering box
    /// center, in pixels at the measurement font size. Positive
    /// means ink sits below box-center; a caller wanting the ink
    /// (not the em-box) to land at a target y must subtract this
    /// from that target.
    ///
    /// `font_size` is the size used at measurement (so the box's
    /// height in pixels is `font_size * line_height_mul`).
    /// `line_height_mul` is the height of the rendering bounds
    /// expressed as a multiple of `font_size` — for the color picker
    /// arms today this is `1.5` (bounds = `fs * 1.5`).
    pub fn y_offset_from_box_center(&self, font_size: f32, line_height_mul: f32) -> f32 {
        (self.line_y + self.y_center()) - font_size * line_height_mul * 0.5
    }
}

/// Shape `glyph` through cosmic-text at `font_size` (pinning
/// `font` when `Some`) and return the [`InkBounds`] of the result.
/// Empty / all-whitespace / tofu input yields a zero bounding box.
///
/// `font_system` and `swash_cache` are passed in rather than taken
/// from the global [`FONT_SYSTEM`] so the primitive composes with
/// existing call sites that already hold the write guard (notably
/// the color picker open path, which measures advances and ink in
/// the same lock scope).
///
/// `y_min` / `y_max` are baseline-relative; `line_y` (also returned
/// on [`InkBounds`]) carries the baseline-from-buffer-top so callers
/// can compute box-relative ink positions via
/// [`InkBounds::y_offset_from_box_center`].
///
/// Costs: allocates a scratch `Buffer`, shapes one line, rasterizes
/// each glyph through `SwashCache::get_image_uncached` (no caching
/// — callers needing repeated access should hold their own cache).
/// Call-once-at-picker-open, not frame-hot.
pub fn measure_glyph_ink_bounds(
    font_system: &mut cosmic_text::FontSystem,
    swash_cache: &mut SwashCache,
    font: Option<AppFont>,
    glyph: &str,
    font_size: f32,
) -> InkBounds {
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, font_size));

    // Pin the requested AppFont family (if any) so sacred-script
    // glyphs shape against the intended face instead of cosmic-text's
    // default fallback. The family name string must outlive `attrs`,
    // so we hold it in a local binding.
    let family_name: Option<String> = font.and_then(|app_font| face_family_name_for_pin(font_system, app_font));
    let attrs = match family_name.as_deref() {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new(),
    };

    buffer.set_text(font_system, glyph, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    let mut out = InkBounds::default();
    let mut any_ink = false;
    let mut advance_total = 0.0f32;

    for run in buffer.layout_runs() {
        // Multi-run shapes overwrite — last run wins. Acceptable
        // because the only caller today shapes a single glyph.
        out.line_y = run.line_y;
        for layout_glyph in run.glyphs.iter() {
            advance_total += layout_glyph.w;
            // `physical` bakes the sub-pixel position into `cache_key`
            // so the rasterized placement reflects the same x
            // fractional as the layout. We only use `cache_key` for
            // the swash lookup; ink-bounds math runs against
            // `layout_glyph.x` directly (pen-relative in pixels).
            let physical = layout_glyph.physical((0.0, 0.0), 1.0);
            if let Some(image) = swash_cache.get_image_uncached(font_system, physical.cache_key) {
                if image.placement.width == 0 || image.placement.height == 0 {
                    continue;
                }
                let pen_x = layout_glyph.x;
                let ink_left = pen_x + image.placement.left as f32;
                let ink_right = ink_left + image.placement.width as f32;
                // `placement.top` is positive for ink above baseline;
                // we flip sign so y grows downward (cosmic-text
                // convention) and ink-above-baseline sits at negative
                // y.
                let ink_top = -(image.placement.top as f32);
                let ink_bottom = ink_top + image.placement.height as f32;
                if !any_ink {
                    out.x_min = ink_left;
                    out.x_max = ink_right;
                    out.y_min = ink_top;
                    out.y_max = ink_bottom;
                    any_ink = true;
                } else {
                    out.x_min = out.x_min.min(ink_left);
                    out.x_max = out.x_max.max(ink_right);
                    out.y_min = out.y_min.min(ink_top);
                    out.y_max = out.y_max.max(ink_bottom);
                }
            }
        }
    }

    out.advance = advance_total;
    out
}

/// Natural-size measurement of a text block laid out as unbroken
/// single lines — cosmic-text still splits on embedded `\n`.
/// Produced by [`measure_text_block_unbounded`] for callers that
/// want to size a box to fit text rather than reflow text to a
/// fixed width.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextBlockSize {
    /// Maximum `line_w` across all laid-out runs, in pixels.
    pub width: f32,
    /// `line_count * line_height`, in pixels.
    pub height: f32,
    /// Number of layout runs produced. `0` for empty input.
    pub line_count: u32,
}

impl TextBlockSize {
    /// Zero-sized measurement — what empty input produces. Useful as
    /// an early-return sentinel.
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
        line_count: 0,
    };
}

/// Shape `text` through cosmic-text without a width constraint and
/// return its natural [`TextBlockSize`]. Embedded `\n` produces
/// additional lines; the returned `width` is the widest run, and
/// `height = line_count * line_height`.
///
/// `font_system` is passed in — not taken from the global
/// [`FONT_SYSTEM`] — so the caller controls the lock scope (§B5).
/// `scale` is the font size in px; `line_height` is the absolute
/// line height in px (not a multiplier), applied uniformly to every
/// line. Empty input returns [`TextBlockSize::ZERO`] without shaping.
///
/// `font` pins the cosmic-text [`Family`] so the measurement uses
/// the same face the renderer will eventually shape with. Pass
/// `None` to fall back to cosmic-text's default (typically a
/// monospace) — historical behavior, but a fragile floor for
/// nodes whose `TextRun.font` pins a wider display face. The pin
/// follows the same `COMPILED_FONT_ID_MAP → face.families.first()`
/// path [`measure_glyph_ink_bounds`] uses, so the two measurement
/// primitives agree on which face name to pin.
///
/// Costs: one scratch `Buffer`, one shaping pass, O(lines) fold
/// over `layout_runs`. No rasterization (no `SwashCache` required).
pub fn measure_text_block_unbounded(
    font_system: &mut cosmic_text::FontSystem,
    text: &str,
    scale: f32,
    line_height: f32,
    font: Option<AppFont>,
) -> TextBlockSize {
    if text.is_empty() {
        return TextBlockSize::ZERO;
    }
    let mut buffer = Buffer::new(font_system, Metrics::new(scale, line_height));
    // `None` on both axes = unbounded; measure natural widths.
    buffer.set_size(font_system, None, None);

    // Pin the requested AppFont family if any — without this, a
    // node pinned to a wide display face measures as if it were
    // cosmic-text's default monospace and the box undersizes by
    // 30–60%. The family-name string must outlive `attrs`, so
    // we hold it in a local binding.
    let family_name: Option<String> = font.and_then(|app_font| face_family_name_for_pin(font_system, app_font));
    let attrs = match family_name.as_deref() {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new(),
    };

    buffer.set_text(font_system, text, &attrs, Shaping::Advanced, None);

    let mut max_w = 0.0_f32;
    let mut line_count = 0_u32;
    for run in buffer.layout_runs() {
        if run.line_w > max_w {
            max_w = run.line_w;
        }
        line_count += 1;
    }
    TextBlockSize {
        width: max_w,
        height: line_count as f32 * line_height,
        line_count,
    }
}
