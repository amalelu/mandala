//! Native event-loop body for [`super::Application::run`]. Uses
//! winit 0.30's `ApplicationHandler` trait: the window is created
//! in `resumed()` the first time it fires, and per-event dispatch
//! flows through [`InitState::handle_event`]. First-time
//! initialisation (GPU surface, renderer, mindmap load, scene
//! build, etc.) lives in [`super::run_native_init::build`].

#![cfg(not(target_arch = "wasm32"))]

use super::*;

use super::freeze_watchdog::FreezeWatchdog;
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
    pub(super) mutation_throttle: MutationFrequencyThrottle,
    pub(super) picker_throttle: MutationFrequencyThrottle,
    pub(super) picker_dirty: bool,
    pub(super) keybinds: ResolvedKeybinds,
}

impl InitState {
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
                event_mouse_click::handle_mouse_input(
                    state,
                    button,
                    self.cursor_pos,
                    &self.modifiers,
                    &mut self.document,
                    &mut self.mindmap_tree,
                    &mut self.app_scene,
                    &mut self.renderer,
                    &mut self.scene_cache,
                    &mut self.drag_state,
                    &mut self.app_mode,
                    &mut self.console_state,
                    &self.console_history,
                    &mut self.label_edit_state,
                    &mut self.text_edit_state,
                    &mut self.color_picker_state,
                    &mut self.last_click,
                    &mut self.hovered_node,
                    &mut self.mutation_throttle,
                    &mut self.picker_dirty,
                );
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
                event_cursor_moved::handle_cursor_moved(
                    position,
                    &self.modifiers,
                    self.window.as_ref(),
                    &mut self.document,
                    &mut self.mindmap_tree,
                    &mut self.app_scene,
                    &mut self.renderer,
                    &mut self.scene_cache,
                    &mut self.cursor_pos,
                    &mut self.drag_state,
                    &mut self.app_mode,
                    &mut self.color_picker_state,
                    &mut self.hovered_node,
                    &mut self.cursor_is_hand,
                    &mut self.picker_dirty,
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
                    &self.modifiers,
                    self.cursor_pos,
                    &mut self.document,
                    &mut self.mindmap_tree,
                    &mut self.app_scene,
                    &mut self.renderer,
                    &mut self.scene_cache,
                    &mut self.app_mode,
                    &mut self.console_state,
                    &mut self.console_history,
                    &mut self.label_edit_state,
                    &mut self.portal_text_edit_state,
                    &mut self.text_edit_state,
                    &mut self.color_picker_state,
                    &mut self.last_click,
                    &mut self.hovered_node,
                    &mut self.picker_dirty,
                    &mut self.keybinds,
                    event_loop,
                );
            }
            Event::AboutToWait => self.drain_frame(),
            _ => {}
        }
    }

    /// Per-frame drain: apply pending drag deltas, refresh
    /// camera-dependent geometry, tick active color-picker
    /// throttles and animations, then drive one render frame.
    fn drain_frame(&mut self) {
        if let DragState::MovingNode {
            ref node_ids,
            ref mut pending_delta,
            ref total_delta,
            individual,
            ..
        } = self.drag_state
        {
            drain_frame::drain_moving_node(
                node_ids,
                pending_delta,
                total_delta,
                individual,
                &mut self.mindmap_tree,
                &self.document,
                &mut self.app_scene,
                &mut self.renderer,
                &mut self.scene_cache,
                &mut self.mutation_throttle,
            );
        }
        if let DragState::DraggingEdgeHandle {
            ref edge_ref,
            ref mut handle,
            ref mut pending_delta,
            ref total_delta,
            ref start_handle_pos,
            ..
        } = self.drag_state
        {
            drain_frame::drain_edge_handle(
                edge_ref,
                handle,
                pending_delta,
                total_delta,
                start_handle_pos,
                &mut self.document,
                &mut self.app_scene,
                &mut self.renderer,
                &mut self.scene_cache,
                &mut self.mutation_throttle,
            );
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
                &mut self.mindmap_tree,
                &mut self.renderer,
            );
        }

        drain_frame::drain_camera_geometry_rebuild(
            matches!(self.drag_state, DragState::MovingNode { .. }),
            &self.document,
            &mut self.app_scene,
            &mut self.renderer,
            &mut self.scene_cache,
        );

        drain_frame::drain_color_picker_hover(
            &mut self.picker_dirty,
            &mut self.picker_throttle,
            &mut self.color_picker_state,
            &mut self.document,
            &mut self.app_scene,
            &mut self.renderer,
        );

        drain_frame::drain_animation_tick(
            &mut self.document,
            &mut self.mindmap_tree,
            &mut self.app_scene,
            &mut self.renderer,
        );

        // Drive the render loop each frame
        self.renderer.process();
    }
}
