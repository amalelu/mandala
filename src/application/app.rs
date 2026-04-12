use std::collections::HashMap;
use std::sync::{Arc, RwLock};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
use pollster::block_on;

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
    apply_drag_delta, apply_tree_highlights,
    HIGHLIGHT_COLOR, REPARENT_SOURCE_COLOR, REPARENT_TARGET_COLOR,
};
use crate::application::frame_throttle::MutationFrequencyThrottle;
use crate::application::keybinds::{Action, ResolvedKeybinds};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::palette::{
    PaletteContext, PaletteEffects, filter_actions, PALETTE_ACTIONS,
};
use crate::application::renderer::Renderer;

use baumhard::gfx_structs::element::GfxElement;
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

/// Session 6D: inline-edit state for a connection's label. When
/// `Open`, all keyboard input is routed to the label-edit handler
/// (just like `PaletteState::Open` captures keys for the palette
/// query). Mutually exclusive with `PaletteState::Open` — the
/// palette check runs first, so opening the palette while editing a
/// label is a no-op.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
enum LabelEditState {
    Closed,
    Open {
        edge_ref: crate::application::document::EdgeRef,
        /// The in-progress buffer. Committed to
        /// `MindEdge.label` on Enter; discarded on Escape.
        buffer: String,
        /// The edge's label value at the moment edit mode opened.
        /// Used to restore state on Escape so the cancel is clean.
        original: Option<String>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
impl LabelEditState {
    fn is_open(&self) -> bool {
        matches!(self, LabelEditState::Open { .. })
    }
}

/// Session 7A: inline multi-line text editor for a node. Entered via
/// double-click on a node (or on empty canvas, which creates a new
/// orphan and opens the editor on it). Key input is routed to
/// `handle_text_edit_key` before the normal keybind dispatch, so
/// Tab/Enter/etc. become literal character inserts while typing.
///
/// Commit is via click-outside-the-edited-node; Esc cancels. The
/// `buffer` is the transient in-progress text; `cursor_char_pos` is a
/// char-index offset within `buffer`. The transient edits flow
/// through Baumhard's `Mutation::AreaDelta` vocabulary applied to the
/// live tree — the model is untouched until commit.
#[derive(Debug, Clone)]
enum TextEditState {
    Closed,
    Open {
        node_id: String,
        /// The in-progress multi-line buffer.
        buffer: String,
        /// Cursor position as a grapheme-cluster index into `buffer`.
        /// Valid range `[0, count_grapheme_clusters(buffer)]`. Stored
        /// in graphemes (not chars or bytes) so backspace over an
        /// emoji or ZWJ cluster removes the whole user-visible
        /// character — see `CODE_CONVENTIONS.md §2`/`§B2`.
        cursor_grapheme_pos: usize,
    },
}

impl TextEditState {
    fn is_open(&self) -> bool {
        matches!(self, TextEditState::Open { .. })
    }
    fn node_id(&self) -> Option<&str> {
        match self {
            TextEditState::Open { node_id, .. } => Some(node_id.as_str()),
            TextEditState::Closed => None,
        }
    }
}

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

/// Session 7A: glyph rendered at the cursor position while a node
/// text editor is open. Reuses the same caret as `LabelEditState`.
const TEXT_EDIT_CARET: char = '\u{258C}';

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

// Session 7A text-edit cursor helpers.
//
// These all operate on **grapheme-cluster indices** (not chars or
// bytes), routing through `baumhard::util::grapheme_chad`. This is
// what `CODE_CONVENTIONS.md §2` and `§B2` mandate for any code that
// touches user-typed text — char indexing splits emoji and combining
// marks mid-cluster, leaving a corrupted buffer the next time the
// renderer shapes it.
//
// For ASCII-only buffers grapheme indices coincide with char indices,
// which is why the existing test suite still passes unchanged.

/// Insert one character at grapheme index `cursor` in `buffer`,
/// returning the new cursor position (one grapheme past the insert).
fn insert_at_cursor(buffer: &mut String, cursor: usize, ch: char) -> usize {
    grapheme_chad::insert_str_at_grapheme(buffer, cursor, &ch.to_string());
    cursor + 1
}

/// Delete the grapheme cluster immediately before `cursor` (Backspace
/// semantics). Returns the new cursor position. No-op at `cursor == 0`.
fn delete_before_cursor(buffer: &mut String, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    grapheme_chad::delete_grapheme_at(buffer, cursor - 1);
    cursor - 1
}

/// Delete the grapheme cluster at `cursor` (Delete semantics). Returns
/// the unchanged cursor position. No-op at end of buffer.
fn delete_at_cursor(buffer: &mut String, cursor: usize) -> usize {
    let total = grapheme_chad::count_grapheme_clusters(buffer);
    if cursor >= total {
        return cursor;
    }
    grapheme_chad::delete_grapheme_at(buffer, cursor);
    cursor
}

/// Return the grapheme index of the start of the line containing
/// `cursor` — i.e. the position just after the most recent `\n`
/// strictly before `cursor`, or 0 if no prior `\n`. `\n` is always its
/// own grapheme cluster, so walking by graphemes is correct here.
fn cursor_to_line_start(buffer: &str, cursor: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    let mut line_start = 0usize;
    for (i, g) in buffer.graphemes(true).enumerate() {
        if i >= cursor {
            break;
        }
        if g == "\n" {
            line_start = i + 1;
        }
    }
    line_start
}

/// Return the grapheme index of the end of the line containing
/// `cursor` — the position of the next `\n` at or after `cursor`, or
/// the total grapheme count if no `\n` follows.
fn cursor_to_line_end(buffer: &str, cursor: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    let mut total = 0usize;
    for (i, g) in buffer.graphemes(true).enumerate() {
        total = i + 1;
        if i >= cursor && g == "\n" {
            return i;
        }
    }
    total
}

/// Move the cursor up one line, preserving the visual column. Column
/// is computed as `cursor - line_start` in graphemes; the new
/// position lands at `prev_line_start + min(col, prev_line_len)`.
/// No-op if already on the first line.
fn move_cursor_up_line(buffer: &str, cursor: usize) -> usize {
    let line_start = cursor_to_line_start(buffer, cursor);
    if line_start == 0 {
        return cursor;
    }
    // Move to the grapheme just before the '\n' that terminates the previous line.
    let prev_line_end = line_start - 1;
    let prev_line_start = cursor_to_line_start(buffer, prev_line_end);
    let col = cursor - line_start;
    let prev_line_len = prev_line_end - prev_line_start;
    prev_line_start + col.min(prev_line_len)
}

/// Move the cursor down one line, preserving the visual column.
/// No-op if already on the last line.
fn move_cursor_down_line(buffer: &str, cursor: usize) -> usize {
    let total = grapheme_chad::count_grapheme_clusters(buffer);
    let line_start = cursor_to_line_start(buffer, cursor);
    let line_end = cursor_to_line_end(buffer, cursor);
    if line_end == total {
        return cursor;
    }
    let next_line_start = line_end + 1;
    let next_line_end = cursor_to_line_end(buffer, next_line_start);
    let col = cursor - line_start;
    let next_line_len = next_line_end - next_line_start;
    next_line_start + col.min(next_line_len)
}

/// Build the display text for the edited node by inserting the caret
/// glyph at the cursor's grapheme position. Used on every keystroke
/// to produce the `Mutation::AreaDelta` payload.
fn insert_caret(buffer: &str, cursor: usize) -> String {
    let byte = grapheme_chad::find_byte_index_of_grapheme(buffer, cursor)
        .unwrap_or(buffer.len());
    let mut out = String::with_capacity(buffer.len() + TEXT_EDIT_CARET.len_utf8());
    out.push_str(&buffer[..byte]);
    out.push(TEXT_EDIT_CARET);
    out.push_str(&buffer[byte..]);
    out
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
                renderer.rebuild_connection_label_buffers(&scene.connection_label_elements);
                renderer.rebuild_portal_buffers(&scene.portal_elements);

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
        let mut label_edit_state = LabelEditState::Closed;
        let mut text_edit_state = TextEditState::Closed;
        let mut color_picker_state =
            crate::application::color_picker::ColorPickerState::Closed;
        // Session 7A: tracks the previous left-click-down for
        // double-click detection. Cleared after a double-click fires.
        let mut last_click: Option<LastClick> = None;
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
                    // Glyph-wheel color picker caches its layout in
                    // ColorPickerState::Open { layout, .. }; the
                    // cached values include the screen-space backdrop
                    // and per-glyph positions, so a resize would
                    // leave hit-tests aimed at the old geometry and
                    // the renderer's overlay buffers anchored at the
                    // pre-resize coordinates. Re-emit both off the
                    // new surface dimensions so picker stays usable
                    // after a resize. Mutually exclusive with the
                    // palette + label edit modals so only one branch
                    // ever fires.
                    if color_picker_state.is_open() {
                        if let Some(doc) = document.as_ref() {
                            rebuild_color_picker_overlay(
                                &mut color_picker_state,
                                doc,
                                &mut renderer,
                            );
                        }
                    }
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

                    // Glyph-wheel color picker click handling. The
                    // picker captures left-click for in-frame hits
                    // (commit at hit position) and out-of-backdrop
                    // clicks (cancel). The CursorMoved branch above
                    // unconditionally early-returns while the picker
                    // is open, so middle-button pan is suppressed
                    // for the duration of the modal — matching the
                    // palette and label-edit modals' behavior.
                    if color_picker_state.is_open()
                        && state == ElementState::Pressed
                        && button == MouseButton::Left
                    {
                        if let Some(doc) = document.as_mut() {
                            handle_color_picker_click(
                                cursor_pos,
                                &mut color_picker_state,
                                doc,
                                &mut mindmap_tree,
                                &mut renderer,
                            );
                        }
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
                                    // Session 7A: mode-exit via target
                                    // click — clear any stale click so
                                    // the first post-mode click can't
                                    // be paired into a double-click.
                                    last_click = None;
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
                                    last_click = None;
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

                                // Session 7A: double-click detection.
                                // If this press within the double-click
                                // window matches the previous one (same
                                // hit target, within time + distance),
                                // open the node text editor — double-click
                                // on a node edits it; on empty space (and
                                // no edge/portal selected) creates a new
                                // orphan and edits that.
                                //
                                // Guard: if the editor is already open on
                                // the same hit target, DO NOT re-open it
                                // — that would silently discard the
                                // in-progress buffer. Let the press fall
                                // through; the corresponding release
                                // will be swallowed as click-inside.
                                let now = now_ms();
                                let already_editing_same_target = text_edit_state
                                    .node_id()
                                    .map(|id| hit_node.as_deref() == Some(id))
                                    .unwrap_or(false);
                                let is_dblclick = !already_editing_same_target
                                    && last_click
                                        .as_ref()
                                        .map(|prev| is_double_click(prev, now, cursor_pos, &hit_node))
                                        .unwrap_or(false);
                                if is_dblclick {
                                    last_click = None;
                                    if let Some(ref node_id) = hit_node {
                                        if let Some(doc) = document.as_mut() {
                                            let nid = node_id.clone();
                                            doc.selection = SelectionState::Single(nid.clone());
                                            // rebuild_all first so the
                                            // selection highlight color
                                            // regions are applied to the
                                            // tree. open_text_edit's
                                            // subsequent apply_text_edit_to_tree
                                            // only touches the Text field
                                            // of the target node's
                                            // GlyphArea (via
                                            // DeltaGlyphArea's selective
                                            // field application) so the
                                            // highlight regions survive
                                            // untouched. If you ever add
                                            // more fields to the caret
                                            // delta, revisit this.
                                            rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                            open_text_edit(
                                                &nid,
                                                false,
                                                doc,
                                                &mut text_edit_state,
                                                &mut mindmap_tree,
                                                &mut renderer,
                                            );
                                        }
                                        return;
                                    } else {
                                        // Empty space: only create an
                                        // orphan if no edge/portal was
                                        // selected (otherwise the user
                                        // was probably aiming at the
                                        // selected edge/portal).
                                        let allow_create = document
                                            .as_ref()
                                            .map(|d| !matches!(
                                                d.selection,
                                                SelectionState::Edge(_) | SelectionState::Portal(_)
                                            ))
                                            .unwrap_or(false);
                                        if allow_create {
                                            if let Some(doc) = document.as_mut() {
                                                let new_id = doc.apply_create_orphan_node(canvas_pos);
                                                doc.undo_stack.push(UndoAction::CreateNode { node_id: new_id.clone() });
                                                doc.selection = SelectionState::Single(new_id.clone());
                                                doc.dirty = true;
                                                rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                                open_text_edit(
                                                    &new_id,
                                                    true,
                                                    doc,
                                                    &mut text_edit_state,
                                                    &mut mindmap_tree,
                                                    &mut renderer,
                                                );
                                            }
                                            return;
                                        }
                                    }
                                }
                                last_click = Some(LastClick {
                                    time: now,
                                    screen_pos: cursor_pos,
                                    hit: hit_node.clone(),
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
                                        // Session 7A: if the node text
                                        // editor is open, the release
                                        // decides whether to commit or
                                        // swallow. If the release lands
                                        // inside the edited node's AABB,
                                        // keep editing (no commit, no
                                        // selection change). Otherwise
                                        // commit and fall through.
                                        if text_edit_state.is_open() {
                                            let release_canvas = renderer.screen_to_canvas(
                                                cursor_pos.0 as f32,
                                                cursor_pos.1 as f32,
                                            );
                                            let inside = text_edit_state
                                                .node_id()
                                                .zip(mindmap_tree.as_ref())
                                                .map(|(id, tree)| {
                                                    crate::application::document::point_in_node_aabb(
                                                        release_canvas, id, tree,
                                                    )
                                                })
                                                .unwrap_or(false);
                                            if inside {
                                                // Click-inside: keep
                                                // editing. Do NOT fall
                                                // through to handle_click
                                                // (that would change the
                                                // selection). Also do
                                                // not transition drag
                                                // state — the release
                                                // is fully consumed.
                                                return;
                                            }
                                            // Click-outside: commit the
                                            // edit first, then fall
                                            // through to the regular
                                            // click path so the new
                                            // selection lands.
                                            if let Some(doc) = document.as_mut() {
                                                close_text_edit(
                                                    true,
                                                    doc,
                                                    &mut text_edit_state,
                                                    &mut mindmap_tree,
                                                    &mut renderer,
                                                );
                                            }
                                        }
                                        // Session 6D: if an edge is selected and
                                        // the cursor hits its label, open the
                                        // inline label editor instead of
                                        // processing a regular click. Takes
                                        // precedence over node / edge selection.
                                        let mut entered_label_edit = false;
                                        if hit_node.is_none() {
                                            // First, a read-only check to see
                                            // whether we should even call the
                                            // editor (hits the selected edge's
                                            // label AABB). Split from the
                                            // `open_label_edit` call so the
                                            // mutable borrow of `document`
                                            // doesn't conflict with the
                                            // immutable read.
                                            let label_edit_target: Option<crate::application::document::EdgeRef> =
                                                if let Some(doc) = document.as_ref() {
                                                    if let SelectionState::Edge(er) = &doc.selection {
                                                        let canvas_pos = renderer.screen_to_canvas(
                                                            cursor_pos.0 as f32,
                                                            cursor_pos.1 as f32,
                                                        );
                                                        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
                                                            &er.from_id,
                                                            &er.to_id,
                                                            &er.edge_type,
                                                        );
                                                        if renderer.hit_test_edge_label(canvas_pos, &edge_key) {
                                                            Some(er.clone())
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                } else {
                                                    None
                                                };
                                            if let Some(er_clone) = label_edit_target {
                                                if let Some(doc) = document.as_mut() {
                                                    open_label_edit(
                                                        &er_clone,
                                                        doc,
                                                        &mut label_edit_state,
                                                        &mut renderer,
                                                    );
                                                    entered_label_edit = true;
                                                }
                                            }
                                        }
                                        if !entered_label_edit {
                                            handle_click(
                                                hit_node,
                                                cursor_pos,
                                                shift_pressed,
                                                &mut document,
                                                &mut mindmap_tree,
                                                &mut renderer,
                                            );
                                        }
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

                    // Glyph-wheel color picker hover preview. Routes
                    // mouse-over to the picker hit-test, updates the
                    // current HSV in place, and lives-previews the
                    // change on the affected edge/portal.
                    if color_picker_state.is_open() {
                        if let Some(doc) = document.as_mut() {
                            handle_color_picker_mouse_move(
                                cursor_pos,
                                &mut color_picker_state,
                                doc,
                                &mut mindmap_tree,
                                &mut renderer,
                            );
                        }
                        return;
                    }

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
                                                apply_tree_highlights(
                                                    &mut new_tree,
                                                    doc.selection
                                                        .selected_ids()
                                                        .into_iter()
                                                        .map(|id| (id, HIGHLIGHT_COLOR)),
                                                );
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
                    let key_name = crate::application::keybinds::key_to_name(&logical_key);

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
                            &mut label_edit_state,
                            &mut color_picker_state,
                            &mut document,
                            &mut mindmap_tree,
                            &mut renderer,
                            &mut scene_cache,
                        );
                        return;
                    }

                    // Glyph-wheel color picker key handling. Mutually
                    // exclusive with palette and label-edit. Steals
                    // all keyboard input the same way: Esc cancels,
                    // Enter commits, Tab cycles theme chips, h/s/v
                    // nudge HSV, character keys are otherwise ignored.
                    if color_picker_state.is_open() {
                        if let Some(doc) = document.as_mut() {
                            handle_color_picker_key(
                                &key_name,
                                &logical_key,
                                &mut color_picker_state,
                                doc,
                                &mut mindmap_tree,
                                &mut renderer,
                            );
                        }
                        return;
                    }

                    // Session 6D: inline label edit modal. Steals keys
                    // the same way the palette does. Escape discards,
                    // Enter commits, Backspace pops, character keys
                    // append.
                    if label_edit_state.is_open() {
                        if let Some(doc) = document.as_mut() {
                            handle_label_edit_key(
                                &key_name,
                                &logical_key,
                                &mut label_edit_state,
                                doc,
                                &mut mindmap_tree,
                                &mut renderer,
                            );
                        }
                        return;
                    }

                    // Session 7A: inline node text editor. Steals keys
                    // the same way the palette / label-edit modals do.
                    // Enter and Tab are literal characters inside the
                    // editor — this is a multi-line paragraph editor,
                    // not an outliner. Esc cancels; commit is via
                    // click-outside in the mouse handler.
                    if text_edit_state.is_open() {
                        if let Some(doc) = document.as_mut() {
                            handle_text_edit_key(
                                &key_name,
                                &logical_key,
                                &mut text_edit_state,
                                doc,
                                &mut mindmap_tree,
                                &mut renderer,
                            );
                        }
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
                                // Session 7A: clear any stale click so a
                                // post-mode click doesn't get retroactively
                                // paired with a pre-mode click into a
                                // spurious double-click.
                                last_click = None;
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
                                    last_click = None;
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
                                    last_click = None;
                                    rebuild_all_with_mode(
                                        doc, &app_mode, hovered_node.as_deref(),
                                        &mut mindmap_tree, &mut renderer,
                                    );
                                }
                            }
                        }
                        Some(Action::DeleteSelection) => {
                            // Session 7A follow-up: node deletion is now
                            // supported alongside edge and portal
                            // deletion. Deleting a node orphans its
                            // immediate children (they become roots) and
                            // removes every edge that touched the node.
                            if let Some(doc) = document.as_mut() {
                                enum DelKind {
                                    Edge(crate::application::document::EdgeRef),
                                    Portal(crate::application::document::PortalRef),
                                    Node(String),
                                    Nodes(Vec<String>),
                                }
                                let kind = match &doc.selection {
                                    SelectionState::Edge(e) => Some(DelKind::Edge(e.clone())),
                                    SelectionState::Portal(p) => Some(DelKind::Portal(p.clone())),
                                    SelectionState::Single(id) => Some(DelKind::Node(id.clone())),
                                    SelectionState::Multi(ids) => Some(DelKind::Nodes(ids.clone())),
                                    SelectionState::None => None,
                                };
                                match kind {
                                    Some(DelKind::Edge(edge_ref)) => {
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
                                    Some(DelKind::Portal(pref)) => {
                                        // `apply_delete_portal` records the
                                        // DeletePortal undo entry internally;
                                        // we just clear selection + rebuild.
                                        if doc.apply_delete_portal(&pref).is_some() {
                                            doc.selection = SelectionState::None;
                                            rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                        }
                                    }
                                    Some(DelKind::Node(id)) => {
                                        if let Some(undo) = doc.delete_node(&id) {
                                            doc.undo_stack.push(undo);
                                            doc.selection = SelectionState::None;
                                            rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                        }
                                    }
                                    Some(DelKind::Nodes(ids)) => {
                                        // Multi-select delete: push one
                                        // undo entry per node so Ctrl+Z
                                        // unwinds them in reverse order.
                                        // Each `delete_node` call is
                                        // self-contained — if a later id
                                        // happens to be a child of an
                                        // earlier one, the earlier delete
                                        // already orphaned it, so the
                                        // later delete just removes the
                                        // (now-root) node.
                                        let mut any = false;
                                        for id in ids {
                                            if let Some(undo) = doc.delete_node(&id) {
                                                doc.undo_stack.push(undo);
                                                any = true;
                                            }
                                        }
                                        if any {
                                            doc.selection = SelectionState::None;
                                            rebuild_all(doc, &mut mindmap_tree, &mut renderer);
                                        }
                                    }
                                    None => {}
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
                        Some(Action::EditSelection) => {
                            // Session 7A follow-up: open the text editor
                            // on the selected single node with its
                            // existing text, cursor at end. The
                            // text-editor steal at the top of the
                            // keyboard dispatch (`text_edit_state.is_open()`
                            // branch above) means this can't fire while
                            // the editor is already open, so Enter-inside-
                            // editor stays literal.
                            if let Some(doc) = document.as_mut() {
                                let target = if let SelectionState::Single(id) = &doc.selection {
                                    Some(id.clone())
                                } else {
                                    None
                                };
                                if let Some(id) = target {
                                    open_text_edit(
                                        &id,
                                        false,
                                        doc,
                                        &mut text_edit_state,
                                        &mut mindmap_tree,
                                        &mut renderer,
                                    );
                                }
                            }
                        }
                        Some(Action::EditSelectionClean) => {
                            // Session 7A follow-up: open the editor with
                            // an empty buffer. On commit, `set_node_text`
                            // replaces the node's text wholesale and
                            // pushes an `EditNodeText` undo entry — no
                            // new undo variant needed.
                            if let Some(doc) = document.as_mut() {
                                let target = if let SelectionState::Single(id) = &doc.selection {
                                    Some(id.clone())
                                } else {
                                    None
                                };
                                if let Some(id) = target {
                                    open_text_edit(
                                        &id,
                                        true,
                                        doc,
                                        &mut text_edit_state,
                                        &mut mindmap_tree,
                                        &mut renderer,
                                    );
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
                                // Labels are emitted per frame (not
                                // cached) so their positions track the
                                // live drag.
                                renderer.rebuild_connection_label_buffers(
                                    &scene.connection_label_elements,
                                );
                                // Portal markers also track the live drag.
                                renderer.rebuild_portal_buffers(&scene.portal_elements);
                                // Edge handles (anchor / midpoint /
                                // control-point ◆ glyphs on a selected
                                // edge) must also track the live drag.
                                // Without this the handles stay pinned
                                // to the pre-drag positions until mouse
                                // release triggers a full rebuild.
                                renderer.rebuild_edge_handle_buffers(&scene.edge_handles);
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
                                // Labels are rebuilt per frame so a
                                // control-point drag keeps the label
                                // correctly anchored to the live path.
                                renderer.rebuild_connection_label_buffers(
                                    &scene.connection_label_elements,
                                );
                                renderer.rebuild_portal_buffers(&scene.portal_elements);
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
                            apply_tree_highlights(
                                &mut new_tree,
                                preview_selection
                                    .selected_ids()
                                    .into_iter()
                                    .map(|id| (id, HIGHLIGHT_COLOR)),
                            );
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
                            renderer.rebuild_connection_label_buffers(
                                &scene.connection_label_elements,
                            );
                            renderer.rebuild_portal_buffers(&scene.portal_elements);
                            // Edge handles (if an edge is selected) must
                            // also follow camera changes — scroll-wheel
                            // zoom with a selected edge used to leave
                            // the handles pinned to stale screen
                            // positions until the next full rebuild.
                            renderer.rebuild_edge_handle_buffers(&scene.edge_handles);
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

        let gfx_arena: Arc<RwLock<Arena<GfxElement>>> = Arc::new(RwLock::new(Arena::new()));
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
                gfx_arena.clone(),
            )
            .await;

            let size = canvas.width();
            let height = canvas.height();
            renderer.process_decree(RenderDecree::SetSurfaceSize(size, height));

            // Load mindmap through Document -> Tree + Scene -> Renderer flow
            let mut doc_opt: Option<MindMapDocument> = None;
            let mut tree_opt: Option<MindMapTree> = None;
            if let Ok(doc) = MindMapDocument::load(&mindmap_path) {
                let mindmap_tree = doc.build_tree();
                renderer.rebuild_buffers_from_tree(&mindmap_tree.tree);
                renderer.fit_camera_to_tree(&mindmap_tree.tree);

                let scene = doc.build_scene(renderer.camera_zoom());
                renderer.rebuild_connection_buffers(&scene.connection_elements);
                renderer.rebuild_border_buffers(&scene.border_elements);
                renderer.rebuild_connection_label_buffers(&scene.connection_label_elements);
                renderer.rebuild_portal_buffers(&scene.portal_elements);
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

                    // Text editor keyboard-steal: if open, route all keys
                    // to the editor. This mirrors the native cascade at
                    // ~line 1348.
                    let editor_is_open = {
                        let borrow = input_for_events.borrow();
                        borrow.as_ref().map(|s| s.text_edit_state.is_open()).unwrap_or(false)
                    };
                    if editor_is_open {
                        let mut input_borrow = input_for_events.borrow_mut();
                        let mut renderer_borrow = renderer_for_events.borrow_mut();
                        if let (Some(input), Some(renderer)) =
                            (input_borrow.as_mut(), renderer_borrow.as_mut())
                        {
                            handle_text_edit_key(
                                &key_name,
                                logical_key,
                                &mut input.text_edit_state,
                                &mut input.document,
                                &mut input.mindmap_tree,
                                renderer,
                            );
                            suppress_for_events.set(input.text_edit_state.is_open());
                        }
                        return;
                    }
                    // Full hotkey dispatch via keybinds deferred to a
                    // later WASM-parity session.
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
                                rebuild_all(&input.document, &mut input.mindmap_tree, renderer);
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
                                    let new_id = input.document.apply_create_orphan_node(canvas_pos);
                                    input.document.undo_stack.push(UndoAction::CreateNode { node_id: new_id.clone() });
                                    input.document.selection = SelectionState::Single(new_id.clone());
                                    input.document.dirty = true;
                                    rebuild_all(&input.document, &mut input.mindmap_tree, renderer);
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
                            rebuild_all(&input.document, &mut input.mindmap_tree, renderer);
                        }
                    }
                }

                Event::WindowEvent {
                    event: WindowEvent::MouseWheel { .. }, ..
                } => {
                    // Zoom deferred to a later WASM-parity session.
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
    label_edit_state: &mut LabelEditState,
    color_picker_state: &mut crate::application::color_picker::ColorPickerState,
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
                            let mut effects = PaletteEffects {
                                document: doc,
                                open_label_edit: None,
                                open_color_picker: None,
                            };
                            (action.execute)(&mut effects);
                            let label_edit_req = effects.open_label_edit.take();
                            let color_picker_req = effects.open_color_picker.take();
                            scene_cache.clear();
                            rebuild_all(doc, mindmap_tree, renderer);
                            // Session 6D: if the action asked to open the
                            // inline label editor (e.g. "Edit connection
                            // label"), transition to the label-edit modal.
                            if let Some(er) = label_edit_req {
                                open_label_edit(
                                    &er,
                                    doc,
                                    label_edit_state,
                                    renderer,
                                );
                            }
                            // Glyph-wheel color picker handoff: a
                            // "Pick * color…" palette action sets
                            // `open_color_picker = Some(target)`. We
                            // drain it after the regular rebuild and
                            // transition to the picker modal.
                            if let Some(target) = color_picker_req {
                                open_color_picker(
                                    target,
                                    doc,
                                    color_picker_state,
                                    renderer,
                                );
                            }
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
    apply_tree_highlights(
        &mut new_tree,
        doc.selection
            .selected_ids()
            .into_iter()
            .map(|id| (id, HIGHLIGHT_COLOR)),
    );
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    rebuild_scene_only(doc, renderer);

    *mindmap_tree = Some(new_tree);
}

/// Narrower cousin of `rebuild_all` that rebuilds only the flat
/// scene pipeline (connections, borders, edge handles, labels,
/// portals) — NOT the tree (node text buffers, node backgrounds).
/// Used by the glyph-wheel color picker's hover path: a per-frame
/// color preview doesn't change node text, borders, or positions,
/// so the tree rebuild is wasted work. Halves the hot-path cost vs
/// `rebuild_all` on maps with many nodes.
fn rebuild_scene_only(doc: &MindMapDocument, renderer: &mut Renderer) {
    let scene = doc.build_scene_with_selection(renderer.camera_zoom());
    renderer.rebuild_connection_buffers(&scene.connection_elements);
    renderer.rebuild_border_buffers(&scene.border_elements);
    renderer.rebuild_edge_handle_buffers(&scene.edge_handles);
    renderer.rebuild_connection_label_buffers(&scene.connection_label_elements);
    renderer.rebuild_portal_buffers(&scene.portal_elements);
}

/// Session 6D: transition into inline label edit mode for the given
/// edge. Seeds the buffer from the edge's current label (or the
/// empty string) and installs a preview override on the renderer so
/// the caret shows up immediately. Callers must ensure the edge
/// still exists in `doc.mindmap.edges` — the function silently
/// returns otherwise.
#[cfg(not(target_arch = "wasm32"))]
fn open_label_edit(
    edge_ref: &crate::application::document::EdgeRef,
    doc: &mut MindMapDocument,
    label_edit_state: &mut LabelEditState,
    renderer: &mut Renderer,
) {
    let edge = match doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)) {
        Some(e) => e,
        None => return,
    };
    let original = edge.label.clone();
    let buffer = original.clone().unwrap_or_default();
    *label_edit_state = LabelEditState::Open {
        edge_ref: edge_ref.clone(),
        buffer: buffer.clone(),
        original,
    };
    // Store the preview on the document so every subsequent
    // `doc.build_scene_*` call picks it up automatically — no renderer
    // field, no read-time override, no belt-and-suspenders branch.
    let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
        &edge_ref.from_id,
        &edge_ref.to_id,
        &edge_ref.edge_type,
    );
    doc.label_edit_preview = Some((edge_key, buffer));
    // Rebuild labels so the caret is visible immediately. The caller
    // already ran `rebuild_all` before this, so the scene is fresh.
    let scene = doc.build_scene_with_selection(renderer.camera_zoom());
    renderer.rebuild_connection_label_buffers(&scene.connection_label_elements);
    renderer.rebuild_portal_buffers(&scene.portal_elements);
}

/// Session 6D: route a keystroke to the inline label editor. Escape
/// discards, Enter commits, Backspace pops the last grapheme,
/// character keys append. Mirrors the `handle_palette_key` pattern.
/// Updates the renderer's preview override on every keystroke so the
/// caret and the edited text render live.
#[cfg(not(target_arch = "wasm32"))]
fn handle_label_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    label_edit_state: &mut LabelEditState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let name = key_name.as_deref();
    match name {
        Some("escape") => {
            close_label_edit(false, doc, label_edit_state, mindmap_tree, renderer);
            return;
        }
        Some("enter") => {
            close_label_edit(true, doc, label_edit_state, mindmap_tree, renderer);
            return;
        }
        Some("backspace") => {
            if let LabelEditState::Open { buffer, .. } = label_edit_state {
                buffer.pop();
            }
        }
        _ => {
            // Append a single-character printable keystroke. Ignore
            // modifier keys, arrow keys, etc. — cursor navigation is
            // deferred to a future session.
            if let Key::Character(c) = logical_key {
                // winit's `Key::Character` may report multi-char
                // sequences on dead keys / IME; accept anything that
                // isn't a control character.
                for ch in c.as_str().chars() {
                    if !ch.is_control() {
                        if let LabelEditState::Open { buffer, .. } = label_edit_state {
                            buffer.push(ch);
                        }
                    }
                }
            } else {
                return;
            }
        }
    }

    // Refresh the preview on the document so the caret + edited text
    // render on the next frame. Cheap: one scene rebuild. The scene
    // builder picks up the new buffer through `doc.label_edit_preview`.
    if let LabelEditState::Open { edge_ref, buffer, .. } = label_edit_state {
        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
            &edge_ref.from_id,
            &edge_ref.to_id,
            &edge_ref.edge_type,
        );
        doc.label_edit_preview = Some((edge_key, buffer.clone()));
        let scene = doc.build_scene_with_selection(renderer.camera_zoom());
        renderer.rebuild_connection_label_buffers(&scene.connection_label_elements);
        renderer.rebuild_portal_buffers(&scene.portal_elements);
    }
}

/// Session 6D: close the inline label editor. If `commit` is true,
/// writes the current buffer into the edge's label (via
/// `document.set_edge_label`) and pushes an undo entry. If false,
/// restores the pre-edit label (equivalent to discarding the buffer)
/// — no undo push because we never mutated the model during typing.
#[cfg(not(target_arch = "wasm32"))]
fn close_label_edit(
    commit: bool,
    doc: &mut MindMapDocument,
    label_edit_state: &mut LabelEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let (edge_ref, buffer, original) = match std::mem::replace(label_edit_state, LabelEditState::Closed) {
        LabelEditState::Open { edge_ref, buffer, original } => (edge_ref, buffer, original),
        LabelEditState::Closed => return,
    };
    doc.label_edit_preview = None;
    if commit {
        let new_val = if buffer.is_empty() { None } else { Some(buffer) };
        // Only push undo if the committed value actually differs from the
        // pre-edit original — avoids a dead undo entry on unchanged text.
        if new_val != original {
            doc.set_edge_label(&edge_ref, new_val);
        }
    }
    // Rebuild so the label reflects the model state (or vanishes if
    // the buffer was empty + original was None).
    rebuild_all(doc, mindmap_tree, renderer);
}

// =====================================================================
// Session 7A: inline node text editor
// =====================================================================

/// Open the text editor on the given node. Seeds the buffer (empty if
/// `from_creation`, else the node's current text), and pushes the
/// initial caret through the Baumhard mutation pipeline so the live
/// tree shows the cursor on the next frame.
///
/// No snapshot of the node's pre-edit text is stored on
/// `TextEditState`: the model is untouched during typing, so the
/// model itself *is* the pre-edit state. `set_node_text` takes its
/// own "before" snapshot at commit time, and cancel just rebuilds
/// the tree from the unchanged model.
fn open_text_edit(
    node_id: &str,
    from_creation: bool,
    doc: &mut MindMapDocument,
    text_edit_state: &mut TextEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let current_text = match doc.mindmap.nodes.get(node_id) {
        Some(n) => n.text.clone(),
        None => return,
    };
    let buffer = if from_creation { String::new() } else { current_text };
    let cursor_grapheme_pos = grapheme_chad::count_grapheme_clusters(&buffer);
    *text_edit_state = TextEditState::Open {
        node_id: node_id.to_string(),
        buffer: buffer.clone(),
        cursor_grapheme_pos,
    };
    // Push the initial (caret-only for creation, or "existing text +
    // caret at end" for edit) through the Baumhard mutation pipeline.
    apply_text_edit_to_tree(
        node_id,
        &buffer,
        cursor_grapheme_pos,
        mindmap_tree,
        renderer,
    );
}

/// Session 7A: commit or cancel the open text editor. Commit writes
/// the final buffer back to the model via `set_node_text`, which is
/// the single source of truth for "did the text change" — it
/// snapshots `before_text`/`before_runs` and only pushes an undo
/// entry if the value actually differs. Cancel just calls
/// `rebuild_all`, which rebuilds the tree from the untouched model —
/// the transient caret-bearing tree state is discarded wholesale.
fn close_text_edit(
    commit: bool,
    doc: &mut MindMapDocument,
    text_edit_state: &mut TextEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let (node_id, buffer) = match std::mem::replace(text_edit_state, TextEditState::Closed) {
        TextEditState::Open { node_id, buffer, .. } => (node_id, buffer),
        TextEditState::Closed => return,
    };
    if commit {
        // `set_node_text` is a no-op on unchanged text and handles
        // its own undo push.
        doc.set_node_text(&node_id, buffer);
    }
    // Full rebuild pulls the tree back to the model — any transient
    // caret-bearing mutations on the live tree are discarded.
    rebuild_all(doc, mindmap_tree, renderer);
}

/// Session 7A: push the current (`buffer`, `cursor`) state into the
/// live Baumhard tree via a `Mutation::AreaDelta { text: Assign }`
/// targeting the edited node's GlyphArea. This is the "utilize
/// Baumhard" path — the buffer is transient UI state on the app
/// layer, but every visual frame goes through the existing
/// `Mutation::apply_to_area` vocabulary. The renderer's text buffers
/// are rebuilt from the mutated tree so the next frame reflects the
/// keystroke.
fn apply_text_edit_to_tree(
    node_id: &str,
    buffer: &str,
    cursor_grapheme_pos: usize,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use baumhard::core::primitives::{
        Applicable, ApplyOperation, ColorFontRegion, ColorFontRegions, Range,
    };

    let tree = match mindmap_tree.as_mut() {
        Some(t) => t,
        None => return,
    };
    let indextree_node_id = match tree.node_map.get(node_id) {
        Some(id) => *id,
        None => return,
    };
    // Grab a mutable handle to the target node's GlyphArea.
    let element = tree.tree.arena.get_mut(indextree_node_id);
    let element = match element {
        Some(n) => n.get_mut(),
        None => return,
    };
    let area = match element.glyph_area_mut() {
        Some(a) => a,
        None => return,
    };

    // Build the display text: buffer with caret glyph inserted at
    // the cursor's grapheme position. This is what cosmic-text will
    // shape.
    let display_text = insert_caret(buffer, cursor_grapheme_pos);

    // Inherit the color of the first existing region so edited text
    // matches the pre-edit styling. Fall back to `None` (renderer
    // default) if there are no regions. `rebuild_buffers_from_tree`
    // only draws characters that fall inside at least one region, so
    // the replacement region has to span the *entire* new text —
    // including the trailing caret glyph — or the caret and any
    // just-typed characters past the original text length would be
    // silently dropped by the span filter (renderer.rs:1500-1520).
    // This was the root cause of the "double-click existing node does
    // nothing" bug: the old regions were left in place, so the caret
    // at char position == old_len was outside every region and never
    // rendered.
    let inherited_color = area
        .regions
        .all_regions()
        .first()
        .and_then(|r| r.color);
    let display_char_count = display_text.chars().count();
    let mut new_regions = ColorFontRegions::new_empty();
    if display_char_count > 0 {
        new_regions.submit_region(ColorFontRegion::new(
            Range::new(0, display_char_count),
            None,
            inherited_color,
        ));
    }

    // Construct the Baumhard delta: Text + ColorFontRegions + Assign.
    // The Assign operation replaces both fields wholesale — see
    // `GlyphArea::apply_operation` at area.rs:261 for regions and
    // area.rs:273 for text.
    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Text(display_text),
        GlyphAreaField::ColorFontRegions(new_regions),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    delta.apply_to(area);

    // Re-shape the node buffers off the mutated tree. This is the
    // existing tree-render path, reused.
    renderer.rebuild_buffers_from_tree(&tree.tree);
}

/// Session 7A: route a keystroke to the open node text editor. All
/// keys are stolen from normal keybind dispatch — Tab and Enter
/// produce literal characters, Esc cancels, arrows/Home/End navigate,
/// Backspace/Delete delete, and printable chars are inserted at the
/// cursor. Every successful mutation is pushed through
/// `apply_text_edit_to_tree` so the tree and renderer stay in sync.
fn handle_text_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    text_edit_state: &mut TextEditState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let name = key_name.as_deref();
    if name == Some("escape") {
        close_text_edit(false, doc, text_edit_state, mindmap_tree, renderer);
        return;
    }

    let (node_id, buffer, cursor) = match text_edit_state {
        TextEditState::Open {
            node_id,
            buffer,
            cursor_grapheme_pos,
            ..
        } => (node_id, buffer, cursor_grapheme_pos),
        TextEditState::Closed => return,
    };

    let mut changed = false;
    match name {
        Some("backspace") => {
            if *cursor > 0 {
                *cursor = delete_before_cursor(buffer, *cursor);
                changed = true;
            }
        }
        Some("delete") => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor = delete_at_cursor(buffer, *cursor);
                changed = true;
            }
        }
        Some("arrowleft") => {
            if *cursor > 0 {
                *cursor -= 1;
                changed = true;
            }
        }
        Some("arrowright") => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor += 1;
                changed = true;
            }
        }
        Some("arrowup") => {
            let new_cursor = move_cursor_up_line(buffer, *cursor);
            if new_cursor != *cursor {
                *cursor = new_cursor;
                changed = true;
            }
        }
        Some("arrowdown") => {
            let new_cursor = move_cursor_down_line(buffer, *cursor);
            if new_cursor != *cursor {
                *cursor = new_cursor;
                changed = true;
            }
        }
        Some("home") => {
            let new_cursor = cursor_to_line_start(buffer, *cursor);
            if new_cursor != *cursor {
                *cursor = new_cursor;
                changed = true;
            }
        }
        Some("end") => {
            let new_cursor = cursor_to_line_end(buffer, *cursor);
            if new_cursor != *cursor {
                *cursor = new_cursor;
                changed = true;
            }
        }
        Some("enter") => {
            *cursor = insert_at_cursor(buffer, *cursor, '\n');
            changed = true;
        }
        Some("tab") => {
            *cursor = insert_at_cursor(buffer, *cursor, '\t');
            changed = true;
        }
        _ => {
            // Printable character: accept each non-control char. Mirrors
            // `handle_label_edit_key` at app.rs ~line 1929.
            if let Key::Character(c) = logical_key {
                for ch in c.as_str().chars() {
                    if !ch.is_control() {
                        *cursor = insert_at_cursor(buffer, *cursor, ch);
                        changed = true;
                    }
                }
            }
        }
    }

    if changed {
        // Text editing only mutates the live tree during typing; the
        // model is untouched until commit (click-outside) or rolled
        // back on cancel (Esc). We clone node_id + buffer to release
        // the mutable borrow on `text_edit_state` before calling
        // `apply_text_edit_to_tree`, which wants its own mutable
        // borrow on `mindmap_tree`.
        let node_id_owned = node_id.clone();
        let buffer_owned = buffer.clone();
        let cursor_snapshot = *cursor;
        apply_text_edit_to_tree(
            &node_id_owned,
            &buffer_owned,
            cursor_snapshot,
            mindmap_tree,
            renderer,
        );
    }
}

// =====================================================================
// Glyph-wheel color picker handlers
// =====================================================================

/// Open the color picker on the given target. Resolves the target ref
/// to a concrete `(kind, index)` pair, captures a pre-picker `UndoAction`
/// snapshot so cancel can restore in place and commit can push a single
/// undo entry, and seeds the picker HSV from the target's currently-
/// displayed color. Mirrors the snapshot pattern in
/// `apply_edge_handle_drag` (line ~520) where in-place mutation during
/// interaction commits with one undo on release.
#[cfg(not(target_arch = "wasm32"))]
fn open_color_picker(
    target: crate::application::color_picker::ColorTarget,
    doc: &mut MindMapDocument,
    state: &mut crate::application::color_picker::ColorPickerState,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{
        current_hsv_at, ARM_BOTTOM_GLYPHS, ARM_LEFT_GLYPHS, ARM_RIGHT_GLYPHS,
        ARM_TOP_GLYPHS, ColorPickerState, HUE_RING_FONT_SCALE, HUE_RING_GLYPHS,
    };

    // Resolve the ref to a (kind, index) up front. If the edge/portal
    // was deleted between the palette opening and Enter being pressed,
    // warn and bail — the modal never opens. Should never happen
    // because the palette holds the event loop, but the defensive
    // check is observable via the log rather than silently swallowed.
    let (kind, target_index) = match target.resolve(doc) {
        Some(pair) => pair,
        None => {
            log::warn!("color picker: target ref did not resolve; ignoring open");
            return;
        }
    };

    // Seed HSV from the currently-displayed (possibly theme-resolved)
    // color so the picker opens right where the user already is.
    let (hue_deg, sat, val) = current_hsv_at(doc, kind, target_index);

    // Measure the widest shaped advance across every crosshair-arm
    // glyph and every hue-ring glyph. These become the spacing units
    // the layout pure-fn uses for cell and ring-slot positions —
    // measuring once here avoids per-hover font-system traffic and
    // keeps `compute_color_picker_layout` pure. Both measurements
    // happen behind the font-system write lock, which is also what
    // the renderer's buffer builders need, so we grab it once.
    let base_font_size: f32 = 16.0;
    let ring_font_size = base_font_size * HUE_RING_FONT_SCALE;
    let (max_cell_advance, max_ring_advance) = {
        let mut font_system = baumhard::font::fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");
        let mut crosshair: Vec<&str> = Vec::with_capacity(40);
        crosshair.extend(ARM_TOP_GLYPHS.iter().copied());
        crosshair.extend(ARM_BOTTOM_GLYPHS.iter().copied());
        crosshair.extend(ARM_LEFT_GLYPHS.iter().copied());
        crosshair.extend(ARM_RIGHT_GLYPHS.iter().copied());
        let cell = crate::application::renderer::measure_max_glyph_advance(
            &mut font_system,
            &crosshair,
            base_font_size,
        );
        let ring_glyphs: Vec<&str> = HUE_RING_GLYPHS.iter().copied().collect();
        let ring = crate::application::renderer::measure_max_glyph_advance(
            &mut font_system,
            &ring_glyphs,
            ring_font_size,
        );
        (cell, ring)
    };

    *state = ColorPickerState::Open {
        kind,
        target_index,
        hue_deg,
        sat,
        val,
        chip_focus: None,
        last_cursor_pos: None,
        max_cell_advance,
        max_ring_advance,
        commit_mode: crate::application::color_picker::CommitMode::Hsv,
        layout: None,
    };

    // Seed the document preview so the initial render already shows
    // the same HSV the picker opened at. Overwritten on the next
    // hover frame, but this avoids a one-frame flash of the original
    // color when the modal opens.
    seed_initial_preview(doc, kind, target_index, hue_deg, sat, val);

    rebuild_color_picker_overlay(state, doc, renderer);
    rebuild_scene_only(doc, renderer);
}

/// Helper: write the initial HSV into `doc.color_picker_preview` on
/// picker open so the first rendered frame already shows the
/// previewed color instead of the model's stored one.
#[cfg(not(target_arch = "wasm32"))]
fn seed_initial_preview(
    doc: &mut MindMapDocument,
    kind: crate::application::color_picker::TargetKind,
    target_index: usize,
    hue_deg: f32,
    sat: f32,
    val: f32,
) {
    use crate::application::color_picker::TargetKind;
    use crate::application::document::ColorPickerPreview;
    use baumhard::util::color::hsv_to_hex;

    let hex = hsv_to_hex(hue_deg, sat, val);
    match kind {
        TargetKind::Edge => {
            if let Some(edge) = doc.mindmap.edges.get(target_index) {
                let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
                doc.color_picker_preview = Some(ColorPickerPreview::Edge { key, color: hex });
            }
        }
        TargetKind::Portal => {
            if let Some(portal) = doc.mindmap.portals.get(target_index) {
                let key = baumhard::mindmap::scene_builder::PortalRefKey::from_portal(portal);
                doc.color_picker_preview = Some(ColorPickerPreview::Portal { key, color: hex });
            }
        }
    }
}

/// Build geometry from the current picker state. Internal helper —
/// callers pick whether to push it through the full rebuild (static
/// + dynamic, called on open and resize) or just the dynamic rebuild
/// (called on hover). Also caches the resulting layout back into the
/// state so the mouse hit-test can read it without re-running the
/// layout pure fn.
#[cfg(not(target_arch = "wasm32"))]
fn compute_picker_geometry(
    state: &mut crate::application::color_picker::ColorPickerState,
    renderer: &Renderer,
) -> Option<crate::application::color_picker::ColorPickerOverlayGeometry> {
    use crate::application::color_picker::{
        compute_color_picker_layout, ColorPickerOverlayGeometry, ColorPickerState,
    };
    use baumhard::util::color::hsv_to_hex;

    // Extract only the fields `compute_color_picker_layout` needs,
    // plus a copy of the backdrop tuple from the cached layout for
    // the cursor-inside-backdrop check. Copying just the 4 floats is
    // ~200 bytes cheaper than cloning the whole ColorPickerLayout
    // (with its Vec<chip_positions> and fixed-size arrays) every
    // hover.
    let (
        target_label,
        hue_deg,
        sat,
        val,
        chip_focus,
        last_cursor_pos,
        max_cell_advance,
        max_ring_advance,
        cached_backdrop,
    ) = match state {
        ColorPickerState::Closed => return None,
        ColorPickerState::Open {
            kind,
            hue_deg,
            sat,
            val,
            chip_focus,
            last_cursor_pos,
            max_cell_advance,
            max_ring_advance,
            layout,
            ..
        } => (
            kind.label(),
            *hue_deg,
            *sat,
            *val,
            *chip_focus,
            *last_cursor_pos,
            *max_cell_advance,
            *max_ring_advance,
            layout.as_ref().map(|l| l.backdrop),
        ),
    };

    // Hex readout is visible when the cursor is inside the backdrop
    // OR a chip is focused. Without a cached layout from a previous
    // rebuild we can't hit-test the backdrop, so fall back to the
    // chip_focus signal — the first rebuild happens at open time
    // before any mouse event, and is followed immediately by a hover
    // rebuild once the cursor enters the window.
    let hex_visible = chip_focus.is_some()
        || match (last_cursor_pos, cached_backdrop) {
            (Some((cx, cy)), Some((bl, bt, bw, bh))) => {
                cx >= bl && cx <= bl + bw && cy >= bt && cy <= bt + bh
            }
            _ => false,
        };

    let geometry = ColorPickerOverlayGeometry {
        target_label,
        hue_deg,
        sat,
        val,
        preview_hex: hsv_to_hex(hue_deg, sat, val),
        chip_focus,
        hex_visible,
        max_cell_advance,
        max_ring_advance,
    };

    // Cache the layout into the state so the mouse hit-test can use it.
    let layout = compute_color_picker_layout(
        &geometry,
        renderer.surface_width() as f32,
        renderer.surface_height() as f32,
    );
    if let ColorPickerState::Open { layout: cached, .. } = state {
        *cached = Some(layout);
    }

    Some(geometry)
}

/// Full picker overlay rebuild — static + dynamic. Called by
/// `open_color_picker`, the `Resized` handler, and `cancel` /
/// `commit` on close (via `None`).
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_color_picker_overlay(
    state: &mut crate::application::color_picker::ColorPickerState,
    _doc: &MindMapDocument,
    renderer: &mut Renderer,
) {
    match compute_picker_geometry(state, renderer) {
        Some(g) => renderer.rebuild_color_picker_overlay_buffers(Some(&g)),
        None => renderer.rebuild_color_picker_overlay_buffers(None),
    }
}

/// Dynamic-only picker overlay rebuild — just the parts whose
/// content changes per hover (sat/val bars, preview glyph, hex
/// readout, chip focus, selected-slot indicator ring). The static
/// buffers (title, hint, hue ring) are left intact, which is the
/// whole reason for the L7 split in the renderer. Used by
/// `apply_picker_preview` and `apply_picker_chip` from the hot
/// hover path.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_color_picker_overlay_dynamic(
    state: &mut crate::application::color_picker::ColorPickerState,
    renderer: &mut Renderer,
) {
    if let Some(g) = compute_picker_geometry(state, renderer) {
        renderer.rebuild_color_picker_dynamic_buffers(&g);
    }
}

/// Cancel the picker: clear the transient document preview and
/// close the modal. The committed model is untouched because the
/// new preview path never writes to it — the entire hover / cancel
/// flow is a pure scene-level substitution.
#[cfg(not(target_arch = "wasm32"))]
fn cancel_color_picker(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::ColorPickerState;

    if matches!(state, ColorPickerState::Closed) {
        return;
    }
    *state = ColorPickerState::Closed;
    doc.color_picker_preview = None;
    renderer.rebuild_color_picker_overlay_buffers(None);
    rebuild_all(doc, mindmap_tree, renderer);
}

/// Commit the picker's currently-previewed color via the regular
/// `set_edge_color` / `set_portal_color` path — a single undo entry
/// is pushed and `ensure_glyph_connection` runs its fork-on-first-
/// edit only at this moment (never during hover). Close the modal.
///
/// The exact call depends on `commit_mode`:
/// - `Hsv`: commit the current HSV hex as a per-edge/portal override.
/// - `Var(raw)`: commit the literal `var(--name)` string so theme
///   resolution runs at render time.
/// - `ResetToInherited`: for edges, call `set_edge_color(None)` to
///   clear the per-edge override. For portals, re-seed to the
///   canvas's `--accent` value (or the raw `var(--accent)` string
///   as a fallback) since `PortalPair.color` is non-optional.
#[cfg(not(target_arch = "wasm32"))]
fn commit_color_picker(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{ColorPickerState, CommitMode, TargetKind};
    use baumhard::util::color::hsv_to_hex;

    let (kind, target_index, hue_deg, sat, val, commit_mode) = match state {
        ColorPickerState::Open { kind, target_index, hue_deg, sat, val, commit_mode, .. } => {
            (*kind, *target_index, *hue_deg, *sat, *val, commit_mode.clone())
        }
        ColorPickerState::Closed => return,
    };

    // Close the modal state first so the subsequent rebuilds don't
    // re-apply the preview.
    *state = ColorPickerState::Closed;
    doc.color_picker_preview = None;

    match kind {
        TargetKind::Edge => {
            let er = doc.mindmap.edges.get(target_index).map(|e| {
                EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type)
            });
            if let Some(er) = er {
                match commit_mode {
                    CommitMode::Hsv => {
                        let hex = hsv_to_hex(hue_deg, sat, val);
                        doc.set_edge_color(&er, Some(&hex));
                    }
                    CommitMode::Var(raw) => {
                        doc.set_edge_color(&er, Some(&raw));
                    }
                    CommitMode::ResetToInherited => {
                        doc.set_edge_color(&er, None);
                    }
                }
            }
        }
        TargetKind::Portal => {
            let pr = doc.mindmap.portals.get(target_index).map(|p| {
                crate::application::document::PortalRef::new(
                    p.label.clone(),
                    p.endpoint_a.clone(),
                    p.endpoint_b.clone(),
                )
            });
            if let Some(pr) = pr {
                match commit_mode {
                    CommitMode::Hsv => {
                        let hex = hsv_to_hex(hue_deg, sat, val);
                        doc.set_portal_color(&pr, &hex);
                    }
                    CommitMode::Var(raw) => {
                        doc.set_portal_color(&pr, &raw);
                    }
                    CommitMode::ResetToInherited => {
                        // `--accent` fallback — same rule as the old
                        // portal Reset path.
                        let resolved = doc
                            .mindmap
                            .canvas
                            .theme_variables
                            .get("--accent")
                            .cloned()
                            .unwrap_or_else(|| "var(--accent)".to_string());
                        doc.set_portal_color(&pr, &resolved);
                    }
                }
            }
        }
    }

    renderer.rebuild_color_picker_overlay_buffers(None);
    rebuild_all(doc, mindmap_tree, renderer);
}

/// Apply the current picker HSV to the document's transient color
/// preview, then rebuild only the scene (not the node tree, which
/// didn't change) + the picker overlay. Hot path: no ref resolution,
/// no model mutation, no snapshot. The scene builder reads the
/// preview via `doc.color_picker_preview` and substitutes it in
/// during emission.
#[cfg(not(target_arch = "wasm32"))]
fn apply_picker_preview(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    _mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{ColorPickerState, CommitMode, TargetKind};
    use crate::application::document::ColorPickerPreview;
    use baumhard::util::color::hsv_to_hex;

    let (kind, target_index, hue_deg, sat, val) = match state {
        ColorPickerState::Open { kind, target_index, hue_deg, sat, val, commit_mode, .. } => {
            // Any HSV movement implicitly cancels a prior chip
            // selection (Var/Reset) — the user moved the wheel, so
            // the commit mode goes back to Hsv.
            *commit_mode = CommitMode::Hsv;
            (*kind, *target_index, *hue_deg, *sat, *val)
        }
        ColorPickerState::Closed => return,
    };
    let hex = hsv_to_hex(hue_deg, sat, val);
    match kind {
        TargetKind::Edge => {
            if let Some(edge) = doc.mindmap.edges.get(target_index) {
                let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
                doc.color_picker_preview = Some(ColorPickerPreview::Edge { key, color: hex });
            }
        }
        TargetKind::Portal => {
            if let Some(portal) = doc.mindmap.portals.get(target_index) {
                let key = baumhard::mindmap::scene_builder::PortalRefKey::from_portal(portal);
                doc.color_picker_preview = Some(ColorPickerPreview::Portal { key, color: hex });
            }
        }
    }
    rebuild_scene_only(doc, renderer);
    rebuild_color_picker_overlay_dynamic(state, renderer);
}

/// Apply a theme-variable chip action to the picker's target. Used
/// for Tab+Enter on a focused chip, and for a direct click on a
/// chip. `ChipAction::Reset` clears edge overrides (the cleanest
/// semantic — edge falls back to `edge.color` or canvas default) and
/// for portals, where `PortalPair.color` is non-optional, re-seeds
/// the portal to the canvas's `--accent` variable value if one
/// exists (else the literal `"var(--accent)"` string, which the
/// theme-var resolver will pass through gracefully).
#[cfg(not(target_arch = "wasm32"))]
fn apply_picker_chip(
    state: &mut crate::application::color_picker::ColorPickerState,
    chip_idx: usize,
    doc: &mut MindMapDocument,
    _mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{
        ChipAction, ColorPickerState, CommitMode, TargetKind, THEME_CHIPS,
    };
    use crate::application::document::ColorPickerPreview;

    let chip = match THEME_CHIPS.get(chip_idx) {
        Some(c) => *c,
        None => return,
    };
    let (kind, target_index) = match state {
        ColorPickerState::Open { kind, target_index, .. } => (*kind, *target_index),
        ColorPickerState::Closed => return,
    };

    // Compute both (a) the display color for the preview (scene
    // substitution string) and (b) the commit mode for Enter. The
    // display color always resolves to something concrete so the
    // user sees the actual color rather than a `var(--name)` literal.
    let (display_color, commit_mode): (String, CommitMode) = match (kind, chip.action) {
        (TargetKind::Edge, ChipAction::Var(raw)) => {
            // Resolve the var string through the theme map to get
            // the concrete display color.
            let resolved = baumhard::util::color::resolve_var(
                raw,
                &doc.mindmap.canvas.theme_variables,
            )
            .to_string();
            (resolved, CommitMode::Var(raw.to_string()))
        }
        (TargetKind::Edge, ChipAction::Reset) => {
            // Preview what the edge will look like after Reset: the
            // inherited `edge.color` (since `cfg.color` becomes
            // None). Resolve that through theme variables for the
            // concrete display hex.
            let display = match doc.mindmap.edges.get(target_index) {
                Some(e) => baumhard::util::color::resolve_var(
                    &e.color,
                    &doc.mindmap.canvas.theme_variables,
                )
                .to_string(),
                None => "#ffffff".to_string(),
            };
            (display, CommitMode::ResetToInherited)
        }
        (TargetKind::Portal, ChipAction::Var(raw)) => {
            let resolved = baumhard::util::color::resolve_var(
                raw,
                &doc.mindmap.canvas.theme_variables,
            )
            .to_string();
            (resolved, CommitMode::Var(raw.to_string()))
        }
        (TargetKind::Portal, ChipAction::Reset) => {
            // Portals have a non-optional color field, so "reset"
            // re-seeds to the canvas's `--accent` value (or the
            // raw `var(--accent)` string as a fallback).
            let resolved = doc
                .mindmap
                .canvas
                .theme_variables
                .get("--accent")
                .cloned()
                .unwrap_or_else(|| "var(--accent)".to_string());
            let display = baumhard::util::color::resolve_var(
                &resolved,
                &doc.mindmap.canvas.theme_variables,
            )
            .to_string();
            (display, CommitMode::ResetToInherited)
        }
    };

    // Update the picker's commit mode so Enter commits the chip's
    // action rather than the HSV hex.
    if let ColorPickerState::Open { commit_mode: cm, .. } = state {
        *cm = commit_mode;
    }

    // Push the display color into the document preview so the
    // scene substitution picks it up on the next rebuild.
    match kind {
        TargetKind::Edge => {
            if let Some(edge) = doc.mindmap.edges.get(target_index) {
                let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
                doc.color_picker_preview = Some(ColorPickerPreview::Edge { key, color: display_color });
            }
        }
        TargetKind::Portal => {
            if let Some(portal) = doc.mindmap.portals.get(target_index) {
                let key = baumhard::mindmap::scene_builder::PortalRefKey::from_portal(portal);
                doc.color_picker_preview = Some(ColorPickerPreview::Portal { key, color: display_color });
            }
        }
    }

    rebuild_scene_only(doc, renderer);
    rebuild_color_picker_overlay_dynamic(state, renderer);
}

/// Route a keystroke to the picker. Esc cancels, Enter commits (or
/// applies the focused chip and commits in one shot), Tab cycles
/// through theme chips, h/H ±15° hue, s/S ±0.1 sat, v/V ±0.1 val.
#[cfg(not(target_arch = "wasm32"))]
fn handle_color_picker_key(
    key_name: &Option<String>,
    logical_key: &Key,
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{ColorPickerState, THEME_CHIPS};

    let name = key_name.as_deref();
    match name {
        Some("escape") => {
            cancel_color_picker(state, doc, mindmap_tree, renderer);
            return;
        }
        Some("enter") => {
            // If a chip is focused, apply it as the final commit;
            // otherwise commit the current HSV preview.
            let focus = if let ColorPickerState::Open { chip_focus, .. } = state {
                *chip_focus
            } else {
                None
            };
            if let Some(idx) = focus {
                apply_picker_chip(state, idx, doc, mindmap_tree, renderer);
            }
            commit_color_picker(state, doc, mindmap_tree, renderer);
            return;
        }
        Some("tab") => {
            if let ColorPickerState::Open { chip_focus, .. } = state {
                let n = THEME_CHIPS.len();
                *chip_focus = match *chip_focus {
                    None => Some(0),
                    Some(i) if i + 1 >= n => None,
                    Some(i) => Some(i + 1),
                };
            }
            // Chip focus lives in the dynamic buffer set — no need
            // to reshape the hue ring or hint.
            rebuild_color_picker_overlay_dynamic(state, renderer);
            return;
        }
        _ => {}
    }
    // Character keys: h/s/v nudges. Use logical_key to keep this
    // case-sensitive (uppercase = bigger nudge).
    if let Key::Character(c) = logical_key {
        let s = c.as_str();
        let mut changed = false;
        if let ColorPickerState::Open { hue_deg, sat, val, .. } = state {
            match s {
                "h" => {
                    *hue_deg = (*hue_deg - 15.0).rem_euclid(360.0);
                    changed = true;
                }
                "H" => {
                    *hue_deg = (*hue_deg + 15.0).rem_euclid(360.0);
                    changed = true;
                }
                "s" => {
                    *sat = (*sat - 0.1).clamp(0.0, 1.0);
                    changed = true;
                }
                "S" => {
                    *sat = (*sat + 0.1).clamp(0.0, 1.0);
                    changed = true;
                }
                "v" => {
                    *val = (*val - 0.1).clamp(0.0, 1.0);
                    changed = true;
                }
                "V" => {
                    *val = (*val + 0.1).clamp(0.0, 1.0);
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            apply_picker_preview(state, doc, mindmap_tree, renderer);
        }
    }
}

/// Mouse-move handler for the picker. Hit-tests the cursor against the
/// cached layout and updates the picker HSV / chip focus to match the
/// hovered element, then live-previews the new color on the target.
#[cfg(not(target_arch = "wasm32"))]
fn handle_color_picker_mouse_move(
    cursor_pos: (f64, f64),
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{
        hit_test_picker, hue_slot_to_degrees, sat_cell_to_value, val_cell_to_value,
        ColorPickerState, PickerHit,
    };

    // Always record the cursor position on the state before hit-
    // testing — `compute_picker_geometry` reads it to toggle
    // `hex_visible` based on "cursor inside backdrop". A move that
    // doesn't hit any interactive element still needs this update so
    // the hex readout can appear/disappear as the cursor crosses the
    // backdrop boundary.
    let cursor = (cursor_pos.0 as f32, cursor_pos.1 as f32);
    if let ColorPickerState::Open { last_cursor_pos, .. } = state {
        *last_cursor_pos = Some(cursor);
    }

    let hit = if let ColorPickerState::Open { layout: Some(layout), .. } = state {
        hit_test_picker(layout, cursor.0, cursor.1)
    } else {
        // Picker closed, or open but the first rebuild hasn't happened
        // yet — no cached layout to hit-test against. The open path
        // always rebuilds before releasing control, so this branch is
        // only reachable during the ~1-line window between construction
        // and the first rebuild call.
        return;
    };

    let mut hsv_changed = false;
    if let ColorPickerState::Open { hue_deg, sat, val, chip_focus, .. } = state {
        match hit {
            PickerHit::Hue(slot) => {
                *hue_deg = hue_slot_to_degrees(slot);
                *chip_focus = None;
                hsv_changed = true;
            }
            PickerHit::SatCell(i) => {
                *sat = sat_cell_to_value(i);
                *chip_focus = None;
                hsv_changed = true;
            }
            PickerHit::ValCell(i) => {
                *val = val_cell_to_value(i);
                *chip_focus = None;
                hsv_changed = true;
            }
            PickerHit::Chip(i) => {
                *chip_focus = Some(i);
            }
            PickerHit::Inside | PickerHit::Outside => {
                *chip_focus = None;
            }
        }
    }

    if hsv_changed {
        apply_picker_preview(state, doc, mindmap_tree, renderer);
    } else {
        // Only chip focus moved, or we're hovering inert padding /
        // outside the backdrop entirely. The dynamic rebuild re-runs
        // `compute_picker_geometry` which picks up the updated
        // cursor_pos and toggles hex_visible accordingly — so even
        // Inside / Outside hits are meaningful rebuild triggers now.
        rebuild_color_picker_overlay_dynamic(state, renderer);
    }
}

/// Click handler for the picker. Out-of-frame clicks cancel; in-frame
/// hits commit at the click location (chip clicks apply the chip's
/// color and commit in one gesture).
#[cfg(not(target_arch = "wasm32"))]
fn handle_color_picker_click(
    cursor_pos: (f64, f64),
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{
        hit_test_picker, hue_slot_to_degrees, sat_cell_to_value, val_cell_to_value,
        ColorPickerState, PickerHit,
    };

    let hit = if let ColorPickerState::Open { layout: Some(layout), .. } = state {
        hit_test_picker(layout, cursor_pos.0 as f32, cursor_pos.1 as f32)
    } else {
        return;
    };

    match hit {
        PickerHit::Outside => {
            // Click outside the backdrop entirely — close as cancel,
            // matching the palette modal's "click outside dismisses"
            // gesture.
            cancel_color_picker(state, doc, mindmap_tree, renderer);
        }
        PickerHit::Inside => {
            // Click inside the backdrop but not on any glyph (e.g.
            // the empty space inside the mandala ring between the
            // crosshair arms). Stay open — the user is aiming and
            // missed; cancelling would feel surprising.
        }
        PickerHit::Hue(slot) => {
            if let ColorPickerState::Open { hue_deg, .. } = state {
                *hue_deg = hue_slot_to_degrees(slot);
            }
            apply_picker_preview(state, doc, mindmap_tree, renderer);
            commit_color_picker(state, doc, mindmap_tree, renderer);
        }
        PickerHit::SatCell(i) => {
            if let ColorPickerState::Open { sat, .. } = state {
                *sat = sat_cell_to_value(i);
            }
            apply_picker_preview(state, doc, mindmap_tree, renderer);
            commit_color_picker(state, doc, mindmap_tree, renderer);
        }
        PickerHit::ValCell(i) => {
            if let ColorPickerState::Open { val, .. } = state {
                *val = val_cell_to_value(i);
            }
            apply_picker_preview(state, doc, mindmap_tree, renderer);
            commit_color_picker(state, doc, mindmap_tree, renderer);
        }
        PickerHit::Chip(i) => {
            apply_picker_chip(state, i, doc, mindmap_tree, renderer);
            commit_color_picker(state, doc, mindmap_tree, renderer);
        }
    }
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
    renderer.rebuild_connection_buffers(&scene.connection_elements);
    renderer.rebuild_border_buffers(&scene.border_elements);
    renderer.rebuild_edge_handle_buffers(&scene.edge_handles);
    renderer.rebuild_connection_label_buffers(&scene.connection_label_elements);
    renderer.rebuild_portal_buffers(&scene.portal_elements);

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

// =====================================================================
// Session 7A: unit tests for pure helpers (cursor math, caret
// insertion, double-click detection, Baumhard mutation round-trip).
// Event-loop integration is verified manually via `cargo run`.
// =====================================================================

#[cfg(test)]
mod text_edit_tests {
    use super::*;

    // -----------------------------------------------------------------
    // Cursor math
    // -----------------------------------------------------------------

    #[test]
    fn test_insert_at_cursor_start() {
        let mut s = String::from("bcd");
        let cursor = insert_at_cursor(&mut s, 0, 'a');
        assert_eq!(s, "abcd");
        assert_eq!(cursor, 1);
    }

    #[test]
    fn test_insert_at_cursor_middle() {
        let mut s = String::from("abd");
        let cursor = insert_at_cursor(&mut s, 2, 'c');
        assert_eq!(s, "abcd");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_insert_at_cursor_end() {
        let mut s = String::from("abc");
        let cursor = insert_at_cursor(&mut s, 3, 'd');
        assert_eq!(s, "abcd");
        assert_eq!(cursor, 4);
    }

    #[test]
    fn test_insert_at_cursor_newline() {
        let mut s = String::from("abcd");
        let cursor = insert_at_cursor(&mut s, 2, '\n');
        assert_eq!(s, "ab\ncd");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_delete_before_cursor_at_start_noop() {
        let mut s = String::from("abc");
        let cursor = delete_before_cursor(&mut s, 0);
        assert_eq!(s, "abc");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_delete_before_cursor_middle() {
        let mut s = String::from("abcd");
        let cursor = delete_before_cursor(&mut s, 2);
        assert_eq!(s, "acd");
        assert_eq!(cursor, 1);
    }

    #[test]
    fn test_delete_at_cursor_end_noop() {
        let mut s = String::from("abc");
        let cursor = delete_at_cursor(&mut s, 3);
        assert_eq!(s, "abc");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_delete_at_cursor_middle() {
        let mut s = String::from("abcd");
        let cursor = delete_at_cursor(&mut s, 1);
        assert_eq!(s, "acd");
        assert_eq!(cursor, 1);
    }

    #[test]
    fn test_cursor_to_line_start_single_line() {
        assert_eq!(cursor_to_line_start("abc", 2), 0);
    }

    #[test]
    fn test_cursor_to_line_start_multiline() {
        let s = "ab\ncd\nef";
        // cursor on 'd' (index 4): line starts at 3
        assert_eq!(cursor_to_line_start(s, 4), 3);
        // cursor on 'f' (index 7): line starts at 6
        assert_eq!(cursor_to_line_start(s, 7), 6);
    }

    #[test]
    fn test_cursor_to_line_end_multiline() {
        let s = "ab\ncd\nef";
        // cursor on 'a' (index 0): end at '\n' position (2)
        assert_eq!(cursor_to_line_end(s, 0), 2);
        // cursor on 'e' (index 6): end at buffer end (8)
        assert_eq!(cursor_to_line_end(s, 6), 8);
    }

    #[test]
    fn test_move_cursor_up_line_preserves_column() {
        let s = "abcd\nwxyz";
        // cursor on 'y' (index 7, col 2 on line 1): up → 'c' (index 2)
        assert_eq!(move_cursor_up_line(s, 7), 2);
    }

    #[test]
    fn test_move_cursor_up_line_short_prev_line() {
        let s = "ab\nwxyz";
        // cursor on 'z' (index 6, col 3 on line 1): up → end of "ab" (index 2)
        assert_eq!(move_cursor_up_line(s, 6), 2);
    }

    #[test]
    fn test_move_cursor_up_line_first_line_is_noop() {
        assert_eq!(move_cursor_up_line("abc", 1), 1);
    }

    #[test]
    fn test_move_cursor_down_line_preserves_column() {
        let s = "abcd\nwxyz";
        // cursor on 'c' (index 2): down → 'y' (index 7)
        assert_eq!(move_cursor_down_line(s, 2), 7);
    }

    #[test]
    fn test_move_cursor_down_line_last_line_is_noop() {
        let s = "ab\ncd";
        assert_eq!(move_cursor_down_line(s, 4), 4);
    }

    // -----------------------------------------------------------------
    // Caret insertion
    // -----------------------------------------------------------------

    #[test]
    fn test_insert_caret_middle() {
        let out = insert_caret("abcd", 2);
        assert_eq!(out, "ab\u{258C}cd");
    }

    #[test]
    fn test_insert_caret_end() {
        let out = insert_caret("abc", 3);
        assert_eq!(out, "abc\u{258C}");
    }

    #[test]
    fn test_insert_caret_empty() {
        let out = insert_caret("", 0);
        assert_eq!(out, "\u{258C}");
    }

    // -----------------------------------------------------------------
    // Double-click detection
    // -----------------------------------------------------------------

    #[test]
    fn test_double_click_same_target_within_window_fires() {
        let prev = LastClick {
            time: 1000.0,
            screen_pos: (100.0, 100.0),
            hit: Some("node-a".to_string()),
        };
        assert!(is_double_click(
            &prev,
            1100.0,
            (101.0, 100.0),
            &Some("node-a".to_string()),
        ));
    }

    #[test]
    fn test_double_click_different_targets_does_not_fire() {
        let prev = LastClick {
            time: 1000.0,
            screen_pos: (100.0, 100.0),
            hit: Some("node-a".to_string()),
        };
        assert!(!is_double_click(
            &prev,
            1100.0,
            (100.0, 100.0),
            &Some("node-b".to_string()),
        ));
    }

    #[test]
    fn test_double_click_too_far_apart_does_not_fire() {
        let prev = LastClick {
            time: 1000.0,
            screen_pos: (100.0, 100.0),
            hit: None,
        };
        // Distance = sqrt(20² + 0²) = 20px → dist² = 400, threshold = 256.
        assert!(!is_double_click(&prev, 1100.0, (120.0, 100.0), &None));
    }

    #[test]
    fn test_double_click_expired_does_not_fire() {
        let prev = LastClick {
            time: 1000.0,
            screen_pos: (100.0, 100.0),
            hit: None,
        };
        assert!(!is_double_click(&prev, 1500.0, (100.0, 100.0), &None));
    }

    #[test]
    fn test_double_click_empty_space_both_misses_fires() {
        // Both clicks landed on no node — valid double-click for
        // the "create orphan" gesture.
        let prev = LastClick {
            time: 1000.0,
            screen_pos: (50.0, 50.0),
            hit: None,
        };
        assert!(is_double_click(&prev, 1150.0, (52.0, 51.0), &None));
    }

    #[test]
    fn test_double_click_exact_boundary_does_not_fire() {
        // At exactly DOUBLE_CLICK_MS elapsed, should NOT fire (uses >= threshold).
        let prev = LastClick {
            time: 1000.0,
            screen_pos: (100.0, 100.0),
            hit: None,
        };
        assert!(!is_double_click(&prev, 1400.0, (100.0, 100.0), &None));
    }

    #[test]
    fn test_double_click_just_under_boundary_fires() {
        let prev = LastClick {
            time: 1000.0,
            screen_pos: (100.0, 100.0),
            hit: None,
        };
        assert!(is_double_click(&prev, 1399.0, (100.0, 100.0), &None));
    }

    // -----------------------------------------------------------------
    // Baumhard Mutation round-trip: constructing and applying a
    // `Mutation::AreaDelta` with `GlyphAreaField::Text + Assign`
    // mutates the target GlyphArea's text in place. This verifies we
    // really are flowing text edits through Baumhard's existing
    // vocabulary instead of patching around it.
    // -----------------------------------------------------------------

    #[test]
    fn test_text_edit_mutation_assigns_via_baumhard() {
        use baumhard::core::primitives::{Applicable, ApplyOperation};
        use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaField};

        let mut area = GlyphArea::new_with_str(
            "initial",
            14.0,
            16.8,
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 30.0),
        );
        let delta = DeltaGlyphArea::new(vec![
            GlyphAreaField::Text("updated".to_string()),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]);
        delta.apply_to(&mut area);
        assert_eq!(area.text, "updated");
    }

    #[test]
    fn test_text_edit_mutation_with_caret_glyph_via_baumhard() {
        use baumhard::core::primitives::{Applicable, ApplyOperation};
        use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaField};

        let mut area = GlyphArea::new_with_str(
            "",
            14.0,
            16.8,
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 30.0),
        );
        let buffer = "hello world";
        let cursor = 5;
        let display_text = insert_caret(buffer, cursor);
        let delta = DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(display_text.clone()),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]);
        delta.apply_to(&mut area);
        // Caret after "hello", before " world".
        assert_eq!(area.text, "hello\u{258C} world");
        assert_eq!(area.text, display_text);
    }

    /// Regression test for the Session 7A follow-up bug: applying a
    /// text edit delta to a GlyphArea with pre-existing
    /// `ColorFontRegions` (i.e. an existing multi-run node) must
    /// replace those regions with one that spans the entire new
    /// display text — including the trailing caret glyph. Otherwise
    /// `rebuild_buffers_from_tree` at renderer.rs:1500-1520 silently
    /// drops any character outside the old ranges, making the caret
    /// and newly-typed characters invisible.
    #[test]
    fn test_text_edit_replaces_stale_regions_to_cover_caret() {
        use baumhard::core::primitives::{
            Applicable, ApplyOperation, ColorFontRegion, ColorFontRegions, Range,
        };
        use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaField};

        // Simulate an existing multi-run node with text "Hello" and
        // a single region over [0, 5) colored red.
        let mut area = GlyphArea::new_with_str(
            "Hello",
            14.0,
            16.8,
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 30.0),
        );
        let red = [1.0f32, 0.0, 0.0, 1.0];
        let mut initial_regions = ColorFontRegions::new_empty();
        initial_regions.submit_region(ColorFontRegion::new(
            Range::new(0, 5),
            None,
            Some(red),
        ));
        area.regions = initial_regions;
        assert_eq!(area.regions.num_regions(), 1);

        // Build the same delta `apply_text_edit_to_tree` produces
        // for buffer="Hello" at cursor=5: text="Hello▌", regions =
        // single region [0, 6) inheriting the red color.
        let buffer = "Hello";
        let cursor = buffer.chars().count();
        let display_text = insert_caret(buffer, cursor);
        let display_char_count = display_text.chars().count();
        let inherited_color = area.regions.all_regions().first().and_then(|r| r.color);
        let mut new_regions = ColorFontRegions::new_empty();
        new_regions.submit_region(ColorFontRegion::new(
            Range::new(0, display_char_count),
            None,
            inherited_color,
        ));

        let delta = DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(display_text.clone()),
            GlyphAreaField::ColorFontRegions(new_regions),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]);
        delta.apply_to(&mut area);

        // Text updated.
        assert_eq!(area.text, "Hello\u{258C}");
        // Exactly one region, covering char positions 0..6 (includes
        // the caret), and inheriting the original red color.
        assert_eq!(area.regions.num_regions(), 1);
        let region = area.regions.all_regions()[0];
        assert_eq!(region.range.start, 0);
        assert_eq!(region.range.end, 6);
        assert_eq!(region.color, Some(red));
    }

    // -----------------------------------------------------------------
    // TextEditState shape + guard semantics
    // -----------------------------------------------------------------

    #[test]
    fn test_text_edit_state_node_id_round_trip() {
        let closed = TextEditState::Closed;
        assert!(closed.node_id().is_none());
        assert!(!closed.is_open());

        let open = TextEditState::Open {
            node_id: "n-42".to_string(),
            buffer: "hi".to_string(),
            cursor_grapheme_pos: 2,
        };
        assert_eq!(open.node_id(), Some("n-42"));
        assert!(open.is_open());
    }

    #[test]
    fn test_text_edit_state_is_open_closed_variant() {
        assert!(!TextEditState::Closed.is_open());
    }

    // -----------------------------------------------------------------
    // Cursor helpers: boundary cases added after perf rewrite
    // -----------------------------------------------------------------

    #[test]
    fn test_cursor_to_line_start_trailing_newline() {
        // Cursor positioned just after a trailing '\n' (on an empty
        // final line). Line start should be the char index right
        // after the '\n', i.e. the cursor itself.
        let s = "abc\n";
        assert_eq!(cursor_to_line_start(s, 4), 4);
    }

    #[test]
    fn test_cursor_to_line_start_at_zero() {
        assert_eq!(cursor_to_line_start("anything", 0), 0);
    }

    #[test]
    fn test_cursor_to_line_start_empty_buffer() {
        assert_eq!(cursor_to_line_start("", 0), 0);
    }

    #[test]
    fn test_cursor_to_line_end_empty_buffer() {
        assert_eq!(cursor_to_line_end("", 0), 0);
    }

    #[test]
    fn test_cursor_to_line_end_cursor_exactly_at_newline() {
        // Cursor is at the '\n' position; line end IS that position.
        let s = "ab\ncd";
        assert_eq!(cursor_to_line_end(s, 2), 2);
    }

    #[test]
    fn test_cursor_to_line_end_walks_past_cursor() {
        // Cursor in the middle of a line, next '\n' several chars ahead.
        let s = "alpha beta\ngamma";
        // Cursor on 'p' (index 2): line_end should be at '\n' (index 10).
        assert_eq!(cursor_to_line_end(s, 2), 10);
    }

    // -----------------------------------------------------------------
    // insert_caret / insert_at_cursor with multi-byte chars
    // -----------------------------------------------------------------

    #[test]
    fn test_insert_caret_with_multibyte_prefix() {
        // 'é' is a 2-byte UTF-8 char. insert_caret must not split it.
        let out = insert_caret("café", 3);
        // "caf" + caret + "é"
        assert_eq!(out, "caf\u{258C}é");
    }

    #[test]
    fn test_insert_at_cursor_with_multibyte_buffer() {
        let mut s = String::from("café");
        // Insert 'x' between 'f' and 'é' (char pos 3).
        let new_cursor = insert_at_cursor(&mut s, 3, 'x');
        assert_eq!(s, "cafxé");
        assert_eq!(new_cursor, 4);
    }

    #[test]
    fn test_delete_before_cursor_with_multibyte() {
        let mut s = String::from("café");
        // Delete the 'é' (grapheme pos 3, cursor at 4).
        let new_cursor = delete_before_cursor(&mut s, 4);
        assert_eq!(s, "caf");
        assert_eq!(new_cursor, 3);
    }

    // -----------------------------------------------------------------
    // Grapheme-cluster regression tests (chunk 3 / §B2)
    // -----------------------------------------------------------------
    //
    // These guard the rule that a single Backspace/Delete removes a
    // whole grapheme cluster, not a Unicode scalar. Pre-chunk-3 the
    // helpers used `chars()` and would corrupt emoji and ZWJ
    // sequences mid-cluster on the first Backspace.

    #[test]
    fn test_cursor_edit_with_emoji_backspace() {
        // 🍕 is a single grapheme but two `char`s (it's a single
        // codepoint above U+FFFF, encoded as a surrogate pair in
        // UTF-16; in UTF-8 it's 4 bytes / 1 char).
        let mut s = String::from("ab🍕cd");
        // Cursor sits just after the pizza (grapheme index 3).
        let new_cursor = delete_before_cursor(&mut s, 3);
        // The whole pizza is gone, not just half of it.
        assert_eq!(s, "abcd");
        assert_eq!(new_cursor, 2);
    }

    #[test]
    fn test_cursor_edit_with_zwj_backspace() {
        // 🧑‍🚀 is a ZWJ sequence: 🧑 + ZWJ + 🚀, three codepoints
        // and five chars, but a single user-visible grapheme cluster.
        // Backspace must remove the whole thing in one keystroke.
        let mut s = String::from("hi🧑\u{200D}🚀!");
        let new_cursor = delete_before_cursor(&mut s, 3);
        assert_eq!(s, "hi!");
        assert_eq!(new_cursor, 2);
    }

    #[test]
    fn test_cursor_edit_with_emoji_delete_forward() {
        // Delete (forward delete) at the position before the pizza
        // removes the whole cluster.
        let mut s = String::from("ab🍕cd");
        let new_cursor = delete_at_cursor(&mut s, 2);
        assert_eq!(s, "abcd");
        // Forward delete leaves the cursor in place.
        assert_eq!(new_cursor, 2);
    }

    #[test]
    fn test_insert_caret_after_emoji() {
        // Caret rendered after a pizza emoji should not split it.
        let out = insert_caret("ab🍕cd", 3);
        assert_eq!(out, "ab🍕\u{258C}cd");
    }

    // -----------------------------------------------------------------
    // "is_double_click + already_editing_same_target" guard semantics
    // -----------------------------------------------------------------
    //
    // The bug report was: double-clicking inside an already-open
    // editor on the same node silently discards the transient buffer
    // because the Pressed path re-opens the editor, clobbering the
    // in-progress buffer. The fix guards the dispatch with a check
    // that re-opens are skipped if the editor is already on that
    // target. We verify the guard predicate here; the actual event
    // loop wiring is manually verified via `cargo run`.

    #[test]
    fn test_double_click_guard_skips_same_target_when_editor_open() {
        let editor = TextEditState::Open {
            node_id: "node-A".to_string(),
            buffer: "in progress".to_string(),
            cursor_grapheme_pos: 11,
        };
        let hit = Some("node-A".to_string());
        let already_editing = editor
            .node_id()
            .map(|id| hit.as_deref() == Some(id))
            .unwrap_or(false);
        assert!(already_editing, "guard must fire for same target");
    }

    #[test]
    fn test_double_click_guard_allows_different_target_when_editor_open() {
        let editor = TextEditState::Open {
            node_id: "node-A".to_string(),
            buffer: "in progress".to_string(),
            cursor_grapheme_pos: 11,
        };
        let hit = Some("node-B".to_string());
        let already_editing = editor
            .node_id()
            .map(|id| hit.as_deref() == Some(id))
            .unwrap_or(false);
        assert!(!already_editing, "guard must NOT fire for different target");
    }

    #[test]
    fn test_double_click_guard_allows_when_editor_closed() {
        let editor = TextEditState::Closed;
        let hit = Some("node-A".to_string());
        let already_editing = editor
            .node_id()
            .map(|id| hit.as_deref() == Some(id))
            .unwrap_or(false);
        assert!(!already_editing, "guard must NOT fire when editor is closed");
    }
}
