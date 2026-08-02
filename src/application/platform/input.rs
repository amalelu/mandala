// SPDX-License-Identifier: MPL-2.0

//! Input-event value types: keyboard keys, modifiers, mouse
//! buttons, scroll deltas, key/button up-down state.
//!
//! Today every type is a `pub use` of the corresponding
//! `winit::*` type. A future backend swap (SDL, custom WASM
//! event-loop, …) replaces these aliases — every inward caller
//! that imports through this module stays put.

pub use winit::event::ElementState;
/// Mouse-button payload — consumed by native event handlers and the
/// color-picker click flow. WASM's bootstrap (`run_wasm/mod.rs`)
/// imports directly from winit, so this re-export is gated to native.
#[cfg(not(target_arch = "wasm32"))]
pub use winit::event::MouseButton;
pub use winit::keyboard::Key;
pub use winit::keyboard::ModifiersState as Modifiers;
/// Named-key payload (`NamedKey::Enter` / `Tab` / etc.) — only
/// reached at the modal-editor test fixture layer; production
/// paths pattern-match on `Key::Named(..)` directly.
#[cfg(test)]
pub use winit::keyboard::NamedKey;

/// Inline-string payload for `Key::Character(...)` — winit reuses
/// `smol_str::SmolStr` here, exposed under the platform name so
/// modal-editor tests can synthesise character payloads without
/// importing the smol-str crate by name. Production paths receive
/// the `SmolStr` already wrapped inside a `Key::Character(...)`
/// from winit, so the type itself is only reached at the test
/// fixture layer.
#[cfg(test)]
pub use winit::keyboard::SmolStr;

/// Wheel/trackpad scroll-delta payload. Both targets route through
/// this seam: the shared `wheel_lines` helper in `app/mod.rs` names
/// the delta by this alias, and each target's wheel handler passes
/// its driver's value straight in. Previously WASM-only, when native
/// still decomposed the delta itself inside its driver dispatcher.
pub use winit::event::MouseScrollDelta;
