//! Shared per-call context for the native event-loop dispatchers.
//!
//! Mouse, keyboard, cursor, and console-submit handlers each need
//! roughly the same bundle of mutable references into `InitState`:
//! the document, renderer, scene, modal-state machines, keybinds,
//! and so on. Before this module existed, every handler spelled the
//! bundle out as twenty separate function parameters and silenced
//! `clippy::too_many_arguments` over the top. That silencing is the
//! §6 smell this type closes.
//!
//! `InputHandlerContext<'a>` borrows each persistent field as
//! `&'a mut T` (or `&'a T` for read-only state); handlers destructure
//! the fields they need via `ctx.field`. Fields that are *per-event*
//! (button press state, key code, modifiers snapshot, cursor position
//! delivered by the event) stay as separate function parameters —
//! the context is about persistent state, not the event payload.
//!
//! Borrow-check-wise, a `&mut InputHandlerContext<'_>` lets a handler
//! take disjoint mutable field borrows under Rust's standard rules,
//! exactly as if the state lived in a single struct.
//!
//! The builder lives on `InitState::input_context` in
//! `src/application/app/run_native.rs`.

#![cfg(not(target_arch = "wasm32"))]

use winit::keyboard::ModifiersState;

use crate::application::color_picker::ColorPickerState;
use crate::application::console::ConsoleState;
use crate::application::document::MindMapDocument;
use crate::application::frame_throttle::MutationFrequencyThrottle;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;

use super::{
    AppMode, DragState, LabelEditState, LastClick, PortalTextEditState, TextEditState,
};

/// Borrowed view of the persistent state every interactive-path
/// dispatcher reads and writes. Built once per event by
/// [`crate::application::app::run_native::InitState::input_context`]
/// and passed to `handle_mouse_input`, `handle_cursor_moved`,
/// `handle_keyboard_input`, and `submit_line`.
///
/// The lifetime `'a` ties every field borrow to a single `&mut
/// InitState` — the struct is a re-packaging of existing borrows,
/// not a new owner of state.
pub(in crate::application::app) struct InputHandlerContext<'a> {
    /// The loaded mindmap document, or `None` before the first
    /// successful `loader::load_from_file`.
    pub document: &'a mut Option<MindMapDocument>,
    /// Baumhard tree projection of the document. Rebuilt / mutated
    /// in lockstep with `document`.
    pub mindmap_tree: &'a mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    /// App-layer scene host owning every tree-rendered component.
    pub app_scene: &'a mut AppScene,
    /// The active renderer.
    pub renderer: &'a mut Renderer,
    /// Per-edge connection glyph cache.
    pub scene_cache: &'a mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    /// Current pointer / drag state machine.
    pub drag_state: &'a mut DragState,
    /// Reparent / Connect modal mode for the next click.
    pub app_mode: &'a mut AppMode,
    /// Console (slash-command overlay) state.
    pub console_state: &'a mut ConsoleState,
    /// Console command-history ring.
    pub console_history: &'a mut Vec<String>,
    /// Inline edge-label editor state.
    pub label_edit_state: &'a mut LabelEditState,
    /// Inline portal-text editor state.
    pub portal_text_edit_state: &'a mut PortalTextEditState,
    /// Inline node text editor state.
    pub text_edit_state: &'a mut TextEditState,
    /// Glyph-wheel color-picker state.
    pub color_picker_state: &'a mut ColorPickerState,
    /// Previous click (time, position, hit) for double-click detection.
    pub last_click: &'a mut Option<LastClick>,
    /// The node the cursor is currently over, if any.
    pub hovered_node: &'a mut Option<String>,
    /// Last-known cursor position in screen space.
    pub cursor_pos: &'a mut (f64, f64),
    /// Modifier snapshot maintained by `ModifiersChanged` events.
    pub modifiers: &'a ModifiersState,
    /// Per-frame cursor-icon flag — flipped to "hand" over a button
    /// node by the cursor-move handler.
    pub cursor_is_hand: &'a mut bool,
    /// Drag-throttle for high-frequency mutation commits (node
    /// drags, edge-handle drags).
    pub mutation_throttle: &'a mut MutationFrequencyThrottle,
    /// Scene-rebuild flag set by interactive color-picker edits.
    pub picker_dirty: &'a mut bool,
    /// Resolved user keybinds.
    pub keybinds: &'a mut ResolvedKeybinds,
}
