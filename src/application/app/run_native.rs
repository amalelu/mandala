// SPDX-License-Identifier: MPL-2.0

//! Native event-loop body for [`super::Application::run`]. Uses
//! winit 0.30's `ApplicationHandler`; first-time init lives in
//! [`super::run_native_init::build`].

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use baumhard::mindmap::tree_builder::MindMapTree;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Event, KeyEvent, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window, WindowId};

use super::color_picker_flow::rebuild_color_picker_overlay;
use super::freeze_watchdog::FreezeWatchdog;
use super::input_context::InputHandlerContext;
use super::run_native_init;
use super::single_line_edit::SingleLineEditor;
use super::text_edit::TextEditState;
use super::{
    drain_frame, event_cursor_moved, event_keyboard, event_mouse_click, Application, DragState,
    InteractionMode, LastClick, Options,
};
use crate::application::common::RenderDecree;
use crate::application::console::ConsoleState;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;

/// Wall-clock interval the event loop holds the FPS overlay's
/// numeric reading on screen after the last live frame before
/// flipping it to the "-" idle marker. Without this grace, an
/// active throttled drag's between-drain gaps (where
/// `needs_continuation` is briefly false while the throttle holds
/// the next drain) would flicker the overlay between the live
/// reading and "-" several times per second, defeating the
/// readout's diagnostic value. One second is long enough to read
/// the number on every drain frame yet short enough that genuine
/// idle visibly transitions to "-".
const FPS_IDLE_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

/// Entry point called from `Application::run` on every non-WASM
/// target. Hands control to winit's event loop; returns when the
/// window is closed.
///
/// Spawns the freeze watchdog before handing off to winit so it
/// can catch a hang anywhere after the window is created, not just
/// inside `drain_inputs`. See
/// [`super::freeze_watchdog::FreezeWatchdog`] for the rationale —
/// short version: Mandala is single-threaded and a same-thread
/// `std::sync::RwLock` re-entry deadlock would otherwise hang
/// silently forever.
pub(super) fn run(app: Application) {
    let event_loop = EventLoop::new().expect("Could not create an EventLoop");
    let mut handler = NativeApp {
        options: app.into_options(),
        init: None,
        watchdog: FreezeWatchdog::spawn(),
    };
    event_loop
        .run_app(&mut handler)
        .expect("Some kind of unexpected error appears to have taken place");
}

/// winit 0.30 `ApplicationHandler` implementor. Holds options
/// pre-resume; on the first `resumed()` it creates the window and
/// builds the fully-initialized [`InitState`]. Subsequent resume
/// callbacks (mobile resume-after-suspend) are idempotent thanks
/// to the `is_some()` guard.
struct NativeApp {
    options: Options,
    init: Option<InitState>,
    /// Freeze watchdog — ticked at the top of every `AboutToWait`
    /// drain and also on every window event, so a frame that
    /// hangs mid-drain or mid-event produces a diagnostic abort
    /// after `freeze_watchdog::FREEZE_THRESHOLD`.
    watchdog: FreezeWatchdog,
}

impl ApplicationHandler for NativeApp {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _: StartCause) {
        // ControlFlow is set authoritatively at the end of every
        // `about_to_wait` based on `InitState::needs_continuation`.
        // No-op here so we don't override that decision on each
        // iteration.
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.init.is_some() {
            return;
        }
        let window = event_loop
            .create_window(Window::default_attributes())
            .expect("Failed to create application window");
        self.init = Some(run_native_init::build(&self.options, Arc::new(window)));
        // Ping once as soon as the window is up so the watchdog
        // knows the main loop has reached a live state. Before
        // this point, the watchdog treats the zeroed atomic as
        // "still initializing" and doesn't enforce the threshold.
        // `unparked` also clears any stale parked state from a
        // prior suspend cycle.
        self.watchdog.unparked();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        // The watchdog's "is the main thread alive" semantics are
        // event-handler-completion-based under ControlFlow::Wait:
        // legitimate idle (no events) is not a hang, only a stuck
        // handler is. `unparked` ticks the activity clock and
        // clears the parked atomic so a hang inside this handler
        // still trips the threshold.
        self.watchdog.unparked();
        if let Some(init) = self.init.as_mut() {
            init.handle_event(event_loop, Event::WindowEvent { window_id, event });
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.watchdog.unparked();
        if let Some(init) = self.init.as_mut() {
            // Pre-snapshot the continuation predicate: if the loop
            // was *entering* this iteration with continuous work
            // (animations, throttled-drag pending, mid-zoom), the
            // drain will do work that needs to land on screen.
            let was_continuing = init.needs_continuation();
            init.drain_inputs();
            let still_continuing = init.needs_continuation();
            // Request a redraw if work happened this iteration
            // (`was_continuing`) OR if more work is pending
            // (`still_continuing`). The OR catches the last
            // animation frame: pre=true, post=false, still need
            // one redraw to land the final state.
            if was_continuing || still_continuing {
                init.window.request_redraw();
            }
            // Active → idle FPS transition: when the loop is about
            // to park and the overlay still shows a live numeric
            // reading, eventually flip it to the idle marker so the
            // overlay reads "FPS: -" while the app sleeps. Defer
            // the flip by `FPS_IDLE_GRACE` so brief between-drain
            // gaps during an active throttled drag don't flicker
            // the overlay between numeric and "-" — the user needs
            // a stable readout to actually use the diagnostic.
            //
            // `fps_idle_defer_deadline` returns `Some(when)` if the
            // last frame is younger than the grace; in that case
            // we hold the loop in `ControlFlow::WaitUntil(when)` so
            // it wakes at the deadline to commit the flip if no
            // events arrived in the meantime. Otherwise the grace
            // has already elapsed (or the FPS is already idle), so
            // we either commit the flip now or do nothing.
            let fps_on = init.renderer.fps_display_mode() != crate::application::common::FpsDisplayMode::Off;
            let fps_defer_deadline = if !still_continuing && fps_on {
                init.renderer.fps_idle_defer_deadline(FPS_IDLE_GRACE)
            } else {
                None
            };
            if !still_continuing && fps_on && fps_defer_deadline.is_none() && init.renderer.has_live_fps() {
                init.renderer.set_fps_idle();
                init.window.request_redraw();
            }
            // ControlFlow::Poll keeps the loop ticking so the next
            // about_to_wait drains again; ControlFlow::WaitUntil
            // parks until either the FPS-idle grace elapses or an
            // event arrives, whichever comes first; ControlFlow::Wait
            // parks the thread until an OS event arrives. The
            // choice is strictly post-drain — pre-drain state may
            // have been continuing while post-drain state is fully
            // idle.
            event_loop.set_control_flow(if still_continuing {
                ControlFlow::Poll
            } else if let Some(deadline) = fps_defer_deadline {
                ControlFlow::WaitUntil(deadline)
            } else {
                ControlFlow::Wait
            });
        }
        // Mark parked AFTER setting ControlFlow so the watchdog
        // begins ignoring silence only once we've committed to
        // sleeping. If ControlFlow::Poll, the next about_to_wait
        // will call `unparked` again immediately.
        self.watchdog.parked();
    }
}

/// All the state that was previously owned by the move-closure
/// body of the native event loop. Constructed in
/// [`NativeApp::resumed`] via
/// [`super::run_native_init::build`] once the window exists;
/// then [`Self::handle_event`] runs the original per-event match
/// body against these fields (via `self.X` for each access).
pub(super) struct InitState {
    pub(super) window: Arc<Window>,
    pub(super) renderer: Renderer,
    pub(super) document: Option<MindMapDocument>,
    pub(super) mindmap_tree: Option<MindMapTree>,
    pub(super) scene_cache: baumhard::mindmap::scene_cache::SceneConnectionCache,
    pub(super) app_scene: crate::application::scene_host::AppScene,
    pub(super) cursor_pos: (f64, f64),
    pub(super) drag_state: DragState,
    pub(super) interaction_mode: InteractionMode,
    pub(super) console_state: ConsoleState,
    pub(super) console_history: Vec<String>,
    pub(super) single_line_edit_state: SingleLineEditor,
    pub(super) text_edit_state: TextEditState,
    pub(super) color_picker_state: crate::application::color_picker::ColorPickerState,
    pub(super) last_click: Option<LastClick>,
    pub(super) hovered_node: Option<String>,
    pub(super) modifiers: ModifiersState,
    pub(super) cursor_is_hand: bool,
    /// Last cursor icon written via `Window::set_cursor`. Used by
    /// the cursor_moved handler to dedup redundant `set_cursor`
    /// calls — winit dedupes these on macOS / X11 but NOT on
    /// Windows (every call → `LoadCursorW` + `SetCursor` + mutex
    /// lock) or Wayland (calls into pointer manager every time),
    /// so the per-event cursor icon update needs an
    /// application-side gate. Initialized to `Default` to match
    /// the as-launched cursor.
    pub(super) cursor_icon_last: CursorIcon,
    /// Throttled, coexistent-with-drag color-picker hover.
    /// Continues to update independently of the active drag
    /// variant (if any), hence a sibling field rather than a
    /// `ThrottledDrag` variant.
    pub(super) picker_hover: super::throttled_interaction::ColorPickerHoverInteraction,
    pub(super) keybinds: ResolvedKeybinds,
    /// User-defined macro registry. Loaded once at startup
    /// (`run_native_init::build`) from `~/.config/mandala/macros.json`;
    /// queried at dispatch time via `keybinds.macro_for(...)`.
    pub(super) macros: crate::application::macros::MacroRegistry,
    /// Touch gesture state machine. Fed by `WindowEvent::Touch`
    /// events (winit), emits `MouseGesture::LongPress` /
    /// `TwoFingerDrag` when one of the supported gestures fires.
    /// Cross-platform peer of WASM's `WasmInputState.touch_recognizer`.
    /// See `SECTIONS_BORDERS_RESIZE_PLAN.md` §6.6 for the gesture
    /// vocabulary and `app/touch_gesture.rs` for the state machine.
    pub(super) touch_recognizer: super::touch_gesture::TouchGestureRecognizer,
    /// Wall-clock at which the current tree-mutating drag began
    /// suppressing the animation tick — `Some(now_ms)` while a
    /// `MovingNode` / `MovingSection` / `SectionResize` /
    /// `NodeResize` drag is in flight, `None` otherwise. On
    /// release we shift each active animation's `start_ms`
    /// forward by the drag duration so the post-release tick
    /// resumes the animation where the drag froze it, instead
    /// of snapping to its `to` state on the first post-release
    /// frame (the `tick_animations` body computes `elapsed = now
    /// - start_ms`, and without the shift the wall-clock-elapsed
    /// during the drag would land past `total`).
    pub(super) anim_pause_start_ms: Option<u64>,
}

impl InitState {
    /// Build the [`InputHandlerContext`] view over this state for a
    /// single dispatcher call. Rebuilt per event because the
    /// returned borrow is tied to `&mut self` — `'_` expires as
    /// soon as the handler returns.
    pub(super) fn input_context(&mut self) -> InputHandlerContext<'_> {
        InputHandlerContext {
            document: &mut self.document,
            mindmap_tree: &mut self.mindmap_tree,
            app_scene: &mut self.app_scene,
            renderer: &mut self.renderer,
            scene_cache: &mut self.scene_cache,
            drag_state: &mut self.drag_state,
            interaction_mode: &mut self.interaction_mode,
            console_state: &mut self.console_state,
            console_history: &mut self.console_history,
            single_line_edit_state: &mut self.single_line_edit_state,
            text_edit_state: &mut self.text_edit_state,
            color_picker_state: &mut self.color_picker_state,
            last_click: &mut self.last_click,
            hovered_node: &mut self.hovered_node,
            cursor_pos: &mut self.cursor_pos,
            modifiers: &self.modifiers,
            cursor_is_hand: &mut self.cursor_is_hand,
            cursor_icon_last: &mut self.cursor_icon_last,
            picker_hover: &mut self.picker_hover,
            keybinds: &self.keybinds,
            macros: &mut self.macros,
        }
    }

    /// Translate a winit `Touch` event into a recognizer ingest +
    /// tick + dispatch step. Returns true when the event drove
    /// any state transition or dispatched a gesture (the caller
    /// should request a redraw on true). Modifier state is fixed
    /// at all-false — touch devices have no modifier keys; the
    /// keybind table's `LongPress` / `TwoFingerDrag` bindings
    /// don't carry Ctrl/Shift/Alt either.
    ///
    /// **Long-press timing wake-up gap**: `tick` is called only
    /// from the events themselves (not on a wall-clock timer),
    /// so a finger held with literally zero `Moved` events
    /// between Started and Ended would miss the long-press
    /// emission. In practice touch hardware emits sub-pixel
    /// jitter `Moved` events constantly while a finger is down,
    /// so the gap is theoretical. A future improvement would set
    /// `ControlFlow::WaitUntil(started_at + LONG_PRESS_MS)` from
    /// the recognizer's `OneFinger` state — deferred to keep
    /// Batch 7's diff small.
    pub(super) fn dispatch_touch_event(&mut self, touch: winit::event::Touch) -> bool {
        use super::touch_gesture::Phase;
        use web_time::Instant;
        // Phase translation, recognizer ingest + tick, and the
        // gesture-to-Action lookup are `cross_dispatch::pointer`'s
        // `drive_touch_event`; the browser runs the same body. Only
        // the dispatch below is native's own — it goes through
        // `dispatch_action` so `NativeOnly` gesture Actions
        // (`EnterResizeMode`, `FastResizeStart`) reach their arms.
        let phase = super::dispatch::touch_phase(touch.phase);
        let pos = (touch.location.x, touch.location.y);
        if let Some(d) = super::dispatch::drive_touch_event(
            &mut self.touch_recognizer,
            &self.keybinds,
            phase,
            touch.id,
            pos,
            Instant::now(),
        ) {
            self.cursor_pos = d.cursor_pos;
            if let Some(a) = d.action {
                let mut ctx = self.input_context();
                let _ = super::dispatch::dispatch_action(a, &mut ctx, None);
                return true;
            }
        }
        // No gesture recognized — but the recognizer may have
        // moved its internal state (e.g. Started → OneFinger).
        // The caller still wants a redraw on Started/Moved so
        // any cursor-following overlay (long-press preview,
        // future gesture chrome) updates.
        matches!(phase, Phase::Started | Phase::Moved)
    }

    /// Per-event dispatch. Most of the per-event work lives in
    /// [`super::event_mouse_click`], [`super::event_cursor_moved`],
    /// and [`super::event_keyboard`]; this method handles the
    /// smaller arms (resize, close, wheel, modifiers) inline and
    /// delegates the larger ones.
    ///
    /// Mutating arms set `redraw_after = true` so the trailing
    /// [`winit::window::Window::request_redraw`] call queues one
    /// `WindowEvent::RedrawRequested` for the next event-loop
    /// iteration. The `RedrawRequested` arm itself is the only
    /// place [`crate::application::renderer::Renderer::process`]
    /// runs — separating "decide to redraw" from "actually
    /// render" lets winit coalesce multiple `request_redraw`
    /// calls in one event chain into a single render.
    pub(super) fn handle_event(&mut self, event_loop: &ActiveEventLoop, event: winit::event::Event<()>) {
        let mut redraw_after = false;
        match event {
            //// WINDOW SPECIFIC ////
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                self.renderer
                    .process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));
                // Glyph-wheel color picker caches its layout in
                // ColorPickerState::Open { layout, .. }; the
                // cached values include the screen-space backdrop
                // and per-glyph positions, so a resize would
                // leave hit-tests aimed at the old geometry and
                // the renderer's overlay buffers anchored at the
                // pre-resize coordinates.
                if self.color_picker_state.is_open() {
                    if let Some(doc) = self.document.as_ref() {
                        rebuild_color_picker_overlay(
                            &mut self.color_picker_state,
                            doc,
                            &mut self.app_scene,
                            &mut self.renderer,
                        );
                    }
                }
                redraw_after = true;
            }
            Event::WindowEvent {
                event: WindowEvent::RedrawRequested,
                ..
            } => {
                // Sole entry to the render path. Redraws are queued
                // via `Window::request_redraw` from event handlers
                // (mutations) and from `about_to_wait` (drain
                // continuations); winit delivers exactly one
                // `RedrawRequested` per coalesced batch.
                self.renderer.process();
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                self.renderer.process_decree(RenderDecree::Terminate);
                event_loop.exit();
            }
            //// MOUSE ////
            Event::WindowEvent {
                event: WindowEvent::MouseInput { state, button, .. },
                ..
            } => {
                let mut ctx = self.input_context();
                event_mouse_click::handle_mouse_input(state, button, &mut ctx);
                redraw_after = true;
            }
            Event::WindowEvent {
                event: WindowEvent::MouseWheel { delta, .. },
                ..
            } => {
                let scroll_y = crate::application::app::wheel_lines(delta);
                // While the console is open, the wheel scrolls the
                // scrollback rather than zooming the canvas — mouse
                // events should follow keyboard focus. Fractional
                // deltas accumulate via `accumulate_wheel_lines` so
                // sub-line-per-tick scrolls don't round to zero.
                if self.console_state.is_open() {
                    let lines =
                        if let crate::application::console::ConsoleState::Open { wheel_accum, .. } =
                            &mut self.console_state
                        {
                            crate::application::app::console_input::accumulate_wheel_lines(
                                wheel_accum,
                                scroll_y as f32,
                            )
                        } else {
                            0
                        };
                    if lines != 0 {
                        crate::application::app::console_input::scroll_console_by_lines(
                            &mut self.console_state,
                            lines,
                        );
                        if let Some(doc) = self.document.as_ref() {
                            crate::application::app::console_input::rebuild_console_overlay(
                                &self.console_state,
                                doc,
                                &mut self.app_scene,
                                &mut self.renderer,
                                &self.keybinds,
                            );
                        }
                        redraw_after = true;
                    }
                } else {
                    // Wheel zoom is routed through `dispatch_action` so
                    // users can rebind `WheelUp` / `WheelDown` to any
                    // Action (or unbind them entirely). Defaults bind
                    // both to `ZoomIn` / `ZoomOut`. If the user
                    // explicitly clears the bindings, wheel events are
                    // silently ignored.
                    let gesture_name = crate::application::app::wheel_gesture(scroll_y).key_name();
                    // `action_for_gesture` falls back to the unmodified
                    // binding when no exact-modifier match exists, so
                    // `Ctrl+Wheel` keeps zooming even though only
                    // `WheelUp` / `WheelDown` are bound in defaults —
                    // pre-branch behavior was modifier-agnostic and
                    // we preserve it here without forcing every user
                    // to enumerate modifier permutations.
                    let action = self.keybinds.action_for_gesture(
                        gesture_name,
                        self.modifiers.control_key(),
                        self.modifiers.shift_key(),
                        self.modifiers.alt_key(),
                    );
                    if let Some(a) = action {
                        let mut bundle = self.input_context();
                        let _ = crate::application::app::dispatch::dispatch_action(a, &mut bundle, None);
                        redraw_after = true;
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. },
                ..
            } => {
                let window = self.window.clone();
                let mut ctx = self.input_context();
                event_cursor_moved::handle_cursor_moved(position, window.as_ref(), &mut ctx);
                // Hover that doesn't drag and doesn't open the
                // picker doesn't change visuals — no redraw needed.
                // Any active drag (Pending → Throttled etc.) or an
                // open picker requires a fresh frame so the drag
                // preview / hover marker tracks the cursor.
                redraw_after =
                    !matches!(self.drag_state, DragState::None) || self.color_picker_state.is_open();
            }
            //// TOUCH ////
            Event::WindowEvent {
                event: WindowEvent::Touch(touch),
                ..
            } => {
                if self.dispatch_touch_event(touch) {
                    redraw_after = true;
                }
            }
            //// KEYBOARD ////
            Event::WindowEvent {
                event: WindowEvent::ModifiersChanged(mods),
                ..
            } => {
                self.modifiers = mods.state();
                // Modifier-only changes don't paint anything until
                // a paired mouse/keyboard event acts on them.
            }
            Event::WindowEvent {
                event:
                    WindowEvent::KeyboardInput {
                        event:
                            KeyEvent {
                                logical_key,
                                state: ElementState::Pressed,
                                ..
                            },
                        ..
                    },
                ..
            } => {
                let mut ctx = self.input_context();
                event_keyboard::handle_keyboard_input(logical_key, event_loop, &mut ctx);
                redraw_after = true;
            }
            _ => {}
        }
        if redraw_after {
            self.window.request_redraw();
        }
    }

    /// Per-frame drain: drive the active throttled drag (if any)
    /// and the always-live picker-hover interaction through the
    /// unified [`super::throttled_interaction::ThrottledInteraction::drive`]
    /// shell, then the non-throttled drains (rect-select overlay,
    /// camera rebuild, animation tick). Does **not** render — the
    /// caller (`NativeApp::about_to_wait`) decides whether to
    /// `request_redraw` based on the post-drain
    /// [`Self::needs_continuation`] state, and the actual `render`
    /// runs from the `WindowEvent::RedrawRequested` arm.
    pub(super) fn drain_inputs(&mut self) {
        use super::throttled_interaction::{DrainContext, ThrottledInteraction};

        // Only the moving-node drag needs to suppress the camera
        // rebuild (it handles offset geometry itself each drain).
        // Snapshot this before the drive() borrow takes `&mut
        // self.drag_state`. Suppresses the camera-driven
        // geometry rebuild while a node is being moved (the
        // drag's own per-frame mutator already keeps the scene
        // current; the camera rebuild would interleave with
        // stale model-side state). `MovingSection` /
        // `SectionResize` / `NodeResize` deliberately don't
        // qualify — sections don't move the parent, and node
        // resize gestures' tree mutations are re-applied by
        // the next drain after any competing rebuild.
        let is_moving_node = matches!(
            self.drag_state,
            DragState::Throttled(ref d)
                if matches!(**d, super::throttled_interaction::ThrottledDrag::MovingNode(_))
        );

        // Suppress the animation tick during drags that mutate
        // the tree per-frame (`MovingNode`, `MovingSection`,
        // `SectionResize`, `NodeResize`). An animation tick on
        // the same frame routes through `sync_node_from_tree`,
        // which would observe the in-progress mid-drag state
        // and write it to the model + undo stack. Edge /
        // portal-label drags don't qualify — they touch the
        // document, not the tree.
        let is_drag_with_tree_mutation = matches!(
            self.drag_state,
            DragState::Throttled(ref d)
                if matches!(
                    **d,
                    super::throttled_interaction::ThrottledDrag::MovingNode(_)
                        | super::throttled_interaction::ThrottledDrag::MovingSection(_)
                        | super::throttled_interaction::ThrottledDrag::SectionResize(_)
                        | super::throttled_interaction::ThrottledDrag::NodeResize(_)
                )
        );

        // Destructure the fields the two throttled-drive call sites
        // share so their `DrainContext` literals can reborrow via
        // `&mut *x` instead of re-spelling `&mut self.X` six times
        // twice. A named inherent helper (`&mut self -> DrainContext`)
        // collides with the `&mut self.drag_state` the throttled-drag
        // arm already holds; a closure over these bindings collides
        // with the second call site's reborrows. Destructuring once,
        // reborrowing per call, is what the borrow checker accepts.
        let Self {
            document,
            mindmap_tree,
            app_scene,
            renderer,
            scene_cache,
            color_picker_state,
            drag_state,
            picker_hover,
            interaction_mode,
            ..
        } = self;

        if let DragState::Throttled(ref mut kind) = *drag_state {
            kind.as_dyn_mut().drive(DrainContext {
                document: &mut *document,
                mindmap_tree: &mut *mindmap_tree,
                app_scene: &mut *app_scene,
                renderer: &mut *renderer,
                scene_cache: &mut *scene_cache,
                color_picker_state: &mut *color_picker_state,
                interaction_mode: &*interaction_mode,
            });
        }

        if color_picker_state.is_open() {
            picker_hover.drive(DrainContext {
                document: &mut *document,
                mindmap_tree: &mut *mindmap_tree,
                app_scene: &mut *app_scene,
                renderer: &mut *renderer,
                scene_cache: &mut *scene_cache,
                color_picker_state: &mut *color_picker_state,
                interaction_mode: &*interaction_mode,
            });
        } else if picker_hover.has_pending() {
            // Picker closed while a throttle-deferred hover drain
            // was still queued. The drain body's closed-picker
            // branch (`color_picker_hover.rs:88-92`) is a no-op
            // rebuild that just clears both flags — going through
            // the throttle would still strand them for the throttle
            // window's worth of frames, during which
            // `needs_continuation` reads `has_pending=true` and
            // pins the loop in `ControlFlow::Poll`. Two boolean
            // writes, no GPU work; bypass the throttle and clear
            // immediately.
            picker_hover.clear_pending();
        }

        if let DragState::SelectingRect {
            start_canvas,
            current_canvas,
        } = &self.drag_state
        {
            drain_frame::drain_selecting_rect(
                *start_canvas,
                *current_canvas,
                &self.document,
                &self.interaction_mode,
                &mut self.mindmap_tree,
                &mut self.renderer,
            );
        }

        drain_frame::drain_camera_geometry_rebuild(
            is_moving_node,
            &self.document,
            &self.interaction_mode,
            &mut self.app_scene,
            &mut self.renderer,
            &mut self.scene_cache,
        );

        // Animation pause/resume — when entering a tree-mutating
        // drag, capture wall-clock; when exiting, shift each
        // active animation's `start_ms` forward by the drag
        // duration so the post-release tick resumes the
        // animation where it froze instead of snapping to its
        // `to` state. Without this, a multi-second drag would
        // leave an in-flight animation observing
        // `elapsed >= total` on the first post-release frame.
        let now = super::now_ms() as u64;
        match (is_drag_with_tree_mutation, self.anim_pause_start_ms) {
            (true, None) => {
                self.anim_pause_start_ms = Some(now);
            }
            (false, Some(pause_start)) => {
                let pause_duration = now.saturating_sub(pause_start);
                if let Some(doc) = self.document.as_mut() {
                    doc.shift_active_animations_start_ms(pause_duration);
                }
                self.anim_pause_start_ms = None;
            }
            _ => {}
        }

        if !is_drag_with_tree_mutation {
            drain_frame::drain_animation_tick(
                &mut self.document,
                &self.interaction_mode,
                &mut self.mindmap_tree,
                &mut self.app_scene,
                &mut self.renderer,
                &mut self.scene_cache,
            );
        }
    }

    /// True iff the event loop should keep iterating without
    /// further input. Drives the choice between `ControlFlow::Wait`
    /// (idle, park until next OS event) and `ControlFlow::Poll`
    /// (continuous frames). Sources of continuation:
    ///
    /// - **Throttled drag with pending state.** The throttle may
    ///   have deferred a drain; without continuation the loop would
    ///   park indefinitely after the last cursor event, leaving
    ///   pending input unflushed.
    /// - **Picker hover with pending state.** Same shape — the
    ///   hover interaction owns its own pending buffer.
    /// - **Active animation.** The animation tick must keep
    ///   advancing until each animation completes.
    /// - **Connection-geometry dirty flag set.** A camera change
    ///   landed during a `MovingNode` drag that suppressed the
    ///   rebuild; the flag will be picked up by the next drain
    ///   that doesn't suppress it.
    ///
    /// `None` / `Pending` / `Panning` / `SelectingRect` drag states
    /// are intentionally event-driven only — they update on
    /// `CursorMoved` and don't need self-driven continuation. The
    /// FPS overlay is also intentionally NOT a continuation source:
    /// a diagnostic should observe behavior, not change it. When
    /// the app idles, the overlay flips to "-" (see
    /// [`crate::application::renderer::Renderer::set_fps_idle`])
    /// rather than forcing the loop to keep rendering.
    pub(super) fn needs_continuation(&self) -> bool {
        use crate::application::app::throttled_interaction::ThrottledInteraction;

        let drag_pending = matches!(
            self.drag_state,
            DragState::Throttled(ref d) if d.as_dyn().needs_continuation()
        );
        let picker_pending = self.picker_hover.has_pending();
        let animations = self.document.as_ref().is_some_and(|d| d.has_active_animations());
        let geometry_dirty = self.renderer.connection_geometry_dirty();

        drag_pending || picker_pending || animations || geometry_dirty
    }
}
