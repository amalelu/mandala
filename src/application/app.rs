use std::collections::HashMap;
use std::sync::{Arc, RwLock};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
use pollster::block_on;
use glam::Vec2;
use indextree::Arena;
use wgpu::{Instance, SurfaceTargetUnsafe};
use winit::event::{ElementState, Event, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ControlFlow;
use winit::keyboard::Key;
use winit::window::CursorIcon;
use winit::{event_loop::EventLoop, window::Window};

use crate::application::common::{InputMode, RenderDecree, WindowMode};
use crate::application::document::{
    EdgeRef, MindMapDocument, SelectionState, UndoAction,
    hit_test, hit_test_edge, rect_select,
    apply_selection_highlight, apply_drag_delta,
    apply_reparent_source_highlight, apply_reparent_target_highlight,
};
use crate::application::frame_throttle::MutationFrequencyThrottle;
use crate::application::keybinds::{Action, ResolvedKeybinds, normalize_key_name};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::palette::{
    PaletteContext, PaletteEffects, filter_actions, PALETTE_ACTIONS,
};
use crate::application::renderer::Renderer;

use baumhard::gfx_structs::element::GfxElement;
#[cfg(not(target_arch = "wasm32"))]
use baumhard::mindmap::custom_mutation::{PlatformContext, Trigger};

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

/// Session 6C command palette state. When open, all keyboard input
/// is routed to the palette (query editing, navigation, execute,
/// close) and regular hotkeys are suppressed until it closes.
#[cfg(not(target_arch = "wasm32"))]
enum PaletteState {
    Closed,
    Open {
        query: String,
        /// Indices into `PALETTE_ACTIONS`, sorted by fuzzy score
        /// descending. Rebuilt on every keystroke.
        filtered: Vec<usize>,
        /// Which row of `filtered` is highlighted; wraps via
        /// Up/Down.
        selected: usize,
    },
}

#[cfg(not(target_arch = "wasm32"))]
impl PaletteState {
    fn is_open(&self) -> bool {
        matches!(self, PaletteState::Open { .. })
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
        // Phase 4(B) keyed incremental rebuild: the document-side cache
        // of per-edge pre-clip sample geometry. Populated lazily by
        // `build_scene_with_cache`; cleared by `rebuild_all` so any
        // structural change to the map forces a fresh scene build.
        // Lives in the event loop state next to `mindmap_tree` so its
        // lifetime matches the interactive session. It is a pure view
        // optimisation — nothing about the model depends on it.
        let mut scene_cache = baumhard::mindmap::scene_cache::SceneConnectionCache::new();

        match MindMapDocument::load(&self.options.mindmap_path) {
            Ok(doc) => {
                // Canvas background: resolve through theme
                // variables so `"var(--bg)"` works, then hand off
                // to the renderer as the render-pass clear color.
                // Replaces the previously-hardcoded black clear.
                let vars = &doc.mindmap.canvas.theme_variables;
                let resolved_bg = baumhard::util::color::resolve_var(
                    &doc.mindmap.canvas.background_color,
                    vars,
                );
                renderer.set_clear_color_from_hex(resolved_bg);

                // Nodes: build Baumhard tree from MindMap hierarchy
                let tree = doc.build_tree();
                renderer.rebuild_buffers_from_tree(&tree.tree);
                renderer.fit_camera_to_tree(&tree.tree);

                // Connections + borders: flat pipeline from RenderScene.
                // `fit_camera_to_tree` above settled the zoom, so pass
                // it through — the scene builder sizes connection
                // glyphs against the actual final zoom rather than the
                // default-init value.
                let scene = doc.build_scene(renderer.camera_zoom());
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
        let mut palette_state = PaletteState::Closed;
        let mut hovered_node: Option<String> = None;
        let mut shift_pressed = false;
        let mut alt_pressed = false;
        let mut ctrl_pressed = false;
        // True while the cursor is hovering a node with any trigger
        // bindings (a "button"). Tracked so we only call set_cursor on
        // transitions instead of every CursorMoved event.
        let mut cursor_is_hand = false;

        // Phase 4(E): the governing-invariant throttle. Per-frame
        // work in the drag path feeds its measured duration into this
        // tracker; when the moving average crosses the refresh
        // budget, `should_drain()` starts returning false on some
        // frames, coalescing multiple ticks' worth of pending delta
        // into a single drain. Responsiveness is preserved because
        // input accumulation keeps running every tick (the mouse
        // cursor never lags; only the dragged node briefly does).
        // The throttle is reset at drag end so a fresh drag starts
        // with n = 1.
        let mut mutation_throttle = MutationFrequencyThrottle::with_default_budget();

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
                    // Session 6C: the palette swallows mouse clicks
                    // as a close gesture. Clicking anywhere while
                    // open dismisses the palette without running
                    // an action, mirroring Escape.
                    if palette_state.is_open() && state == ElementState::Pressed {
                        palette_state = PaletteState::Closed;
                        renderer.rebuild_palette_overlay_buffers(None);
                        return;
                    }
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
                                let canvas_pos = renderer.screen_to_canvas(
                                    cursor_pos.0 as f32,
                                    cursor_pos.1 as f32,
                                );
                                let hit_node = mindmap_tree.as_ref().and_then(|tree| {
                                    hit_test(canvas_pos, tree)
                                });
                                // If an edge is currently selected, check
                                // whether the cursor is over one of its
                                // grab-handles. This check has precedence
                                // over the node hit at threshold-cross
                                // time — see the `Pending` → drag
                                // transition below. Returns `None` if no
                                // edge is selected, nothing is in range,
                                // or the hit test infrastructure isn't
                                // ready yet.
                                let hit_edge_handle = match document.as_ref() {
                                    Some(doc) => match &doc.selection {
                                        SelectionState::Edge(er) => {
                                            let tol = EDGE_HANDLE_HIT_TOLERANCE_PX
                                                * renderer.canvas_per_pixel();
                                            doc.hit_test_edge_handle(canvas_pos, er, tol)
                                                .map(|(kind, _pos)| (er.clone(), kind))
                                        }
                                        _ => None,
                                    },
                                    None => None,
                                };
                                drag_state = DragState::Pending {
                                    start_pos: cursor_pos,
                                    hit_node,
                                    hit_edge_handle,
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
                                        // Flush any remaining pending delta to the tree before drop.
                                        // This always runs regardless of the throttle — on release
                                        // we want the final position committed in full, even if
                                        // the throttle was mid-stretch skipping intermediate drains.
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
                                        // Drag ended — reset the throttle so the next drag
                                        // starts at n = 1 without inheriting any residual
                                        // throttling from this one.
                                        mutation_throttle.reset();
                                    }
                                    DragState::DraggingEdgeHandle { edge_ref, handle, original, start_handle_pos, total_delta, pending_delta: _ } => {
                                        // The drain loop has been writing
                                        // each new edge state directly
                                        // into the model. Before release,
                                        // flush one last write using the
                                        // full `total_delta` (independent
                                        // of any throttled pending drain)
                                        // so the final committed state
                                        // matches the cursor position
                                        // exactly. Reaching this branch
                                        // means the drag threshold was
                                        // crossed, so push an EditEdge
                                        // undo with the pre-drag snapshot
                                        // unconditionally.
                                        if let Some(doc) = document.as_mut() {
                                            apply_edge_handle_drag(
                                                doc,
                                                &edge_ref,
                                                handle,
                                                start_handle_pos,
                                                total_delta,
                                            );
                                            if let Some(idx) = doc.edge_index(&edge_ref) {
                                                doc.undo_stack.push(UndoAction::EditEdge {
                                                    index: idx,
                                                    before: original,
                                                });
                                                doc.dirty = true;
                                            }
                                            rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                        }
                                        mutation_throttle.reset();
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

                    // Hand cursor over button-like nodes (nodes with any
                    // trigger bindings). Only recomputed when idle — during
                    // a drag the cursor should stay as-is.
                    if matches!(drag_state, DragState::None) {
                        let over_button = match (document.as_ref(), mindmap_tree.as_ref()) {
                            (Some(doc), Some(tree)) => {
                                let canvas_pos = renderer.screen_to_canvas(
                                    cursor_pos.0 as f32, cursor_pos.1 as f32,
                                );
                                hit_test(canvas_pos, tree)
                                    .and_then(|id| doc.mindmap.nodes.get(&id))
                                    .map(|n| !n.trigger_bindings.is_empty())
                                    .unwrap_or(false)
                            }
                            _ => false,
                        };
                        if over_button != cursor_is_hand {
                            self.window.set_cursor(if over_button {
                                CursorIcon::Pointer
                            } else {
                                CursorIcon::Default
                            });
                            cursor_is_hand = over_button;
                        }
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
                        DragState::DraggingEdgeHandle { total_delta, pending_delta, .. } => {
                            // Same accumulation pattern as MovingNode —
                            // actual edge mutation + buffer rebuild
                            // happens in `AboutToWait`. Keeps the
                            // CursorMoved path cheap so fast mouse
                            // motion doesn't bottleneck on scene builds.
                            let old_canvas = renderer.screen_to_canvas(prev_pos.0 as f32, prev_pos.1 as f32);
                            let new_canvas = renderer.screen_to_canvas(cursor_pos.0 as f32, cursor_pos.1 as f32);
                            let delta = new_canvas - old_canvas;

                            *total_delta += delta;
                            *pending_delta += delta;
                        }
                        DragState::Pending { start_pos, hit_node, hit_edge_handle } => {
                            let dist_x = cursor_pos.0 - start_pos.0;
                            let dist_y = cursor_pos.1 - start_pos.1;
                            if dist_x * dist_x + dist_y * dist_y > 25.0 {
                                // Past threshold — decide what kind of drag
                                // this is. Handle grabs take precedence over
                                // node hits so that clicking a control-point
                                // handle that happens to overlap a node
                                // still enters the reshape flow.
                                if let Some((edge_ref, handle_kind)) = hit_edge_handle.take() {
                                    // Grab the pre-edit snapshot + start
                                    // position so the drain loop can do
                                    // absolute-positioning math.
                                    if let Some(doc) = document.as_mut() {
                                        if let Some(original) = doc
                                            .mindmap
                                            .edges
                                            .iter()
                                            .find(|e| edge_ref.matches(e))
                                            .cloned()
                                        {
                                            let canvas_pos = renderer.screen_to_canvas(
                                                start_pos.0 as f32,
                                                start_pos.1 as f32,
                                            );
                                            let start_handle_pos = doc
                                                .hit_test_edge_handle(
                                                    canvas_pos,
                                                    &edge_ref,
                                                    f32::INFINITY,
                                                )
                                                .map(|(_, p)| p)
                                                .unwrap_or(canvas_pos);
                                            // Phase 4(B) invariant: a new
                                            // drag starts with a clean
                                            // scene cache — see the
                                            // MovingNode branch for the
                                            // full rationale.
                                            scene_cache.clear();
                                            drag_state = DragState::DraggingEdgeHandle {
                                                edge_ref,
                                                handle: handle_kind,
                                                original,
                                                start_handle_pos,
                                                total_delta: Vec2::ZERO,
                                                pending_delta: Vec2::ZERO,
                                            };
                                            return;
                                        }
                                    }
                                }
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
                                    // Phase 4(B): start each drag with a
                                    // clean scene cache. Between drags the
                                    // model may have changed structurally
                                    // (undo, reparent, edge CRUD, node
                                    // edit) and cached entries would not
                                    // reflect the current map. Clearing
                                    // here means the first drag frame
                                    // re-populates the cache from scratch
                                    // and subsequent frames get the
                                    // incremental-rebuild benefit.
                                    scene_cache.clear();
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

                    // Session 6C: when the palette is open, it steals
                    // all keyboard input. Character keys go into the
                    // query, Up/Down navigate, Enter executes, Escape
                    // closes. Regular hotkeys are suppressed until
                    // the palette closes.
                    if palette_state.is_open() {
                        handle_palette_key(
                            &key_name,
                            &logical_key,
                            &mut palette_state,
                            &mut document,
                            &mut mindmap_tree,
                            &mut renderer,
                            &mut scene_cache,
                        );
                        return;
                    }

                    // Opening the palette is a pre-action lookup:
                    // `/` with no modifiers opens it regardless of
                    // what the keybinds layer says (we don't want a
                    // user to accidentally rebind away their only
                    // discovery path).
                    if key_name.as_deref() == Some("/")
                        && !ctrl_pressed && !alt_pressed
                    {
                        let ctx = document.as_ref().map(|doc| PaletteContext { document: doc });
                        if let Some(ctx) = ctx {
                            let filtered = filter_actions("", &ctx);
                            palette_state = PaletteState::Open {
                                query: String::new(),
                                filtered,
                                selected: 0,
                            };
                            if let Some(doc) = document.as_ref() {
                                rebuild_palette_overlay(
                                    &palette_state, doc, &mut renderer,
                                );
                            }
                        }
                        return;
                    }

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
                    // Flush any accumulated drag delta (once per frame, not per mouse event),
                    // gated by the mutation-frequency throttle. When the moving
                    // average of this block's work duration exceeds the refresh
                    // budget, `should_drain()` starts returning false on some
                    // frames — `pending_delta` stays intact and the next
                    // successful drain folds in whatever motion arrived in the
                    // meantime. This holds the governing invariant:
                    // responsiveness is preserved at the cost of briefer
                    // chunking in the visual update cadence.
                    if let DragState::MovingNode { ref node_ids, ref mut pending_delta, ref total_delta, individual, .. } = drag_state {
                        if *pending_delta != Vec2::ZERO && mutation_throttle.should_drain() {
                            let work_start = Instant::now();

                            if let Some(tree) = mindmap_tree.as_mut() {
                                for nid in node_ids {
                                    apply_drag_delta(tree, nid, pending_delta.x, pending_delta.y, !individual);
                                }
                                renderer.rebuild_buffers_from_tree(&tree.tree);
                            }

                            // Rebuild connections and borders with position offsets.
                            //
                            // Phase 4(B): use the cache-aware scene build so
                            // only edges whose endpoints appear in `offsets`
                            // get re-sampled. The renderer's keyed rebuild
                            // methods then only re-shape the buffers for
                            // dirty elements; stable elements have just
                            // their `pos` patched in place.
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

                                // First frame of a drag? The scene_cache
                                // was cleared at drag start so it is
                                // still empty. Skip the incremental-
                                // rebuild path and do a FULL renderer
                                // rebuild: this guarantees the renderer's
                                // per-edge buffer map matches the new
                                // offset-applied scene exactly, rather
                                // than reusing pre-drag shaped buffers
                                // whose cap visibility / glyph count
                                // might not align with the moved edge's
                                // new layout. After this first frame,
                                // the scene_cache is populated and the
                                // incremental path below kicks in.
                                let first_frame_of_drag = scene_cache.is_empty();

                                // The dirty edge set = every edge touching
                                // any moved node. The scene cache holds the
                                // reverse index, so this is O(sum of edges-
                                // per-moved-node), not O(edges in map).
                                let mut dirty_edge_keys: std::collections::HashSet<
                                    baumhard::mindmap::scene_cache::EdgeKey
                                > = std::collections::HashSet::new();
                                for nid in offsets.keys() {
                                    for k in scene_cache.edges_touching(nid) {
                                        dirty_edge_keys.insert(k.clone());
                                    }
                                }
                                let dirty_node_ids: std::collections::HashSet<String> =
                                    offsets.keys().cloned().collect();

                                let scene = doc.build_scene_with_cache(
                                    &offsets,
                                    &mut scene_cache,
                                    renderer.camera_zoom(),
                                );

                                if first_frame_of_drag {
                                    // `None` = treat every element as
                                    // dirty → full re-shape of the keyed
                                    // maps. One-time per drag; subsequent
                                    // frames are incremental.
                                    renderer.rebuild_connection_buffers_keyed(
                                        &scene.connection_elements,
                                        None,
                                    );
                                    renderer.rebuild_border_buffers_keyed(
                                        &scene.border_elements,
                                        None,
                                    );
                                } else {
                                    renderer.rebuild_connection_buffers_keyed(
                                        &scene.connection_elements,
                                        Some(&dirty_edge_keys),
                                    );
                                    renderer.rebuild_border_buffers_keyed(
                                        &scene.border_elements,
                                        Some(&dirty_node_ids),
                                    );
                                }
                            }

                            *pending_delta = Vec2::ZERO;
                            mutation_throttle.record_work_duration(work_start.elapsed());
                        }
                    }
                    // Session 6C edge-handle drag drain. Mirrors the
                    // MovingNode drain above but writes the edge in
                    // place instead of moving nodes. The scene cache
                    // is invalidated for the single dirty edge so the
                    // next build re-samples just that edge; everything
                    // else rides the incremental rebuild path.
                    if let DragState::DraggingEdgeHandle {
                        ref edge_ref,
                        ref mut handle,
                        ref mut pending_delta,
                        ref total_delta,
                        ref start_handle_pos,
                        ..
                    } = drag_state {
                        if *pending_delta != Vec2::ZERO && mutation_throttle.should_drain() {
                            let work_start = Instant::now();
                            if let Some(doc) = document.as_mut() {
                                let new_handle = apply_edge_handle_drag(
                                    doc,
                                    edge_ref,
                                    *handle,
                                    *start_handle_pos,
                                    *total_delta,
                                );
                                *handle = new_handle;

                                let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
                                    &edge_ref.from_id,
                                    &edge_ref.to_id,
                                    &edge_ref.edge_type,
                                );
                                scene_cache.invalidate_edge(&edge_key);

                                let offsets: HashMap<String, (f32, f32)> = HashMap::new();
                                let mut dirty_edge_keys: std::collections::HashSet<
                                    baumhard::mindmap::scene_cache::EdgeKey
                                > = std::collections::HashSet::new();
                                dirty_edge_keys.insert(edge_key);

                                let scene = doc.build_scene_with_cache(
                                    &offsets,
                                    &mut scene_cache,
                                    renderer.camera_zoom(),
                                );
                                renderer.rebuild_connection_buffers_keyed(
                                    &scene.connection_elements,
                                    Some(&dirty_edge_keys),
                                );
                                renderer.rebuild_edge_handle_buffers(&scene.edge_handles);
                            }
                            *pending_delta = Vec2::ZERO;
                            mutation_throttle.record_work_duration(work_start.elapsed());
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

                    // Camera (pan/zoom/resize) changed → rebuild
                    // connection buffers against the new viewport. On
                    // zoom, the document-side scene cache is also stale
                    // because effective font size depends on zoom, so
                    // clear it before the rebuild re-samples.
                    //
                    // Skipped when a node drag is in progress: the
                    // MovingNode branch above rebuilds with the drag
                    // offsets on its next non-zero `pending_delta` using
                    // the current camera, and rebuilding here with
                    // empty offsets would flicker dragged connections
                    // back to their pre-drag positions for one frame.
                    // Wheel-zoom during an active drag with zero
                    // `pending_delta` leaves connections stale for one
                    // frame until the next mouse-move flush — an
                    // acceptable tradeoff to keep the two dirty sources
                    // separate. Always take the flags (even when
                    // skipped) so they don't leak across drag frames.
                    let geometry_dirty = renderer.take_connection_geometry_dirty();
                    let viewport_dirty = renderer.take_connection_viewport_dirty();
                    if (geometry_dirty || viewport_dirty)
                        && !matches!(drag_state, DragState::MovingNode { .. })
                    {
                        if let Some(doc) = document.as_ref() {
                            if geometry_dirty {
                                // `ensure_zoom` inside
                                // `build_scene_with_cache` would also
                                // catch this, but clearing explicitly
                                // here keeps the ordering readable
                                // next to the rebuild.
                                scene_cache.clear();
                            }
                            let scene = doc.build_scene_with_cache(
                                &HashMap::new(),
                                &mut scene_cache,
                                renderer.camera_zoom(),
                            );
                            renderer.rebuild_connection_buffers_keyed(
                                &scene.connection_elements,
                                None, // treat all as dirty; buffer cache was cleared
                            );
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

                // Connections + borders: flat pipeline from RenderScene.
                // `fit_camera_to_tree` above settled the zoom, so pass
                // it through for correct glyph sizing.
                let scene = document.build_scene(renderer.camera_zoom());
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
/// Handle a keystroke while the command palette is open. Character
/// keys append to the query (and re-filter), Backspace pops, Up/Down
/// navigate the filtered list, Enter runs the selected action, and
/// Escape closes without running anything. Regular hotkeys are
/// suppressed — this runs entirely outside the keybinds resolver.
#[cfg(not(target_arch = "wasm32"))]
fn handle_palette_key(
    key_name: &Option<String>,
    logical_key: &Key,
    palette_state: &mut PaletteState,
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let name = match key_name.as_deref() {
        Some(n) => n,
        None => return,
    };
    match name {
        "escape" => {
            *palette_state = PaletteState::Closed;
            renderer.rebuild_palette_overlay_buffers(None);
        }
        "enter" => {
            let (id_to_run, ran) = {
                if let PaletteState::Open { filtered, selected, .. } = palette_state {
                    if let Some(idx) = filtered.get(*selected).copied() {
                        (Some(PALETTE_ACTIONS[idx].id), true)
                    } else {
                        (None, false)
                    }
                } else {
                    (None, false)
                }
            };
            if ran {
                if let Some(doc) = document.as_mut() {
                    if let Some(id) = id_to_run {
                        if let Some((_, action)) = crate::application::palette::action_by_id(id) {
                            let mut effects = PaletteEffects { document: doc };
                            (action.execute)(&mut effects);
                            scene_cache.clear();
                            rebuild_all(doc, mindmap_tree, renderer);
                        }
                    }
                }
            }
            *palette_state = PaletteState::Closed;
            renderer.rebuild_palette_overlay_buffers(None);
        }
        "arrowup" | "up" => {
            if let PaletteState::Open { filtered, selected, .. } = palette_state {
                if !filtered.is_empty() && *selected > 0 {
                    *selected -= 1;
                }
            }
            if let Some(doc) = document.as_ref() {
                rebuild_palette_overlay(palette_state, doc, renderer);
            }
        }
        "arrowdown" | "down" => {
            if let PaletteState::Open { filtered, selected, .. } = palette_state {
                if !filtered.is_empty() && *selected + 1 < filtered.len() {
                    *selected += 1;
                }
            }
            if let Some(doc) = document.as_ref() {
                rebuild_palette_overlay(palette_state, doc, renderer);
            }
        }
        "backspace" => {
            if let PaletteState::Open { query, filtered, selected, .. } = palette_state {
                query.pop();
                if let Some(doc) = document.as_ref() {
                    let ctx = PaletteContext { document: doc };
                    *filtered = filter_actions(query, &ctx);
                    *selected = 0;
                    rebuild_palette_overlay(palette_state, doc, renderer);
                }
            }
        }
        _ => {
            // Character input: append to the query. Only accept
            // single-char keys to avoid stuffing "ArrowLeft" or
            // "Control" into the query text.
            if let Key::Character(c) = logical_key {
                if let PaletteState::Open { query, filtered, selected, .. } = palette_state {
                    query.push_str(c.as_ref());
                    if let Some(doc) = document.as_ref() {
                        let ctx = PaletteContext { document: doc };
                        *filtered = filter_actions(query, &ctx);
                        *selected = 0;
                        rebuild_palette_overlay(palette_state, doc, renderer);
                    }
                }
            }
        }
    }
}

/// Build the palette overlay geometry from the current state and
/// push it to the renderer for glyph-rendering. Called whenever the
/// palette opens, the query changes, or the selected row changes.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_palette_overlay(
    palette_state: &PaletteState,
    _document: &MindMapDocument,
    renderer: &mut Renderer,
) {
    use crate::application::renderer::{PaletteOverlayGeometry, PaletteOverlayRow};
    let (query, filtered, selected) = match palette_state {
        PaletteState::Closed => {
            renderer.rebuild_palette_overlay_buffers(None);
            return;
        }
        PaletteState::Open { query, filtered, selected } => (query, filtered, *selected),
    };
    let rows: Vec<PaletteOverlayRow> = filtered
        .iter()
        .map(|&idx| {
            let a = &PALETTE_ACTIONS[idx];
            PaletteOverlayRow {
                label: a.label.to_string(),
                description: a.description.to_string(),
            }
        })
        .collect();
    let geometry = PaletteOverlayGeometry {
        query_text: query.clone(),
        rows,
        selected_row: selected,
    };
    renderer.rebuild_palette_overlay_buffers(Some(&geometry));
}

/// Apply a full edge-handle drag to the document model in place —
/// writes the new control point / anchor into
/// `doc.mindmap.edges[idx]` based on the current cursor delta.
/// Called every frame during the drag and once more at release to
/// commit the final position. Mutates `handle` in place when a
/// `Midpoint` handle crosses over into a fresh control point so
/// subsequent frames take the `ControlPoint(0)` path.
#[cfg(not(target_arch = "wasm32"))]
fn apply_edge_handle_drag(
    doc: &mut MindMapDocument,
    edge_ref: &EdgeRef,
    handle: baumhard::mindmap::scene_builder::EdgeHandleKind,
    start_handle_pos: Vec2,
    total_delta: Vec2,
) -> baumhard::mindmap::scene_builder::EdgeHandleKind {
    use baumhard::mindmap::model::ControlPoint;
    use baumhard::mindmap::scene_builder::EdgeHandleKind;

    let idx = match doc.edge_index(edge_ref) {
        Some(i) => i,
        None => return handle,
    };
    let (from_center, to_center) = {
        let edge = &doc.mindmap.edges[idx];
        let from_node = match doc.mindmap.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => return handle,
        };
        let to_node = match doc.mindmap.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => return handle,
        };
        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);
        (
            Vec2::new(from_pos.x + from_size.x * 0.5, from_pos.y + from_size.y * 0.5),
            Vec2::new(to_pos.x + to_size.x * 0.5, to_pos.y + to_size.y * 0.5),
        )
    };
    let new_handle_canvas = start_handle_pos + total_delta;

    let edge = &mut doc.mindmap.edges[idx];
    match handle {
        EdgeHandleKind::ControlPoint(i) => {
            let center = if i == 0 { from_center } else { to_center };
            let offset = new_handle_canvas - center;
            while edge.control_points.len() <= i {
                edge.control_points.push(ControlPoint { x: 0.0, y: 0.0 });
            }
            edge.control_points[i] = ControlPoint {
                x: offset.x as f64,
                y: offset.y as f64,
            };
            EdgeHandleKind::ControlPoint(i)
        }
        EdgeHandleKind::Midpoint => {
            // First drained frame of a "curve this line" gesture:
            // insert a single control point (quadratic Bezier,
            // offset from source node center) at the new cursor
            // position. Subsequent frames promote to ControlPoint(0).
            let offset = new_handle_canvas - from_center;
            edge.control_points.clear();
            edge.control_points.push(ControlPoint {
                x: offset.x as f64,
                y: offset.y as f64,
            });
            EdgeHandleKind::ControlPoint(0)
        }
        EdgeHandleKind::AnchorFrom => {
            // Pick the side of from_node whose midpoint is closest to
            // the new cursor position. Value in 1..=4 (top/right/
            // bottom/left) — never 0 (auto) during manual drag.
            let from_node = doc.mindmap.nodes.get(&edge.from_id).unwrap();
            let node_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
            let node_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
            edge.anchor_from = nearest_anchor_side(new_handle_canvas, node_pos, node_size);
            EdgeHandleKind::AnchorFrom
        }
        EdgeHandleKind::AnchorTo => {
            let to_node = doc.mindmap.nodes.get(&edge.to_id).unwrap();
            let node_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
            let node_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);
            edge.anchor_to = nearest_anchor_side(new_handle_canvas, node_pos, node_size);
            EdgeHandleKind::AnchorTo
        }
    }
}

/// Given a canvas-space position and a node's AABB, return the
/// anchor code (1=top, 2=right, 3=bottom, 4=left) of the edge
/// midpoint closest to the position. Used by the anchor-handle
/// drag to snap the anchor to whichever side the cursor is nearest.
#[cfg(not(target_arch = "wasm32"))]
fn nearest_anchor_side(point: Vec2, node_pos: Vec2, node_size: Vec2) -> i32 {
    let half_w = node_size.x * 0.5;
    let half_h = node_size.y * 0.5;
    let top = Vec2::new(node_pos.x + half_w, node_pos.y);
    let right = Vec2::new(node_pos.x + node_size.x, node_pos.y + half_h);
    let bottom = Vec2::new(node_pos.x + half_w, node_pos.y + node_size.y);
    let left = Vec2::new(node_pos.x, node_pos.y + half_h);
    let candidates = [(1, top), (2, right), (3, bottom), (4, left)];
    candidates
        .iter()
        .min_by(|a, b| {
            let da = a.1.distance_squared(point);
            let db = b.1.distance_squared(point);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(code, _)| *code)
        .unwrap_or(0)
}

fn rebuild_all(
    doc: &MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let mut new_tree = doc.build_tree();
    apply_selection_highlight(&mut new_tree, &doc.selection);
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    let scene = doc.build_scene_with_selection(renderer.camera_zoom());
    renderer.rebuild_connection_buffers(&scene.connection_elements);
    renderer.rebuild_border_buffers(&scene.border_elements);
    renderer.rebuild_edge_handle_buffers(&scene.edge_handles);

    *mindmap_tree = Some(new_tree);
}

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
                if let Some(tree) = mindmap_tree.as_mut() {
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

    let scene = doc.build_scene_with_selection(renderer.camera_zoom());
    renderer.rebuild_connection_buffers(&scene.connection_elements);
    renderer.rebuild_border_buffers(&scene.border_elements);
    renderer.rebuild_edge_handle_buffers(&scene.edge_handles);

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
