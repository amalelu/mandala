// SPDX-License-Identifier: MPL-2.0

//! Glyph-wheel color picker flow: open / commit / cancel / per-frame
//! handlers + the §B2 dispatcher (`rebuild_color_picker_overlay`)
//! the event loop calls each frame. Public surface stays
//! `pub(in crate::application::app)` — `console_input` calls the
//! lifecycle entries, the event loop calls the rest.
//!
//! **Where the picker's effects run.** Commit, cancel and the six
//! HSV nudges are user-named effects, so their bodies live in
//! `dispatch_action` (CODE_CONVENTIONS §3) and this module supplies
//! the pieces: [`picker_op_for`] classifies an `Action`, and the
//! lifecycle terminals ([`cancel_color_picker`],
//! [`commit_color_picker`], [`commit_color_picker_to_selection`],
//! [`apply_picker_nudge`]) do the work. The key and click routers
//! here decide *which* Action a keystroke or press names and hand
//! it to the funnel; only the picker's Copy / Paste / Cut stay
//! modal-local.

mod click;
mod commit;
mod geometry;
mod key;
mod mouse;
mod open;
mod rebuild;

pub(in crate::application::app) use click::{end_color_picker_gesture, handle_color_picker_click, PickerClick};
// Picker lifecycle terminals. Re-exported because the
// `dispatch_action` picker arm — not this module — owns the
// commit / cancel bodies now (CODE_CONVENTIONS §3: user-named
// effects belong in the funnel).
pub(in crate::application::app) use commit::{
    cancel_color_picker, close_color_picker_standalone, commit_color_picker,
    commit_color_picker_to_selection,
};
pub(in crate::application::app) use key::{
    apply_picker_nudge, handle_color_picker_clipboard_key, picker_decline_reason, picker_op_for,
    PickerOp,
};
pub(in crate::application::app) use mouse::handle_color_picker_mouse_move;
pub(in crate::application::app) use open::{open_color_picker_contextual, open_color_picker_standalone};
pub(in crate::application::app) use rebuild::rebuild_color_picker_overlay;
