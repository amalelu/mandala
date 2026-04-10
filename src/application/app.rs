use std::sync::{Arc, RwLock};

#[cfg(not(target_arch = "wasm32"))]
use pollster::block_on;
use glam::Vec2;
use indextree::Arena;
use wgpu::{Instance, SurfaceTargetUnsafe};
use winit::event::{ElementState, Event, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ControlFlow;
use winit::keyboard::{Key, NamedKey};
use winit::{event_loop::EventLoop, window::Window};

use crate::application::common::{InputMode, RenderDecree, WindowMode};
use crate::application::document::{MindMapDocument, SelectionState, UndoAction, hit_test, apply_selection_highlight, apply_drag_delta};
use crate::application::renderer::Renderer;

use baumhard::gfx_structs::element::GfxElement;

/// Tracks the current drag interaction state.
#[cfg(not(target_arch = "wasm32"))]
enum DragState {
    /// No drag in progress.
    None,
    /// Mouse is down but hasn't moved past the drag threshold yet.
    Pending { start_pos: (f64, f64), hit_node: Option<String> },
    /// Dragging to pan the camera (started on empty space).
    Panning,
    /// Dragging a node to reposition it.
    MovingNode {
        node_id: String,
        /// Accumulated total delta in canvas coords (for model sync on drop).
        total_delta: Vec2,
        /// Whether dragging only the individual node (alt+drag) vs subtree.
        individual: bool,
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
    pub fn run(mut self) {
        use baumhard::mindmap::tree_builder::MindMapTree;

        // Single-threaded architecture: App owns the Renderer directly
        baumhard::font::fonts::init();

        // Initialize graphics arena (core GfxElement infrastructure)
        let gfx_arena: Arc<RwLock<Arena<GfxElement>>> = Arc::new(RwLock::new(Arena::new()));

        let unsafe_target = unsafe { SurfaceTargetUnsafe::from_window(self.window.as_ref()) }
            .expect("Failed to create a SurfaceTargetUnsafe");
        let instance = Instance::default();
        let surface = unsafe { instance.create_surface_unsafe(unsafe_target) }.unwrap();

        let mut renderer = block_on(Renderer::new(
            instance,
            surface,
            Arc::clone(&self.window),
            gfx_arena.clone(),
        ));

        // Configure initial surface size
        let size = self.window.inner_size();
        renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));

        // Update arena buffers
        renderer.process_decree(RenderDecree::ArenaUpdate);

        // Load mindmap — document and tree persist for interactive use
        let mut document: Option<MindMapDocument> = None;
        let mut mindmap_tree: Option<MindMapTree> = None;

        match MindMapDocument::load(&self.options.mindmap_path) {
            Ok(doc) => {
                // Nodes: build Baumhard tree from MindMap hierarchy
                let tree = doc.build_tree();
                renderer.rebuild_buffers_from_tree(&tree.tree);
                renderer.fit_camera_to_tree(&tree.tree);

                // Connections + borders: flat pipeline from RenderScene
                let scene = doc.build_scene();
                renderer.rebuild_connection_buffers(&scene.connection_elements);
                renderer.rebuild_border_buffers(&scene.border_elements);

                mindmap_tree = Some(tree);
                document = Some(doc);
            }
            Err(e) => {
                log::error!("{}", e);
            }
        }

        // Start rendering
        renderer.process_decree(RenderDecree::StartRender);

        // Input state
        let mut cursor_pos: (f64, f64) = (0.0, 0.0);
        let mut drag_state = DragState::None;
        let mut shift_pressed = false;
        let mut alt_pressed = false;
        let mut ctrl_pressed = false;

        self.event_loop.run(move |event, _window_target| {
            _ = (&self.window, &mut self.options);

            _window_target.set_control_flow(ControlFlow::Poll);

            match event {
                //////////////////////////
                //// WINDOW SPECIFIC ////
                ////////////////////////
                Event::WindowEvent {
                    event: WindowEvent::Resized(size),
                    ..
                } => {
                    renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => {
                    renderer.process_decree(RenderDecree::Terminate);
                    std::process::exit(0);
                }
                ////////////////
                //// MOUSE ////
                //////////////
                Event::WindowEvent {
                    event: WindowEvent::MouseInput { state, button, .. },
                    ..
                } => {
                    match button {
                        MouseButton::Middle => {
                            if state == ElementState::Pressed {
                                drag_state = DragState::Panning;
                            } else {
                                drag_state = DragState::None;
                            }
                        }
                        MouseButton::Left => {
                            if state == ElementState::Pressed {
                                // Hit test to determine if clicking on a node
                                let hit_node = mindmap_tree.as_ref().and_then(|tree| {
                                    let canvas_pos = renderer.screen_to_canvas(
                                        cursor_pos.0 as f32,
                                        cursor_pos.1 as f32,
                                    );
                                    hit_test(canvas_pos, tree)
                                });
                                drag_state = DragState::Pending {
                                    start_pos: cursor_pos,
                                    hit_node,
                                };
                            } else {
                                // Released
                                match std::mem::replace(&mut drag_state, DragState::None) {
                                    DragState::Pending { hit_node, .. } => {
                                        // No drag occurred — treat as click
                                        handle_click(
                                            hit_node,
                                            shift_pressed,
                                            &mut document,
                                            &mut mindmap_tree,
                                            &mut renderer,
                                        );
                                    }
                                    DragState::MovingNode { node_id, total_delta, individual } => {
                                        // Drop: sync to model, full rebuild, push undo
                                        if let Some(doc) = document.as_mut() {
                                            let dx = total_delta.x as f64;
                                            let dy = total_delta.y as f64;
                                            let undo_data = if individual {
                                                doc.apply_move_single(&node_id, dx, dy)
                                                    .into_iter().collect()
                                            } else {
                                                doc.apply_move_subtree(&node_id, dx, dy)
                                            };
                                            doc.undo_stack.push(UndoAction::MoveNodes {
                                                original_positions: undo_data,
                                            });
                                            doc.dirty = true;

                                            // Full rebuild from model
                                            rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                        }
                                    }
                                    DragState::Panning | DragState::None => {}
                                }
                            }
                        }
                        _ => {}
                    }
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
                    renderer.process_decree(RenderDecree::CameraZoom {
                        screen_x: cursor_pos.0 as f32,
                        screen_y: cursor_pos.1 as f32,
                        factor: factor as f32,
                    });
                }
                Event::WindowEvent {
                    event: WindowEvent::CursorMoved { position, .. },
                    ..
                } => {
                    let prev_pos = cursor_pos;
                    cursor_pos = (position.x, position.y);

                    match &mut drag_state {
                        DragState::Panning => {
                            let dx = cursor_pos.0 - prev_pos.0;
                            let dy = cursor_pos.1 - prev_pos.1;
                            renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                        }
                        DragState::MovingNode { node_id, total_delta, individual } => {
                            // Convert screen delta to canvas delta
                            let old_canvas = renderer.screen_to_canvas(prev_pos.0 as f32, prev_pos.1 as f32);
                            let new_canvas = renderer.screen_to_canvas(cursor_pos.0 as f32, cursor_pos.1 as f32);
                            let canvas_dx = new_canvas.x - old_canvas.x;
                            let canvas_dy = new_canvas.y - old_canvas.y;

                            *total_delta += Vec2::new(canvas_dx, canvas_dy);

                            // Apply in-place mutation to tree for visual preview
                            if let Some(tree) = mindmap_tree.as_mut() {
                                apply_drag_delta(tree, node_id, canvas_dx, canvas_dy, !*individual);
                                renderer.rebuild_buffers_from_tree(&tree.tree);
                            }
                        }
                        DragState::Pending { start_pos, hit_node } => {
                            let dist_x = cursor_pos.0 - start_pos.0;
                            let dist_y = cursor_pos.1 - start_pos.1;
                            if dist_x * dist_x + dist_y * dist_y > 25.0 {
                                // Past threshold — decide: move node or pan camera
                                if let Some(node_id) = hit_node.take() {
                                    // Ensure the node is selected
                                    if let Some(doc) = document.as_mut() {
                                        if !doc.selection.is_selected(&node_id) {
                                            doc.selection = SelectionState::Single(node_id.clone());
                                            // Rebuild with highlight for the newly selected node
                                            if let Some(tree) = mindmap_tree.as_mut() {
                                                let mut new_tree = doc.build_tree();
                                                apply_selection_highlight(&mut new_tree, &doc.selection);
                                                renderer.rebuild_buffers_from_tree(&new_tree.tree);
                                                *tree = new_tree;
                                            }
                                        }
                                    }
                                    drag_state = DragState::MovingNode {
                                        node_id,
                                        total_delta: Vec2::ZERO,
                                        individual: alt_pressed,
                                    };
                                } else {
                                    drag_state = DragState::Panning;
                                    let dx = cursor_pos.0 - prev_pos.0;
                                    let dy = cursor_pos.1 - prev_pos.1;
                                    renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                                }
                            }
                        }
                        DragState::None => {}
                    }
                }
                ////////////////////
                //// KEYBOARD ////
                //////////////////
                Event::WindowEvent {
                    event: WindowEvent::ModifiersChanged(modifiers),
                    ..
                } => {
                    shift_pressed = modifiers.state().shift_key();
                    alt_pressed = modifiers.state().alt_key();
                    ctrl_pressed = modifiers.state().control_key();
                }
                Event::WindowEvent {
                    event: WindowEvent::KeyboardInput {
                        event: KeyEvent {
                            logical_key,
                            state: ElementState::Pressed,
                            ..
                        },
                        ..
                    },
                    ..
                } => {
                    let is_undo = match &logical_key {
                        Key::Named(NamedKey::Undo) => true,
                        Key::Character(c) if ctrl_pressed && c.as_ref() == "z" => true,
                        _ => false,
                    };
                    if is_undo {
                        if let Some(doc) = document.as_mut() {
                            if doc.undo() {
                                rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                            }
                        }
                    }
                }
                Event::AboutToWait => {
                    // Drive the render loop each frame
                    renderer.process();
                }
                _ => {}
            }
        }).expect("Some kind of unexpected error appears to have taken place")
    }

    #[cfg(target_arch = "wasm32")]
    pub fn run(mut self) {
        use wasm_bindgen::JsCast;
        use winit::platform::web::WindowExtWebSys;

        baumhard::font::fonts::init();

        // Attach canvas to DOM
        let canvas = self.window.canvas().expect("Failed to get canvas");
        let web_window = web_sys::window().expect("No global window");
        let document = web_window.document().expect("No document");
        let body = document.body().expect("No body");
        body.append_child(&canvas).expect("Failed to append canvas");
        canvas.set_width(web_window.inner_width().unwrap().as_f64().unwrap() as u32);
        canvas.set_height(web_window.inner_height().unwrap().as_f64().unwrap() as u32);

        let gfx_arena: Arc<RwLock<Arena<GfxElement>>> = Arc::new(RwLock::new(Arena::new()));
        let renderer_window = Arc::clone(&self.window);

        // On WASM, check for ?map= query parameter to override the default path
        let mindmap_path = {
            let web_window = web_sys::window().expect("No global window");
            let search = web_window.location().search().unwrap_or_default();
            if let Some(map_param) = search.strip_prefix("?map=") {
                map_param.to_string()
            } else {
                self.options.mindmap_path.clone()
            }
        };

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
                gfx_arena.clone(),
            )
            .await;

            let size = canvas.width();
            let height = canvas.height();
            renderer.process_decree(RenderDecree::SetSurfaceSize(size, height));

            // Load mindmap through Document -> Tree + Scene -> Renderer flow
            if let Ok(document) = MindMapDocument::load(&mindmap_path) {
                // Nodes: build Baumhard tree from MindMap hierarchy
                let mindmap_tree = document.build_tree();
                renderer.rebuild_buffers_from_tree(&mindmap_tree.tree);
                renderer.fit_camera_to_tree(&mindmap_tree.tree);

                // Connections + borders: flat pipeline from RenderScene
                let scene = document.build_scene();
                renderer.rebuild_connection_buffers(&scene.connection_elements);
                renderer.rebuild_border_buffers(&scene.border_elements);
            }

            renderer.process_decree(RenderDecree::StartRender);

            // Store renderer in a RefCell for the event loop
            let renderer = std::cell::RefCell::new(renderer);

            // WASM render loop via requestAnimationFrame
            use wasm_bindgen::closure::Closure;
            let f: std::rc::Rc<std::cell::RefCell<Option<Closure<dyn FnMut()>>>> =
                std::rc::Rc::new(std::cell::RefCell::new(None));
            let g = f.clone();

            *g.borrow_mut() = Some(Closure::new(move || {
                renderer.borrow_mut().process();
                request_animation_frame(f.borrow().as_ref().unwrap());
            }));
            request_animation_frame(g.borrow().as_ref().unwrap());
        });

        // The event loop handles input but doesn't block on WASM
        let mut cursor_pos: (f64, f64) = (0.0, 0.0);
        let mut is_panning = false;

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
                Event::WindowEvent {
                    event: WindowEvent::MouseInput { state, button, .. }, ..
                } => {
                    if button == MouseButton::Left || button == MouseButton::Middle {
                        is_panning = state == ElementState::Pressed;
                    }
                }
                Event::WindowEvent {
                    event: WindowEvent::MouseWheel { delta, .. }, ..
                } => {
                    let _scroll_y = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
                    };
                    // TODO: WASM input needs to be forwarded to the renderer
                }
                Event::WindowEvent {
                    event: WindowEvent::CursorMoved { position, .. }, ..
                } => {
                    let _dx = position.x - cursor_pos.0;
                    let _dy = position.y - cursor_pos.1;
                    cursor_pos = (position.x, position.y);
                    // TODO: WASM input needs to be forwarded to the renderer
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
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_all(
    doc: &MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let mut new_tree = doc.build_tree();
    apply_selection_highlight(&mut new_tree, &doc.selection);
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    let scene = doc.build_scene();
    renderer.rebuild_connection_buffers(&scene.connection_elements);
    renderer.rebuild_border_buffers(&scene.border_elements);

    *mindmap_tree = Some(new_tree);
}

/// Handle a click event: update selection, rebuild tree with highlight.
#[cfg(not(target_arch = "wasm32"))]
fn handle_click(
    hit: Option<String>,
    shift_pressed: bool,
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let doc = match document.as_mut() {
        Some(d) => d,
        None => return,
    };

    // Update selection state
    match (&hit, shift_pressed) {
        (Some(id), false) => {
            doc.selection = SelectionState::Single(id.clone());
        }
        (Some(id), true) => {
            // Shift+click: toggle node in/out of multi-selection
            match &doc.selection {
                SelectionState::None => {
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
            doc.selection = SelectionState::None;
        }
        (None, true) => {
            // Shift+click on empty space: keep current selection
        }
    }

    // Rebuild tree with selection highlight applied
    rebuild_all(doc, mindmap_tree, renderer);
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
}
