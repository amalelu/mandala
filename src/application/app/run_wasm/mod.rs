// SPDX-License-Identifier: MPL-2.0

//! WASM event-loop body for [`super::Application::run`]. Builds a
//! [`WasmApp`] [`ApplicationHandler`] and hands it to winit-web's
//! [`EventLoopExtWebSys::spawn_app`]; the browser's main thread
//! drives the loop from there. Synchronous DOM setup (canvas
//! attach, `tabindex` focus, keydown preventDefault listener)
//! finishes inside [`run`] before `spawn_app` is called; the
//! async path (`Renderer::new`, document fetch, rAF render loop
//! install) is *spawned* before `spawn_app` via
//! `wasm_bindgen_futures::spawn_local` but resumes on later
//! microtask ticks, so [`WasmApp`]'s `renderer` / `input` cells
//! start `None` and the handler's match arms guard accordingly.
//!
//! The [`ApplicationHandler::window_event`] callback funnels into
//! [`WasmApp::handle_window_event`], which owns the dispatch
//! match: six arms fan out to per-arm `handle_*` inherent methods
//! defined in sibling `event_*.rs` files (Resized, ModifiersChanged,
//! KeyboardInput, CursorMoved, MouseInput, MouseWheel) plus an
//! inline `CloseRequested` no-op and a catch-all that drops the
//! arms WASM doesn't currently consume. Mirrors native's per-arm
//! split under `app/event_*.rs` so cross-target convergence
//! reviews can diff the two layouts side by side.

#![cfg(target_arch = "wasm32")]

mod event_cursor_moved;
mod event_keyboard;
mod event_modifiers;
mod event_mouse_click;
mod event_mouse_wheel;
mod event_resized;
mod event_touch;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use baumhard::mindmap::tree_builder::MindMapTree;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::platform::web::EventLoopExtWebSys;
use winit::window::WindowId;

use super::scene_rebuild::{rebuild_all, warm_scene_at_load};
use super::text_edit::TextEditState;
use super::{Application, LastClick};
use crate::application::common::RenderDecree;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::{Action, ResolvedKeybinds};
use crate::application::renderer::Renderer;

/// Pending left-click awaiting a release. `None` on init and after
/// release consumed; `Empty` after a click-down on empty canvas;
/// `Node(id)` after a click-down on a node. Full drag machine
/// (pan, move, reparent, connect) deferred to a later
/// WASM-parity session.
///
/// Module-level (not closure-local) so helpers in sibling modules
/// can take `&WasmInputState` parameters that include the field.
pub(super) enum PendingClick {
    None,
    Empty,
    Node(String),
    /// Cursor landed on a specific section inside a multi-section
    /// node at mouse-down. Committed at mouse-up into
    /// `SelectionState::Section { node_id, section_idx }` so
    /// per-section verbs (text edit, font, color) target only
    /// that section. Single-section nodes route through `Node`
    /// instead, preserving the whole-node click semantic for
    /// migrated maps.
    Section {
        node_id: String,
        section_idx: usize,
    },
    /// Cursor landed on a portal **icon** at mouse-down. Committed
    /// at mouse-up into a `SelectionState::PortalLabel`. Carries
    /// both the owning-edge key and the endpoint id the marker
    /// belongs to, matching the native click dispatch surface.
    PortalMarker {
        edge_key: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint_node_id: String,
    },
    /// Cursor landed on a portal **text** at mouse-down.
    /// Committed at mouse-up into a `SelectionState::PortalText`.
    /// Shares the identity shape with `PortalMarker`; only the
    /// mouse-up selection routing differs.
    PortalText {
        edge_key: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint_node_id: String,
    },
    /// Cursor landed on a line-mode edge's label AABB at
    /// mouse-down. Committed at mouse-up into
    /// `SelectionState::EdgeLabel` so per-label color / font /
    /// copy operations target the label instead of the edge
    /// body. Double-click is handled inline by the press-time
    /// dispatcher — WASM doesn't open the inline editor modal
    /// yet so the dbl-click branch falls back to the same
    /// selection commit for parity with single click.
    EdgeLabel(baumhard::mindmap::scene_cache::EdgeKey),
}

/// Shared state between the WASM rAF render loop and the winit
/// event loop. Mirrors native's `InputHandlerContext` for the 9
/// fields both targets share. Promoted to module-level (was
/// closure-local in `run`) so the `cross_dispatch` /
/// `dispatch_macro_core` helpers can take `&mut WasmInputState`
/// directly — closure-local types aren't reachable from sibling
/// modules.
pub(super) struct WasmInputState {
    pub document: MindMapDocument,
    pub mindmap_tree: Option<MindMapTree>,
    pub text_edit_state: TextEditState,
    pub last_click: Option<LastClick>,
    pub cursor_pos: (f64, f64),
    pub pending_click: PendingClick,
    pub modifiers: crate::application::platform::input::Modifiers,
    /// High-level interaction mode (Default / Reparent / Connect /
    /// NodeEdit / Resize). Cross-platform field per Batch 1 of
    /// `SECTIONS_BORDERS_RESIZE_PLAN.md` — the enum is target-agnostic
    /// even though some mode transitions still depend on native-only
    /// dispatch arms (Reparent / Connect). WASM construction
    /// initializes to `Default`; later batches add mode-aware click
    /// routing here.
    pub interaction_mode: super::InteractionMode,
    /// Mirror of native's `app_scene` so the canvas-scene tree
    /// path (borders, eventually connections / portals) works
    /// identically on WASM. Threaded into every
    /// `rebuild_all` / `rebuild_scene_only` call below.
    pub app_scene: crate::application::scene_host::AppScene,
    /// Mirror of native's `scene_cache` so the cache-aware
    /// cache-aware connection pass skips `sample_path`
    /// work for unchanged edges. Threaded into every rebuild
    /// helper the same way native does.
    pub scene_cache: baumhard::mindmap::scene_cache::SceneConnectionCache,
    /// Macro registry. Mirrors native's `InputHandlerContext::macros`
    /// — App + User tiers loaded at startup; Map + Inline tiers
    /// refreshed by `loader::rebuild_document_macros` whenever a
    /// document is loaded. Consulted by the keyboard handler's
    /// Action → Macro → CustomMutation fall-through.
    pub macros: crate::application::macros::MacroRegistry,
    /// Touch gesture recognizer. Cross-platform peer of native's
    /// `InitState.touch_recognizer`. WASM mobile is the *primary*
    /// surface this targets (mobile-browser tap-and-hold is
    /// unreachable through `MouseInput`/`CursorMoved` events
    /// alone — winit dispatches them as
    /// `WindowEvent::Touch` instead).
    pub touch_recognizer: super::touch_gesture::TouchGestureRecognizer,
}

impl WasmInputState {
    /// Build the cross-platform `InputContextCore` view for
    /// `dispatch_action_core::dispatch_compatible`. `renderer` and
    /// `keybinds` aren't fields on `WasmInputState` (held in the
    /// closure scope of `run`); the caller passes them in. Mirrors
    /// native's `InputHandlerContext::split_borrow` shape — short
    /// `'s` lifetime tied to `&mut self`, dropped before the next
    /// re-borrow of `self`.
    pub(super) fn input_context_core<'s>(
        &'s mut self,
        renderer: &'s mut Renderer,
        keybinds: &'s crate::application::keybinds::ResolvedKeybinds,
    ) -> super::input_context_core::InputContextCore<'s> {
        super::input_context_core::InputContextCore {
            document: Some(&mut self.document),
            mindmap_tree: &mut self.mindmap_tree,
            app_scene: &mut self.app_scene,
            renderer,
            scene_cache: &mut self.scene_cache,
            text_edit_state: &mut self.text_edit_state,
            last_click: &mut self.last_click,
            cursor_pos: &mut self.cursor_pos,
            modifiers: &self.modifiers,
            keybinds,
            macros: &mut self.macros,
            interaction_mode: &mut self.interaction_mode,
        }
    }
}

/// WASM impl of `MacroDispatchTarget`. Wraps
/// `&mut WasmInputState` + `&mut Renderer` and forwards each
/// operation to the same helpers the keyboard handler uses.
/// Privilege gating happens in
/// `dispatch_macro_core::dispatch_macro` (single-source contract).
struct WasmMacroDispatchTarget<'a> {
    input: &'a mut WasmInputState,
    renderer: &'a mut Renderer,
    /// `keybinds` belongs in `InputContextCore` (Track C); WASM
    /// holds its `ResolvedKeybinds` in the closure scope of `run`,
    /// not on `WasmInputState`. Threaded here so
    /// `MacroDispatchTarget::dispatch_action` can build the core.
    keybinds: &'a crate::application::keybinds::ResolvedKeybinds,
}

impl<'a> super::dispatch::macro_core::MacroDispatchTarget for WasmMacroDispatchTarget<'a> {
    fn registry(&self) -> &crate::application::macros::MacroRegistry {
        &self.input.macros
    }

    fn dispatch_action(&mut self, action: Action) -> super::dispatch::DispatchOutcome {
        // Track C: route through the unified cross-platform
        // dispatcher. Pre-Track-C this called the WASM-only
        // `dispatch_compatible_action_wasm` shim; that shim is
        // deleted and both the keyboard handler and this trait
        // impl reach `dispatch_action_core::dispatch_compatible`.
        // The mixed-branch lift below restores `Handled` returns
        // for `ExitMode`/`EditSelection*` so the macro loop's
        // `any_ran` flag bumps correctly — see
        // `lift_mixed_branch_for_wasm_macro`'s rustdoc.
        //
        // A macro step carries no `DispatchHit`: it is a keystroke-
        // level instruction with no pointer event behind it. Both
        // `None`s below are that one fact — no hit means the
        // `DoubleClickActivate` arm returns before reaching the apply
        // half, so there is no `DoubleClickResidual` either, and the
        // lift is told so rather than left to infer it.
        let hit = None;
        let double_click_residual = None;
        let outcome = {
            let mut core = self.input.input_context_core(self.renderer, self.keybinds);
            super::dispatch::action_core::dispatch_compatible(&action, &mut core, hit)
        };
        super::dispatch::action_core::lift_mixed_branch_for_wasm_macro(
            &action,
            double_click_residual,
            outcome,
        )
    }

    fn apply_custom_mutation(&mut self, id: &str, node_id: &str) -> bool {
        let cm = self.input.document.mutation_registry.get(id).cloned();
        let Some(cm) = cm else {
            log::warn!("macro step: unknown custom-mutation id '{}'", id);
            return false;
        };
        let now = super::now_ms() as u64;
        let applied = super::dispatch::apply_keybind_custom_mutation(
            &mut self.input.document,
            &mut self.input.mindmap_tree,
            &mut self.input.scene_cache,
            &cm,
            node_id,
            now,
        );
        if applied {
            // Match native pre-Track-B: rebuild via plain
            // `rebuild_all` (no extra `scene_cache.clear()`).
            // `apply_keybind_custom_mutation` already cleared the
            // cache on the non-animated branch; the animated
            // branch deliberately leaves it for the animation
            // envelope to invalidate. Going through
            // `rebuild_after_geometry_change` here would clear
            // again on non-animated and add an unwanted clear on
            // animated.
            super::scene_rebuild::rebuild_all(
                &self.input.document,
                &self.input.interaction_mode,
                &mut self.input.mindmap_tree,
                &mut self.input.app_scene,
                self.renderer,
                &mut self.input.scene_cache,
            );
            true
        } else {
            false
        }
    }

    fn execute_console_line(&mut self, line: &str) -> bool {
        // WASM has no `execute_console_line` runtime
        // (`console_input` is `cfg(not(target_arch = "wasm32"))`).
        // The privilege gate already rejects ConsoleLine from
        // non-User tiers above this; User-tier ConsoleLine on WASM
        // logs warn + skips per `format/macros.md` §
        // ConsoleLine on WASM. The macro continues to the next
        // step — fail-soft, matching the User-tier "step failed"
        // posture native uses for unknown CustomMutation ids.
        //
        // Returns `false` so the macro's `any_ran` flag doesn't
        // bump on the no-op path — a `[ConsoleLine]`-only macro
        // on WASM correctly reports "didn't fire" so the keystroke
        // isn't artificially consumed. Mirrors native's pre-doc-
        // load posture (false return on the warn arm).
        log::warn!(
            "macros: ConsoleLine step '{}' has no console runtime on WASM; skipping",
            line,
        );
        false
    }

    fn current_selection_node_id(&self) -> Option<String> {
        if let crate::application::document::SelectionState::Single(nid) = &self.input.document.selection {
            Some(nid.clone())
        } else {
            None
        }
    }

    fn has_node(&self, node_id: &str) -> bool {
        self.input.document.mindmap.nodes.contains_key(node_id)
    }
}

/// winit 0.30 [`ApplicationHandler`] for the WASM target. Mirrors
/// the native `super::run_native`'s `NativeApp` shape so each
/// target's event-loop entry point reads the same way. Plain
/// code-span: that module is native-gated, so an intra-doc link to
/// it cannot resolve in this target's doc build.
///
/// Initial DOM setup (canvas attach, `tabindex` focus, keydown
/// preventDefault listener, async renderer construction, document
/// fetch, rAF render loop install) is kicked off inside [`run`]
/// before [`EventLoopExtWebSys::spawn_app`] hands control to winit;
/// the handler then receives [`WindowEvent`]s on the browser's main
/// thread until the tab is torn down. `spawn_app` requires the
/// handler be `'static`, which is why every shared field is
/// `Rc`/`Arc`-owned rather than borrowed.
///
/// The `winit::window::Window` itself is held by the [`Renderer`]
/// (`Arc<Window>` field, populated inside `Renderer::new` from a
/// clone made before this handler is constructed); the handler does
/// not need its own clone for keep-alive purposes and never reads
/// the window directly from event-handler context.
struct WasmApp {
    /// Shared with the rAF render loop set up in [`run`]. `None`
    /// until the async `Renderer::new` future inside `spawn_local`
    /// resolves.
    ///
    /// **Re-borrow contract.** Any code path that's already
    /// holding `self.renderer.borrow*()` (per-arm methods do this
    /// at the top of their body) must NOT call back into a
    /// dispatch path that re-clones this `Rc` and re-borrows it —
    /// `RefCell` defers the conflict to runtime panic. Today no
    /// such re-entry exists (cross_dispatch arms see only the
    /// `RebuildContext` projection, not the outer `Rc`); future
    /// arms wiring `WasmInputState`-internal Rc handles must
    /// keep this contract.
    renderer: Rc<RefCell<Option<Renderer>>>,
    /// Shared with the rAF render loop. `None` until the document
    /// fetch + tree build inside `spawn_local` completes.
    ///
    /// Same re-borrow contract as [`Self::renderer`] applies.
    input: Rc<RefCell<Option<WasmInputState>>>,
    /// Shared with the canvas keydown listener so the editor's
    /// open / close transitions can flip its `preventDefault` flag.
    ///
    /// **`Cell`, not `RefCell`** — load-bearing. Several per-arm
    /// methods call `self.suppress_keys.set(...)` while
    /// `self.input.borrow_mut()` is still live; if the type were
    /// `Rc<RefCell<bool>>` those calls would have to drop the
    /// input borrow first to avoid the runtime borrow conflict.
    /// `Cell::set` doesn't take a borrow so the two operations
    /// can interleave freely. Don't change the type.
    suppress_keys: Rc<Cell<bool>>,
    /// Resolved keybind table — built once in [`run`] from
    /// `Options::keybind_config`, read on every key event.
    keybinds: ResolvedKeybinds,
}

impl ApplicationHandler for WasmApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // Canvas + window were created and DOM-attached before
        // `spawn_app` ran (browser DOM ordering is driven by JS
        // events, not winit's resume cycle), so first-fire is a
        // no-op. winit-web also re-fires `resumed` after a
        // bfcache restore; the empty body is idempotent so no
        // guard is needed (unlike `NativeApp::resumed`'s
        // `is_some()` check, which protects window re-creation).
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        self.handle_window_event(event);
    }
}

impl WasmApp {
    /// Per-event dispatch — seven arms fan out into per-arm
    /// methods defined in sibling `event_*.rs` files, plus an
    /// inline `RedrawRequested` arm that runs the renderer's
    /// `process()` (the sole entry to render on web), an inline
    /// `CloseRequested` no-op (browser tabs don't really close)
    /// and a `_ => {}` catch-all that silently drops the arms
    /// winit fires that WASM doesn't currently consume (touch
    /// events, IME composition, focus changes). Each fan-out
    /// arm extracts only the fields that arm needs.
    ///
    /// Mutating arms set `redraw_after = true`; the trailing
    /// `Renderer::request_redraw` call hands a redraw request
    /// to winit-web, which translates to an internal rAF
    /// dispatch. The browser only paints when the canvas
    /// actually changed.
    fn handle_window_event(&mut self, event: WindowEvent) {
        let mut redraw_after = false;
        match event {
            WindowEvent::RedrawRequested => {
                // Sole entry to the render path on web. Coalesced
                // by winit-web from any number of `request_redraw`
                // calls in the prior event chain.
                if let Some(r) = self.renderer.borrow_mut().as_mut() {
                    r.process();
                }
            }
            WindowEvent::Resized(s) => {
                self.handle_resized(s);
                redraw_after = true;
            }
            WindowEvent::CloseRequested => {
                // WASM doesn't really close
            }
            WindowEvent::ModifiersChanged(m) => {
                self.handle_modifiers_changed(m);
                // Modifier-only changes don't paint anything until
                // a paired mouse/keyboard event acts on them.
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state: ElementState::Pressed,
                        logical_key,
                        ..
                    },
                ..
            } => {
                self.handle_keyboard_input(logical_key);
                redraw_after = true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.handle_cursor_moved(position);
                // WASM doesn't yet have full drag/picker state
                // (see `WasmInputState` field comments) so the
                // CursorMoved redraw conditions native checks for
                // collapse to "yes" for any in-progress click.
                // Conservative: redraw on every CursorMoved that
                // arrived while a click is pending.
                redraw_after = !matches!(
                    self.input.borrow().as_ref().map(|s| matches!(s.pending_click, PendingClick::None)),
                    Some(true) | None,
                );
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.handle_mouse_input(state);
                redraw_after = true;
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                // WASM right-button: `DragState` machinery is
                // `cfg(not(target_arch = "wasm32"))` so there's no
                // `PendingRight` → `FastResizeStart` pipeline yet
                // (lands with §6.6 touch parity / `TwoFingerDrag`).
                // What we *can* do today is dispatch the bound
                // `MouseGesture::RightClick` action on release —
                // single-press right-click is just an action lookup,
                // no DragState involved. Default-unbound so it's a
                // no-op for users who didn't opt in; users who did
                // get the action they bound. Without this arm the
                // event drops silently.
                self.handle_right_button(state);
                redraw_after = true;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_mouse_wheel(delta);
                redraw_after = true;
            }
            WindowEvent::Touch(touch) => {
                if self.handle_touch_event(touch) {
                    redraw_after = true;
                }
            }
            // Catch-all for winit `WindowEvent` variants WASM
            // doesn't yet route. The notable un-wired ones:
            //
            // - `WindowEvent::Ime` — IME composition strings.
            //   Required for non-Latin text editing inside the
            //   inline node-text editor. Modal-handler-side
            //   work; the dispatch funnel doesn't see literal
            //   IME payloads (§3 carve-out for `winit::Key`).
            // - `WindowEvent::Focused`, `CursorEntered` /
            //   `CursorLeft` — informational; could drive a
            //   "canvas inactive" overlay in the future.
            //
            // Drop silently rather than log because winit fires
            // every variant on every event tick; a log would
            // burn 60Hz × variants. When any of the above gets
            // wired, lift the corresponding match arm out of
            // this catch-all into a dedicated handle_* method.
            _ => {}
        }
        if redraw_after {
            if let Some(r) = self.renderer.borrow().as_ref() {
                r.request_redraw();
            }
        }
    }
}

/// Run the browser event loop against `app`.
pub(super) fn run(mut app: Application) {
    use baumhard::mindmap::tree_builder::MindMapTree;
    use wasm_bindgen::JsCast;
    use winit::platform::web::WindowExtWebSys;

    baumhard::font::fonts::init();

    // Load keybindings from the WASM environment (URL query param or
    // localStorage) with a defaults fallback. Failure is non-fatal —
    // see KeybindConfig::load_for_web().
    app.options.keybind_config = crate::application::keybinds::KeybindConfig::load_for_web();

    // Attach canvas to DOM
    let canvas = app.window.canvas().expect("Failed to get canvas");
    let web_window = web_sys::window().expect("No global window");
    let document = web_window.document().expect("No document");
    let body = document.body().expect("No body");
    body.append_child(&canvas).expect("Failed to append canvas");
    canvas.set_width(
        web_window
            .inner_width()
            .expect("web_window.inner_width before first frame")
            .as_f64()
            .expect("inner_width JsValue is f64") as u32,
    );
    canvas.set_height(
        web_window
            .inner_height()
            .expect("web_window.inner_height before first frame")
            .as_f64()
            .expect("inner_height JsValue is f64") as u32,
    );
    let cw = canvas.width();
    let ch = canvas.height();
    log::info!("WASM: canvas sized {}x{}", cw, ch);
    if cw == 0 || ch == 0 {
        log::warn!("WASM: canvas has zero dimension — render surface will be empty");
    }

    // Canvas must be focusable for keyboard events to reach winit.
    // Without tabindex, an HTMLCanvasElement never receives focus.
    canvas.set_attribute("tabindex", "0").ok();
    let _ = canvas.focus();

    // Re-focus on mousedown so clicking the canvas after tabbing
    // to another element restores keyboard input.
    {
        let canvas_for_focus = canvas.clone();
        let focus_cb =
            wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
                let _ = canvas_for_focus.focus();
            });
        canvas
            .add_event_listener_with_callback("mousedown", focus_cb.as_ref().unchecked_ref())
            .ok();
        focus_cb.forget(); // leak — lives for the page lifetime
    }

    // Suppress the browser's default context menu on right-click
    // so the canvas can use right-button gestures
    // (`MouseGesture::RightClick` / `RightDrag`,
    // `Action::FastResizeStart` per Batch 4 of
    // SECTIONS_BORDERS_RESIZE_PLAN.md §6.3). Without this, every
    // right-press on the canvas would pop the browser's context
    // menu and the gesture would never reach the threshold-cross.
    //
    // **Shift+RightClick bypass**: holding Shift on the right-press
    // lets the browser's default context menu through. Standard
    // browser convention for "let me get to the OS / dev menu past
    // a page handler" — users debugging the canvas (Inspect Element,
    // screenshotting glyph layout, filing bug reports) get the
    // ergonomic path back. Shift modifier is also unused by any
    // shipped right-button binding, so the convention doesn't
    // collide with anything.
    {
        let pd_cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
            move |evt: web_sys::Event| {
                use wasm_bindgen::JsCast;
                let allow_default = evt
                    .dyn_ref::<web_sys::MouseEvent>()
                    .map(|me| me.shift_key())
                    .unwrap_or(false);
                if !allow_default {
                    evt.prevent_default();
                }
            },
        );
        canvas
            .add_event_listener_with_callback("contextmenu", pd_cb.as_ref().unchecked_ref())
            .ok();
        pd_cb.forget(); // leak — lives for the page lifetime
    }

    // preventDefault on keydown while the text editor is open so
    // Tab/Enter/Backspace/arrows don't fire browser defaults
    // (tab-navigation, history-back, page-scroll).
    let suppress_keys: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    {
        let suppress = suppress_keys.clone();
        let pd_cb =
            wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |evt: web_sys::Event| {
                if suppress.get() {
                    evt.prevent_default();
                }
            });
        canvas
            .add_event_listener_with_callback("keydown", pd_cb.as_ref().unchecked_ref())
            .ok();
        pd_cb.forget();
    }

    let renderer_window = Arc::clone(&app.window);

    // On WASM, check for ?map= query parameter to override the default path
    let mindmap_path = {
        let web_window = web_sys::window().expect("No global window");
        let search = web_window.location().search().unwrap_or_default();
        let mut map_path: Option<String> = None;
        let trimmed = search.trim_start_matches('?');
        for pair in trimmed.split('&') {
            if let Some(val) = pair.strip_prefix("map=") {
                map_path = Some(val.to_string());
            }
        }
        map_path.unwrap_or_else(|| app.options.mindmap_path.clone())
    };

    // Shared state between the rAF render loop and the winit event
    // loop. Two RefCells so input handlers can borrow InputState
    // and Renderer simultaneously without conflict.
    // `WasmInputState` and `PendingClick` are now declared at module
    // scope (above `fn run`) so cross-platform helpers can take them
    // by `&mut`. Construction of the actual instance happens later in
    // `spawn_local`.

    let renderer_rc: Rc<RefCell<Option<Renderer>>> = Rc::new(RefCell::new(None));
    let input_rc: Rc<RefCell<Option<WasmInputState>>> = Rc::new(RefCell::new(None));

    // Clone Rcs for the spawn_local init future
    let renderer_for_init = renderer_rc.clone();
    let input_for_init = input_rc.clone();

    // Renderer init is async on the browser (adapter + surface setup
    // are Promise-backed). Spawn as a future so the event loop doesn't
    // block waiting for wgpu.
    wasm_bindgen_futures::spawn_local(async move {
        let mut renderer = Renderer::bootstrap_wasm(renderer_window, canvas.clone()).await;
        log::info!("WASM: adapter + surface + renderer ready");

        let size = canvas.width();
        let height = canvas.height();
        renderer.process_decree(RenderDecree::SetSurfaceSize(size, height));
        log::info!("WASM: surface configured {}x{}", size, height);

        // std::fs is unavailable in the browser; fetch over the page origin instead.
        let mut doc_opt: Option<MindMapDocument> = None;
        let mut tree_opt: Option<MindMapTree> = None;
        // Lifted to outer scope so the warm-cache and warm-AppScene
        // built during init survive into `WasmInputState` (the
        // active event-loop scene host) instead of being dropped at
        // the end of init. Native does this implicitly because both
        // halves share the same `InitState`; on WASM the rAF loop
        // and the input state are separate Rc<RefCell> cells, so we
        // build once and move into the input state.
        let mut init_app_scene = crate::application::scene_host::AppScene::new();
        let mut init_scene_cache = baumhard::mindmap::scene_cache::SceneConnectionCache::new();
        match fetch_map_json(&mindmap_path).await {
            Ok(json) => match MindMapDocument::from_json_str(&json, Some(mindmap_path.clone())) {
                Ok(mut doc) => {
                    // Canvas background: resolve through theme variables
                    // so `"var(--bg)"` works, then hand off to the
                    // renderer as the render-pass clear color. Mirrors
                    // run_native.rs so the WASM canvas paints against
                    // the doc's configured background instead of the
                    // default pitch black.
                    let vars = &doc.mindmap.canvas.theme_variables;
                    let resolved_bg =
                        baumhard::util::color::resolve_var(&doc.mindmap.canvas.background_color, vars);
                    renderer.set_clear_color_from_hex(resolved_bg);

                    // Four-source mutation registry, matching the native
                    // path: app bundle (shipped in the binary) < user
                    // source (?mutations= query param + localStorage) <
                    // map (custom_mutations in the .mindmap.json) <
                    // inline (on individual nodes). Plus the Rust-backed
                    // handlers for layouts too structural for pure data.
                    let (app_mutations, user_mutations) =
                        crate::application::document::mutations_loader::load_app_and_user();
                    doc.build_mutation_registry_with_app_and_user(&app_mutations, &user_mutations);
                    crate::application::document::mutations::register_builtin_handlers(&mut doc);

                    let mindmap_tree = doc.build_tree();
                    renderer.rebuild_buffers_from_tree(&mindmap_tree.tree);
                    renderer.fit_camera_to_tree(&mindmap_tree.tree);

                    // Every canvas role projected once at load plus
                    // the handle-tree allocator warm —
                    // `warm_scene_at_load`, the body native's init
                    // runs too. `fit_camera_to_tree` above settled
                    // the zoom, which that helper reads.
                    warm_scene_at_load(&doc, &mut init_app_scene, &mut renderer, &mut init_scene_cache);
                    tree_opt = Some(mindmap_tree);
                    doc_opt = Some(doc);
                }
                Err(e) => {
                    log::error!(
                        "WASM: failed to construct document from '{}': {}",
                        mindmap_path,
                        e
                    );
                    show_load_error_overlay(&mindmap_path, &e.to_string());
                }
            },
            Err(e) => {
                log::error!("WASM: failed to fetch '{}': {}", mindmap_path, e);
                show_load_error_overlay(&mindmap_path, &e);
            }
        }

        renderer.process_decree(RenderDecree::StartRender);
        log::info!("WASM: StartRender dispatched, rAF loop starting");

        // Pre-warm allocators on the rebuild_all critical path.
        // Mirrors native init — see the run_native_init.rs doc for
        // what this does and why.
        if let Some(doc) = doc_opt.as_ref() {
            // WASM init runs in Default mode.
            let init_mode = super::InteractionMode::Default;
            rebuild_all(
                doc,
                &init_mode,
                &mut tree_opt,
                &mut init_app_scene,
                &mut renderer,
                &mut init_scene_cache,
            );
        }

        // Pre-warm the render pipeline: one full render cycle so
        // the wgpu driver compiles pipeline shaders, the swapchain
        // allocates its first backing image, and the glyph atlas
        // is populated before the first user-driven frame.
        renderer.prewarm();

        // Populate the shared state now that init is complete.
        *renderer_for_init.borrow_mut() = Some(renderer);

        if let Some(doc) = doc_opt {
            // App + User tiers from the bundled JSON / `?macros=` /
            // localStorage, then the document-derived Map and Inline
            // tiers. Body is `macros::loader::build_macro_registry`,
            // which native's init calls too — including the
            // `log::info!` shape, so cross-target log triage stays
            // uniform.
            let macros = crate::application::macros::loader::build_macro_registry(Some(&doc));

            *input_for_init.borrow_mut() = Some(WasmInputState {
                document: doc,
                mindmap_tree: tree_opt,
                text_edit_state: TextEditState::Closed,
                last_click: None,
                cursor_pos: (0.0, 0.0),
                pending_click: PendingClick::None,
                modifiers: crate::application::platform::input::Modifiers::empty(),
                interaction_mode: super::InteractionMode::Default,
                // Move the warm AppScene + scene_cache built during
                // init into the live event-loop state so the
                // pre-warm work isn't dropped at the end of init.
                app_scene: init_app_scene,
                scene_cache: init_scene_cache,
                macros,
                touch_recognizer: super::touch_gesture::TouchGestureRecognizer::new(),
            });
        }

        // Event-driven redraw model: instead of an unconditional
        // self-rescheduling rAF (which kept the main thread busy at
        // 60Hz even when the canvas was visually idle), we kick a
        // single `Window::request_redraw` here to paint the first
        // frame and rely on `WasmApp::handle_window_event`'s
        // `WindowEvent::RedrawRequested` arm to do all subsequent
        // renders. Mutating event handlers call
        // `Renderer::request_redraw` after their work, which
        // winit-web translates to an internal rAF dispatch — so the
        // browser only paints when the canvas actually changed.
        if let Some(r) = renderer_for_init.borrow().as_ref() {
            r.request_redraw();
        }
    });

    // Resolve the keybind config once. `action_for(key, ctrl, shift, alt)`
    // answers the dispatch question for every keydown.
    let keybinds: ResolvedKeybinds = app.options.keybind_config.resolve();

    // Hand control to winit. The handler owns the renderer / input
    // cells (shared with the rAF render loop), the editor-suppress
    // flag (shared with the canvas keydown listener), and the
    // resolved keybind table. `spawn_app` returns immediately on
    // the web target — the browser's main thread keeps running and
    // dispatches events through the handler until the tab tears down.
    // The renderer (constructed inside `spawn_local`) already holds
    // an `Arc<Window>` clone made before this handler is built, so
    // the window outlives the loop without the handler needing its
    // own clone.
    let handler = WasmApp {
        renderer: renderer_rc,
        input: input_rc,
        suppress_keys,
        keybinds,
    };
    app.event_loop.spawn_app(handler);
}

/// HTTP-fetch a mindmap JSON file. Maps are bundled into the page
/// origin by trunk's `copy-dir` directive in `web/index.html`.
async fn fetch_map_json(url: &str) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or("no global window")?;
    let promise = window.fetch_with_str(url);
    let resp_value = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("fetch failed: {:?}", e))?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "fetch did not return a Response".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {} {}", resp.status(), resp.status_text()));
    }
    let text_promise = resp
        .text()
        .map_err(|e| format!("Response::text() failed: {:?}", e))?;
    wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|e| format!("reading response body failed: {:?}", e))?
        .as_string()
        .ok_or_else(|| "response body was not a string".to_string())
}

/// Surface a map-load failure as a DOM overlay rather than only
/// `console.log`. The legacy WASM failure mode ("blank canvas
/// when the map is pre-section") leaves users with no visible
/// hint that anything went wrong; this overlay names the file
/// and the loader error so the `maptool convert --sections`
/// migration pointer is copyable. Idempotent — calling twice
/// replaces the existing overlay's text.
fn show_load_error_overlay(map_path: &str, error: &str) {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else { return };
    let Some(document) = window.document() else { return };
    let Some(body) = document.body() else { return };

    const OVERLAY_ID: &str = "mandala-map-load-error";
    let overlay = match document.get_element_by_id(OVERLAY_ID) {
        Some(existing) => existing,
        None => {
            let Ok(div) = document.create_element("div") else { return };
            let _ = div.set_attribute("id", OVERLAY_ID);
            let style = "position:fixed;top:1rem;left:1rem;right:1rem;\
                         z-index:9999;padding:1rem;\
                         background:#1a1a1a;color:#ff6b6b;\
                         border:1px solid #ff6b6b;border-radius:4px;\
                         font-family:monospace;font-size:14px;\
                         white-space:pre-wrap;user-select:text;";
            let _ = div.set_attribute("style", style);
            if body.append_child(&div).is_err() {
                return;
            }
            div
        }
    };
    if let Some(html_elem) = overlay.dyn_ref::<web_sys::HtmlElement>() {
        let text = format!(
            "Failed to load map '{}'.\n\nError: {}\n\n\
             If this is a pre-section map, run:\n\
             maptool convert --sections <path>",
            map_path, error
        );
        html_elem.set_inner_text(&text);
    }
}
