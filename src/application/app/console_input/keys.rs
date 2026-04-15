//! Console line-editor key names.
//!
//! `keybinds::normalize_key_name` lowercases the winit key identifier,
//! so every console-handled key matches the lowercase forms here.
//! Kept in its own file so both the dispatcher and any future
//! test-only matchers can share them without a cyclic dep. The
//! parent `console_input/mod.rs` already carries
//! `#![cfg(not(target_arch = "wasm32"))]` at module level, so this
//! file inherits the native-only gate without a redundant attribute.

pub(super) const CONSOLE_KEY_ESCAPE: &str = "escape";
pub(super) const CONSOLE_KEY_ENTER: &str = "enter";
pub(super) const CONSOLE_KEY_TAB: &str = "tab";
pub(super) const CONSOLE_KEY_ARROW_UP: &str = "arrowup";
pub(super) const CONSOLE_KEY_UP: &str = "up";
pub(super) const CONSOLE_KEY_ARROW_DOWN: &str = "arrowdown";
pub(super) const CONSOLE_KEY_DOWN: &str = "down";
pub(super) const CONSOLE_KEY_ARROW_LEFT: &str = "arrowleft";
pub(super) const CONSOLE_KEY_LEFT: &str = "left";
pub(super) const CONSOLE_KEY_ARROW_RIGHT: &str = "arrowright";
pub(super) const CONSOLE_KEY_RIGHT: &str = "right";
pub(super) const CONSOLE_KEY_HOME: &str = "home";
pub(super) const CONSOLE_KEY_END: &str = "end";
pub(super) const CONSOLE_KEY_BACKSPACE: &str = "backspace";
pub(super) const CONSOLE_KEY_DELETE: &str = "delete";
pub(super) const CONSOLE_KEY_SPACE: &str = "space";
pub(super) const CONSOLE_KEY_CTRL_A: &str = "a";
pub(super) const CONSOLE_KEY_CTRL_C: &str = "c";
pub(super) const CONSOLE_KEY_CTRL_E: &str = "e";
pub(super) const CONSOLE_KEY_CTRL_U: &str = "u";
pub(super) const CONSOLE_KEY_CTRL_W: &str = "w";
