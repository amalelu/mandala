//! Native event-loop body for [`super::Application::run`]. Uses
//! winit 0.30's `ApplicationHandler` trait: the window is created
//! in `resumed()` the first time it fires, and per-event dispatch
//! flows through [`InitState::handle_event`]. First-time
//! initialisation (GPU surface, renderer, mindmap load, scene
//! build, etc.) lives in [`super::run_native_init::build`].

#![cfg(not(target_arch = "wasm32"))]

use super::*;

use super::freeze_watchdog::FreezeWatchdog;
use super::input_context::InputHandlerContext;
use baumhard::mindmap::tree_builder::MindMapTree;
use winit::application::ApplicationHandler;
use winit::event::StartCause;
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

/// Entry point called from `Application::run` on every non-WASM
/// target. Hands control to winit's event loop; returns when the
/// window is closed.
///
/// Spawns the freeze watchdog before handing off to winit so it
/// can catch a hang anywhere after the window is created, not just
/// inside `drain_frame`. See
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
/// builds the fully-initialised [`InitState`]. Subsequent resume
/// callbacks (mobile resume-after-suspend) are idempotent thanks
/// to the `is_some()` guard.
struct NativeApp {
    options: Options,
    init: Option<InitState>,
    /// Freeze watchdog — ticked at the top of every `AboutToWait`
    /// drain and also on every window event, so a frame that
    /// hangs mid-drain or mid-event produces a diagnostic abort
    /// after [`super::freeze_watchdog::FREEZE_THRESHOLD`].
    watchdog: FreezeWatchdog,
}

impl ApplicationHandler for NativeApp {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, _: StartCause) {
        event_loop.set_control_flow(ControlFlow::Poll);
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
        self.watchdog.tick();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.watchdog.tick();
        if let Some(init) = self.init.as_mut() {
            init.handle_event(event_loop, Event::WindowEvent { window_id, event });
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.watchdog.tick();
        if let Some(init) = self.init.as_mut() {
            init.handle_event(event_loop, Event::AboutToWait);
        }
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
    pub(super) app_mode: AppMode,
    pub(super) console_state: ConsoleState,
    pub(super) console_history: Vec<String>,
    pub(super) label_edit_state: LabelEditState,
    pub(super) portal_text_edit_state: PortalTextEditState,
    pub(super) text_edit_state: TextEditState,
    pub(super) color_picker_state: crate::application::color_picker::ColorPickerState,
    pub(super) last_click: Option<LastClick>,
    pub(super) hovered_node: Option<String>,
    pub(super) modifiers: ModifiersState,
    pub(super) cursor_is_hand: bool,
    /// Throttled, coexistent-with-drag color-picker hover.
    /// Continues to update independently of the active drag
    /// variant (if any), hence a sibling field rather than a
    /// `ThrottledDrag` variant.
    pub(super) picker_hover: super::throttled_interaction::ColorPickerHoverInteraction,
    pub(super) keybinds: ResolvedKeybinds,
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
            app_mode: &mut self.app_mode,
            console_state: &mut self.console_state,
            console_history: &mut self.console_history,
            label_edit_state: &mut self.label_edit_state,
            portal_text_edit_state: &mut self.portal_text_edit_state,
            text_edit_state: &mut self.text_edit_state,
            color_picker_state: &mut self.color_picker_state,
            last_click: &mut self.last_click,
            hovered_node: &mut self.hovered_node,
            cursor_pos: &mut self.cursor_pos,
            modifiers: &self.modifiers,
            cursor_is_hand: &mut self.cursor_is_hand,
            picker_hover: &mut self.picker_hover,
            keybinds: &mut self.keybinds,
        }
    }

    /// Per-event dispatch. Most of the per-event work lives in
    /// [`super::event_mouse_click`], [`super::event_cursor_moved`],
    /// and [`super::event_keyboard`]; this method handles the
    /// smaller arms (resize, close, wheel, modifiers) inline and
    /// delegates the larger ones.
    pub(super) fn handle_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: winit::event::Event<()>,
    ) {
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
                event_mouse_click::handle_mouse_input(state, button, self.input_context());
            }
            Event::WindowEvent {
                event: WindowEvent::MouseWheel { delta, .. },
                ..
            } => {
                let scroll_y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
                };
                let factor = if scroll_y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                self.renderer.process_decree(RenderDecree::CameraZoom {
                    screen_x: self.cursor_pos.0 as f32,
                    screen_y: self.cursor_pos.1 as f32,
                    factor: factor as f32,
                });
            }
            Event::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. },
                ..
            } => {
                let window = self.window.clone();
                event_cursor_moved::handle_cursor_moved(
                    position,
                    window.as_ref(),
                    self.input_context(),
                );
            }
            //// KEYBOARD ////
            Event::WindowEvent {
                event: WindowEvent::ModifiersChanged(mods),
                ..
            } => {
                self.modifiers = mods.state();
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
                event_keyboard::handle_keyboard_input(
                    logical_key,
                    event_loop,
                    self.input_context(),
                );
            }
            Event::AboutToWait => self.drain_frame(),
            _ => {}
        }
    }

    /// Per-frame drain: drive the active throttled drag (if any)
    /// and the always-live picker-hover interaction through the
    /// unified [`ThrottledInteraction::drive`] shell, then the
    /// non-throttled drains (rect-select overlay, camera
    /// rebuild, animation tick), then one render frame.
    fn drain_frame(&mut self) {
        use super::throttled_interaction::{DrainContext, ThrottledInteraction};

        // Only the moving-node drag needs to suppress the camera
        // rebuild (it handles offset geometry itself each drain).
        // Snapshot this before the drive() borrow takes `&mut
        // self.drag_state`.
        let is_moving_node = matches!(
            self.drag_state,
            DragState::Throttled(super::throttled_interaction::ThrottledDrag::MovingNode(_)),
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
            });
        }

        picker_hover.drive(DrainContext {
            document: &mut *document,
            mindmap_tree: &mut *mindmap_tree,
            app_scene: &mut *app_scene,
            renderer: &mut *renderer,
            scene_cache: &mut *scene_cache,
            color_picker_state: &mut *color_picker_state,
        });

        if let DragState::SelectingRect {
            start_canvas,
            current_canvas,
        } = &self.drag_state
        {
            drain_frame::drain_selecting_rect(
                *start_canvas,
                *current_canvas,
                &self.document,
                &mut self.mindmap_tree,
                &mut self.renderer,
            );
        }

        drain_frame::drain_camera_geometry_rebuild(
            is_moving_node,
            &self.document,
            &mut self.app_scene,
            &mut self.renderer,
            &mut self.scene_cache,
        );

        drain_frame::drain_animation_tick(
            &mut self.document,
            &mut self.mindmap_tree,
            &mut self.app_scene,
            &mut self.renderer,
            &mut self.scene_cache,
        );

        // Drive the render loop each frame
        self.renderer.process();
    }
}
