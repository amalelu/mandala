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
// // Do not remove the following "unused" imports
//@formatter:off
use serde::{Deserialize, Serialize};
// Include generated file (from build.rs script)
include!(concat!(env!("OUT_DIR"), "/generated_fonts_data.rs"));

fn load_font_sources() -> FxHashMap<AppFont, Source> {
    let mut map = FxHashMap::default();
    for a in FONT_DATA {
        map.insert(a.0, Source::Binary(Arc::new(a.1)));
    }
    return map;
}

/// WARNING! This function will wait for, and then lock font-system write access
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
    pub static ref FONT_SOURCES: FxHashMap<AppFont, Source> = load_font_sources();
    pub static ref FONT_SYSTEM: RwLock<FontSystem> = RwLock::new(FontSystem::new());
    pub static ref COMPILED_FONT_ID_MAP: FxHashMap<AppFont, TinyVec<[ID; 8]>> = load_fonts();
}

pub fn init() {
    // This ensures that load_fonts gets called, which requires exclusive lock over the font system
    COMPILED_FONT_ID_MAP.capacity();
}

pub fn do_for_all_sources<F>(mut closure: F)
where
    F: FnMut(AppFont, Source),
{
    for (key, value) in &*FONT_SOURCES {
        closure(*key, value.clone());
    }
}

pub fn get_font_source(name: &AppFont) -> Source {
    return FONT_SOURCES.get(name).unwrap().clone();
}

/// This is only for testing
pub fn get_some_font() -> Source {
    let mut rng = rand::thread_rng();
    return FONT_SOURCES.values().choose(&mut rng).unwrap().clone();
}

pub const DEFAULT_FONT_COLOR: Color = Color::rgba(0, 0, 0, 255);

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

/// WARNING! This function will wait for, and then lock font-system write access
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

/// WARNING! This function will wait for, and then lock font-system write access
pub fn create_cosmic_editor(scale: f32, line_height: f32, bound_x: f32, bound_y: f32) -> Editor<'static> {
    debug!("Waiting for font-system write lock");
    let mut font_system = FONT_SYSTEM.write().expect("FontSystem lock was poisoned");
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(scale, line_height));
    buffer.set_size(&mut font_system, Some(bound_x), Some(bound_y));
    buffer.set_wrap(&mut font_system, Wrap::Word);
    return Editor::new(buffer);
}

pub fn unwrap_buffer_ref<'a>(buffer_ref: &'a BufferRef) -> &'a Buffer {
    return match buffer_ref {

    BufferRef::Owned(owned) => {&owned}BufferRef::Borrowed(borrowed) => {borrowed}BufferRef::Arc(arc) => {arc.as_ref()}}
}

pub fn adjust_buffer_metrics(buffer: &mut Buffer, metrics: Metrics) {
    debug!("Waiting for font-system write lock");
    let mut font_system = FONT_SYSTEM.write().expect("FontSystem lock was poisoned");
    buffer.set_metrics(&mut font_system, metrics);
}

/// Ink bounding box of a shaped glyph string, measured at a specific
/// font size. Sibling of the `measure_max_glyph_advance` scalar
/// measurement (currently in `src/application/renderer.rs` as
/// pre-existing debt per CODE_CONVENTIONS.md §1; tracked to move
/// here on the way past) — where advance measures just how wide the
/// glyph pushes the pen, ink bounds measure where the visible
/// pixels actually land.
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
#[derive(Clone, Copy, Debug, Default)]
pub struct InkBounds {
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
    pub advance: f32,
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
}

/// Shape `glyph` through cosmic-text at `font_size` (pinning
/// `font` when `Some`) and return the [`InkBounds`] of the result.
/// Empty / all-whitespace / tofu input yields a zero bounding box.
///
/// `font_system` and `swash_cache` are passed in rather than taken
/// from the global [`FONT_SYSTEM`] so the primitive composes with
/// existing call sites that already hold the write guard (notably
/// the color picker open path in `src/application/app.rs`, which
/// measures advances and ink in the same lock scope).
///
/// Costs: shapes a one-line buffer (`O(1)` lookup into the shape
/// cache after the first call per glyph) and rasterizes each glyph
/// via `SwashCache::get_image_uncached` (no caching — callers that
/// need repeated access should hold their own cache). Allocates a
/// scratch `Buffer`. Call-once-at-picker-open, not frame-hot.
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
