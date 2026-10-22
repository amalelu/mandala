// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::Touch` arm for WASM. Mirrors the native shape
//! at `run_native.rs::dispatch_touch_event` — feed the
//! [`crate::application::app::touch_gesture::TouchGestureRecognizer`]
//! state machine, then run whichever of the two routes the
//! recognized gesture takes.
//!
//! This is the arm the browser's whole touch vocabulary arrives
//! through, and the browser is the surface `super`'s module header
//! calls the *primary* one this project targets: a phone sees no
//! `MouseInput` / `CursorMoved` for a hold, a second finger, or a
//! drag that never pressed a button, so anything not handled here is
//! not handled at all.

#![cfg(target_arch = "wasm32")]

use crate::application::app::dispatch::{self, DispatchOutcome, TouchStep};
use crate::application::app::touch_gesture::Phase;
use std::sync::atomic::AtomicBool;
use web_time::Instant;
use winit::event::Touch;

/// One-shot warn-log latch: fires the first time a recognized
/// touch gesture maps to an Action whose body is `NativeOnly`,
/// so a mobile-browser user who taps-and-holds (default-bound to
/// `EnterResizeMode`) sees evidence in the dev console rather
/// than wondering why their gesture is dead. Static + `swap` is
/// the same shape `event_mouse_click::handle_right_button` uses
/// for the equivalent right-button warning.
///
/// Only `LongPress` can reach it. Tap, pan and pinch resolve to no
/// `Action` at all — they run `dispatch::apply_touch_effect`, which
/// is one cross-platform body — so no amount of ordinary touch
/// input on the browser can produce a dead gesture.
static WARNED_NATIVE_ONLY: AtomicBool = AtomicBool::new(false);

impl super::WasmApp {
    /// Handle one `WindowEvent::Touch`. Returns true when the
    /// runtime should request a redraw (always true for
    /// Started/Moved so future cursor-following overlays update;
    /// true for Ended only when a gesture actually ran).
    pub(super) fn handle_touch_event(&mut self, touch: Touch) -> bool {
        let phase = dispatch::touch_phase(touch.phase);
        let pos = (touch.location.x, touch.location.y);
        let now = Instant::now();
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let (Some(input), Some(renderer)) = (input_borrow.as_mut(), renderer_borrow.as_mut()) else {
            return false;
        };
        // Phase translation, recognizer ingest + tick, and the
        // routing of what it recognized are the shared body native
        // runs too, and so is `apply_touch_effect`. What is left
        // below is the browser's own dispatch of a keybind-routed
        // gesture: `dispatch_compatible` plus the `NativeOnly`
        // warn-log.
        let idle_redraw = matches!(phase, Phase::Started | Phase::Moved);
        let Some(step) = dispatch::drive_touch_event(
            &mut input.touch_recognizer,
            &self.keybinds,
            phase,
            touch.id,
            pos,
            now,
        ) else {
            return idle_redraw;
        };
        match step {
            TouchStep::Dispatch(d) => {
                // Move the cursor to the gesture's reported pos so the
                // dispatched Action sees the right cursor.
                input.cursor_pos = d.cursor_pos;
                let name = d.gesture_name;
                let Some(a) = d.action else {
                    return idle_redraw;
                };
                let mut core = input.input_context_core(renderer, &self.keybinds);
                let outcome = dispatch::action_core::dispatch_compatible(&a, &mut core, None);
                // When the bound Action is `NativeOnly` — the
                // shipped `LongPress` default, `EnterResizeMode`, is
                // one — `dispatch_compatible` returns `Unhandled` and
                // there is no graceful fallback on WASM: the user
                // gets no log, no chrome and no model change. The
                // reporting body is shared with the double-click and
                // wheel sites, which reach the same dead end; only
                // the latch and the remedy are per-input-class.
                if matches!(outcome, DispatchOutcome::Unhandled) {
                    crate::application::app::warn_unhandled_native_only_once(
                        &WARNED_NATIVE_ONLY,
                        name,
                        &a,
                        "Long-press is deliberately bound to a native-only Action: it is the \
                         touch peer of the keyboard's `r`, and binding the browser's long-press \
                         to something else would make one gesture mean two things. Its parity \
                         rides on the InteractionMode::Resize chrome porting to the browser. Tap, \
                         one-finger pan and pinch-zoom need no binding and work here today; \
                         rebind `enter_resize_mode`'s LongPress entry to a Compatible action \
                         (e.g. zoom_in / select_all / a custom macro) to opt out.",
                    );
                }
                true
            }
            TouchStep::Effect(effect) => {
                {
                    let mut core = input.input_context_core(renderer, &self.keybinds);
                    dispatch::apply_touch_effect(effect, &mut core);
                }
                input.reproject_after_camera_change(renderer);
                true
            }
        }
    }
}
