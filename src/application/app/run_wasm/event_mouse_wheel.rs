// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::MouseWheel` arm. Resolves the scroll direction to a
//! `MouseGesture`, looks up whatever the user has bound to it, and
//! runs that Action through the funnel — the same three steps native
//! takes. `WheelUp` / `WheelDown` are default-bound to `ZoomIn` /
//! `ZoomOut`; rebinding or unbinding them takes effect here.
//!
//! Pre-unification this arm hardcoded a 1.1× zoom factor, emitted the
//! `CameraZoom` decree itself, and rebuilt every canvas role
//! unconditionally — so a browser user's `keybinds.json` had no say
//! in what the wheel did.

#![cfg(target_arch = "wasm32")]

use crate::application::platform::input::MouseScrollDelta;

use super::PendingClick;
use crate::application::app::dispatch::DispatchOutcome;
use crate::application::app::{dispatch, wheel_gesture, wheel_lines};
use std::sync::atomic::AtomicBool;

/// One-shot latch for the wheel input class. See
/// [`crate::application::app::warn_unhandled_native_only_once`].
static WARNED_NATIVE_ONLY_WHEEL: AtomicBool = AtomicBool::new(false);

impl super::WasmApp {
    pub(super) fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        let lines = wheel_lines(delta);
        let gesture_name = wheel_gesture(lines).key_name();
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let (Some(input), Some(renderer)) = (input_borrow.as_mut(), renderer_borrow.as_mut()) else {
            return;
        };
        // A scroll mid-click invalidates the pending selection: the
        // canvas coord the user pressed over has shifted to a new
        // screen position, so committing the pending click on the
        // eventual mouse-up would select whatever now sits under the
        // release cursor — not what the user pressed on. Clear it so
        // release falls through to empty-click handling. Runs
        // regardless of what (if anything) the wheel is bound to: the
        // press-time coordinate is stale either way.
        input.pending_click = PendingClick::None;

        // `action_for_gesture` falls back to the unmodified binding
        // when no exact-modifier match exists, so Ctrl+Wheel keeps
        // zooming even though only bare `WheelUp` / `WheelDown` ship
        // bound. An explicitly-cleared binding means the wheel is
        // silently ignored — same as native.
        let action = self.keybinds.action_for_gesture(
            gesture_name,
            input.modifiers.control_key(),
            input.modifiers.shift_key(),
            input.modifiers.alt_key(),
        );
        let Some(a) = action else {
            return;
        };
        let outcome = {
            let mut core = input.input_context_core(renderer, &self.keybinds);
            dispatch::action_core::dispatch_compatible(&a, &mut core, None)
        };
        // The wheel has no sanctioned carve-out — every wheel
        // binding either runs or does nothing at all — so an
        // `Unhandled` here is always a `NativeOnly` Action the user
        // bound to the wheel. Newly reachable: before this PR the
        // browser hardcoded the zoom and never consulted the table.
        if matches!(outcome, DispatchOutcome::Unhandled) {
            crate::application::app::warn_unhandled_native_only_once(
                &WARNED_NATIVE_ONLY_WHEEL,
                gesture_name,
                &a,
                "Rebind the wheel to a Compatible action (e.g. zoom_in / zoom_out) \
                 to opt out.",
            );
        }
        // Native picks the post-camera-change reprojection up in its
        // per-frame drain; the browser has no drain loop, so every
        // camera mover on this target runs the same body. See
        // `WasmInputState::reproject_after_camera_change` for the
        // three ways it is *not* identical to native's drain.
        input.reproject_after_camera_change(renderer);
    }
}
