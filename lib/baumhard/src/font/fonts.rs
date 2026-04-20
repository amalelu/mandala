//! Compiled-in font table, shared `FontSystem`, and cosmic-text
//! editor factories. The `AppFont` enum + `FONT_DATA` array are
//! emitted by `build.rs` at crate-compile time so the binary carries
//! every font it might need without touching the filesystem at run
//! time.

use std::sync::{Arc, RwLock};

use cosmic_text::fontdb::ID;
use cosmic_text::{Attrs, AttrsList, Buffer, BufferRef, Color, Edit, Editor, Family, Metrics, Shaping, Stretch, Style, SwashCache, Weight, Wrap};
use cosmic_text::fontdb::Source;
use cosmic_text::FontSystem;
use lazy_static::lazy_static;
use log::{debug};
use rand::seq::IteratorRandom;
use rustc_hash::FxHashMap;
use tinyvec::TinyVec;

use crate::font::fonts::AppFont::*;
// Serde derives are used by the generated AppFont enum below.
//@formatter:off
use serde::{Deserialize, Serialize};
// Build-time generated: defines `AppFont` and `FONT_DATA`.
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
/// Acquires the `FONT_SYSTEM` **write** lock; callers that already
/// hold any lock on it will deadlock. Costs: one lock acquisition
/// plus one `load_font_source` call per entry in [`FONT_SOURCES`].
fn load_fonts() -> FxHashMap<AppFont, TinyVec<[ID; 8]>> {
    debug!("Waiting for font-system write lock");
    let mut font_system = FONT_SYSTEM
        .write()
        .expect("Failed to retrieve font system lock");
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
/// compiled-in font. Call once at program start before any shaping
/// / measurement path.
pub fn init() {
    COMPILED_FONT_ID_MAP.capacity();
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
    let mut rng = rand::thread_rng();
    return FONT_SOURCES.values().choose(&mut rng).unwrap().clone();
}

/// Opaque black. The default foreground colour for newly-built
/// `AttrsList`s.
pub const DEFAULT_FONT_COLOR: Color = Color::rgba(0, 0, 0, 255);

/// Build a single-span `AttrsList` pinned to `font_family_name`,
/// opaque-black text, normal style / stretch / weight. Convenience
/// for call sites that need a baseline attribute set before layering
/// per-region overrides on top.
pub fn get_default_attr_list(font_family_name: &str) -> AttrsList {
    AttrsList::new(
        &Attrs::new()
            .family(Family::Name(font_family_name))
            .color(DEFAULT_FONT_COLOR)
            .style(Style::Normal)
            .stretch(Stretch::Normal)
            .weight(Weight::NORMAL),
    )
}

/// Build a cosmic-text `Editor` seeded with `text`, shaping it
/// against the given `font_id`.
///
/// Acquires the `FONT_SYSTEM` **write** lock for the duration of the
/// call; callers holding any existing guard on [`FONT_SYSTEM`] will
/// deadlock. Panics if `font_id` is missing from
/// [`COMPILED_FONT_ID_MAP`] or if the face cannot be resolved.
pub fn create_cosmic_editor_str(
    font_id: &AppFont,
    scale: f32,
    line_height: f32,
    text: &str,
) -> Editor<'static> {
    debug!("Waiting for font-system write lock");
    let mut font_system = FONT_SYSTEM.write().expect("FontSystem lock was poisoned");

    let buffer = Buffer::new(&mut font_system, Metrics::new(scale, line_height));

    let mut editor = Editor::new(buffer);

    let font_id = COMPILED_FONT_ID_MAP.get(font_id).expect("Font not found");

    let face = font_system.db().face(font_id[0]).unwrap();

    editor.insert_string(text, Some(get_default_attr_list(&face.families[0].0)));
    return editor;
}

/// Build an empty cosmic-text `Editor` with word-wrap enabled at the
/// given bounds.
///
/// Acquires the `FONT_SYSTEM` **write** lock for the duration of the
/// call; callers holding any existing guard on [`FONT_SYSTEM`] will
/// deadlock.
pub fn create_cosmic_editor(scale: f32, line_height: f32, bound_x: f32, bound_y: f32) -> Editor<'static> {
    debug!("Waiting for font-system write lock");
    let mut font_system = FONT_SYSTEM.write().expect("FontSystem lock was poisoned");
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(scale, line_height));
    buffer.set_size(&mut font_system, Some(bound_x), Some(bound_y));
    buffer.set_wrap(&mut font_system, Wrap::Word);
    return Editor::new(buffer);
}

/// Borrow the inner `Buffer` out of a cosmic-text `BufferRef`,
/// regardless of its Owned / Borrowed / Arc variant.
pub fn unwrap_buffer_ref<'a>(buffer_ref: &'a BufferRef) -> &'a Buffer {
    return match buffer_ref {

    BufferRef::Owned(owned) => {&owned}BufferRef::Borrowed(borrowed) => {borrowed}BufferRef::Arc(arc) => {arc.as_ref()}}
}

/// Replace the `Metrics` on `buffer`. Acquires the `FONT_SYSTEM`
/// **write** lock for the set; callers holding any existing guard on
/// [`FONT_SYSTEM`] will deadlock.
pub fn adjust_buffer_metrics(buffer: &mut Buffer, metrics: Metrics) {
    debug!("Waiting for font-system write lock");
    let mut font_system = FONT_SYSTEM.write().expect("FontSystem lock was poisoned");
    buffer.set_metrics(&mut font_system, metrics);
}

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
    let family_name: Option<String> = font.and_then(|app_font| {
        let ids = COMPILED_FONT_ID_MAP.get(&app_font)?;
        let face = font_system.db().face(ids[0])?;
        Some(face.families.first()?.0.clone())
    });
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
