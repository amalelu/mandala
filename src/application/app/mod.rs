use std::collections::HashMap;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
use pollster::block_on;

#[cfg(not(target_arch = "wasm32"))]
mod color_picker_flow;
#[cfg(not(target_arch = "wasm32"))]
mod console_input;
#[cfg(not(target_arch = "wasm32"))]
mod edge_drag;
#[cfg(not(target_arch = "wasm32"))]
mod label_edit;
#[cfg(not(target_arch = "wasm32"))]
mod run_native;
mod scene_rebuild;
mod text_edit;
#[cfg(not(target_arch = "wasm32"))]
use edge_drag::apply_edge_handle_drag;
use text_edit::{
    close_text_edit, delete_at_cursor, delete_before_cursor, handle_text_edit_key,
    insert_at_cursor, open_text_edit, TextEditState,
};
#[cfg(not(target_arch = "wasm32"))]
use label_edit::{handle_label_edit_key, open_label_edit, LabelEditState};
#[cfg(not(target_arch = "wasm32"))]
use color_picker_flow::{
    end_color_picker_gesture, handle_color_picker_click, handle_color_picker_key,
    handle_color_picker_mouse_move, rebuild_color_picker_overlay,
};
#[cfg(not(target_arch = "wasm32"))]
use console_input::{
    handle_console_key, load_console_history, rebuild_console_overlay, save_console_history,
    save_document_to_bound_path,
};
use scene_rebuild::{
    flush_canvas_scene_buffers, rebuild_all, rebuild_scene_only, update_border_tree_static,
    update_border_tree_with_offsets, update_connection_label_tree, update_connection_tree,
    update_edge_handle_tree, update_portal_tree,
};

/// Cross-platform monotonic clock returning milliseconds since first call.
/// Native: uses `Instant` (guaranteed monotonic). WASM: uses
/// `performance.now()` (monotonic from page load, Spectre-clamped to ≥1ms
/// but fine for our 400ms double-click window).
#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> f64 {
    use std::sync::OnceLock;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}
use glam::Vec2;
use wgpu::{Instance, SurfaceTargetUnsafe};
use winit::event::{ElementState, Event, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ControlFlow;
use winit::keyboard::ModifiersState;
use winit::window::CursorIcon;
use winit::{event_loop::EventLoop, window::Window};

use crate::application::common::{InputMode, RenderDecree, WindowMode};
use crate::application::document::{
    EdgeRef, MindMapDocument, SelectionState, UndoAction,
    hit_test, hit_test_edge, rect_select,
    apply_drag_delta, apply_tree_highlights,
    HIGHLIGHT_COLOR, REPARENT_SOURCE_COLOR, REPARENT_TARGET_COLOR,
};
use crate::application::frame_throttle::MutationFrequencyThrottle;
use crate::application::keybinds::{Action, ResolvedKeybinds};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::console::ConsoleState;
use crate::application::renderer::Renderer;

#[cfg(not(target_arch = "wasm32"))]
use baumhard::mindmap::custom_mutation::{PlatformContext, Trigger};
use baumhard::util::grapheme_chad;

/// Screen-space click tolerance (in pixels) for edge hit testing. Converted
/// to canvas units via `Renderer::canvas_per_pixel()` so the click target
/// stays visually stable across zoom levels.
#[cfg(not(target_arch = "wasm32"))]
const EDGE_HIT_TOLERANCE_PX: f32 = 8.0;

/// Screen-space click tolerance (in pixels) for edge grab-handle hit
/// testing in Session 6C. Slightly larger than the edge-path
/// tolerance above because handles are point-like and need a bit
/// more grab-area to feel forgiving.
#[cfg(not(target_arch = "wasm32"))]
const EDGE_HANDLE_HIT_TOLERANCE_PX: f32 = 12.0;


/// Tracks the previous left-click in screen space so a second click
/// within a short time + distance window is recognized as a
/// double-click. Double-click fires on the second `Pressed` event,
/// not the second release. `time` is `f64` milliseconds from the
/// cross-platform `now_ms()` helper.
#[derive(Debug, Clone)]
struct LastClick {
    time: f64,
    screen_pos: (f64, f64),
    /// Which node, if any, the first click landed on. Two clicks with
    /// the same `hit` (both `Some(id)` for the same id, or both
    /// `None`) qualify as a double-click.
    hit: Option<String>,
}

/// Double-click window in milliseconds. Matches GNOME/winit convention.
const DOUBLE_CLICK_MS: f64 = 400.0;

/// Double-click maximum distance² in screen-space pixels.
const DOUBLE_CLICK_DIST_SQ: f64 = 16.0 * 16.0;


/// Returns `true` when a new click-down qualifies as a double-click
/// given the previous click. Pure helper so cursor/time math can be
/// unit-tested without a winit event loop.
fn is_double_click(
    prev: &LastClick,
    new_time_ms: f64,
    new_screen_pos: (f64, f64),
    new_hit: &Option<String>,
) -> bool {
    let elapsed = new_time_ms - prev.time;
    if elapsed < 0.0 || elapsed >= DOUBLE_CLICK_MS {
        return false;
    }
    let dx = new_screen_pos.0 - prev.screen_pos.0;
    let dy = new_screen_pos.1 - prev.screen_pos.1;
    if dx * dx + dy * dy >= DOUBLE_CLICK_DIST_SQ {
        return false;
    }
    &prev.hit == new_hit
}

/// Pure router for the label-edit key loop. Given the winit
/// key-name lowercased (`"backspace"`, `"arrowleft"`, ...) and
/// the optional character payload from `Key::Character` (IME /
/// dead-key sequences can carry multiple chars), mutates
/// `(buffer, cursor)` in place through the same
/// `grapheme_chad` helpers `handle_text_edit_key` uses and
/// returns `true` iff any state changed. Separated out so the
/// routing logic is testable without standing up a winit event
/// loop; `handle_label_edit_key` is the thin shell that routes
/// scene / renderer plumbing around this.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn route_label_edit_key(
    name: Option<&str>,
    typed: Option<&str>,
    buffer: &mut String,
    cursor: &mut usize,
) -> bool {
    match name {
        Some("backspace") => {
            if *cursor > 0 {
                *cursor = delete_before_cursor(buffer, *cursor);
                return true;
            }
            false
        }
        Some("delete") => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor = delete_at_cursor(buffer, *cursor);
                return true;
            }
            false
        }
        Some("arrowleft") => {
            if *cursor > 0 {
                *cursor -= 1;
                return true;
            }
            false
        }
        Some("arrowright") => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor += 1;
                return true;
            }
            false
        }
        Some("home") => {
            if *cursor != 0 {
                *cursor = 0;
                return true;
            }
            false
        }
        Some("end") => {
            let end = grapheme_chad::count_grapheme_clusters(buffer);
            if *cursor != end {
                *cursor = end;
                return true;
            }
            false
        }
        _ => {
            // Printable character: accept each non-control char.
            // Mirrors `handle_text_edit_key` — winit's
            // `Key::Character` can carry IME / dead-key
            // sequences, so iterate.
            let Some(typed) = typed else {
                return false;
            };
            let mut changed = false;
            for ch in typed.chars() {
                if !ch.is_control() {
                    *cursor = insert_at_cursor(buffer, *cursor, ch);
                    changed = true;
                }
            }
            changed
        }
    }
}


/// Tracks the high-level interaction mode. Normal handles the usual
/// select/drag/pan flow; Reparent mode is entered via Ctrl+P and captures
/// the next left-click as a "choose reparent target" gesture. Connect mode
/// is entered via Ctrl+D and captures the next left-click as a "choose
/// connection target" gesture to create a cross_link edge.
#[cfg(not(target_arch = "wasm32"))]
enum AppMode {
    Normal,
    /// Reparent mode: the user is choosing a new parent for `sources`.
    /// The next left-click on a node attaches all sources as its last children;
    /// a left-click on empty canvas promotes them to root. Esc cancels.
    Reparent { sources: Vec<String> },
    /// Connect mode: the user is drawing a new cross_link edge from `source`.
    /// The next left-click on a target node creates the edge; a left-click
    /// on empty canvas cancels. Esc also cancels.
    Connect { source: String },
}

/// Tracks the current drag interaction state.
#[cfg(not(target_arch = "wasm32"))]
enum DragState {
    /// No drag in progress.
    None,
    /// Mouse is down but hasn't moved past the drag threshold yet.
    Pending {
        start_pos: (f64, f64),
        hit_node: Option<String>,
        /// If an edge was selected at mouse-down time and the cursor
        /// landed on one of that edge's grab-handles, this records
        /// which handle the user is about to drag. Populated in
        /// `MouseInput::Pressed`, consumed at the threshold-cross
        /// transition in `CursorMoved`. Takes precedence over
        /// `hit_node` — clicking a handle always wins over clicking
        /// the node behind it.
        hit_edge_handle: Option<(EdgeRef, baumhard::mindmap::scene_builder::EdgeHandleKind)>,
    },
    /// Dragging to pan the camera (started on empty space).
    Panning,
    /// Dragging node(s) to reposition them.
    MovingNode {
        /// The node IDs being moved. Single node, or all selected nodes (shift+drag).
        node_ids: Vec<String>,
        /// Accumulated total delta in canvas coords (for model sync on drop).
        total_delta: Vec2,
        /// Delta accumulated since last frame, applied in AboutToWait.
        pending_delta: Vec2,
        /// Whether dragging only the individual node(s) (alt+drag) vs subtrees.
        individual: bool,
    },
    /// Dragging a grab-handle on the selected edge to reshape it.
    /// Session 6C: handles come in four kinds (see
    /// `scene_builder::EdgeHandleKind`): two anchor endpoints, any
    /// existing control points, and a midpoint handle on straight
    /// edges that curves them into a quadratic Bezier on first drag.
    DraggingEdgeHandle {
        edge_ref: EdgeRef,
        /// Which handle is being dragged. `Midpoint` is only the
        /// initial kind — after the first drain frame inserts a
        /// fresh control point, this mutates in place to
        /// `ControlPoint(0)` so subsequent frames take the CP path.
        handle: baumhard::mindmap::scene_builder::EdgeHandleKind,
        /// Full snapshot of the edge at drag start, for undo and
        /// for checking whether the drag actually changed anything
        /// on release (single-pixel no-op shouldn't leave undo
        /// droppings).
        original: baumhard::mindmap::model::MindEdge,
        /// Canvas-space position of the handle at drag start. Used
        /// to recompute the new CP position from an absolute cursor
        /// location instead of an accumulated delta, which avoids
        /// drift for non-CP handles.
        start_handle_pos: Vec2,
        /// Accumulated delta since drag start.
        total_delta: Vec2,
        /// Delta accumulated since last frame, applied in AboutToWait.
        pending_delta: Vec2,
    },
    /// Shift+drag on empty space: rubber-band selection rectangle.
    SelectingRect {
        /// Canvas-space corner where the drag started.
        start_canvas: Vec2,
        /// Canvas-space corner at current cursor position.
        current_canvas: Vec2,
    },
}

/**
Represents the root container of the application
Manages the winit window and event_loop, and launches the rendering pipeline
 **/
pub struct Application {
    options: Options,
    event_loop: EventLoop<()>,
    window: Arc<Window>,
}

impl Application {
    pub fn new(options: Options) -> Self {
        let event_loop = EventLoop::new().expect("Could not create an EventLoop");

        let window = event_loop.create_window(Window::default_attributes()).expect("Failed to create application window");

        Application {
            options,
            event_loop,
            window: Arc::new(window),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(self) {
        run_native::run(self)
    }


    #[cfg(target_arch = "wasm32")]
    pub fn run(mut self) {
        use wasm_bindgen::JsCast;
        use winit::platform::web::WindowExtWebSys;
        use std::rc::Rc;
        use std::cell::{Cell, RefCell};
        use baumhard::mindmap::tree_builder::MindMapTree;

        baumhard::font::fonts::init();

        // Load keybindings from the WASM environment (URL query param or
        // localStorage) with a defaults fallback. Failure is non-fatal —
        // see KeybindConfig::load_for_web().
        self.options.keybind_config =
            crate::application::keybinds::KeybindConfig::load_for_web();

        // Attach canvas to DOM
        let canvas = self.window.canvas().expect("Failed to get canvas");
        let web_window = web_sys::window().expect("No global window");
        let document = web_window.document().expect("No document");
        let body = document.body().expect("No body");
        body.append_child(&canvas).expect("Failed to append canvas");
        canvas.set_width(web_window.inner_width().unwrap().as_f64().unwrap() as u32);
        canvas.set_height(web_window.inner_height().unwrap().as_f64().unwrap() as u32);

        // Canvas must be focusable for keyboard events to reach winit.
        // Without tabindex, an HTMLCanvasElement never receives focus.
        canvas.set_attribute("tabindex", "0").ok();
        let _ = canvas.focus();

        // Re-focus on mousedown so clicking the canvas after tabbing
        // to another element restores keyboard input.
        {
            let canvas_for_focus = canvas.clone();
            let focus_cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
                move |_: web_sys::Event| {
                    let _ = canvas_for_focus.focus();
                },
            );
            canvas
                .add_event_listener_with_callback("mousedown", focus_cb.as_ref().unchecked_ref())
                .ok();
            focus_cb.forget(); // leak — lives for the page lifetime
        }

        // preventDefault on keydown while the text editor is open so
        // Tab/Enter/Backspace/arrows don't fire browser defaults
        // (tab-navigation, history-back, page-scroll).
        let suppress_keys: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        {
            let suppress = suppress_keys.clone();
            let pd_cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
                move |evt: web_sys::Event| {
                    if suppress.get() {
                        evt.prevent_default();
                    }
                },
            );
            canvas
                .add_event_listener_with_callback("keydown", pd_cb.as_ref().unchecked_ref())
                .ok();
            pd_cb.forget();
        }

        let renderer_window = Arc::clone(&self.window);

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
            map_path.unwrap_or_else(|| self.options.mindmap_path.clone())
        };

        // Shared state between the rAF render loop and the winit event
        // loop. Two RefCells so input handlers can borrow InputState
        // and Renderer simultaneously without conflict.
        /// Pending left-click awaiting a release. `None` on init and after
        /// release consumed; `Empty` after a click-down on empty canvas;
        /// `Node(id)` after a click-down on a node. Full drag machine
        /// (pan, move, reparent, connect) deferred to a later
        /// WASM-parity session.
        enum PendingClick {
            None,
            Empty,
            Node(String),
        }
        struct WasmInputState {
            document: MindMapDocument,
            mindmap_tree: Option<MindMapTree>,
            text_edit_state: TextEditState,
            last_click: Option<LastClick>,
            cursor_pos: (f64, f64),
            pending_click: PendingClick,
            modifiers: winit::keyboard::ModifiersState,
            /// Mirror of native's `app_scene` so the canvas-scene
            /// tree path (borders, eventually connections/portals)
            /// works identically on WASM. Threaded into every
            /// `rebuild_all` / `rebuild_scene_only` call below.
            app_scene: crate::application::scene_host::AppScene,
        }

        let renderer_rc: Rc<RefCell<Option<Renderer>>> = Rc::new(RefCell::new(None));
        let input_rc: Rc<RefCell<Option<WasmInputState>>> = Rc::new(RefCell::new(None));

        // Clone Rcs for the spawn_local init future
        let renderer_for_init = renderer_rc.clone();
        let input_for_init = input_rc.clone();

        // On WASM we run single-threaded -- spawn the renderer init as a future
        wasm_bindgen_futures::spawn_local(async move {
            let instance = Instance::default();
            let surface = instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
                .expect("Failed to create surface");
            let mut renderer = Renderer::new(
                instance,
                surface,
                renderer_window,
            )
            .await;

            let size = canvas.width();
            let height = canvas.height();
            renderer.process_decree(RenderDecree::SetSurfaceSize(size, height));

            // Load mindmap through Document -> Tree + Scene -> Renderer flow
            let mut doc_opt: Option<MindMapDocument> = None;
            let mut tree_opt: Option<MindMapTree> = None;
            // Local AppScene used only for the initial border tree
            // build; it's then dropped, and `WasmInputState`'s own
            // `app_scene` takes over for the live event loop.
            let mut init_app_scene =
                crate::application::scene_host::AppScene::new();
            if let Ok(doc) = MindMapDocument::load(&mindmap_path) {
                let mindmap_tree = doc.build_tree();
                renderer.rebuild_buffers_from_tree(&mindmap_tree.tree);
                renderer.fit_camera_to_tree(&mindmap_tree.tree);

                let scene = doc.build_scene(renderer.camera_zoom());
                update_connection_tree(&scene, &mut init_app_scene);
                update_border_tree_static(&doc, &mut init_app_scene);
                update_portal_tree(
                    &doc,
                    &std::collections::HashMap::new(),
                    &mut init_app_scene,
                    renderer,
                );
                update_connection_label_tree(&scene, &mut init_app_scene, renderer);
                flush_canvas_scene_buffers(&mut init_app_scene, renderer);
                tree_opt = Some(mindmap_tree);
                doc_opt = Some(doc);
            }

            renderer.process_decree(RenderDecree::StartRender);

            // Populate the shared state now that init is complete.
            *renderer_for_init.borrow_mut() = Some(renderer);

            if let Some(doc) = doc_opt {
                *input_for_init.borrow_mut() = Some(WasmInputState {
                    document: doc,
                    mindmap_tree: tree_opt,
                    text_edit_state: TextEditState::Closed,
                    last_click: None,
                    cursor_pos: (0.0, 0.0),
                    pending_click: PendingClick::None,
                    modifiers: winit::keyboard::ModifiersState::empty(),
                    app_scene: crate::application::scene_host::AppScene::new(),
                });
            }

            // WASM render loop via requestAnimationFrame
            use wasm_bindgen::closure::Closure;
            let renderer_for_raf = renderer_for_init.clone();
            let f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> =
                Rc::new(RefCell::new(None));
            let g = f.clone();

            *g.borrow_mut() = Some(Closure::new(move || {
                if let Some(r) = renderer_for_raf.borrow_mut().as_mut() {
                    r.process();
                }
                request_animation_frame(f.borrow().as_ref().unwrap());
            }));
            request_animation_frame(g.borrow().as_ref().unwrap());
        });

        // Resolve the keybind config once. `action_for(key, ctrl, shift, alt)`
        // answers the dispatch question for every keydown.
        let keybinds: ResolvedKeybinds = self.options.keybind_config.resolve();

        // Clone Rcs for the event loop closure
        let renderer_for_events = renderer_rc.clone();
        let input_for_events = input_rc.clone();
        let suppress_for_events = suppress_keys.clone();

        self.event_loop.run(move |event, _window_target| {
            _ = (&self.window, &mut self.options);

            match event {
                Event::WindowEvent {
                    event: WindowEvent::Resized(_size), ..
                } => {
                    // On WASM, resize is handled by the renderer's own loop
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested, ..
                } => {
                    // WASM doesn't really close
                }

                // --- Modifier tracking ---
                Event::WindowEvent {
                    event: WindowEvent::ModifiersChanged(mods), ..
                } => {
                    if let Some(input) = input_for_events.borrow_mut().as_mut() {
                        input.modifiers = mods.state();
                    }
                }

                // --- Keyboard input ---
                Event::WindowEvent {
                    event: WindowEvent::KeyboardInput {
                        event: KeyEvent {
                            state: ElementState::Pressed,
                            logical_key: ref logical_key,
                            ..
                        },
                        ..
                    },
                    ..
                } => {
                    let key_name = crate::application::keybinds::key_to_name(logical_key);

                    let mut input_borrow = input_for_events.borrow_mut();
                    let mut renderer_borrow = renderer_for_events.borrow_mut();
                    let (Some(input), Some(renderer)) =
                        (input_borrow.as_mut(), renderer_borrow.as_mut())
                    else { return; };

                    // Editor keyboard-steal: if open, route all keys
                    // to the editor so hotkeys don't collide with typed text.
                    if input.text_edit_state.is_open() {
                        handle_text_edit_key(
                            &key_name,
                            logical_key,
                            &mut input.text_edit_state,
                            &mut input.document,
                            &mut input.mindmap_tree,
                            renderer,
                        );
                        suppress_for_events.set(input.text_edit_state.is_open());
                        return;
                    }

                    // Hotkey dispatch via keybinds.
                    let action = key_name.as_deref().and_then(|k| {
                        keybinds.action_for(
                            k,
                            input.modifiers.control_key(),
                            input.modifiers.shift_key(),
                            input.modifiers.alt_key(),
                        )
                    });
                    match action {
                        Some(Action::Undo) => {
                            if input.document.undo() {
                                rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                            }
                        }
                        Some(Action::CreateOrphanNode) => {
                            let canvas_pos = renderer.screen_to_canvas(
                                input.cursor_pos.0 as f32,
                                input.cursor_pos.1 as f32,
                            );
                            input.document.create_orphan_and_select(canvas_pos);
                            rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                        }
                        Some(Action::OrphanSelection) => {
                            if input.document.apply_orphan_selection_with_undo() {
                                rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                            }
                        }
                        Some(Action::DeleteSelection) => {
                            if input.document.apply_delete_selection() {
                                rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                            }
                        }
                        Some(a @ (Action::EditSelection | Action::EditSelectionClean)) => {
                            let clean = matches!(a, Action::EditSelectionClean);
                            if let SelectionState::Single(id) = &input.document.selection {
                                let nid = id.clone();
                                open_text_edit(
                                    &nid, clean,
                                    &mut input.document,
                                    &mut input.text_edit_state,
                                    &mut input.mindmap_tree,
                                    renderer,
                                );
                                suppress_for_events.set(input.text_edit_state.is_open());
                            }
                        }
                        Some(Action::CancelMode) => {
                            // No AppMode on WASM yet; clear any pending click
                            // so a post-Esc click isn't retroactively paired
                            // with a pre-Esc click.
                            input.last_click = None;
                        }
                        Some(a @ (Action::EnterReparentMode | Action::EnterConnectMode)) => {
                            // AppMode state is still native-only; these
                            // actions are deferred on WASM (would require
                            // de-gating AppMode, hovered_node, and the
                            // reparent/connect click handlers).
                            log::debug!("WASM: mode-based action {:?} deferred", a);
                        }
                        Some(a) => {
                            // Actions whose backing surface is native-only
                            // (console, filesystem-backed save). On WASM
                            // they're acknowledged in the log and ignored.
                            log::debug!("WASM: action {:?} not supported", a);
                        }
                        None => {}
                    }
                }

                // --- Mouse input ---
                Event::WindowEvent {
                    event: WindowEvent::CursorMoved { position, .. }, ..
                } => {
                    if let Some(input) = input_for_events.borrow_mut().as_mut() {
                        input.cursor_pos = (position.x, position.y);
                    }
                }

                Event::WindowEvent {
                    event: WindowEvent::MouseInput {
                        state: btn_state,
                        button: MouseButton::Left,
                        ..
                    },
                    ..
                } => {
                    if btn_state == ElementState::Pressed {
                        // --- Left mouse Pressed ---
                        let mut input_borrow = input_for_events.borrow_mut();
                        let Some(input) = input_borrow.as_mut() else { return; };

                        // Compute canvas position via renderer
                        let canvas_pos = {
                            let renderer_borrow = renderer_for_events.borrow();
                            match renderer_borrow.as_ref() {
                                Some(r) => r.screen_to_canvas(
                                    input.cursor_pos.0 as f32,
                                    input.cursor_pos.1 as f32,
                                ),
                                None => return,
                            }
                        };

                        // Hit test against nodes
                        let hit_node: Option<String> = input.mindmap_tree
                            .as_ref()
                            .and_then(|tree| {
                                crate::application::document::hit_test(canvas_pos, tree)
                            });

                        // Double-click detection
                        let now = now_ms();
                        let already_editing_same_target = input.text_edit_state
                            .node_id()
                            .map(|id| hit_node.as_deref() == Some(id))
                            .unwrap_or(false);
                        let is_dblclick = !already_editing_same_target
                            && input.last_click
                                .as_ref()
                                .map(|prev| is_double_click(prev, now, input.cursor_pos, &hit_node))
                                .unwrap_or(false);

                        if is_dblclick {
                            input.last_click = None;

                            let mut renderer_borrow = renderer_for_events.borrow_mut();
                            let Some(renderer) = renderer_borrow.as_mut() else { return; };

                            if let Some(ref node_id) = hit_node {
                                let nid = node_id.clone();
                                input.document.selection = SelectionState::Single(nid.clone());
                                rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                                open_text_edit(
                                    &nid, false,
                                    &mut input.document,
                                    &mut input.text_edit_state,
                                    &mut input.mindmap_tree,
                                    renderer,
                                );
                            } else {
                                let allow_create = !matches!(
                                    input.document.selection,
                                    SelectionState::Edge(_) | SelectionState::Portal(_)
                                );
                                if allow_create {
                                    let new_id = input.document.create_orphan_and_select(canvas_pos);
                                    rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                                    open_text_edit(
                                        &new_id, true,
                                        &mut input.document,
                                        &mut input.text_edit_state,
                                        &mut input.mindmap_tree,
                                        renderer,
                                    );
                                }
                            }
                            suppress_for_events.set(input.text_edit_state.is_open());
                            return;
                        }

                        input.pending_click = match hit_node.clone() {
                            Some(id) => PendingClick::Node(id),
                            None => PendingClick::Empty,
                        };
                        input.last_click = Some(LastClick {
                            time: now,
                            screen_pos: input.cursor_pos,
                            hit: hit_node,
                        });
                    } else {
                        // --- Left mouse Released ---
                        let mut input_borrow = input_for_events.borrow_mut();
                        let Some(input) = input_borrow.as_mut() else { return; };

                        let pending = std::mem::replace(&mut input.pending_click, PendingClick::None);
                        if matches!(pending, PendingClick::None) { return; }

                        if input.text_edit_state.is_open() {
                            let mut renderer_borrow = renderer_for_events.borrow_mut();
                            let Some(renderer) = renderer_borrow.as_mut() else { return; };
                            let release_canvas = renderer.screen_to_canvas(
                                input.cursor_pos.0 as f32,
                                input.cursor_pos.1 as f32,
                            );

                            let inside_edit_node = input.text_edit_state
                                .node_id()
                                .zip(input.mindmap_tree.as_ref())
                                .map(|(id, tree)| {
                                    crate::application::document::point_in_node_aabb(
                                        release_canvas, id, tree,
                                    )
                                })
                                .unwrap_or(false);

                            if inside_edit_node {
                                return;
                            }

                            close_text_edit(
                                true,
                                &mut input.document,
                                &mut input.text_edit_state,
                                &mut input.mindmap_tree,
                                renderer,
                            );
                            suppress_for_events.set(false);
                            return;
                        }

                        // Plain selection click
                        input.document.selection = match pending {
                            PendingClick::Node(node_id) => SelectionState::Single(node_id),
                            _ => SelectionState::None,
                        };
                        let mut renderer_borrow = renderer_for_events.borrow_mut();
                        if let Some(renderer) = renderer_borrow.as_mut() {
                            rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                        }
                    }
                }

                Event::WindowEvent {
                    event: WindowEvent::MouseWheel { delta, .. }, ..
                } => {
                    let scroll_y = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
                    };
                    let factor = if scroll_y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    let mut input_borrow = input_for_events.borrow_mut();
                    let mut renderer_borrow = renderer_for_events.borrow_mut();
                    if let (Some(input), Some(renderer)) =
                        (input_borrow.as_mut(), renderer_borrow.as_mut())
                    {
                        renderer.process_decree(RenderDecree::CameraZoom {
                            screen_x: input.cursor_pos.0 as f32,
                            screen_y: input.cursor_pos.1 as f32,
                            factor: factor as f32,
                        });
                        // Zoom touches scene geometry (connection glyph
                        // sample spacing, viewport cull rect) but not the
                        // node text tree — scene-only rebuild is enough.
                        rebuild_scene_only(&input.document, &mut input.app_scene, renderer);
                    }
                }

                _ => {}
            }
        }).expect("Event loop error");
    }
}

#[cfg(target_arch = "wasm32")]
fn request_animation_frame(f: &wasm_bindgen::closure::Closure<dyn FnMut()>) {
    use wasm_bindgen::JsCast;
    web_sys::window()
        .unwrap()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .unwrap();
}

/// Rebuild tree from model with selection highlight, plus connections and borders.
/// When the current selection is an edge, its `ConnectionElement` gets a
/// cyan color override baked in via `build_scene_with_selection()`.


/// Handle a click event: update selection, rebuild tree with highlight.
/// When the node hit test misses, falls through to edge hit testing so
/// the user can click on a connection path to select it. If the clicked
/// node has an `OnClick` trigger binding, the bound custom mutation fires
/// (both node mutations and any document actions) after the selection
/// update.
#[cfg(not(target_arch = "wasm32"))]
fn handle_click(
    hit: Option<String>,
    cursor_pos: (f64, f64),
    shift_pressed: bool,
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let doc = match document.as_mut() {
        Some(d) => d,
        None => return,
    };

    // Fire any OnClick triggers before the selection update so that
    // document actions (theme switches etc.) take effect before the
    // scene rebuild below picks up the new state. Node mutations go
    // into the tree via `apply_custom_mutation`, which owns the
    // model-sync + undo-push for Persistent behavior.
    if let Some(id) = hit.as_ref() {
        let triggered = doc.find_triggered_mutations(
            id, &Trigger::OnClick, &PlatformContext::Desktop,
        );
        if !triggered.is_empty() {
            // `find_triggered_mutations` returned cloned CustomMutations so
            // we can iterate without holding an immutable borrow on doc.
            for cm in triggered {
                if cm.timing.as_ref().is_some_and(|t| t.duration_ms > 0) {
                    // Animated: snapshot from/to and start an
                    // instance. The AboutToWait tick interpolates
                    // and commits the final mutation at completion.
                    doc.start_animation(&cm, id, now_ms() as u64);
                } else if let Some(tree) = mindmap_tree.as_mut() {
                    doc.apply_custom_mutation(&cm, id, tree);
                }
                doc.apply_document_actions(&cm);
            }
        }
    }

    // Update selection state
    match (&hit, shift_pressed) {
        (Some(id), false) => {
            doc.selection = SelectionState::Single(id.clone());
        }
        (Some(id), true) => {
            // Shift+click: toggle node in/out of multi-selection.
            // Shift+click on an edge selection promotes the clicked node
            // to a fresh single selection (no edge multi-select).
            match &doc.selection {
                SelectionState::None
                | SelectionState::Edge(_)
                | SelectionState::Portal(_) => {
                    doc.selection = SelectionState::Single(id.clone());
                }
                SelectionState::Single(existing) => {
                    if existing == id {
                        doc.selection = SelectionState::None;
                    } else {
                        doc.selection = SelectionState::Multi(vec![existing.clone(), id.clone()]);
                    }
                }
                SelectionState::Multi(existing) => {
                    let mut ids = existing.clone();
                    if let Some(pos) = ids.iter().position(|i| i == id) {
                        ids.remove(pos);
                        doc.selection = match ids.len() {
                            0 => SelectionState::None,
                            1 => SelectionState::Single(ids.into_iter().next().unwrap()),
                            _ => SelectionState::Multi(ids),
                        };
                    } else {
                        ids.push(id.clone());
                        doc.selection = SelectionState::Multi(ids);
                    }
                }
            }
        }
        (None, false) => {
            // Node miss — fall through: first try portal markers
            // (small glyphs floating above the top-right corner of
            // their endpoint nodes), then edge hit testing, then
            // finally deselect.
            let canvas_pos = renderer.screen_to_canvas(
                cursor_pos.0 as f32, cursor_pos.1 as f32,
            );
            if let Some(pkey) = renderer.hit_test_portal(canvas_pos) {
                doc.selection = SelectionState::Portal(
                    crate::application::document::PortalRef::new(
                        pkey.label, pkey.endpoint_a, pkey.endpoint_b,
                    ),
                );
            } else {
                let tolerance = EDGE_HIT_TOLERANCE_PX * renderer.canvas_per_pixel();
                let edge_hit = hit_test_edge(canvas_pos, &doc.mindmap, tolerance);
                doc.selection = match edge_hit {
                    Some(edge_ref) => SelectionState::Edge(edge_ref),
                    None => SelectionState::None,
                };
            }
        }
        (None, true) => {
            // Shift+click on empty space: keep current selection (no edge
            // hit test — shift is reserved for multi-node).
        }
    }

    // Rebuild tree with selection highlight applied
    rebuild_all(doc, mindmap_tree, app_scene, renderer);
}

/// Rebuild tree, connections, and borders like `rebuild_all`, but additionally
/// overlays reparent-mode highlights on top of the normal selection highlight.
/// `hovered_node` is the node currently under the cursor (highlighted green as
/// the drop target) when in reparent mode; it is ignored in Normal mode.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_all_with_mode(
    doc: &MindMapDocument,
    app_mode: &AppMode,
    hovered_node: Option<&str>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let mut new_tree = doc.build_tree();

    // Build a single flat list of (mind_node_id, color) pairs that
    // `apply_tree_highlights` applies via baumhard's mutator/walker.
    // Order matters: later entries override earlier ones via the
    // repeated `SetRegionColor` mutation, so selection (cyan) is
    // listed first, then mode-specific source (orange), then the
    // hovered target (green). This matches the previous behavior
    // where reparent_source_highlight was documented to override
    // selection_highlight on conflict.
    let mut highlights: Vec<(&str, [f32; 4])> = doc
        .selection
        .selected_ids()
        .into_iter()
        .map(|id| (id, HIGHLIGHT_COLOR))
        .collect();
    match app_mode {
        AppMode::Reparent { sources } => {
            for s in sources {
                highlights.push((s.as_str(), REPARENT_SOURCE_COLOR));
            }
            if let Some(h) = hovered_node {
                if !sources.iter().any(|s| s == h) {
                    highlights.push((h, REPARENT_TARGET_COLOR));
                }
            }
        }
        AppMode::Connect { source } => {
            highlights.push((source.as_str(), REPARENT_SOURCE_COLOR));
            if let Some(h) = hovered_node {
                if h != source {
                    highlights.push((h, REPARENT_TARGET_COLOR));
                }
            }
        }
        AppMode::Normal => {}
    }
    apply_tree_highlights(&mut new_tree, highlights);
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    let scene = doc.build_scene_with_selection(renderer.camera_zoom());
    update_connection_tree(&scene, app_scene);
    update_border_tree_static(doc, app_scene);
    update_portal_tree(doc, &std::collections::HashMap::new(), app_scene, renderer);
    update_edge_handle_tree(&scene, app_scene);
    update_connection_label_tree(&scene, app_scene, renderer);
    flush_canvas_scene_buffers(app_scene, renderer);

    *mindmap_tree = Some(new_tree);
}

/// Handle a left-click while in connect mode: hit-test for a target node
/// and create a new `cross_link` edge from the source node to the target.
/// Clicking on empty canvas, the source itself, or a node that already has
/// a cross_link from the source is a silent no-op. Exits connect mode and
/// rebuilds the scene unconditionally.
#[cfg(not(target_arch = "wasm32"))]
fn handle_connect_target_click(
    cursor_pos: (f64, f64),
    app_mode: &mut AppMode,
    hovered_node: &mut Option<String>,
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let source = match std::mem::replace(app_mode, AppMode::Normal) {
        AppMode::Connect { source } => source,
        _ => return,
    };
    *hovered_node = None;

    // Hit-test the cursor position against the current tree.
    let target: Option<String> = mindmap_tree.as_ref().and_then(|tree| {
        let canvas_pos = renderer.screen_to_canvas(cursor_pos.0 as f32, cursor_pos.1 as f32);
        hit_test(canvas_pos, tree)
    });

    if let Some(doc) = document.as_mut() {
        if let Some(target_id) = target {
            if let Some(idx) = doc.create_cross_link_edge(&source, &target_id) {
                doc.undo_stack.push(UndoAction::CreateEdge { index: idx });
                // Select the newly-created edge so the user gets immediate
                // visual confirmation and can Delete it or style it next.
                doc.selection = SelectionState::Edge(EdgeRef::new(
                    source.clone(),
                    target_id,
                    "cross_link",
                ));
                doc.dirty = true;
            }
        }
        // Full rebuild regardless — exiting the mode requires clearing
        // orange/green highlights.
        rebuild_all(doc, mindmap_tree, app_scene, renderer);
    }
}

/// Handle a left-click while in reparent mode: hit-test for a target node and
/// perform the reparent (or promote to root if clicked on empty canvas). Exits
/// reparent mode and rebuilds the scene unconditionally.
#[cfg(not(target_arch = "wasm32"))]
fn handle_reparent_target_click(
    cursor_pos: (f64, f64),
    app_mode: &mut AppMode,
    hovered_node: &mut Option<String>,
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let sources = match std::mem::replace(app_mode, AppMode::Normal) {
        AppMode::Reparent { sources } => sources,
        AppMode::Normal | AppMode::Connect { .. } => return,
    };
    *hovered_node = None;

    // Hit-test the cursor position against the current tree.
    let target: Option<String> = mindmap_tree.as_ref().and_then(|tree| {
        let canvas_pos = renderer.screen_to_canvas(cursor_pos.0 as f32, cursor_pos.1 as f32);
        hit_test(canvas_pos, tree)
    });

    if let Some(doc) = document.as_mut() {
        // target = Some(id) → reparent under that node as last child
        // target = None     → promote sources to root (click on empty canvas)
        let undo_data = doc.apply_reparent(&sources, target.as_deref());
        if !undo_data.entries.is_empty() {
            doc.undo_stack.push(UndoAction::ReparentNodes {
                entries: undo_data.entries,
                old_edges: undo_data.old_edges,
            });
            doc.dirty = true;
        }
        // Full rebuild: tree structure changed even if a no-op, the mode exit
        // requires clearing the orange/green highlights.
        rebuild_all(doc, mindmap_tree, app_scene, renderer);
    }
}

/**
Launch and run options for the application and the application instance
 **/
#[derive(Clone)]
pub struct Options {
    pub launch_gpu_prefer_low_power: bool,
    pub should_exit: bool,
    pub window_mode: WindowMode,
    pub ui_scale: i8,
    pub window_title_text: &'static str,
    pub input_mode: InputMode,
    pub avail_cores: usize,
    pub render_must_be_main: bool,
    pub mindmap_path: String,
    /// The user's keybinding configuration (already loaded from file or
    /// defaults). The event loop resolves this into a `ResolvedKeybinds`
    /// at startup and dispatches keyboard events through it.
    pub keybind_config: crate::application::keybinds::KeybindConfig,
}

// =====================================================================
// Session 7A: unit tests for pure helpers (cursor math, caret
// insertion, double-click detection, Baumhard mutation round-trip).
// Event-loop integration is verified manually via `cargo run`.
// =====================================================================


#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;
