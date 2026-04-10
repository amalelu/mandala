use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[cfg(not(target_arch = "wasm32"))]
use pollster::block_on;
use glam::Vec2;
use indextree::Arena;
use wgpu::{Instance, SurfaceTargetUnsafe};
use winit::event::{ElementState, Event, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ControlFlow;
use winit::keyboard::Key;
use winit::{event_loop::EventLoop, window::Window};

use crate::application::common::{InputMode, RenderDecree, WindowMode};
use crate::application::document::{
    EdgeRef, MindMapDocument, SelectionState, UndoAction,
    hit_test, hit_test_edge, rect_select,
    apply_selection_highlight, apply_drag_delta,
    apply_reparent_source_highlight, apply_reparent_target_highlight,
};
use crate::application::keybinds::{Action, ResolvedKeybinds, normalize_key_name};
use crate::application::renderer::Renderer;

use baumhard::gfx_structs::element::GfxElement;

/// Screen-space click tolerance (in pixels) for edge hit testing. Converted
/// to canvas units via `Renderer::canvas_per_pixel()` so the click target
/// stays visually stable across zoom levels.
#[cfg(not(target_arch = "wasm32"))]
const EDGE_HIT_TOLERANCE_PX: f32 = 8.0;

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
    Pending { start_pos: (f64, f64), hit_node: Option<String> },
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
        let mut app_mode = AppMode::Normal;
        let mut hovered_node: Option<String> = None;
        let mut shift_pressed = false;
        let mut alt_pressed = false;
        let mut ctrl_pressed = false;

        // Resolve keybindings once at startup. Users can rebind any key
        // by shipping a `keybinds.json` (see `keybinds.rs` for the format).
        let keybinds: ResolvedKeybinds = self.options.keybind_config.resolve();

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
                            // In reparent or connect mode, left-click (release) is consumed as
                            // a "choose target" gesture and never transitions to Pending/drag.
                            if matches!(app_mode, AppMode::Reparent { .. }) {
                                if state == ElementState::Released {
                                    handle_reparent_target_click(
                                        cursor_pos,
                                        &mut app_mode,
                                        &mut hovered_node,
                                        &mut document,
                                        &mut mindmap_tree,
                                        &mut renderer,
                                    );
                                }
                                // Pressed: swallow — do not transition drag state
                            } else if matches!(app_mode, AppMode::Connect { .. }) {
                                if state == ElementState::Released {
                                    handle_connect_target_click(
                                        cursor_pos,
                                        &mut app_mode,
                                        &mut hovered_node,
                                        &mut document,
                                        &mut mindmap_tree,
                                        &mut renderer,
                                    );
                                }
                                // Pressed: swallow
                            } else if state == ElementState::Pressed {
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
                                            cursor_pos,
                                            shift_pressed,
                                            &mut document,
                                            &mut mindmap_tree,
                                            &mut renderer,
                                        );
                                    }
                                    DragState::MovingNode { node_ids, total_delta, pending_delta, individual } => {
                                        // Flush any remaining pending delta to the tree before drop
                                        if pending_delta != Vec2::ZERO {
                                            if let Some(tree) = mindmap_tree.as_mut() {
                                                for nid in &node_ids {
                                                    apply_drag_delta(tree, nid, pending_delta.x, pending_delta.y, !individual);
                                                }
                                            }
                                        }
                                        // Drop: sync to model, full rebuild, push undo
                                        if let Some(doc) = document.as_mut() {
                                            let dx = total_delta.x as f64;
                                            let dy = total_delta.y as f64;
                                            let undo_data = doc.apply_move_multiple(&node_ids, dx, dy, individual);
                                            doc.undo_stack.push(UndoAction::MoveNodes {
                                                original_positions: undo_data,
                                            });
                                            doc.dirty = true;

                                            // Full rebuild from model
                                            rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                        }
                                    }
                                    DragState::SelectingRect { start_canvas, current_canvas } => {
                                        // Finalize: select all nodes in the rectangle
                                        renderer.clear_overlay_buffers();
                                        if let (Some(doc), Some(tree)) = (document.as_mut(), mindmap_tree.as_ref()) {
                                            let hits = rect_select(start_canvas, current_canvas, tree);
                                            doc.selection = match hits.len() {
                                                0 => SelectionState::None,
                                                1 => SelectionState::Single(hits.into_iter().next().unwrap()),
                                                _ => SelectionState::Multi(hits),
                                            };
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

                    // Reparent or Connect mode: hit-test under cursor to update the hover
                    // target highlight. Skip the regular drag-state handling (no drag in
                    // these modes).
                    if matches!(app_mode, AppMode::Reparent { .. } | AppMode::Connect { .. }) {
                        let new_hover = mindmap_tree.as_ref().and_then(|tree| {
                            let canvas_pos = renderer.screen_to_canvas(
                                cursor_pos.0 as f32, cursor_pos.1 as f32,
                            );
                            hit_test(canvas_pos, tree)
                        });
                        if new_hover != hovered_node {
                            hovered_node = new_hover;
                            if let Some(doc) = document.as_ref() {
                                rebuild_all_with_mode(
                                    doc, &app_mode, hovered_node.as_deref(),
                                    &mut mindmap_tree, &mut renderer,
                                );
                            }
                        }
                        return;
                    }

                    match &mut drag_state {
                        DragState::Panning => {
                            let dx = cursor_pos.0 - prev_pos.0;
                            let dy = cursor_pos.1 - prev_pos.1;
                            renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                        }
                        DragState::MovingNode { total_delta, pending_delta, .. } => {
                            // Convert screen delta to canvas delta and accumulate.
                            // Actual mutation + rebuild happens in AboutToWait (once per frame).
                            let old_canvas = renderer.screen_to_canvas(prev_pos.0 as f32, prev_pos.1 as f32);
                            let new_canvas = renderer.screen_to_canvas(cursor_pos.0 as f32, cursor_pos.1 as f32);
                            let delta = new_canvas - old_canvas;

                            *total_delta += delta;
                            *pending_delta += delta;
                        }
                        DragState::Pending { start_pos, hit_node } => {
                            let dist_x = cursor_pos.0 - start_pos.0;
                            let dist_y = cursor_pos.1 - start_pos.1;
                            if dist_x * dist_x + dist_y * dist_y > 25.0 {
                                // Past threshold — decide: move node or pan camera
                                if let Some(node_id) = hit_node.take() {
                                    // Ensure the dragged node is selected
                                    if let Some(doc) = document.as_mut() {
                                        if !doc.selection.is_selected(&node_id) {
                                            doc.selection = SelectionState::Single(node_id.clone());
                                            if let Some(tree) = mindmap_tree.as_mut() {
                                                let mut new_tree = doc.build_tree();
                                                apply_selection_highlight(&mut new_tree, &doc.selection);
                                                renderer.rebuild_buffers_from_tree(&new_tree.tree);
                                                *tree = new_tree;
                                            }
                                        }
                                    }
                                    // Shift+drag: move all selected nodes together
                                    let node_ids = if shift_pressed {
                                        if let Some(doc) = document.as_ref() {
                                            let mut ids: Vec<String> = doc.selection.selected_ids()
                                                .iter().map(|s| s.to_string()).collect();
                                            if !ids.contains(&node_id) {
                                                ids.push(node_id);
                                            }
                                            ids
                                        } else {
                                            vec![node_id]
                                        }
                                    } else {
                                        vec![node_id]
                                    };
                                    drag_state = DragState::MovingNode {
                                        node_ids,
                                        total_delta: Vec2::ZERO,
                                        pending_delta: Vec2::ZERO,
                                        individual: alt_pressed,
                                    };
                                } else if shift_pressed {
                                    // Shift+drag on empty space: rubber-band selection
                                    let start_canvas = renderer.screen_to_canvas(
                                        start_pos.0 as f32, start_pos.1 as f32,
                                    );
                                    let current_canvas = renderer.screen_to_canvas(
                                        cursor_pos.0 as f32, cursor_pos.1 as f32,
                                    );
                                    drag_state = DragState::SelectingRect {
                                        start_canvas,
                                        current_canvas,
                                    };
                                } else {
                                    drag_state = DragState::Panning;
                                    let dx = cursor_pos.0 - prev_pos.0;
                                    let dy = cursor_pos.1 - prev_pos.1;
                                    renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                                }
                            }
                        }
                        DragState::SelectingRect { ref mut current_canvas, .. } => {
                            *current_canvas = renderer.screen_to_canvas(
                                cursor_pos.0 as f32, cursor_pos.1 as f32,
                            );
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
                    // Resolve the pressed key through the configured keybinds.
                    // Convert the winit `Key` into the lowercase string form
                    // that `KeyBind::parse` produces so the comparison is
                    // symmetric.
                    let key_name = match &logical_key {
                        Key::Character(c) => Some(normalize_key_name(c.as_ref())),
                        Key::Named(named) => Some(normalize_key_name(&format!("{:?}", named))),
                        _ => None,
                    };
                    let action = key_name.as_deref().and_then(|k| {
                        keybinds.action_for(k, ctrl_pressed, shift_pressed, alt_pressed)
                    });

                    match action {
                        Some(Action::Undo) => {
                            if let Some(doc) = document.as_mut() {
                                if doc.undo() {
                                    rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                }
                            }
                        }
                        Some(Action::CancelMode) => {
                            if matches!(app_mode, AppMode::Reparent { .. } | AppMode::Connect { .. }) {
                                app_mode = AppMode::Normal;
                                hovered_node = None;
                                if let Some(doc) = document.as_ref() {
                                    rebuild_all_with_mode(
                                        doc, &app_mode, hovered_node.as_deref(),
                                        &mut mindmap_tree, &mut renderer,
                                    );
                                }
                            }
                        }
                        Some(Action::EnterReparentMode) => {
                            if let Some(doc) = document.as_ref() {
                                let sel: Vec<String> = doc.selection.selected_ids()
                                    .iter().map(|s| s.to_string()).collect();
                                if !sel.is_empty() {
                                    app_mode = AppMode::Reparent { sources: sel };
                                    hovered_node = None;
                                    rebuild_all_with_mode(
                                        doc, &app_mode, hovered_node.as_deref(),
                                        &mut mindmap_tree, &mut renderer,
                                    );
                                }
                            }
                        }
                        Some(Action::EnterConnectMode) => {
                            if let Some(doc) = document.as_ref() {
                                if let SelectionState::Single(source) = &doc.selection {
                                    app_mode = AppMode::Connect { source: source.clone() };
                                    hovered_node = None;
                                    rebuild_all_with_mode(
                                        doc, &app_mode, hovered_node.as_deref(),
                                        &mut mindmap_tree, &mut renderer,
                                    );
                                }
                            }
                        }
                        Some(Action::DeleteSelection) => {
                            // Currently wired to edge deletion. Node deletion
                            // is scoped to a future roadmap milestone.
                            if let Some(doc) = document.as_mut() {
                                let maybe_edge_ref = match &doc.selection {
                                    SelectionState::Edge(e) => Some(e.clone()),
                                    _ => None,
                                };
                                if let Some(edge_ref) = maybe_edge_ref {
                                    if let Some((idx, edge)) = doc.remove_edge(&edge_ref) {
                                        doc.undo_stack.push(UndoAction::DeleteEdge {
                                            index: idx,
                                            edge,
                                        });
                                        doc.selection = SelectionState::None;
                                        doc.dirty = true;
                                        rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                    }
                                }
                            }
                        }
                        Some(Action::CreateOrphanNode) => {
                            if let Some(doc) = document.as_mut() {
                                let canvas_pos = renderer.screen_to_canvas(
                                    cursor_pos.0 as f32, cursor_pos.1 as f32,
                                );
                                let new_id = doc.apply_create_orphan_node(canvas_pos);
                                doc.undo_stack.push(UndoAction::CreateNode {
                                    node_id: new_id.clone(),
                                });
                                doc.selection = SelectionState::Single(new_id);
                                doc.dirty = true;
                                rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                            }
                        }
                        Some(Action::OrphanSelection) => {
                            if let Some(doc) = document.as_mut() {
                                let sel: Vec<String> = doc.selection.selected_ids()
                                    .iter().map(|s| s.to_string()).collect();
                                if !sel.is_empty() {
                                    let undo_data = doc.apply_orphan_selection(&sel);
                                    if !undo_data.entries.is_empty() {
                                        doc.undo_stack.push(UndoAction::ReparentNodes {
                                            entries: undo_data.entries,
                                            old_edges: undo_data.old_edges,
                                        });
                                        doc.dirty = true;
                                    }
                                    rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                }
                            }
                        }
                        None => {}
                    }
                }
                Event::AboutToWait => {
                    // Flush any accumulated drag delta (once per frame, not per mouse event)
                    if let DragState::MovingNode { ref node_ids, ref mut pending_delta, ref total_delta, individual, .. } = drag_state {
                        if *pending_delta != Vec2::ZERO {
                            if let Some(tree) = mindmap_tree.as_mut() {
                                for nid in node_ids {
                                    apply_drag_delta(tree, nid, pending_delta.x, pending_delta.y, !individual);
                                }
                                renderer.rebuild_buffers_from_tree(&tree.tree);
                            }

                            // Rebuild connections and borders with position offsets
                            if let Some(doc) = document.as_ref() {
                                let mut offsets: HashMap<String, (f32, f32)> = HashMap::new();
                                let delta = (total_delta.x, total_delta.y);
                                for nid in node_ids {
                                    offsets.insert(nid.clone(), delta);
                                    if !individual {
                                        for desc_id in doc.mindmap.all_descendants(nid) {
                                            offsets.insert(desc_id, delta);
                                        }
                                    }
                                }
                                let scene = doc.build_scene_with_offsets(&offsets);
                                renderer.rebuild_connection_buffers(&scene.connection_elements);
                                renderer.rebuild_border_buffers(&scene.border_elements);
                            }

                            *pending_delta = Vec2::ZERO;
                        }
                    }
                    // Update selection rectangle overlay + preview highlight (once per frame)
                    if let DragState::SelectingRect { start_canvas, current_canvas } = &drag_state {
                        let sc = *start_canvas;
                        let cc = *current_canvas;
                        let min = Vec2::new(sc.x.min(cc.x), sc.y.min(cc.y));
                        let max = Vec2::new(sc.x.max(cc.x), sc.y.max(cc.y));
                        renderer.rebuild_selection_rect_overlay(min, max);

                        // Preview: rebuild tree with intersecting nodes highlighted
                        if let Some(doc) = document.as_ref() {
                            let mut new_tree = doc.build_tree();
                            let hits = rect_select(sc, cc, &new_tree);
                            let preview_selection = match hits.len() {
                                0 => SelectionState::None,
                                1 => SelectionState::Single(hits.into_iter().next().unwrap()),
                                _ => SelectionState::Multi(hits),
                            };
                            apply_selection_highlight(&mut new_tree, &preview_selection);
                            renderer.rebuild_buffers_from_tree(&new_tree.tree);
                            mindmap_tree = Some(new_tree);
                        }
                    }
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

        // Load keybindings from the WASM environment (URL query param or
        // localStorage) with a defaults fallback. Failure is non-fatal —
        // see KeybindConfig::load_for_web().
        self.options.keybind_config =
            crate::application::keybinds::KeybindConfig::load_for_web();
        let _keybinds: ResolvedKeybinds = self.options.keybind_config.resolve();
        // TODO (WASM): once keyboard input is forwarded to the document,
        // dispatch through `_keybinds.action_for(...)` the same way the
        // native path does.

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
            // Handle both "?map=foo" and "?keybinds=...&map=foo" forms.
            let mut map_path: Option<String> = None;
            let trimmed = search.trim_start_matches('?');
            for pair in trimmed.split('&') {
                if let Some(val) = pair.strip_prefix("map=") {
                    map_path = Some(val.to_string());
                }
            }
            map_path.unwrap_or_else(|| self.options.mindmap_path.clone())
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
/// When the current selection is an edge, its `ConnectionElement` gets a
/// cyan color override baked in via `build_scene_with_selection()`.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_all(
    doc: &MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let mut new_tree = doc.build_tree();
    apply_selection_highlight(&mut new_tree, &doc.selection);
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    let scene = doc.build_scene_with_selection();
    renderer.rebuild_connection_buffers(&scene.connection_elements);
    renderer.rebuild_border_buffers(&scene.border_elements);

    *mindmap_tree = Some(new_tree);
}

/// Handle a click event: update selection, rebuild tree with highlight.
/// When the node hit test misses, falls through to edge hit testing so
/// the user can click on a connection path to select it.
#[cfg(not(target_arch = "wasm32"))]
fn handle_click(
    hit: Option<String>,
    cursor_pos: (f64, f64),
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
            // Shift+click: toggle node in/out of multi-selection.
            // Shift+click on an edge selection promotes the clicked node
            // to a fresh single selection (no edge multi-select).
            match &doc.selection {
                SelectionState::None | SelectionState::Edge(_) => {
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
            // Node miss — try edge hit testing before deselecting.
            let canvas_pos = renderer.screen_to_canvas(
                cursor_pos.0 as f32, cursor_pos.1 as f32,
            );
            let tolerance = EDGE_HIT_TOLERANCE_PX * renderer.canvas_per_pixel();
            let edge_hit = hit_test_edge(canvas_pos, &doc.mindmap, tolerance);
            doc.selection = match edge_hit {
                Some(edge_ref) => SelectionState::Edge(edge_ref),
                None => SelectionState::None,
            };
        }
        (None, true) => {
            // Shift+click on empty space: keep current selection (no edge
            // hit test — shift is reserved for multi-node).
        }
    }

    // Rebuild tree with selection highlight applied
    rebuild_all(doc, mindmap_tree, renderer);
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
    renderer: &mut Renderer,
) {
    let mut new_tree = doc.build_tree();
    apply_selection_highlight(&mut new_tree, &doc.selection);
    match app_mode {
        AppMode::Reparent { sources } => {
            // Orange on all source nodes (overrides any cyan selection color).
            apply_reparent_source_highlight(&mut new_tree, sources);
            // Green on the hovered target (if any and not also a source).
            if let Some(h) = hovered_node {
                if !sources.iter().any(|s| s == h) {
                    apply_reparent_target_highlight(&mut new_tree, h);
                }
            }
        }
        AppMode::Connect { source } => {
            // Orange on the source node, green on the hovered target (if
            // it's not the source itself). Reuses the reparent color scheme.
            apply_reparent_source_highlight(&mut new_tree, std::slice::from_ref(source));
            if let Some(h) = hovered_node {
                if h != source {
                    apply_reparent_target_highlight(&mut new_tree, h);
                }
            }
        }
        AppMode::Normal => {}
    }
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    let scene = doc.build_scene_with_selection();
    renderer.rebuild_connection_buffers(&scene.connection_elements);
    renderer.rebuild_border_buffers(&scene.border_elements);

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
        rebuild_all(doc, mindmap_tree, renderer);
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
        rebuild_all(doc, mindmap_tree, renderer);
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
