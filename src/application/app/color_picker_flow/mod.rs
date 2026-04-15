//! Glyph-wheel color picker flow: open / commit / cancel / per-frame
//! mouse + keyboard handlers + the §B2 dispatcher
//! (`rebuild_color_picker_overlay`) the event loop calls each frame.
//!
//! Pulled out of `app/mod.rs` so the picker's HSV / handle-resolution
//! logic doesn't bloat the event loop. Public surface stays
//! `pub(in crate::application::app)` — `console_input` calls
//! `open_color_picker_contextual` / `_standalone` /
//! `cancel_color_picker` / `close_color_picker_standalone`, and the
//! event loop calls every other entry point in the file.
//!
//! Split by lifecycle:
//! - [`open`] — open / re-open helpers + initial preview seeding.
//! - [`geometry`] — `compute_picker_geometry` (pre-rebuild layout pass).
//! - [`rebuild`] — `rebuild_color_picker_overlay` dispatcher.
//! - [`commit`] — cancel / close / commit / preview application.
//! - [`key`] — keyboard dispatch (h/s/v nudges, Esc, Enter).
//! - [`mouse`] — mouse-move hit-test + gesture feed.
//! - [`click`] — click dispatch + gesture start/end.

mod commit;
mod geometry;
mod key;
mod mouse;
mod click;
mod open;
mod rebuild;

pub(in crate::application::app) use commit::{
    apply_picker_preview, cancel_color_picker, close_color_picker_standalone, commit_color_picker,
    commit_color_picker_to_selection,
};
pub(in crate::application::app) use click::{end_color_picker_gesture, handle_color_picker_click};
pub(in crate::application::app) use geometry::compute_picker_geometry;
pub(in crate::application::app) use key::handle_color_picker_key;
pub(in crate::application::app) use mouse::handle_color_picker_mouse_move;
pub(in crate::application::app) use open::{
    open_color_picker_contextual, open_color_picker_standalone, open_picker_inner,
    seed_initial_preview,
};
pub(in crate::application::app) use rebuild::rebuild_color_picker_overlay;
