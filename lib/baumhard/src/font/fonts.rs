use std::sync::{Arc, RwLock};

use cosmic_text::fontdb::ID;
use cosmic_text::{Attrs, AttrsList, Buffer, BufferRef, Color, Edit, Editor, Family, Metrics, Stretch, Style, Weight, Wrap};
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
