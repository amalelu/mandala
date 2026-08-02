// SPDX-License-Identifier: MPL-2.0

//! Shared per-call context for the native event-loop dispatchers.
//!
//! Each handler reaches into the same mutable bundle on `InitState`.
//! `InputHandlerContext<'a>` borrows each persistent field as
//! `&'a mut T` so handlers can destructure it; per-event payloads stay
//! as separate function parameters. Built once per event in
//! [`super::run_native::InitState::input_context`].

#![cfg(not(target_arch = "wasm32"))]

use crate::application::platform::input::Modifiers as ModifiersState;

use crate::application::color_picker::ColorPickerState;
use crate::application::console::ConsoleState;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::macros::MacroRegistry;
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;

use super::single_line_edit::SingleLineEditor;
use super::text_edit::TextEditState;
use super::throttled_interaction::{ColorPickerHoverInteraction, DrainContext, ReleaseCommit};
use super::{DragState, InteractionMode, LastClick};

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
    /// High-level interaction mode (`Default`, `Reparent`, `Connect`,
    /// `NodeEdit`, `Resize`). Drives click routing, mode-gated chrome
    /// (resize anchors, target highlights), and selection resolution.
    /// Mutated by mode-transition Action arms (`EnterReparentMode`,
    /// `EnterConnectMode`, `EnterResizeMode`, `ExitMode`,
    /// `ReparentToTarget`, `ConnectToTarget`).
    pub interaction_mode: &'a mut InteractionMode,
    /// Console (slash-command overlay) state.
    pub console_state: &'a mut ConsoleState,
    /// Console command-history ring.
    pub console_history: &'a mut Vec<String>,
    /// Inline single-line editor state (edge label / portal
    /// caption).
    pub single_line_edit_state: &'a mut SingleLineEditor,
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
    /// Last cursor icon written via `Window::set_cursor`. The
    /// cursor_moved handler skips the per-frame `set_cursor`
    /// when the desired icon equals this. Required because winit
    /// dedupes `set_cursor` only on macOS / X11; Windows + Wayland
    /// re-issue the underlying OS call every time.
    pub cursor_icon_last: &'a mut winit::window::CursorIcon,
    /// Throttled color-picker hover interaction. The picker's
    /// input paths set `.dirty` when HSV state changes; the
    /// per-frame drain rebuilds the scene + overlay through the
    /// unified adaptive-throttle shell.
    pub picker_hover: &'a mut ColorPickerHoverInteraction,
    /// Resolved user keybinds. `&` (immutable) — no dispatch path
    /// mutates keybinds; query methods (`action_for_*`, `macro_for`,
    /// `custom_mutation_for`, `has_*`) are all `&self`. Track-C
    /// review reflagged the original `&mut` as dead.
    pub keybinds: &'a ResolvedKeybinds,
    /// Macro registry (App + User tiers loaded at startup; Map tier
    /// re-loaded whenever a document is replaced via `open` / `new`).
    /// Mutable so the document-replace path in
    /// `execute_console_line` can rebuild the Map tier without
    /// taking a separate ad-hoc borrow.
    pub macros: &'a mut MacroRegistry,
}

impl<'a> InputHandlerContext<'a> {
    /// Renderer-free slice for a throttled drag's release commit —
    /// see
    /// [`super::throttled_interaction::release`]. The commit body
    /// mutates the model and the scene cache, then hands back a
    /// [`super::throttled_interaction::ReleaseRefresh`] the caller
    /// runs against [`Self::drain_context`].
    pub(in crate::application::app) fn release_commit(&mut self) -> ReleaseCommit<'_> {
        ReleaseCommit {
            document: &mut *self.document,
            mindmap_tree: &mut *self.mindmap_tree,
            scene_cache: &mut *self.scene_cache,
        }
    }

    /// The per-frame drain bundle, rebuilt from the event-loop
    /// context. `drain_inputs` builds the same struct from
    /// `InitState` directly; the release path reaches it from here
    /// so a drag's release-commit and its per-frame drains share
    /// one context type.
    pub(in crate::application::app) fn drain_context(&mut self) -> DrainContext<'_> {
        DrainContext {
            document: &mut *self.document,
            mindmap_tree: &mut *self.mindmap_tree,
            app_scene: &mut *self.app_scene,
            renderer: &mut *self.renderer,
            scene_cache: &mut *self.scene_cache,
            color_picker_state: &mut *self.color_picker_state,
            interaction_mode: &*self.interaction_mode,
        }
    }

    /// Decompose into the cross-platform
    /// [`super::input_context_core::InputContextCore`] +
    /// native-only [`super::input_context_core::NativeContextExt`]
    /// pair the unified `dispatch_action` expects post-Track-C.
    /// Each returned view re-borrows from `self` so the original
    /// `InputHandlerContext` is borrowed as `&mut` for the
    /// duration; the views are dropped before the dispatcher
    /// returns. WASM constructs an `InputContextCore` directly via
    /// `WasmInputState::input_context_core` and passes `None` for
    /// the extension.
    ///
    /// `'s` is shorter than `'a` — the views borrow from `self`,
    /// not from the original outer borrow source.
    pub fn split_borrow<'s>(
        &'s mut self,
    ) -> (
        super::input_context_core::InputContextCore<'s>,
        super::input_context_core::NativeContextExt<'s>,
    ) {
        (
            super::input_context_core::InputContextCore {
                document: self.document.as_mut(),
                mindmap_tree: &mut *self.mindmap_tree,
                app_scene: &mut *self.app_scene,
                renderer: &mut *self.renderer,
                scene_cache: &mut *self.scene_cache,
                text_edit_state: &mut *self.text_edit_state,
                last_click: &mut *self.last_click,
                cursor_pos: &mut *self.cursor_pos,
                modifiers: self.modifiers,
                keybinds: self.keybinds,
                macros: &mut *self.macros,
                interaction_mode: &mut *self.interaction_mode,
            },
            super::input_context_core::NativeContextExt {
                drag_state: &mut *self.drag_state,
                console_state: &mut *self.console_state,
                console_history: &mut *self.console_history,
                single_line_edit_state: &mut *self.single_line_edit_state,
                color_picker_state: &mut *self.color_picker_state,
                hovered_node: &mut *self.hovered_node,
                cursor_is_hand: &mut *self.cursor_is_hand,
                cursor_icon_last: &mut *self.cursor_icon_last,
                picker_hover: &mut *self.picker_hover,
            },
        )
    }
}
