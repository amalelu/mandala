// SPDX-License-Identifier: MPL-2.0

//! Compiled-in font table, shared `FontSystem`, and cosmic-text
//! editor factories. The `AppFont` enum + `FONT_DATA` array are
//! emitted by `build.rs` at crate-compile time so the binary carries
//! every font it might need without touching the filesystem at run
//! time.

use std::cell::Cell;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, ThreadId};
use std::time::Duration;

use cosmic_text::fontdb::Source;
use cosmic_text::fontdb::ID;
use cosmic_text::FontSystem;
use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, SwashCache};
use lazy_static::lazy_static;
use log::debug;
use parking_lot::{RecursiveRwLock, RecursiveRwLockReadGuard, RecursiveRwLockWriteGuard};
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
    map
}

/// Register every compiled-in font with [`FONT_SYSTEM`], returning
/// the `AppFont → fontdb ID` map callers use to resolve faces.
///
/// Acquires the `FONT_SYSTEM` **write** guard through
/// [`acquire_font_system_write`] (§B5: every write acquire routes
/// through the helper). A caller already holding any `FONT_SYSTEM`
/// guard on this thread makes the acquire panic at once, naming both
/// sites, instead of hanging — but that never happens in practice
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
    compiled_font_id_map
}

lazy_static! {
    /// `AppFont → fontdb::Source` map built once from the compiled-in
    /// `FONT_DATA` byte arrays.
    pub static ref FONT_SOURCES: FxHashMap<AppFont, Source> = load_font_sources();
    /// Global cosmic-text `FontSystem`. Every cosmic-text operation
    /// (shaping, layout, measurement) goes through this single
    /// lock-guarded instance. A `RecursiveRwLock` — parking_lot's
    /// reader-biased lock — because every read is taken recursively;
    /// see [`acquire_font_system_read`].
    ///
    /// **Private on purpose.** The only ways to take the lock are
    /// [`acquire_font_system_write`], [`try_acquire_font_system_write`]
    /// and [`acquire_font_system_read`], because those are the ones
    /// that record who holds it: a bare `FONT_SYSTEM.write()` would
    /// hold the lock invisibly, and a same-thread re-acquire under it
    /// could no longer be told from ordinary contention.
    static ref FONT_SYSTEM: RecursiveRwLock<FontSystem> = RecursiveRwLock::new(FontSystem::new());
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

/// Floor for any font metric handed to the text shaper, in points.
///
/// Zero is the value that kills the process: cosmic-text's
/// `Buffer::new` asserts `line_height != 0.0`, and an assert inside
/// the scene build is an abort, not a frame to degrade. A merely
/// *tiny* metric is its own denial of service — the border fill
/// fitter divides a node's width by the glyph advance, so a
/// sub-point size asks for hundreds of millions of glyphs in one
/// string. Half a point is already invisible at any zoom the camera
/// reaches, so nothing authorable lives below it.
///
/// Lives here, beside the shaper that enforces it, because both the
/// document trust boundary (`mindmap::model::validate`, which
/// re-exports this) and the mutation path (`GlyphArea`'s scale
/// setters) have to agree on one number.
pub const MIN_FONT_SIZE_PT: f32 = 0.5;

/// Ceiling for any font metric handed to the text shaper, in points.
/// Generous enough for a glyph used as a full-screen design element
/// (the canonical fixture's largest run is 74pt) and small enough
/// that one rasterized glyph cannot exhaust the atlas.
pub const MAX_FONT_SIZE_PT: f32 = 4096.0;

/// Force an authored font metric into
/// [`MIN_FONT_SIZE_PT`]..=[`MAX_FONT_SIZE_PT`].
///
/// The two non-finite cases are deliberately *not* treated alike.
/// An infinity carries an intent the bounds can honor — "as large
/// as possible" clamps to the ceiling, and `f32::clamp` already
/// does that correctly. `NaN` carries none, and `f32::clamp`
/// propagates it rather than rejecting it, so it would reach the
/// shaper as a metric that makes every downstream comparison false.
/// That is the one case that falls back to `fallback`, which
/// callers pass as the metric already in place.
///
/// Cost: O(1).
pub fn clamp_font_metric(value: f32, fallback: f32) -> f32 {
    if value.is_nan() {
        return fallback;
    }
    value.clamp(MIN_FONT_SIZE_PT, MAX_FONT_SIZE_PT)
}

/// Clamp a screen-space font size into the window `[min, max]`,
/// tolerating a window the document got backwards.
///
/// **`f32::clamp` panics when its bounds cross**, and the size
/// cascades hand it a `min` and a `max` that need not come from the
/// same place: a label may set only `min_font_size_pt` and inherit
/// `max` from its edge, a portal endpoint from its config. Checking
/// each struct's own pair at load therefore cannot see every window
/// the cascade can build — the two halves are individually valid and
/// jointly inverted.
///
/// So the ordering is enforced where the window is actually used.
/// An inverted pair is read as the window the author described
/// rather than a crash, both bounds are pulled into the shaper's
/// domain first, and a `NaN` size takes the floor instead of
/// propagating.
///
/// Cost: O(1).
pub fn clamp_to_font_window(value: f32, min: f32, max: f32) -> f32 {
    let min = clamp_font_metric(min, MIN_FONT_SIZE_PT);
    let max = clamp_font_metric(max, MAX_FONT_SIZE_PT);
    let (low, high) = if min <= max { (min, max) } else { (max, min) };
    if value.is_nan() {
        return low;
    }
    value.clamp(low, high)
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
/// name. Faces fontdb knows about but that aren't compiled in map to
/// `Any` so the cosmic-text fallback picks them up. **On native that
/// set is not empty:** `FONT_SYSTEM` is `FontSystem::new()`, which
/// indexes the host's fontconfig, so every family the machine has
/// installed lands in this index too. The browser build has no
/// fontconfig and sees only the compiled-in faces, which is the
/// native-only divergence `format/fonts.md` opens by describing.
/// Sorting is alphabetical so a UI list is stable.
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

    let font_system = acquire_font_system_read("build_family_index");
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

/// Longest one thread waits on **another** thread's hold of
/// `FONT_SYSTEM` before the wait is declared a deadlock and the
/// process panics instead of hanging.
///
/// This bound is for cross-thread waits only. A thread that already
/// holds the lock and asks for it again never waits at all: it is
/// refused on the spot (see [`refuse_reentry`]), because that case is
/// decidable and a timeout is a poor way to decide it. The earlier
/// design used a 5 s timeout for both, which turned ordinary
/// parallel-test contention into a spurious crash — two tests hold the
/// lock for seconds in a debug build — while taking five seconds to
/// report a genuine re-entrancy bug.
///
/// 60 s sits above any legitimate hold seen under load, and below the
/// point where a hung process stops being diagnosable. A genuinely
/// slower holder than this would be a bug in its own right. Natively
/// the `FreezeWatchdog` aborts a silent UI thread after 10 s, so for
/// the main thread that backstop fires first.
///
/// `pub(crate)` so the metric cache reuses the *same* budget for its
/// miss-path acquires rather than duplicating the literal.
pub(crate) const FONT_SYSTEM_LOCK_TIMEOUT: Duration = Duration::from_secs(60);

/// What the current thread holds of `FONT_SYSTEM`.
///
/// Thread-local because re-entrancy is, by definition, a thread
/// acquiring a lock it already holds: the answer to "would this
/// acquire deadlock against myself?" is a fact about the calling
/// thread alone, and needs no registry of other threads.
#[derive(Clone, Copy)]
struct Held {
    /// The site that took the write guard this thread holds, if any.
    write_site: Option<&'static str>,
    /// How many read guards this thread holds.
    readers: usize,
}

thread_local! {
    static HELD: Cell<Held> = const { Cell::new(Held { write_site: None, readers: 0 }) };
}

thread_local! {
    /// This thread's id, fetched once. `thread::current()` clones the
    /// thread's handle; the write path wants only the id, on every
    /// acquire.
    static THIS_THREAD: ThreadId = thread::current().id();
}

/// Who holds the write guard right now, across all threads. Read only
/// to make the deadlock panic say *whom* it is waiting on; never
/// consulted to decide anything.
static WRITER: Mutex<Option<(&'static str, ThreadId)>> = Mutex::new(None);

/// Set when a panic starts while a thread holds the write guard,
/// because cosmic-text may have been left mid-mutation. `parking_lot`
/// has no poisoning of its own, so this reproduces what
/// `std::sync::RwLock` does: every later acquire panics rather than
/// shaping against a half-updated font system. Sticky for the life of
/// the process.
static POISONED: AtomicBool = AtomicBool::new(false);

/// Whether a holder has panicked and left `FONT_SYSTEM` poisoned. Once
/// true it stays true, and every acquire helper panics from then on.
///
/// For a caller that catches a panic and wants to know whether
/// carrying on is possible at all: `Renderer::prewarm` re-raises the
/// panic it caught when this says the font system went down with it,
/// because the next frame's first acquire would end the process anyway
/// — with a message about the poison rather than about its cause.
pub fn font_system_poisoned() -> bool {
    POISONED.load(Ordering::SeqCst)
}

/// Update this thread's [`Held`] record. `try_with` rather than
/// `with`: a guard dropped from inside a thread-local destructor runs
/// after `HELD` itself may be gone, and failing to update a record
/// nobody can read any more must not turn into a second panic.
fn update_held(f: impl FnOnce(&mut Held)) {
    let _ = HELD.try_with(|cell| {
        let mut held = cell.get();
        f(&mut held);
        cell.set(held);
    });
}

fn current_writer() -> Option<(&'static str, ThreadId)> {
    *WRITER.lock().unwrap_or_else(|p| p.into_inner())
}

fn set_writer(writer: Option<(&'static str, ThreadId)>) {
    *WRITER.lock().unwrap_or_else(|p| p.into_inner()) = writer;
}

/// This thread's id. Falls back to `thread::current()` only inside a
/// thread-local destructor, after `THIS_THREAD` itself is gone.
fn this_thread() -> ThreadId {
    THIS_THREAD
        .try_with(|id| *id)
        .unwrap_or_else(|_| thread::current().id())
}

/// Makes a guard type `!Send` (and `!Sync`) whatever `parking_lot`'s
/// features say. The holding record is thread-local, so a guard
/// released on a thread other than the one that took it would clear
/// the wrong thread's record: the taker would then look re-entrant on
/// its next acquire, or stop being caught when it really is.
/// `parking_lot`'s own guards are `!Send` only while no crate in the
/// graph enables its `send_guard` feature, and Cargo unifies features
/// across the whole graph, so that is not ours to rely on.
type NotSend = PhantomData<*const ()>;

/// Which kind of access an acquire is asking for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Write,
}

/// Panic if this thread already holds `FONT_SYSTEM` in a way that
/// makes the requested access wait on itself.
///
/// Three cases, all deadlocks by construction and all caught here
/// without waiting:
///
/// - write while holding the write guard;
/// - write while holding a read guard (read-then-write);
/// - read while holding the write guard.
///
/// A read while holding only read guards is fine — it is taken
/// recursively, see [`acquire_font_system_read`].
fn refuse_reentry(site: &'static str, access: Access) {
    let held = HELD.try_with(Cell::get).unwrap_or(Held {
        write_site: None,
        readers: 0,
    });
    if let Some(holder) = held.write_site {
        panic!(
            "FONT_SYSTEM re-entrant acquire (site: {site}): this thread \
             already holds the write guard taken at `{holder}`, so waiting \
             for it can never succeed. Thread the held `&mut FontSystem` \
             into a `*_with` variant instead of re-acquiring."
        );
    }
    if access == Access::Write && held.readers > 0 {
        panic!(
            "FONT_SYSTEM write acquire while holding a read guard (site: \
             {site}): this thread holds {} read guard(s), and a writer \
             cannot proceed until they are released. Drop the read guard \
             first.",
            held.readers
        );
    }
}

/// Panic if a holder panicked mid-mutation. Checked
/// *after* acquiring, because the poisoning thread sets the flag
/// before it releases the lock — a check made before waiting would
/// miss one that happened while we waited.
fn refuse_if_poisoned(site: &'static str) {
    if POISONED.load(Ordering::SeqCst) {
        panic!(
            "FONT_SYSTEM lock is poisoned (site: {site}). A prior \
             holder panicked while holding the guard."
        );
    }
}

/// Exclusive access to the global `FontSystem`.
///
/// Derefs to [`FontSystem`], so it is passed wherever a
/// `&mut FontSystem` is wanted. It exists as a type of its own,
/// rather than handing out `parking_lot`'s guard, to run code on
/// release: clearing this thread's holding record and marking the
/// lock poisoned if the holder is unwinding.
pub struct FontSystemWriteGuard {
    guard: RecursiveRwLockWriteGuard<'static, FontSystem>,
    /// Whether this thread was already unwinding when it took the
    /// guard. Such a guard is taken by cleanup code running *during* a
    /// panic that began elsewhere; that panic cannot have interrupted
    /// a mutation made under this guard, so it must not poison the
    /// lock on release. The same rule `std`'s poison flag applies.
    panicking_at_acquire: bool,
    _not_send: NotSend,
}

impl Deref for FontSystemWriteGuard {
    type Target = FontSystem;
    fn deref(&self) -> &FontSystem {
        &self.guard
    }
}

impl DerefMut for FontSystemWriteGuard {
    fn deref_mut(&mut self) -> &mut FontSystem {
        &mut self.guard
    }
}

impl Drop for FontSystemWriteGuard {
    /// Runs before the inner guard is released, so the record is
    /// cleared while the lock is still ours: the next holder cannot
    /// set its record and then have it erased by this one.
    fn drop(&mut self) {
        if thread::panicking() && !self.panicking_at_acquire {
            POISONED.store(true, Ordering::SeqCst);
        }
        set_writer(None);
        update_held(|held| held.write_site = None);
    }
}

/// Shared access to the global `FontSystem`. Read-only; see
/// [`FontSystemWriteGuard`] for why it is a type of its own.
pub struct FontSystemReadGuard {
    guard: RecursiveRwLockReadGuard<'static, FontSystem>,
    _not_send: NotSend,
}

impl Deref for FontSystemReadGuard {
    type Target = FontSystem;
    fn deref(&self) -> &FontSystem {
        &self.guard
    }
}

impl Drop for FontSystemReadGuard {
    fn drop(&mut self) {
        update_held(|held| held.readers = held.readers.saturating_sub(1));
    }
}

/// Record that this thread now holds the write guard, then panic if
/// the lock is poisoned. The wrapper is built *before* the check, so
/// the panic unwinds through [`FontSystemWriteGuard::drop`], which
/// releases the lock and clears the record.
fn adopt_write(
    site: &'static str,
    guard: RecursiveRwLockWriteGuard<'static, FontSystem>,
) -> FontSystemWriteGuard {
    update_held(|held| held.write_site = Some(site));
    set_writer(Some((site, this_thread())));
    let adopted = FontSystemWriteGuard {
        guard,
        panicking_at_acquire: thread::panicking(),
        _not_send: PhantomData,
    };
    refuse_if_poisoned(site);
    adopted
}

/// Acquire the `FONT_SYSTEM` write guard, panicking with `site` in the
/// message rather than ever waiting forever.
///
/// There are **three sanctioned ways** to take the lock, and no
/// others — `FONT_SYSTEM` is private so a raw `.write()` does not
/// compile:
///
/// 1. **This function**, the default. Used by every call site that
///    must measure or shape and can afford to wait: `load_fonts`, the
///    metric cache's miss path, and the renderer's border / handle /
///    overlay rebuilds.
/// 2. [`try_acquire_font_system_write`] — non-blocking, for the
///    renderer's interactive paths, which skip a frame on contention
///    rather than stall render.
/// 3. [`acquire_font_system_read`] — shared access, for the family
///    index.
///
/// What each failure looks like, because they are different bugs:
///
/// - **This thread already holds the lock**: panics immediately,
///   naming both sites. No timeout is involved.
/// - **Another thread holds it**: waits, queued fairly behind earlier
///   waiters, for up to `FONT_SYSTEM_LOCK_TIMEOUT` (60 s), then panics
///   naming the holder. Contention between threads is ordinary — the
///   test harness produces plenty — and is never an error by itself.
/// - **A holder panicked mid-mutation**: panics, as a poisoned
///   `std::sync::RwLock` does. A panic that was already unwinding when
///   the guard was taken does not count (see
///   [`FontSystemWriteGuard`]).
///
/// Guard-holding callers that need to measure a nested value must
/// thread their `&mut FontSystem` into a `*_with` variant (see
/// `metric_cache` / `border_run_specs_with`) rather than re-acquire.
///
/// **No clock on the uncontended path.** `std::time::Instant::now()`
/// traps on `wasm32-unknown-unknown`, and the browser takes only the
/// uncontended path: it has one thread, and re-entrancy is refused
/// before any wait. This function therefore reads no clock itself and
/// leaves timing to the lock's own slow path. `tests/wasm_font_lock.rs`
/// runs this function on wasm32 to keep that true.
///
/// `site` is a short static string naming the call site; it appears
/// in every panic message so the stack makes the culprit obvious even
/// in a stripped release build.
pub fn acquire_font_system_write(site: &'static str) -> FontSystemWriteGuard {
    acquire_font_system_write_with_timeout(site, FONT_SYSTEM_LOCK_TIMEOUT)
}

/// Internal worker for [`acquire_font_system_write`] with a
/// caller-chosen cross-thread ceiling. Exposed so tests can exercise
/// the deadlock panic without waiting the full production budget.
pub fn acquire_font_system_write_with_timeout(site: &'static str, timeout: Duration) -> FontSystemWriteGuard {
    refuse_reentry(site, Access::Write);
    match FONT_SYSTEM.try_write_for(timeout) {
        Some(guard) => adopt_write(site, guard),
        None => {
            let holder = match current_writer() {
                Some((holder_site, thread)) => format!("`{holder_site}` on {thread:?}"),
                None => "reader(s), which hold no write record".to_string(),
            };
            panic!(
                "FONT_SYSTEM write lock not available after {timeout:?} \
                 (site: {site}). It is held by {holder}. Another thread \
                 holding it this long is a deadlock, or a guard held across \
                 blocking work."
            );
        }
    }
}

/// Non-blocking [`acquire_font_system_write`]: `None` when another
/// thread holds the lock, and the caller should skip the frame and try
/// again on the next cycle. That is the one transient it reports.
///
/// The other two failures panic exactly as the blocking form does,
/// because neither clears on a later frame:
///
/// - **This thread already holds it.** "Another thread is mid-shape,
///   try next frame" is a legitimate skip; "this thread already holds
///   it" is a bug that would otherwise skip every frame from then on
///   in silence.
/// - **The lock is poisoned.** Skipping would not keep the process
///   alive: the same frame's blocking acquires — the tree, border and
///   overlay rebuilds and the metric cache's miss path — panic on the
///   poison regardless, so a `None` here only moves the end one call
///   later and files it under contention. `Renderer::prewarm`, the one
///   place that catches a render panic, re-raises it when it left the
///   lock poisoned (see [`font_system_poisoned`]).
pub fn try_acquire_font_system_write(site: &'static str) -> Option<FontSystemWriteGuard> {
    refuse_reentry(site, Access::Write);
    Some(adopt_write(site, FONT_SYSTEM.try_write()?))
}

/// Shared access to the `FontSystem`, with the same never-wait-forever
/// contract as [`acquire_font_system_write`].
///
/// Taken *recursively*: a thread that holds one read guard and asks
/// for a second must not queue behind a writer that is itself waiting
/// for the first, which an ordinary read would do and which deadlocks.
/// `FONT_SYSTEM` is a `RecursiveRwLock` for exactly this, so its plain
/// `try_read_for` already behaves that way. parking_lot 0.13 replaced
/// the old `RwLock::try_read_recursive_for` with that dedicated type.
pub fn acquire_font_system_read(site: &'static str) -> FontSystemReadGuard {
    refuse_reentry(site, Access::Read);
    match FONT_SYSTEM.try_read_for(FONT_SYSTEM_LOCK_TIMEOUT) {
        Some(guard) => {
            update_held(|held| held.readers += 1);
            let adopted = FontSystemReadGuard {
                guard,
                _not_send: PhantomData,
            };
            refuse_if_poisoned(site);
            adopted
        }
        None => {
            let holder = match current_writer() {
                Some((holder_site, thread)) => format!("`{holder_site}` on {thread:?}"),
                None => "an unrecorded holder".to_string(),
            };
            panic!(
                "FONT_SYSTEM read lock not available after {FONT_SYSTEM_LOCK_TIMEOUT:?} \
                 (site: {site}). It is held by {holder}."
            );
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

// `get_font_source` and `get_some_font` stood here: two `pub`
// helpers, each a bare `unwrap()` on the way to a `Source`, and
// neither called from anywhere in the workspace — not the app, not
// maptool, not the benches, not a test. CODE_CONVENTIONS §9 makes a
// bare `unwrap()` outside tests a bug, and §10 says to delete rather
// than to deprecate; dressing two dead functions in `expect` messages
// nobody will ever read is the other thing. `do_for_all_sources`
// above is the live way to reach [`FONT_SOURCES`].

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
/// from the global `FONT_SYSTEM` so the primitive composes with
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
    let family_name: Option<String> =
        font.and_then(|app_font| face_family_name_for_pin(font_system, app_font));
    let attrs = match family_name.as_deref() {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new(),
    };

    buffer.set_text(glyph, &attrs, Shaping::Advanced, None);
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
/// `FONT_SYSTEM` — so the caller controls the lock scope (§B5).
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
    buffer.set_size(None, None);

    // Pin the requested AppFont family if any — without this, a
    // node pinned to a wide display face measures as if it were
    // cosmic-text's default monospace and the box undersizes by
    // 30–60%. The family-name string must outlive `attrs`, so
    // we hold it in a local binding.
    let family_name: Option<String> =
        font.and_then(|app_font| face_family_name_for_pin(font_system, app_font));
    let attrs = match family_name.as_deref() {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new(),
    };

    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    // cosmic-text 0.19 made the setters lazy: `set_text` / `set_size`
    // only mark the buffer dirty, and the raw `layout_runs()` reads
    // whatever was last shaped — nothing, for a fresh buffer. Shape
    // explicitly, or every block measures 0 × 0. 0.18 shaped inside
    // `set_text`, which is why this call was not needed before.
    buffer.shape_until_scroll(font_system, false);

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

#[cfg(test)]
mod tests {
    //! The lock's own contract: what it refuses, what it waits for, and
    //! what it records. Inline rather than in `font/tests/` because that
    //! tree is the benchmark-reusable one (§T2.2), and a test that holds
    //! the process-global lock on purpose has no business being iterated
    //! by criterion. The wasm32 half of the contract is
    //! `tests/wasm_font_lock.rs`.

    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::process::Command;
    use std::sync::atomic::AtomicBool;
    use std::sync::{mpsc, Arc};
    use std::time::Instant;

    /// Set in a child process to the name of the one scenario it runs.
    /// Poisoning is process-wide and permanent, so a scenario that
    /// poisons the lock — on purpose, or through the bug it is
    /// guarding against — runs in a process of its own rather than
    /// failing every other test in this binary.
    const CHILD_SCENARIO_ENV: &str = "MANDALA_FONT_LOCK_CHILD_SCENARIO";

    /// Whether this process is a child launched for `scenario`.
    fn is_child_for(scenario: &str) -> bool {
        std::env::var(CHILD_SCENARIO_ENV).is_ok_and(|name| name == scenario)
    }

    /// Re-run this test binary filtered to the test `scenario`, with
    /// [`CHILD_SCENARIO_ENV`] set, and require it to print `marker`.
    /// A no-op inside a child, so a child never launches another.
    fn run_child_scenario(scenario: &str, marker: &str) {
        if std::env::var_os(CHILD_SCENARIO_ENV).is_some() {
            return;
        }
        let test_path = format!(
            "{}::{scenario}",
            module_path!()
                .split_once("::")
                .expect("module paths start with the crate")
                .1
        );
        let out = Command::new(std::env::current_exe().expect("the test binary has a path"))
            .args(["--exact", &test_path, "--nocapture", "--test-threads=1"])
            .env(CHILD_SCENARIO_ENV, scenario)
            .output()
            .expect("the test binary must be re-runnable");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains(marker),
            "the `{scenario}` scenario failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The message `f` panicked with. Panics itself if `f` does not.
    ///
    /// Every test here that provokes a panic does so *inside* this
    /// helper while holding its guard *outside* it. The guard then
    /// drops after `catch_unwind` has returned, when the thread is no
    /// longer unwinding — dropping it mid-unwind is precisely what
    /// poisons the lock, for the whole process.
    fn panic_message(f: impl FnOnce()) -> String {
        let payload = catch_unwind(AssertUnwindSafe(f)).expect_err("the closure was supposed to panic");
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .expect("the panic payload should be a string")
    }

    /// Hold the write guard on a new thread until told to let go.
    /// Returns once the thread really holds it.
    fn hold_on_another_thread(site: &'static str) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
        let (held_tx, held_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let holder = thread::spawn(move || {
            let _guard = acquire_font_system_write(site);
            held_tx.send(()).expect("the test is listening");
            // Closed channel and message both mean "let go".
            let _ = release_rx.recv();
        });
        held_rx.recv().expect("the holder should take the lock");
        (release_tx, holder)
    }

    #[test]
    fn test_reentrant_write_panics_at_once_naming_both_sites() {
        let outer = acquire_font_system_write("test::outer");
        let started = Instant::now();
        let message = panic_message(|| {
            let _ = acquire_font_system_write("test::inner");
        });
        let waited = started.elapsed();
        drop(outer);

        assert!(message.contains("re-entrant"), "unexpected message: {message}");
        assert!(
            message.contains("test::outer"),
            "must name the site holding it: {message}"
        );
        assert!(
            message.contains("test::inner"),
            "must name the site asking: {message}"
        );
        // Decided from the thread's own record, so it cannot have
        // waited on a clock. The bound is far below the 60 s ceiling
        // and far above scheduler noise.
        assert!(
            waited < Duration::from_secs(5),
            "a re-entrant acquire waited {waited:?} instead of failing at once"
        );
    }

    #[test]
    fn test_write_while_holding_a_read_guard_panics_at_once() {
        let reader = acquire_font_system_read("test::reader");
        let message = panic_message(|| {
            let _ = acquire_font_system_write("test::writer");
        });
        drop(reader);
        assert!(message.contains("read guard"), "unexpected message: {message}");
    }

    #[test]
    fn test_read_while_holding_the_write_guard_panics_at_once() {
        let writer = acquire_font_system_write("test::writer");
        let message = panic_message(|| {
            let _ = acquire_font_system_read("test::reader");
        });
        drop(writer);
        assert!(message.contains("re-entrant"), "unexpected message: {message}");
    }

    #[test]
    fn test_try_acquire_on_a_thread_that_holds_it_panics_rather_than_skipping() {
        let outer = acquire_font_system_write("test::outer");
        let message = panic_message(|| {
            let _ = try_acquire_font_system_write("test::try");
        });
        drop(outer);
        assert!(
            message.contains("re-entrant"),
            "a same-thread hold must not read as ordinary contention: {message}"
        );
    }

    #[test]
    fn test_nested_reads_on_one_thread_are_allowed() {
        let outer = acquire_font_system_read("test::outer");
        let inner = acquire_font_system_read("test::inner");
        let _faces = inner.db().len();
        drop(inner);
        drop(outer);
    }

    #[test]
    fn test_dropping_the_guard_clears_the_threads_record() {
        drop(acquire_font_system_write("test::first"));
        // Would panic as re-entrant if the first drop left its record.
        drop(acquire_font_system_write("test::second"));
        drop(acquire_font_system_read("test::third"));
        drop(acquire_font_system_write("test::fourth"));
    }

    #[test]
    fn test_try_acquire_is_none_while_another_thread_holds_it() {
        let (release, holder) = hold_on_another_thread("test::holder");
        assert!(
            try_acquire_font_system_write("test::try").is_none(),
            "a hold on another thread is contention, and `try_` reports it as `None`"
        );
        release.send(()).expect("the holder is waiting");
        holder.join().expect("the holder should not panic");
        // Blocking, not `try_`: sibling tests take the lock too, so a
        // `try_` here could lose to one of them and say nothing about
        // this lock.
        drop(acquire_font_system_write("test::after"));
    }

    /// The shape of the failure this design replaced: a long hold with
    /// a queue behind it. Every waiter must come out the other side
    /// with the lock, not with a panic, because contention between
    /// threads is not an error.
    ///
    /// What each waiter checks is an ordering, not a duration: it got
    /// the lock only after the holder was told to let go. A duration
    /// would depend on when the scheduler got round to starting the
    /// waiter's thread, which on a loaded runner is late enough to fail
    /// a lower bound.
    #[test]
    fn test_waiters_queue_behind_a_long_hold_instead_of_failing() {
        let (release, holder) = hold_on_another_thread("test::long_holder");
        let released = Arc::new(AtomicBool::new(false));
        let waiters: Vec<_> = (0..6)
            .map(|_| {
                let released = Arc::clone(&released);
                thread::spawn(move || {
                    let guard = acquire_font_system_write("test::waiter");
                    let after_release = released.load(Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(20));
                    drop(guard);
                    after_release
                })
            })
            .collect();
        // Time for the waiters to park behind the hold. A waiter that
        // has not started yet simply arrives after the release; nothing
        // below depends on how many were parked.
        thread::sleep(Duration::from_millis(300));
        // Set before the release is sent, so a waiter that reads `false`
        // got the lock while the holder still had it.
        released.store(true, Ordering::SeqCst);
        release.send(()).expect("the holder is waiting");
        holder.join().expect("the holder should not panic");
        for waiter in waiters {
            let after_release = waiter
                .join()
                .expect("a waiter panicked instead of waiting its turn");
            assert!(
                after_release,
                "a waiter got the lock while another thread held it"
            );
        }
    }

    #[test]
    fn test_timeout_names_the_site_holding_the_lock() {
        let (release, holder) = hold_on_another_thread("test::named_holder");
        let message = panic_message(|| {
            let _ = acquire_font_system_write_with_timeout("test::impatient", Duration::from_millis(100));
        });
        release.send(()).expect("the holder is waiting");
        holder.join().expect("the holder should not panic");

        assert!(
            message.contains("FONT_SYSTEM write lock not available"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("test::impatient"),
            "must name the waiter: {message}"
        );
        // The point of recording the holder: a deadlock report that says
        // who is in the way. Sibling tests may be the registered writer
        // at the instant of the panic, so only the format is pinned.
        assert!(
            message.contains("It is held by"),
            "must say who holds it: {message}"
        );
    }

    /// A panic while holding the guard poisons every later acquire.
    /// Runs in a child process; see [`CHILD_SCENARIO_ENV`].
    #[test]
    fn test_panic_while_holding_poisons_later_acquires() {
        run_child_scenario("test_child_scenario_poisoning", "POISON_SCENARIO_OK");
    }

    /// The child half of the test above. A no-op in an ordinary run.
    #[test]
    fn test_child_scenario_poisoning() {
        if !is_child_for("test_child_scenario_poisoning") {
            return;
        }
        let poisoner = thread::spawn(|| {
            let _guard = acquire_font_system_write("test::poisoner");
            panic!("deliberate: a holder dying mid-mutation");
        });
        assert!(poisoner.join().is_err(), "the poisoner was supposed to panic");

        let message = panic_message(|| {
            let _ = acquire_font_system_write("test::victim");
        });
        assert!(message.contains("poisoned"), "unexpected message: {message}");
        let message = panic_message(|| {
            let _ = acquire_font_system_read("test::reading_victim");
        });
        assert!(message.contains("poisoned"), "unexpected message: {message}");
        // Poison never clears, so the non-blocking form does not
        // disguise it as contention: a `None` would only move the end
        // to the frame's next blocking acquire.
        let message = panic_message(|| {
            let _ = try_acquire_font_system_write("test::non_blocking_victim");
        });
        assert!(message.contains("poisoned"), "unexpected message: {message}");
        assert!(
            font_system_poisoned(),
            "the public query must agree with the helpers"
        );
        // And the refusals stranded no record: had one stayed behind,
        // this would read as re-entrant rather than poisoned.
        let message = panic_message(|| {
            let _ = acquire_font_system_write("test::after_the_refusals");
        });
        assert!(
            message.contains("poisoned") && !message.contains("re-entrant"),
            "unexpected message: {message}"
        );
        println!("POISON_SCENARIO_OK");
    }

    /// A guard taken and released by cleanup code that runs *during* an
    /// unwind does not poison the lock: the panic began before the
    /// guard existed, so it cannot have interrupted a mutation made
    /// under it. Runs in a child process, because the bug it guards
    /// against would poison this whole binary.
    #[test]
    fn test_acquire_during_an_unwind_does_not_poison() {
        run_child_scenario("test_child_scenario_acquire_during_unwind", "UNWIND_SCENARIO_OK");
    }

    /// The child half of the test above. A no-op in an ordinary run.
    #[test]
    fn test_child_scenario_acquire_during_unwind() {
        if !is_child_for("test_child_scenario_acquire_during_unwind") {
            return;
        }
        /// Takes and releases the font lock from its destructor, the way
        /// a cache flush or a log line in some `Drop` would.
        struct ShapesOnDrop;
        impl Drop for ShapesOnDrop {
            fn drop(&mut self) {
                assert!(thread::panicking(), "the scenario needs this to run mid-unwind");
                drop(acquire_font_system_write("test::cleanup_during_unwind"));
                drop(try_acquire_font_system_write("test::try_cleanup_during_unwind"));
            }
        }
        let unwinder = thread::spawn(|| {
            let _cleanup = ShapesOnDrop;
            panic!("deliberate: a panic that never touched the font lock");
        });
        assert!(unwinder.join().is_err(), "the unwinder was supposed to panic");
        assert!(
            !font_system_poisoned(),
            "a guard taken during an unwind poisoned the lock on release"
        );
        drop(acquire_font_system_write("test::after_the_unwind"));
        println!("UNWIND_SCENARIO_OK");
    }
}
