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
use winit::keyboard::{Key, ModifiersState};
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
use crate::application::console::{
    commands::Command,
    completion::complete as complete_console,
    parser::{parse, Args, ParseResult},
    ConsoleContext, ConsoleEffects, ConsoleLine, ConsoleState, ExecResult, MAX_HISTORY,
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

/// Session 6D: inline-edit state for a connection's label. When
/// `Open`, all keyboard input is routed to the label-edit handler
/// (just like `ConsoleState::Open` captures keys for the console
/// input line). Mutually exclusive with `ConsoleState::Open` — the
/// console check runs first, so opening the console while editing a
/// label is a no-op.
///
/// Mirrors [`TextEditState`] in shape (buffer + grapheme cursor),
/// per CODE_CONVENTIONS §1: every keystroke routes through
/// `grapheme_chad` so backspace over an emoji removes the whole
/// cluster, not a stray byte. The buffer is threaded into the
/// scene_builder via [`MindMapDocument::label_edit_preview`]; the
/// connection-label tree's §B2 mutator path (Phase 1.3) picks up
/// the new text + caret without rebuilding the arena.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
enum LabelEditState {
    Closed,
    Open {
        edge_ref: crate::application::document::EdgeRef,
        /// The in-progress buffer. Committed to
        /// `MindEdge.label` on Enter; discarded on Escape.
        buffer: String,
        /// Cursor position as a grapheme-cluster index into
        /// `buffer`. Valid range
        /// `[0, count_grapheme_clusters(buffer)]`. Stored in
        /// graphemes (not chars or bytes) so backspace over an
        /// emoji or ZWJ cluster removes the whole user-visible
        /// character — same invariant as
        /// [`TextEditState::Open::cursor_grapheme_pos`].
        cursor_grapheme_pos: usize,
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
fn route_label_edit_key(
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
        // App-level scene host. Owns the canvas-space tree for
        // borders today (registered via `update_border_tree_*`) and
        // grows to host the console / color-picker overlays in
        // Sessions 3 / 4 of the unified-rendering refactor. Kept
        // next to `mindmap_tree` so all tree-shaped components
        // share the interactive-session lifetime.
        let mut app_scene = crate::application::scene_host::AppScene::new();

        match MindMapDocument::load(&self.options.mindmap_path) {
            Ok(mut doc) => {
                doc.build_mutation_registry();
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
                update_connection_tree(&scene, &mut app_scene);
                update_border_tree_static(&doc, &mut app_scene);
                update_portal_tree(
                    &doc,
                    &std::collections::HashMap::new(),
                    &mut app_scene,
                    &mut renderer,
                );
                update_connection_label_tree(&scene, &mut app_scene, &mut renderer);
                flush_canvas_scene_buffers(&mut app_scene, &mut renderer);

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
        let mut console_state = ConsoleState::Closed;
        // Cross-session history loaded from disk on startup; appended
        // to on every Enter; written back on close. See
        // `load_console_history` / `save_console_history`.
        let mut console_history: Vec<String> = load_console_history();
        let mut label_edit_state = LabelEditState::Closed;
        let mut text_edit_state = TextEditState::Closed;
        let mut color_picker_state =
            crate::application::color_picker::ColorPickerState::Closed;
        // Session 7A: tracks the previous left-click-down for
        // double-click detection. Cleared after a double-click fires.
        let mut last_click: Option<LastClick> = None;
        let mut hovered_node: Option<String> = None;
        let mut modifiers = ModifiersState::empty();
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
        // Picker hover gate: every cursor-move into the picker
        // updates `state.{hue_deg,sat,val}` + `doc.color_picker_preview`
        // synchronously (cheap, no shaping), but the actual scene
        // + overlay rebuild is deferred to the `AboutToWait`
        // drain so it runs at most once per display frame and
        // self-throttles further if it falls behind. Same
        // pattern as the drag path's `mutation_throttle` /
        // `pending_delta` pair.
        let mut picker_throttle = MutationFrequencyThrottle::with_default_budget();
        let mut picker_dirty: bool = false;

        // Resolve keybindings once at startup. Users can rebind any key
        // by shipping a `keybinds.json` (see `keybinds.rs` for the format).
        let mut keybinds: ResolvedKeybinds = self.options.keybind_config.resolve();

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
                                &mut app_scene,
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
                    // The console swallows mouse clicks as a close
                    // gesture. Clicking anywhere while open dismisses
                    // the console without running a command, mirroring
                    // Escape.
                    if console_state.is_open() && state == ElementState::Pressed {
                        save_console_history(&console_history);
                        console_state = ConsoleState::Closed;
                        renderer.rebuild_console_overlay_buffers(&mut app_scene, None);
                        return;
                    }

                    // Glyph-wheel color picker click handling. The
                    // picker captures both left- and right-mouse
                    // buttons:
                    // - LMB on a `DragAnchor` → wheel-move gesture;
                    //   on any other hit → preview / commit / chip
                    //   focus.
                    // - RMB on a `DragAnchor` → wheel-resize
                    //   gesture (drag away to grow, toward to shrink).
                    //   RMB elsewhere is currently a no-op — only
                    //   the empty backdrop region acts as the resize
                    //   handle, mirroring the LMB-move convention.
                    // Release of either button ends any active
                    // gesture. In **Standalone** (persistent
                    // palette) mode, clicks outside the picker
                    // backdrop fall through to normal dispatch —
                    // otherwise the user couldn't select anything
                    // else while the palette was open. In
                    // **Contextual** mode the picker captures
                    // everything; outside-click cancels.
                    if color_picker_state.is_open()
                        && matches!(button, MouseButton::Left | MouseButton::Right)
                    {
                        let consumed = if state == ElementState::Pressed {
                            if let Some(doc) = document.as_mut() {
                                handle_color_picker_click(
                                    cursor_pos,
                                    button,
                                    &mut color_picker_state,
                                    doc,
                                    &mut mindmap_tree,
                                    &mut app_scene,
                                    &mut renderer,
                                )
                            } else {
                                true
                            }
                        } else {
                            // Release — end any active wheel gesture.
                            // If no gesture was active (e.g.
                            // Standalone + outside-press fell
                            // through), this is a no-op and the
                            // release should also fall through.
                            end_color_picker_gesture(&mut color_picker_state)
                        };
                        if consumed {
                            return;
                        }
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
                                        &mut app_scene,
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
                                        &mut app_scene,
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
                                            rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                            open_text_edit(
                                                &nid,
                                                false,
                                                doc,
                                                &mut text_edit_state,
                                                &mut mindmap_tree,
                                                &mut app_scene,
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
                                                let new_id = doc.create_orphan_and_select(canvas_pos);
                                                rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                                open_text_edit(
                                                    &new_id,
                                                    true,
                                                    doc,
                                                    &mut text_edit_state,
                                                    &mut mindmap_tree,
                                                    &mut app_scene,
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
                                                    &mut app_scene,
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
                                                        &mut app_scene,
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
                                                modifiers.shift_key(),
                                                &mut document,
                                                &mut mindmap_tree,
                                                &mut app_scene,
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
                                            rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
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
                                            rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
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
                                            rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
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
                    // change on the affected edge/portal. In
                    // Standalone mode with no active gesture and
                    // the cursor outside the backdrop the move
                    // falls through so the canvas's own hover
                    // (button-node cursor, etc.) keeps working.
                    if color_picker_state.is_open() {
                        let consumed = if let Some(doc) = document.as_mut() {
                            handle_color_picker_mouse_move(
                                cursor_pos,
                                &mut color_picker_state,
                                doc,
                                &mut picker_dirty,
                            )
                        } else {
                            true
                        };
                        if consumed {
                            return;
                        }
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
                                    &mut mindmap_tree, &mut app_scene, &mut renderer,
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
                                    let node_ids = if modifiers.shift_key() {
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
                                        individual: modifiers.alt_key(),
                                    };
                                } else if modifiers.shift_key() {
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
                    event: WindowEvent::ModifiersChanged(mods),
                    ..
                } => {
                    modifiers = mods.state();
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

                    // When the console is open, it steals all
                    // keyboard input. Character keys insert at the
                    // cursor, Tab triggers completion, Up/Down walks
                    // history, Enter parses + executes, Escape
                    // closes. Regular hotkeys are suppressed until
                    // the console closes.
                    if console_state.is_open() {
                        handle_console_key(
                            &key_name,
                            &logical_key,
                            modifiers.control_key(),
                            &mut console_state,
                            &mut console_history,
                            &mut label_edit_state,
                            &mut color_picker_state,
                            &mut document,
                            &mut mindmap_tree,
                            &mut app_scene,
                            &mut renderer,
                            &mut scene_cache,
                            &mut keybinds,
                        );
                        return;
                    }

                    // Glyph-wheel color picker key handling.
                    // Mutually exclusive with console and label-edit
                    // for the keys it claims (Esc, Enter, h/s/v/
                    // H/S/V). Any other key — notably the console
                    // trigger `/` — falls through so the Standalone
                    // persistent palette doesn't deadlock the user
                    // out of the normal keybind dispatch.
                    if color_picker_state.is_open() {
                        let consumed = if let Some(doc) = document.as_mut() {
                            handle_color_picker_key(
                                &key_name,
                                &logical_key,
                                &mut color_picker_state,
                                doc,
                                &mut mindmap_tree,
                                &mut picker_dirty,
                                &mut app_scene,
                                &mut renderer,
                            )
                        } else {
                            false
                        };
                        if consumed {
                            return;
                        }
                    }

                    // Session 6D: inline label edit modal. Steals keys
                    // the same way the console does. Escape discards,
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
                                &mut app_scene,
                                &mut renderer,
                            );
                        }
                        return;
                    }

                    // Session 7A: inline node text editor. Steals keys
                    // the same way the console / label-edit modals do.
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
                                &mut app_scene,
                                &mut renderer,
                            );
                        }
                        return;
                    }

                    let action = key_name.as_deref().and_then(|k| {
                        keybinds.action_for(
                            k,
                            modifiers.control_key(),
                            modifiers.shift_key(),
                            modifiers.alt_key(),
                        )
                    });

                    match action {
                        Some(Action::OpenConsole) => {
                            // Toggle: already open → close. Otherwise
                            // construct a fresh state seeded with the
                            // persisted history. Rebuild the overlay
                            // so the frame appears immediately.
                            if console_state.is_open() {
                                save_console_history(&console_history);
                                console_state = ConsoleState::Closed;
                                renderer.rebuild_console_overlay_buffers(&mut app_scene, None);
                            } else {
                                console_state = ConsoleState::open(console_history.clone());
                                if let Some(doc) = document.as_ref() {
                                    rebuild_console_overlay(
                                        &console_state, doc, &mut app_scene, &mut renderer, &keybinds,
                                    );
                                }
                            }
                            return;
                        }
                        Some(Action::Undo) => {
                            if let Some(doc) = document.as_mut() {
                                // Ctrl+Z during an animation
                                // fast-forwards to completion so the
                                // animation's own undo entry lands on
                                // the stack before we pop — Ctrl+Z
                                // reverses the animation's effect in
                                // one keystroke, matching the
                                // post-completion Ctrl+Z behaviour.
                                if doc.has_active_animations() {
                                    doc.fast_forward_animations(mindmap_tree.as_mut());
                                }
                                if doc.undo() {
                                    rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
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
                                        &mut mindmap_tree, &mut app_scene, &mut renderer,
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
                                        &mut mindmap_tree, &mut app_scene, &mut renderer,
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
                                        &mut mindmap_tree, &mut app_scene, &mut renderer,
                                    );
                                }
                            }
                        }
                        Some(Action::DeleteSelection) => {
                            if let Some(doc) = document.as_mut() {
                                if doc.apply_delete_selection() {
                                    rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                }
                            }
                        }
                        Some(Action::CreateOrphanNode) => {
                            if let Some(doc) = document.as_mut() {
                                let canvas_pos = renderer.screen_to_canvas(
                                    cursor_pos.0 as f32, cursor_pos.1 as f32,
                                );
                                doc.create_orphan_and_select(canvas_pos);
                                rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                            }
                        }
                        Some(Action::OrphanSelection) => {
                            if let Some(doc) = document.as_mut() {
                                if doc.apply_orphan_selection_with_undo() {
                                    rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                }
                            }
                        }
                        Some(a @ (Action::EditSelection | Action::EditSelectionClean)) => {
                            // Open the text editor on the selected single
                            // node. `EditSelectionClean` opens with an empty
                            // buffer (the "clean slate" retype gesture);
                            // `EditSelection` opens on the node's current
                            // text with cursor at end. The text-editor
                            // steal at the top of keyboard dispatch means
                            // these never fire while the editor is already
                            // open, so Enter/Backspace stay literal inside
                            // the editor.
                            let clean = matches!(a, Action::EditSelectionClean);
                            if let Some(doc) = document.as_mut() {
                                if let SelectionState::Single(id) = &doc.selection {
                                    let nid = id.clone();
                                    open_text_edit(
                                        &nid,
                                        clean,
                                        doc,
                                        &mut text_edit_state,
                                        &mut mindmap_tree,
                                        &mut app_scene,
                                        &mut renderer,
                                    );
                                }
                            }
                        }
                        None => {
                            // No built-in action matched — try the
                            // user-defined `custom_mutation_bindings`.
                            // If a combo resolves to a mutation id,
                            // apply it on the current Single
                            // selection. Non-Single selections quietly
                            // skip (no status-bar UI yet).
                            if let Some(id) = key_name.as_deref().and_then(|k| {
                                keybinds
                                    .custom_mutation_for(
                                        k,
                                        modifiers.control_key(),
                                        modifiers.shift_key(),
                                        modifiers.alt_key(),
                                    )
                                    .map(|s| s.to_string())
                            }) {
                                if let Some(doc) = document.as_mut() {
                                    if let SelectionState::Single(nid) = doc.selection.clone() {
                                        let mutation =
                                            doc.mutation_registry.get(&id).cloned();
                                        if let (Some(m), Some(tree)) =
                                            (mutation, mindmap_tree.as_mut())
                                        {
                                            doc.apply_custom_mutation(&m, &nid, tree);
                                            scene_cache.clear();
                                            rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                        }
                                    }
                                }
                            }
                        }
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
                                // Border-side dirty-node tracking
                                // disappeared along with the legacy
                                // `rebuild_border_buffers_keyed`
                                // call — the tree-path border helper
                                // takes the full `offsets` map and
                                // rebuilds the canvas-scene tree
                                // wholesale. Connection drag still
                                // needs `dirty_edge_keys` for its
                                // own incremental rebuild below.

                                let scene = doc.build_scene_with_cache(
                                    &offsets,
                                    &mut scene_cache,
                                    renderer.camera_zoom(),
                                );

                                // Tree-path connection rebuild ignores
                                // the dirty set today (Session 2f-perf
                                // tracks the per-edge incremental
                                // shaping cache). Both branches reduce
                                // to the same call.
                                let _ = (first_frame_of_drag, &dirty_edge_keys);
                                update_connection_tree(&scene, &mut app_scene);
                                // Borders go through the canvas-scene
                                // tree path; drag offsets land on the
                                // tree builder via this helper. The
                                // legacy keyed border path used to
                                // patch positions in place (cheap)
                                // while the tree path re-shapes every
                                // border (more expensive) — that's a
                                // known regression to address with a
                                // tree-side incremental cache, see the
                                // unified-rendering plan Session 2f.
                                update_border_tree_with_offsets(doc, &offsets, &mut app_scene);
                                // Labels are emitted per frame (not
                                // cached) so their positions track the
                                // live drag.
                                update_connection_label_tree(&scene, &mut app_scene, &mut renderer);
                                // Portal markers also track the live drag.
                                update_portal_tree(
                                    doc, &offsets, &mut app_scene, &mut renderer,
                                );
                                // Edge handles (anchor / midpoint /
                                // control-point ◆ glyphs on a selected
                                // edge) must also track the live drag.
                                // Without this the handles stay pinned
                                // to the pre-drag positions until mouse
                                // release triggers a full rebuild.
                                update_edge_handle_tree(&scene, &mut app_scene);
                                // Single buffer-walk after the batch.
                                flush_canvas_scene_buffers(&mut app_scene, &mut renderer);
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
                                let _ = &dirty_edge_keys;
                                update_connection_tree(&scene, &mut app_scene);
                                update_edge_handle_tree(&scene, &mut app_scene);
                                // Labels are rebuilt per frame so a
                                // control-point drag keeps the label
                                // correctly anchored to the live path.
                                update_connection_label_tree(&scene, &mut app_scene, &mut renderer);
                                update_portal_tree(
                                    doc,
                                    &std::collections::HashMap::new(),
                                    &mut app_scene,
                                    &mut renderer,
                                );
                                flush_canvas_scene_buffers(&mut app_scene, &mut renderer);
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
                            update_connection_tree(&scene, &mut app_scene);
                            update_connection_label_tree(&scene, &mut app_scene, &mut renderer);
                            update_portal_tree(
                                doc,
                                &HashMap::new(),
                                &mut app_scene,
                                &mut renderer,
                            );
                            // Edge handles (if an edge is selected) must
                            // also follow camera changes — scroll-wheel
                            // zoom with a selected edge used to leave
                            // the handles pinned to stale screen
                            // positions until the next full rebuild.
                            update_edge_handle_tree(&scene, &mut app_scene);
                            flush_canvas_scene_buffers(&mut app_scene, &mut renderer);
                        }
                    }

                    // Picker hover / chip drain. Mouse-move and
                    // chip-focus handlers set `picker_dirty`
                    // whenever HSV state changes; this gate runs
                    // the actual rebuild at most once per
                    // refresh budget. `picker_throttle` self-
                    // tunes via the moving-average mechanism so
                    // a heavy map (where rebuild_scene_only is
                    // expensive) gets fewer drains per second
                    // rather than dropping frames.
                    if picker_dirty && picker_throttle.should_drain() {
                        if let (Some(doc), true) =
                            (document.as_mut(), color_picker_state.is_open())
                        {
                            let work_start = std::time::Instant::now();
                            rebuild_scene_only(doc, &mut app_scene, &mut renderer);
                            rebuild_color_picker_overlay(
                                &mut color_picker_state,
                                doc,
                                &mut app_scene,
                                &mut renderer,
                            );
                            picker_dirty = false;
                            picker_throttle.record_work_duration(work_start.elapsed());
                        } else {
                            // Picker closed between event and drain — drop the dirty flag.
                            picker_dirty = false;
                        }
                    }

                    // Phase 4: tick any active animations. Each
                    // tick lerps the from / to snapshots into the
                    // model and (on completion) routes the final
                    // state through `apply_custom_mutation` so the
                    // standard model-sync + undo-push runs once.
                    // Drives `rebuild_all` only when something
                    // actually advanced — sleeping in Poll mode
                    // when no animations are active is automatic.
                    let animation_advanced = match document.as_mut() {
                        Some(doc) if doc.has_active_animations() => {
                            doc.tick_animations(now_ms() as u64, mindmap_tree.as_mut())
                        }
                        _ => false,
                    };
                    if animation_advanced {
                        if let Some(doc) = document.as_ref() {
                            rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
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
                gfx_arena.clone(),
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
// --- Console line-editor key names ---------------------------------
//
// `keybinds::normalize_key_name` lowercases the winit key identifier,
// so every console-handled key matches the lowercase forms here. Kept
// local to `app.rs` because this is the only module that dispatches
// on them.
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ESCAPE: &str = "escape";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ENTER: &str = "enter";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_TAB: &str = "tab";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ARROW_UP: &str = "arrowup";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_UP: &str = "up";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ARROW_DOWN: &str = "arrowdown";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_DOWN: &str = "down";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ARROW_LEFT: &str = "arrowleft";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_LEFT: &str = "left";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_ARROW_RIGHT: &str = "arrowright";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_RIGHT: &str = "right";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_HOME: &str = "home";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_END: &str = "end";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_BACKSPACE: &str = "backspace";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_DELETE: &str = "delete";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_SPACE: &str = "space";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_CTRL_A: &str = "a";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_CTRL_C: &str = "c";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_CTRL_E: &str = "e";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_CTRL_U: &str = "u";
#[cfg(not(target_arch = "wasm32"))]
const CONSOLE_KEY_CTRL_W: &str = "w";

/// Handle a keystroke while the console is open. The console is a
/// shell-style line editor: char input inserts at the cursor, Tab
/// cycles completions, Up/Down walks history, Enter parses +
/// executes the buffered line, and Escape closes. Regular hotkeys
/// are suppressed — this runs entirely outside the keybinds
/// resolver.
///
/// Cursor arithmetic throughout this function is **grapheme-indexed**,
/// not byte-indexed, to satisfy CODE_CONVENTIONS §2. All mutations
/// route through `baumhard::util::grapheme_chad` so ZWJ emoji and
/// combining marks are treated as atomic cursor cells.
#[cfg(not(target_arch = "wasm32"))]
fn handle_console_key(
    key_name: &Option<String>,
    logical_key: &Key,
    ctrl_pressed: bool,
    console_state: &mut ConsoleState,
    console_history: &mut Vec<String>,
    label_edit_state: &mut LabelEditState,
    color_picker_state: &mut crate::application::color_picker::ColorPickerState,
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    keybinds: &mut ResolvedKeybinds,
) {
    use baumhard::util::grapheme_chad::{
        count_grapheme_clusters, delete_front_unicode, delete_grapheme_at,
        find_byte_index_of_grapheme, insert_str_at_grapheme,
    };

    let name = match key_name.as_deref() {
        Some(n) => n,
        None => return,
    };
    // Ctrl-chords take priority over named / character handling so
    // Ctrl+C / Ctrl+A / etc. don't get swallowed by the `_` branch.
    if ctrl_pressed {
        match name {
            CONSOLE_KEY_CTRL_C => {
                // Clear input without closing — same as the shell
                // muscle-memory: Ctrl+C abandons the current line.
                if let ConsoleState::Open { input, cursor, history_idx, .. } = console_state {
                    input.clear();
                    *cursor = 0;
                    *history_idx = None;
                }
                recompute_console_completions(console_state, document.as_ref());
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
                return;
            }
            CONSOLE_KEY_CTRL_A => {
                if let ConsoleState::Open { cursor, .. } = console_state {
                    *cursor = 0;
                }
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
                return;
            }
            CONSOLE_KEY_CTRL_E => {
                if let ConsoleState::Open { cursor, input, .. } = console_state {
                    *cursor = count_grapheme_clusters(input);
                }
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
                return;
            }
            CONSOLE_KEY_CTRL_U => {
                // Kill to start of line — drop the first `cursor`
                // grapheme clusters via `delete_front_unicode`.
                if let ConsoleState::Open { input, cursor, .. } = console_state {
                    delete_front_unicode(input, *cursor);
                    *cursor = 0;
                }
                recompute_console_completions(console_state, document.as_ref());
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
                return;
            }
            CONSOLE_KEY_CTRL_W => {
                // Kill word before cursor (whitespace-separated).
                // Walk back through grapheme clusters, first skipping
                // trailing whitespace, then the word — everything is
                // kept grapheme-indexed.
                if let ConsoleState::Open { input, cursor, .. } = console_state {
                    let end_g = *cursor;
                    // Collect graphemes up to `end_g` so we can walk
                    // them backwards without re-parsing.
                    use unicode_segmentation::UnicodeSegmentation;
                    let prefix_bytes = find_byte_index_of_grapheme(input, end_g)
                        .unwrap_or(input.len());
                    let clusters: Vec<&str> = input[..prefix_bytes].graphemes(true).collect();
                    let mut start_g = clusters.len();
                    while start_g > 0
                        && clusters[start_g - 1].chars().all(|c| c.is_whitespace())
                    {
                        start_g -= 1;
                    }
                    while start_g > 0
                        && !clusters[start_g - 1].chars().all(|c| c.is_whitespace())
                    {
                        start_g -= 1;
                    }
                    for _ in 0..(end_g - start_g) {
                        delete_grapheme_at(input, start_g);
                    }
                    *cursor = start_g;
                }
                recompute_console_completions(console_state, document.as_ref());
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
                return;
            }
            _ => {}
        }
    }
    match name {
        CONSOLE_KEY_ESCAPE => {
            // Two-tier Esc: if the completion popup is open, first
            // press dismisses it; second press (with no popup)
            // closes the console entirely. Matches the
            // "temporary-UI-first" muscle memory from vim and
            // browser address bars.
            let had_popup = matches!(
                console_state,
                ConsoleState::Open { completions, .. } if !completions.is_empty()
            );
            if had_popup {
                if let ConsoleState::Open { completions, completion_idx, .. } = console_state {
                    completions.clear();
                    *completion_idx = None;
                }
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
            } else {
                save_console_history(console_history);
                *console_state = ConsoleState::Closed;
                renderer.rebuild_console_overlay_buffers(app_scene, None);
            }
        }
        CONSOLE_KEY_ENTER => {
            // Snapshot input, reset state, then parse + execute.
            // Append the executed line to persistent history + the
            // in-state history copy, then re-rebuild the overlay.
            let line = match console_state {
                ConsoleState::Open { input, .. } => std::mem::take(input),
                ConsoleState::Closed => return,
            };
            if let ConsoleState::Open {
                cursor,
                history_idx,
                scrollback,
                completions,
                completion_idx,
                history,
                ..
            } = console_state
            {
                *cursor = 0;
                *history_idx = None;
                completions.clear();
                *completion_idx = None;
                scrollback.push(ConsoleLine::Input(format!("> {}", line)));
                if !line.trim().is_empty() {
                    // Dedup against the most recent history entry —
                    // the shell convention that repeated commands
                    // don't clutter the stack.
                    if history.last().map(|s| s.as_str()) != Some(line.as_str()) {
                        history.push(line.clone());
                        if history.len() > MAX_HISTORY {
                            let drop = history.len() - MAX_HISTORY;
                            history.drain(..drop);
                        }
                        console_history.push(line.clone());
                        if console_history.len() > MAX_HISTORY {
                            let drop = console_history.len() - MAX_HISTORY;
                            console_history.drain(..drop);
                        }
                    }
                }
                if let Some(doc) = document.as_mut() {
                    execute_console_line(
                        &line,
                        console_state,
                        label_edit_state,
                        color_picker_state,
                        doc,
                        mindmap_tree,
                        app_scene,
                        renderer,
                        scene_cache,
                    );
                }
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_TAB => {
            // Tab accepts the highlighted completion (or index 0 if
            // somehow no row is highlighted). The popup is live —
            // it's already populated by the per-keystroke recompute
            // that the character-input arms run below, so Tab has
            // no "first-press compute" branch anymore.
            accept_console_completion(console_state);
            recompute_console_completions(console_state, document.as_ref());
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_ARROW_UP | CONSOLE_KEY_UP => {
            // If the popup is open, Up moves the selection toward
            // the top of the list; otherwise it walks history
            // backwards.
            let moved_in_popup = nav_popup(console_state, -1);
            if !moved_in_popup {
                if let ConsoleState::Open { input, cursor, history, history_idx, .. } = console_state {
                    if !history.is_empty() {
                        let next = match history_idx {
                            None => history.len() - 1,
                            Some(0) => 0,
                            Some(i) => *i - 1,
                        };
                        *history_idx = Some(next);
                        *input = history[next].clone();
                        *cursor = count_grapheme_clusters(input);
                    }
                }
                recompute_console_completions(console_state, document.as_ref());
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_ARROW_DOWN | CONSOLE_KEY_DOWN => {
            // Down moves the selection toward the prompt (the row
            // closest to the input line). Same branch logic as Up.
            let moved_in_popup = nav_popup(console_state, 1);
            if !moved_in_popup {
                if let ConsoleState::Open { input, cursor, history, history_idx, .. } = console_state {
                    match history_idx {
                        Some(i) if *i + 1 < history.len() => {
                            let next = *i + 1;
                            *history_idx = Some(next);
                            *input = history[next].clone();
                            *cursor = count_grapheme_clusters(input);
                        }
                        Some(_) => {
                            *history_idx = None;
                            input.clear();
                            *cursor = 0;
                        }
                        None => {}
                    }
                }
                recompute_console_completions(console_state, document.as_ref());
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_ARROW_LEFT | CONSOLE_KEY_LEFT => {
            if let ConsoleState::Open { cursor, .. } = console_state {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_ARROW_RIGHT | CONSOLE_KEY_RIGHT => {
            if let ConsoleState::Open { cursor, input, .. } = console_state {
                let max = count_grapheme_clusters(input);
                if *cursor < max {
                    *cursor += 1;
                }
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_HOME => {
            if let ConsoleState::Open { cursor, .. } = console_state {
                *cursor = 0;
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_END => {
            if let ConsoleState::Open { cursor, input, .. } = console_state {
                *cursor = count_grapheme_clusters(input);
            }
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_BACKSPACE => {
            if let ConsoleState::Open { input, cursor, .. } = console_state {
                if *cursor > 0 {
                    *cursor -= 1;
                    delete_grapheme_at(input, *cursor);
                }
            }
            recompute_console_completions(console_state, document.as_ref());
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_DELETE => {
            if let ConsoleState::Open { input, cursor, .. } = console_state {
                if *cursor < count_grapheme_clusters(input) {
                    delete_grapheme_at(input, *cursor);
                }
            }
            recompute_console_completions(console_state, document.as_ref());
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        CONSOLE_KEY_SPACE => {
            // winit delivers the spacebar as `Key::Named(NamedKey::Space)`
            // rather than a `Key::Character(" ")`, so the `_` arm below
            // (which only handles `Key::Character`) would drop it. Insert
            // a literal space here instead.
            if let ConsoleState::Open { input, cursor, history_idx, .. } = console_state {
                insert_str_at_grapheme(input, *cursor, " ");
                *cursor += 1;
                *history_idx = None;
            }
            recompute_console_completions(console_state, document.as_ref());
            if let Some(doc) = document.as_ref() {
                rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
            }
        }
        _ => {
            // Character input: insert at cursor, one grapheme at a
            // time. Filter control chars — dead keys / IME can
            // occasionally ship control payloads via
            // `Key::Character` and those must not land in the input
            // buffer as literal glyphs. Inserts go through
            // `insert_str_at_grapheme` so the cursor stays a
            // grapheme index.
            if let Key::Character(c) = logical_key {
                if let ConsoleState::Open {
                    input, cursor, history_idx, ..
                } = console_state
                {
                    for ch in c.as_str().chars() {
                        if ch.is_control() {
                            continue;
                        }
                        let mut buf = [0u8; 4];
                        let encoded = ch.encode_utf8(&mut buf);
                        insert_str_at_grapheme(input, *cursor, encoded);
                        *cursor += 1;
                    }
                    *history_idx = None;
                }
                recompute_console_completions(console_state, document.as_ref());
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
            }
        }
    }
}

/// Re-run the completion engine against the current input and
/// cursor, populating `completions` and defaulting `completion_idx`
/// to the bottom row (row closest to the prompt — which is what
/// Down-then-Tab muscle memory expects to land on first).
#[cfg(not(target_arch = "wasm32"))]
fn recompute_console_completions(
    console_state: &mut ConsoleState,
    document: Option<&MindMapDocument>,
) {
    use baumhard::util::grapheme_chad::find_byte_index_of_grapheme;
    let Some(doc) = document else { return };
    if let ConsoleState::Open {
        input,
        cursor,
        completions,
        completion_idx,
        ..
    } = console_state
    {
        let byte_cursor = find_byte_index_of_grapheme(input, *cursor).unwrap_or(input.len());
        let ctx = ConsoleContext::from_document(doc);
        let new = complete_console(input, byte_cursor, &ctx);
        *completions = new
            .into_iter()
            .map(|c| crate::application::console::completion::Completion {
                text: c.text,
                display: c.display,
                hint: c.hint,
            })
            .collect();
        // Default highlight: the first row. Matches the terminal /
        // IDE convention where the top candidate is "most likely".
        // Users Down-arrow toward the prompt when they want a
        // different row.
        *completion_idx = if completions.is_empty() { None } else { Some(0) };
    }
}

/// Move the completion highlight by `step` (-1 for Up, +1 for Down).
/// Returns `true` if a popup was present and the move was consumed;
/// `false` when there's no popup, letting the caller fall through
/// to history navigation.
#[cfg(not(target_arch = "wasm32"))]
fn nav_popup(console_state: &mut ConsoleState, step: i32) -> bool {
    if let ConsoleState::Open { completions, completion_idx, .. } = console_state {
        if completions.is_empty() {
            return false;
        }
        let len = completions.len() as i32;
        let cur = completion_idx.map(|i| i as i32).unwrap_or(-1);
        let next = ((cur + step).rem_euclid(len)) as usize;
        *completion_idx = Some(next);
        return true;
    }
    false
}

/// Replace the current token (or kv-value slot) under the cursor
/// with the highlighted completion's `text`, advancing the cursor
/// past the replacement.
///
/// Trailing-space rule:
/// - positional / command-name: append a space (next token starts fresh)
/// - kv-key (text ends in `=`): no space (value follows immediately)
/// - kv-value: no space (user may still be typing a quoted value,
///   or wants to type an adjacent kv pair)
///
/// No-op if no popup is present.
#[cfg(not(target_arch = "wasm32"))]
fn accept_console_completion(console_state: &mut ConsoleState) {
    use baumhard::util::grapheme_chad::{count_grapheme_clusters, find_byte_index_of_grapheme};
    use unicode_segmentation::UnicodeSegmentation;
    let ConsoleState::Open {
        input,
        cursor,
        completions,
        completion_idx,
        ..
    } = console_state
    else {
        return;
    };
    if completions.is_empty() {
        return;
    }
    let idx = completion_idx.unwrap_or(completions.len() - 1);
    let Some(cand) = completions.get(idx).cloned() else {
        return;
    };

    // Find the start of the token under the cursor: walk back from
    // the cursor position past non-whitespace grapheme clusters,
    // treating `key=value` as one token (so a kv-value completion
    // replaces only the value portion).
    let cursor_byte = find_byte_index_of_grapheme(input, *cursor).unwrap_or(input.len());
    let before: Vec<&str> = input[..cursor_byte].graphemes(true).collect();
    let mut start_g = before.len();
    while start_g > 0 && !before[start_g - 1].chars().all(|c| c.is_whitespace()) {
        start_g -= 1;
    }
    // If the token contains an `=`, and we're completing a kv-value,
    // the replacement starts *after* the `=`.
    let token: String = before[start_g..].concat();
    let is_kv_value_replace = matches!(token.find('='), Some(pos) if pos > 0);
    let replace_from = if is_kv_value_replace {
        let eq_pos = token.find('=').expect("guarded by is_kv_value_replace");
        let graph_before_eq = token[..eq_pos].graphemes(true).count();
        start_g + graph_before_eq + 1
    } else {
        start_g
    };

    // Delete graphemes from replace_from..cursor, then insert the
    // candidate text at replace_from.
    let replace_from_byte =
        find_byte_index_of_grapheme(input, replace_from).unwrap_or(input.len());
    input.replace_range(replace_from_byte..cursor_byte, &cand.text);
    *cursor = replace_from + count_grapheme_clusters(&cand.text);

    // Trailing space rule: append only when the completion closes a
    // positional / command-name / kv-key (i.e. the next logical
    // thing is a *new* token). A kv-value replacement never gets a
    // trailing space — the user may still be typing a quoted value
    // or an adjacent kv pair directly. A kv-key replacement (text
    // ending in `=`) also gets no space — the value comes next.
    let wants_trailing_space = !is_kv_value_replace && !cand.text.ends_with('=');
    if wants_trailing_space {
        let cursor_byte_after =
            find_byte_index_of_grapheme(input, *cursor).unwrap_or(input.len());
        let next_is_ws = input[cursor_byte_after..]
            .chars()
            .next()
            .map(|c| c.is_whitespace())
            .unwrap_or(true);
        if !next_is_ws {
            input.insert_str(cursor_byte_after, " ");
            *cursor += 1;
        } else if cursor_byte_after == input.len() {
            input.push(' ');
            *cursor += 1;
        }
    }
}

/// Parse and execute a console line. Drains deferred modal handoffs
/// (`open_label_edit`, `open_color_picker`), custom mutation apply
/// requests (`run_mutation`, needs tree access), binding overlay
/// updates (`bind_mutation` / `unbind_mutation`, need
/// `ResolvedKeybinds` access), and alias writes (`set_alias`).
/// Appends the result to the scrollback; rebuilds the scene on any
/// document mutation.
#[cfg(not(target_arch = "wasm32"))]
fn execute_console_line(
    line: &str,
    console_state: &mut ConsoleState,
    label_edit_state: &mut LabelEditState,
    color_picker_state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    if line.trim().is_empty() {
        return;
    }
    let (cmd, args) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => return,
        ParseResult::Unknown(ref head) => {
            push_scrollback_error(
                console_state,
                format!("unknown command: {}", head),
            );
            return;
        }
    };
    let cmd: &'static Command = cmd;
    let mut effects = ConsoleEffects::new(doc);
    let result = (cmd.execute)(&Args::new(&args), &mut effects);
    let label_edit_req = effects.open_label_edit.take();
    let color_picker_req = effects.open_color_picker.take();
    let color_picker_standalone_req = effects.open_color_picker_standalone;
    let color_picker_close_req = effects.close_color_picker;
    let close_after = effects.close_console;

    // Emit the command's result lines into the scrollback.
    match result {
        ExecResult::Ok(s) => {
            if !s.is_empty() {
                push_scrollback_output(console_state, s);
            }
        }
        ExecResult::Err(s) => push_scrollback_error(console_state, s),
        ExecResult::Lines(lines) => {
            for l in lines {
                push_scrollback_output(console_state, l);
            }
        }
    }

    // Any successful command may have mutated the doc; rebuild.
    scene_cache.clear();
    rebuild_all(doc, mindmap_tree, app_scene, renderer);

    if let Some(er) = label_edit_req {
        open_label_edit(&er, doc, label_edit_state, app_scene, renderer);
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if let Some(target) = color_picker_req {
        open_color_picker_contextual(target, doc, color_picker_state, app_scene, renderer);
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if color_picker_standalone_req {
        open_color_picker_standalone(doc, color_picker_state, app_scene, renderer);
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if color_picker_close_req {
        close_color_picker_standalone(
            color_picker_state,
            doc,
            mindmap_tree,
            app_scene,
            renderer,
        );
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if close_after {
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn push_scrollback_output(state: &mut ConsoleState, text: String) {
    if let ConsoleState::Open { scrollback, .. } = state {
        scrollback.push(ConsoleLine::Output(text));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn push_scrollback_error(state: &mut ConsoleState, text: String) {
    if let ConsoleState::Open { scrollback, .. } = state {
        scrollback.push(ConsoleLine::Error(text));
    }
}

/// Build the console overlay geometry from the current state and
/// push it to the renderer. Called whenever the console opens, the
/// input changes, or scrollback / completions update.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_console_overlay(
    console_state: &ConsoleState,
    _document: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    keybinds: &ResolvedKeybinds,
) {
    use crate::application::renderer::{
        ConsoleOverlayCompletion, ConsoleOverlayGeometry, ConsoleOverlayLine,
        ConsoleOverlayLineKind,
    };
    let (input, cursor, scrollback, completions, selected_completion) = match console_state {
        ConsoleState::Closed => {
            renderer.rebuild_console_overlay_buffers(app_scene, None);
            return;
        }
        ConsoleState::Open {
            input,
            cursor,
            scrollback,
            completions,
            completion_idx,
            ..
        } => (input, *cursor, scrollback, completions, *completion_idx),
    };
    let scrollback_lines: Vec<ConsoleOverlayLine> = scrollback
        .iter()
        .map(|l| match l {
            ConsoleLine::Input(t) => ConsoleOverlayLine {
                text: t.clone(),
                kind: ConsoleOverlayLineKind::Input,
            },
            ConsoleLine::Output(t) => ConsoleOverlayLine {
                text: t.clone(),
                kind: ConsoleOverlayLineKind::Output,
            },
            ConsoleLine::Error(t) => ConsoleOverlayLine {
                text: t.clone(),
                kind: ConsoleOverlayLineKind::Error,
            },
        })
        .collect();
    let completion_geo: Vec<ConsoleOverlayCompletion> = completions
        .iter()
        .map(|c| ConsoleOverlayCompletion {
            text: c.text.clone(),
            hint: c.hint.clone(),
        })
        .collect();
    let geometry = ConsoleOverlayGeometry {
        input: input.clone(),
        cursor_grapheme: cursor,
        scrollback: scrollback_lines,
        completions: completion_geo,
        selected_completion,
        font_family: keybinds.console_font.clone(),
        font_size: keybinds.console_font_size,
    };
    renderer.rebuild_console_overlay_buffers(app_scene, Some(&geometry));
}

/// Load persisted console history from `$XDG_STATE_HOME/mandala/history`
/// (or `$HOME/.local/state/mandala/history`). Returns an empty vec
/// on any failure — history is best-effort and must never take the
/// app down.
#[cfg(not(target_arch = "wasm32"))]
fn load_console_history() -> Vec<String> {
    let path = match console_history_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<String> = contents
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();
    if out.len() > MAX_HISTORY {
        let drop = out.len() - MAX_HISTORY;
        out.drain(..drop);
    }
    out
}

/// Write the current history to disk. Best-effort — logs and moves
/// on if the directory can't be created or the file can't be written.
#[cfg(not(target_arch = "wasm32"))]
fn save_console_history(history: &[String]) {
    let path = match console_history_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!("console history: create dir {}: {}", parent.display(), e);
            return;
        }
    }
    let start = history.len().saturating_sub(MAX_HISTORY);
    let body: String = history[start..].join("\n") + "\n";
    if let Err(e) = std::fs::write(&path, body) {
        log::warn!("console history: write {}: {}", path.display(), e);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn console_history_path() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        if !xdg.is_empty() {
            let mut p = PathBuf::from(xdg);
            p.push("mandala");
            p.push("history");
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            let mut p = PathBuf::from(home);
            p.push(".local");
            p.push("state");
            p.push("mandala");
            p.push("history");
            return Some(p);
        }
    }
    None
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
    app_scene: &mut crate::application::scene_host::AppScene,
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

    rebuild_scene_only(doc, app_scene, renderer);

    *mindmap_tree = Some(new_tree);
}

/// Narrower cousin of `rebuild_all` that rebuilds only the flat
/// scene pipeline (connections, borders, edge handles, labels,
/// portals) — NOT the tree (node text buffers, node backgrounds).
/// Used by the glyph-wheel color picker's hover path: a per-frame
/// color preview doesn't change node text, borders, or positions,
/// so the tree rebuild is wasted work. Halves the hot-path cost vs
/// `rebuild_all` on maps with many nodes.
fn rebuild_scene_only(
    doc: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let scene = doc.build_scene_with_selection(renderer.camera_zoom());
    update_connection_tree(&scene, app_scene);
    update_border_tree_static(doc, app_scene);
    update_portal_tree(doc, &std::collections::HashMap::new(), app_scene, renderer);
    update_edge_handle_tree(&scene, app_scene);
    update_connection_label_tree(&scene, app_scene, renderer);
    flush_canvas_scene_buffers(app_scene, renderer);
}

// =====================================================================
// Canvas-tree update helpers.
//
// Each helper builds a baumhard tree for one canvas role and
// registers it into `AppScene`'s canvas sub-scene. **They do not
// re-walk the scene into renderer buffers** — that's the caller's
// responsibility, via `flush_canvas_scene_buffers`. Folding the
// flush into each helper would cost N tree walks per
// rebuild_scene_only call (one per role) when 1 suffices.
// =====================================================================

/// Build the border tree (no drag offsets) and register it under
/// [`CanvasRole::Borders`]. Caller must follow with
/// [`flush_canvas_scene_buffers`] before the next render.
fn update_border_tree_static(
    doc: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_border_tree_with_offsets(doc, &std::collections::HashMap::new(), app_scene);
}

fn update_border_tree_with_offsets(
    doc: &MindMapDocument,
    offsets: &std::collections::HashMap<String, (f32, f32)>,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    use crate::application::scene_host::CanvasRole;
    let tree = baumhard::mindmap::tree_builder::build_border_tree(&doc.mindmap, offsets);
    app_scene.register_canvas(CanvasRole::Borders, tree, glam::Vec2::ZERO);
}

/// Build or in-place update the portal tree under
/// [`CanvasRole::Portals`]. Selection-cyan and color-preview
/// override rules mirror `scene_builder::build_scene`. Hands the
/// AABB-keyed hitbox map back to the renderer so the legacy
/// `Renderer::hit_test_portal` keeps working until hit-test
/// routing migrates to [`Scene::component_at`].
///
/// **§B2 dispatch.** Drag, color-preview, and selection toggle
/// all leave the visible-portal *identity sequence* unchanged —
/// the same pairs in the same order, only their positions /
/// colors / regions move. For those continuous interactions we
/// take the in-place mutator path
/// (`build_portal_mutator_tree_from_pairs` →
/// `apply_canvas_mutator`), which reuses the existing tree arena
/// instead of allocating a new one each frame. When portals are
/// added, removed, or a fold reveals/hides an endpoint, the
/// identity sequence shifts and we fall back to a full rebuild.
/// Mirrors the canonical pattern from the picker (commit
/// `ceaeeb4`), now applied to a nested-channel tree.
fn update_portal_tree(
    doc: &MindMapDocument,
    offsets: &std::collections::HashMap<String, (f32, f32)>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::document::ColorPickerPreview;
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
    use baumhard::mindmap::tree_builder::{
        build_portal_mutator_tree_from_pairs, build_portal_tree_from_pairs,
        portal_identity_sequence, portal_pair_data, PortalColorPreviewRef, SelectedPortalRef,
    };

    let selected_owned = doc
        .selection
        .selected_portal()
        .map(|p| (p.label.clone(), p.endpoint_a.clone(), p.endpoint_b.clone()));
    let selected: Option<SelectedPortalRef> = selected_owned
        .as_ref()
        .map(|(l, a, b)| (l.as_str(), a.as_str(), b.as_str()));

    let preview: Option<PortalColorPreviewRef> = match &doc.color_picker_preview {
        Some(ColorPickerPreview::Portal { key, color }) => Some(PortalColorPreviewRef {
            label: key.label.as_str(),
            endpoint_a: key.endpoint_a.as_str(),
            endpoint_b: key.endpoint_b.as_str(),
            color: color.as_str(),
        }),
        _ => None,
    };

    let pairs = portal_pair_data(&doc.mindmap, offsets, selected, preview);
    let signature = hash_canvas_signature(&portal_identity_sequence(&pairs));

    match app_scene.canvas_dispatch(CanvasRole::Portals, signature) {
        CanvasDispatch::InPlaceMutator => {
            let result = build_portal_mutator_tree_from_pairs(&pairs);
            renderer.set_portal_hitboxes(result.hitboxes);
            app_scene.apply_canvas_mutator(CanvasRole::Portals, &result.mutator);
        }
        CanvasDispatch::FullRebuild => {
            let result = build_portal_tree_from_pairs(&pairs);
            renderer.set_portal_hitboxes(result.hitboxes);
            app_scene.register_canvas(CanvasRole::Portals, result.tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(CanvasRole::Portals, signature);
        }
    }
}

/// Build or in-place update the connection tree under
/// [`CanvasRole::Connections`].
///
/// **§B2 dispatch.** Selection toggle, color preview, and theme
/// switches change only per-glyph fields (color regions, body
/// glyph) without altering the per-edge structural shape (cap
/// presence, body-glyph count). For those calls we take the
/// in-place mutator path. Endpoint drag resamples the path and
/// the body-glyph count typically shifts every few pixels — the
/// identity sequence drops the equality and we fall back to a
/// full rebuild. The dispatcher hashes
/// `connection_identity_sequence` to make the choice.
fn update_connection_tree(
    scene: &baumhard::mindmap::scene_builder::RenderScene,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
    use baumhard::mindmap::tree_builder::{
        build_connection_mutator_tree, build_connection_tree, connection_identity_sequence,
    };

    let signature =
        hash_canvas_signature(&connection_identity_sequence(&scene.connection_elements));
    match app_scene.canvas_dispatch(CanvasRole::Connections, signature) {
        CanvasDispatch::InPlaceMutator => {
            let mutator = build_connection_mutator_tree(&scene.connection_elements);
            app_scene.apply_canvas_mutator(CanvasRole::Connections, &mutator);
        }
        CanvasDispatch::FullRebuild => {
            let tree = build_connection_tree(&scene.connection_elements);
            app_scene.register_canvas(CanvasRole::Connections, tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(CanvasRole::Connections, signature);
        }
    }
}

/// Build or in-place update the connection-label tree under
/// [`CanvasRole::ConnectionLabels`]. Threads the per-edge AABB
/// hitbox map back to the renderer so `hit_test_edge_label`
/// keeps working.
///
/// **§B2 dispatch.** Inline label edits (Phase 2.1's hot path),
/// color changes, and label movement keep the structural identity
/// (the per-edge `EdgeKey` sequence) stable; the in-place mutator
/// path runs and the arena is reused. Adding or removing a label,
/// or selection-edge reorderings, change the identity and
/// trigger a full rebuild.
fn update_connection_label_tree(
    scene: &baumhard::mindmap::scene_builder::RenderScene,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
    use baumhard::mindmap::tree_builder::{
        build_connection_label_mutator_tree, build_connection_label_tree,
        connection_label_identity_sequence,
    };

    let signature = hash_canvas_signature(&connection_label_identity_sequence(
        &scene.connection_label_elements,
    ));
    match app_scene.canvas_dispatch(CanvasRole::ConnectionLabels, signature) {
        CanvasDispatch::InPlaceMutator => {
            let result = build_connection_label_mutator_tree(&scene.connection_label_elements);
            renderer.set_connection_label_hitboxes(result.hitboxes);
            app_scene.apply_canvas_mutator(CanvasRole::ConnectionLabels, &result.mutator);
        }
        CanvasDispatch::FullRebuild => {
            let result = build_connection_label_tree(&scene.connection_label_elements);
            renderer.set_connection_label_hitboxes(result.hitboxes);
            app_scene.register_canvas(
                CanvasRole::ConnectionLabels,
                result.tree,
                glam::Vec2::ZERO,
            );
            app_scene.set_canvas_signature(CanvasRole::ConnectionLabels, signature);
        }
    }
}

/// Build or in-place update the edge-handle tree under
/// [`CanvasRole::EdgeHandles`].
///
/// **§B2 dispatch.** Dragging a handle moves only its position;
/// the handle set's *identity sequence* (the
/// kind-derived channels emitted by
/// [`baumhard::mindmap::tree_builder::edge_handle_identity_sequence`])
/// stays constant for the duration of one drag. We take the in-place
/// mutator path under that condition, reusing the existing arena
/// instead of allocating a fresh one each frame. When the handle
/// set's structure shifts — selection moves to a different edge
/// shape, or a midpoint drag spawns a control point — the identity
/// sequence changes and we fall back to a full rebuild. Mirrors the
/// dispatch shape used in `update_portal_tree`.
fn update_edge_handle_tree(
    scene: &baumhard::mindmap::scene_builder::RenderScene,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
    use baumhard::mindmap::tree_builder::{
        build_edge_handle_mutator_tree, build_edge_handle_tree,
        edge_handle_identity_sequence,
    };

    let signature = hash_canvas_signature(&edge_handle_identity_sequence(&scene.edge_handles));
    match app_scene.canvas_dispatch(CanvasRole::EdgeHandles, signature) {
        CanvasDispatch::InPlaceMutator => {
            let mutator = build_edge_handle_mutator_tree(&scene.edge_handles);
            app_scene.apply_canvas_mutator(CanvasRole::EdgeHandles, &mutator);
        }
        CanvasDispatch::FullRebuild => {
            let tree = build_edge_handle_tree(&scene.edge_handles);
            app_scene.register_canvas(CanvasRole::EdgeHandles, tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(CanvasRole::EdgeHandles, signature);
        }
    }
}

/// Walk every canvas-scene tree once and rebuild the renderer's
/// `canvas_scene_buffers`. Call this **once** after a batch of
/// `update_*_tree` invocations — calling it inside each helper
/// would multiply the per-frame shaping cost by the number of
/// roles touched.
fn flush_canvas_scene_buffers(
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    renderer.rebuild_canvas_scene_buffers(app_scene);
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
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let edge = match doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)) {
        Some(e) => e,
        None => return,
    };
    let original = edge.label.clone();
    let buffer = original.clone().unwrap_or_default();
    // Cursor lands at the end of the existing label, matching the
    // `TextEditState` open-on-existing-text behaviour.
    let cursor_grapheme_pos = grapheme_chad::count_grapheme_clusters(&buffer);
    *label_edit_state = LabelEditState::Open {
        edge_ref: edge_ref.clone(),
        buffer: buffer.clone(),
        cursor_grapheme_pos,
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
    doc.label_edit_preview = Some((edge_key, insert_caret(&buffer, cursor_grapheme_pos)));
    // Rebuild labels so the caret is visible immediately. The caller
    // already ran `rebuild_all` before this, so the scene is fresh.
    let scene = doc.build_scene_with_selection(renderer.camera_zoom());
    update_connection_label_tree(&scene, app_scene, renderer);
    update_portal_tree(doc, &std::collections::HashMap::new(), app_scene, renderer);
}

/// Session 6D + Phase 2.1: route a keystroke to the inline label
/// editor. Escape discards, Enter commits, navigation keys move the
/// grapheme cursor, Backspace/Delete remove a grapheme cluster
/// (never a stray byte), printable characters insert at the cursor.
///
/// Mirrors [`handle_text_edit_key`] in shape: every text mutation
/// goes through `grapheme_chad` so emoji and ZWJ clusters survive
/// edits intact (CODE_CONVENTIONS §1). Multi-line is intentionally
/// out of scope — labels are short, single-line; Enter commits, not
/// inserts. Cursor navigation is constrained to the one row.
#[cfg(not(target_arch = "wasm32"))]
fn handle_label_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    label_edit_state: &mut LabelEditState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let name = key_name.as_deref();
    if name == Some("escape") {
        close_label_edit(false, doc, label_edit_state, mindmap_tree, app_scene, renderer);
        return;
    }
    if name == Some("enter") {
        close_label_edit(true, doc, label_edit_state, mindmap_tree, app_scene, renderer);
        return;
    }

    let Some((buffer, cursor)) = (match label_edit_state {
        LabelEditState::Open {
            buffer,
            cursor_grapheme_pos,
            ..
        } => Some((buffer, cursor_grapheme_pos)),
        LabelEditState::Closed => None,
    }) else {
        return;
    };

    let typed = match logical_key {
        Key::Character(c) => Some(c.as_str()),
        _ => None,
    };
    if !route_label_edit_key(name, typed, buffer, cursor) {
        return;
    }

    // Refresh the preview on the document so the caret + edited text
    // render on the next frame. The connection-label tree's §B2
    // mutator path (Phase 1.3) picks up the new text without
    // rebuilding the arena because the per-edge identity sequence
    // stays constant during a label edit.
    if let LabelEditState::Open {
        edge_ref,
        buffer,
        cursor_grapheme_pos,
        ..
    } = label_edit_state
    {
        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
            &edge_ref.from_id,
            &edge_ref.to_id,
            &edge_ref.edge_type,
        );
        doc.label_edit_preview = Some((edge_key, insert_caret(buffer, *cursor_grapheme_pos)));
        let scene = doc.build_scene_with_selection(renderer.camera_zoom());
        update_connection_label_tree(&scene, app_scene, renderer);
        update_portal_tree(doc, &std::collections::HashMap::new(), app_scene, renderer);
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
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let (edge_ref, buffer, original) = match std::mem::replace(label_edit_state, LabelEditState::Closed) {
        LabelEditState::Open { edge_ref, buffer, original, .. } => (edge_ref, buffer, original),
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
    rebuild_all(doc, mindmap_tree, app_scene, renderer);
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
    app_scene: &mut crate::application::scene_host::AppScene,
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
        app_scene,
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
    app_scene: &mut crate::application::scene_host::AppScene,
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
    rebuild_all(doc, mindmap_tree, app_scene, renderer);
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
    app_scene: &mut crate::application::scene_host::AppScene,
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
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let name = key_name.as_deref();
    if name == Some("escape") {
        close_text_edit(false, doc, text_edit_state, mindmap_tree, app_scene, renderer);
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
            app_scene,
            renderer,
        );
    }
}

// =====================================================================
// Glyph-wheel color picker handlers
// =====================================================================

/// Open the color picker in contextual mode, bound to the given
/// target. Resolves the target ref to a concrete handle, seeds HSV
/// from the target's currently-displayed color, and shows the
/// modal-style wheel. Commit writes to the bound target; Esc and
/// outside-click cancel (restore the original). See
/// [`open_color_picker_standalone`] for the persistent-palette flavor
/// that writes to the document's current selection.
#[cfg(not(target_arch = "wasm32"))]
fn open_color_picker_contextual(
    target: crate::application::color_picker::ColorTarget,
    doc: &mut MindMapDocument,
    state: &mut crate::application::color_picker::ColorPickerState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{current_hsv_at, PickerMode};

    // Resolve the target to a picker handle up front. If the
    // edge / portal / node was deleted between the open trigger
    // and Enter being pressed, warn and bail — the picker never
    // opens. Should never happen because the dispatcher runs
    // synchronously, but defensive.
    let handle = match target.resolve(doc) {
        Some(h) => h,
        None => {
            log::warn!("color picker: target ref did not resolve; ignoring open");
            return;
        }
    };

    // Seed HSV from the currently-displayed (possibly theme-resolved)
    // color so the picker opens right where the user already is.
    let hsv = current_hsv_at(doc, &handle);

    // Seed the document preview so the initial render already shows
    // the same HSV the picker opened at. Overwritten on the next
    // hover frame, but this avoids a one-frame flash of the original
    // color when the picker opens. Nodes don't have a scene-builder
    // preview path yet (commit-only for the first version of the
    // `color bg/text/border` picker flow), so the helper is a no-op
    // for the Node arm.
    seed_initial_preview(doc, &handle, hsv.0, hsv.1, hsv.2);

    open_picker_inner(
        PickerMode::Contextual { handle },
        hsv,
        doc,
        state,
        app_scene,
        renderer,
    );
}

/// Open the color picker in standalone mode — a persistent palette
/// that applies the current HSV to the document's selection on ࿕
/// click and stays open until dismissed via `color picker off`.
#[cfg(not(target_arch = "wasm32"))]
fn open_color_picker_standalone(
    doc: &mut MindMapDocument,
    state: &mut crate::application::color_picker::ColorPickerState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::PickerMode;

    // Seed to a plausible starting color (red, full saturation, full
    // value). The user will nudge within seconds, so the exact seed
    // doesn't matter much — red-at-the-top matches the hue ring's
    // 12-o'clock slot so the wheel opens with the ring's "start" cell
    // highlighted.
    let hsv = (0.0_f32, 1.0_f32, 1.0_f32);
    open_picker_inner(PickerMode::Standalone, hsv, doc, state, app_scene, renderer);
}

/// Shared picker-open core: measures glyph advances (one font-system
/// lock per open, amortized across the whole session) and writes the
/// `Open` state. Split out so Contextual and Standalone modes share a
/// single measurement pass and state-init shape.
#[cfg(not(target_arch = "wasm32"))]
fn open_picker_inner(
    mode: crate::application::color_picker::PickerMode,
    (hue_deg, sat, val): (f32, f32, f32),
    doc: &mut MindMapDocument,
    state: &mut crate::application::color_picker::ColorPickerState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{
        arm_bottom_glyphs, arm_left_glyphs, arm_right_glyphs, arm_top_glyphs, hue_ring_font_scale,
        hue_ring_glyphs, ColorPickerState,
    };

    // Measure the widest shaped advance across every crosshair-arm
    // glyph and every hue-ring glyph. These become the spacing units
    // the layout pure-fn uses for cell and ring-slot positions —
    // measuring once here avoids per-hover font-system traffic and
    // keeps `compute_color_picker_layout` pure. Both measurements
    // happen behind the font-system write lock, which is also what
    // the renderer's buffer builders need, so we grab it once.
    // Measurement font size: pick the spec's `font_max` so the
    // ratios captured here are accurate across the full
    // `[font_min, font_max]` range the layout fn might pick. The
    // ratios `max_cell_advance / measurement_font_size` and
    // `max_ring_advance / (measurement_font_size * ring_scale)` are
    // dimensionless and stable across font sizes (cosmic-text
    // shapes proportionally), so the layout can scale them to
    // whatever font_size it derives from the window-size formula.
    let geom = &crate::application::widgets::color_picker_widget::load_spec().geometry;
    let measurement_font_size: f32 = geom.font_max;
    let ring_font_size = measurement_font_size * hue_ring_font_scale();
    let (max_cell_advance, max_ring_advance) = {
        let mut font_system = baumhard::font::fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");
        let mut crosshair: Vec<&str> = Vec::with_capacity(40);
        crosshair.extend(arm_top_glyphs().iter().copied());
        crosshair.extend(arm_bottom_glyphs().iter().copied());
        crosshair.extend(arm_left_glyphs().iter().copied());
        crosshair.extend(arm_right_glyphs().iter().copied());
        let cell = crate::application::renderer::measure_max_glyph_advance(
            &mut font_system,
            &crosshair,
            measurement_font_size,
        );
        let ring_glyphs: Vec<&str> = hue_ring_glyphs().iter().copied().collect();
        let ring = crate::application::renderer::measure_max_glyph_advance(
            &mut font_system,
            &ring_glyphs,
            ring_font_size,
        );
        (cell, ring)
    };

    *state = ColorPickerState::Open {
        mode,
        hue_deg,
        sat,
        val,
        last_cursor_pos: None,
        max_cell_advance,
        max_ring_advance,
        measurement_font_size,
        layout: None,
        center_override: None,
        size_scale: 1.0,
        gesture: None,
        hovered_hit: None,
        pending_error_flash: false,
    };

    rebuild_color_picker_overlay(state, doc, app_scene, renderer);
    rebuild_scene_only(doc, app_scene, renderer);
}

/// Helper: write the initial HSV into `doc.color_picker_preview` on
/// picker open so the first rendered frame already shows the
/// previewed color instead of the model's stored one.
#[cfg(not(target_arch = "wasm32"))]
fn seed_initial_preview(
    doc: &mut MindMapDocument,
    handle: &crate::application::color_picker::PickerHandle,
    hue_deg: f32,
    sat: f32,
    val: f32,
) {
    use crate::application::color_picker::PickerHandle;
    use crate::application::document::ColorPickerPreview;
    use baumhard::util::color::hsv_to_hex;

    let hex = hsv_to_hex(hue_deg, sat, val);
    match handle {
        PickerHandle::Edge(index) => {
            if let Some(edge) = doc.mindmap.edges.get(*index) {
                let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
                doc.color_picker_preview = Some(ColorPickerPreview::Edge { key, color: hex });
            }
        }
        PickerHandle::Portal(index) => {
            if let Some(portal) = doc.mindmap.portals.get(*index) {
                let key = baumhard::mindmap::scene_builder::PortalRefKey::from_portal(portal);
                doc.color_picker_preview = Some(ColorPickerPreview::Portal { key, color: hex });
            }
        }
        PickerHandle::Node { .. } => {
            // Node preview not yet plumbed through the scene
            // builder — commit-only for v1. The picker still opens
            // and lets the user pick + commit; it just doesn't
            // hover-preview on the underlying node.
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
    // (with its fixed-size cell-position arrays) every
    // hover.
    let (
        target_label,
        hue_deg,
        sat,
        val,
        last_cursor_pos,
        max_cell_advance,
        max_ring_advance,
        measurement_font_size,
        size_scale,
        cached_backdrop,
        center_override,
        hovered_hit,
    ) = match state {
        ColorPickerState::Closed => return None,
        ColorPickerState::Open {
            mode,
            hue_deg,
            sat,
            val,
            last_cursor_pos,
            max_cell_advance,
            max_ring_advance,
            measurement_font_size,
            layout,
            center_override,
            size_scale,
            hovered_hit,
            ..
        } => (
            match mode {
                crate::application::color_picker::PickerMode::Contextual { handle } => {
                    handle.label()
                }
                crate::application::color_picker::PickerMode::Standalone => "",
            },
            *hue_deg,
            *sat,
            *val,
            *last_cursor_pos,
            *max_cell_advance,
            *max_ring_advance,
            *measurement_font_size,
            *size_scale,
            layout.as_ref().map(|l| l.backdrop),
            *center_override,
            *hovered_hit,
        ),
    };

    // Hex readout is visible when the cursor is inside the backdrop.
    // Without a cached layout from a previous rebuild we can't
    // hit-test the backdrop, so the first rebuild lands without the
    // hex showing; it appears on the first hover rebuild after the
    // cursor enters the window.
    let hex_visible = match (last_cursor_pos, cached_backdrop) {
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
        hex_visible,
        max_cell_advance,
        max_ring_advance,
        measurement_font_size,
        size_scale,
        center_override,
        hovered_hit,
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

/// Picker overlay update entry point. Dispatches between the
/// initial-build path and the §B2-compliant in-place mutator path:
///
/// - **Closed** (`compute_picker_geometry` returns `None`): unregister
///   the overlay tree by passing `None` to the buffer rebuild.
/// - **First open** (no tree registered): build a fresh tree via
///   [`Renderer::rebuild_color_picker_overlay_buffers`].
/// - **Already open** (tree exists in `AppScene`): apply a
///   `MutatorTree<GfxMutator>` of `Assign`-style `DeltaGlyphArea`s
///   keyed by stable channel, mutating the existing arena in place.
///   This is the §B2 "mutation, not rebuild" path — it still
///   re-shapes every cell through cosmic-text (the §B1 perf gap
///   tracked in `ROADMAP.md` as the hash-keyed shape cache
///   follow-up), but the picker's tree arena is no longer
///   re-allocated per hover.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_color_picker_overlay(
    state: &mut crate::application::color_picker::ColorPickerState,
    _doc: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::scene_host::OverlayRole;
    let Some(geometry) = compute_picker_geometry(state, renderer) else {
        renderer.rebuild_color_picker_overlay_buffers(app_scene, None);
        return;
    };
    if app_scene.overlay_id(OverlayRole::ColorPicker).is_some() {
        renderer.apply_color_picker_overlay_mutator(app_scene, &geometry);
    } else {
        renderer.rebuild_color_picker_overlay_buffers(app_scene, Some(&geometry));
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
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::ColorPickerState;

    if matches!(state, ColorPickerState::Closed) {
        return;
    }
    *state = ColorPickerState::Closed;
    doc.color_picker_preview = None;
    renderer.rebuild_color_picker_overlay_buffers(app_scene, None);
    rebuild_all(doc, mindmap_tree, app_scene, renderer);
}

/// Close the standalone color picker without committing. Called by
/// the `color picker off` console command. Functionally identical to
/// `cancel_color_picker` — both close the picker and clear the
/// transient preview — but named distinctly because Standalone mode
/// has no "original" to cancel back to; the function exists so
/// call-sites read clearly.
#[cfg(not(target_arch = "wasm32"))]
fn close_color_picker_standalone(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    cancel_color_picker(state, doc, mindmap_tree, app_scene, renderer);
}

/// Commit the picker's currently-previewed HSV value via the regular
/// `set_edge_color` / `set_portal_color` / `set_node_*_color` path —
/// a single undo entry is pushed and `ensure_glyph_connection` runs
/// its fork-on-first-edit only at this moment (never during hover).
/// Close the modal.
///
/// The picker only commits concrete HSV hex values now that the
/// theme-variable chip row has been retired; theme-variable editing
/// lives elsewhere in the UI.
#[cfg(not(target_arch = "wasm32"))]
fn commit_color_picker(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{ColorPickerState, NodeColorAxis, PickerHandle};
    use baumhard::util::color::hsv_to_hex;

    let (handle, hue_deg, sat, val) = match state {
        ColorPickerState::Open {
            mode: crate::application::color_picker::PickerMode::Contextual { handle },
            hue_deg,
            sat,
            val,
            ..
        } => (handle.clone(), *hue_deg, *sat, *val),
        // Standalone mode has no bound target — commit is handled by
        // `commit_color_picker_to_selection` instead; this function
        // is Contextual-only. Being reached in Standalone mode means
        // the caller picked the wrong commit path.
        ColorPickerState::Open { .. } => {
            log::warn!(
                "commit_color_picker called in non-contextual mode; \
                 use commit_color_picker_to_selection for Standalone mode"
            );
            return;
        }
        ColorPickerState::Closed => return,
    };

    // Close the modal state first so the subsequent rebuilds don't
    // re-apply the preview.
    *state = ColorPickerState::Closed;
    doc.color_picker_preview = None;

    let hex = hsv_to_hex(hue_deg, sat, val);
    match handle {
        PickerHandle::Edge(index) => {
            let er = doc
                .mindmap
                .edges
                .get(index)
                .map(|e| EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type));
            if let Some(er) = er {
                doc.set_edge_color(&er, Some(&hex));
            }
        }
        PickerHandle::Portal(index) => {
            let pr = doc.mindmap.portals.get(index).map(|p| {
                crate::application::document::PortalRef::new(
                    p.label.clone(),
                    p.endpoint_a.clone(),
                    p.endpoint_b.clone(),
                )
            });
            if let Some(pr) = pr {
                doc.set_portal_color(&pr, &hex);
            }
        }
        PickerHandle::Node { id, axis } => {
            match axis {
                NodeColorAxis::Bg => {
                    doc.set_node_bg_color(&id, hex);
                }
                NodeColorAxis::Text => {
                    doc.set_node_text_color(&id, hex);
                }
                NodeColorAxis::Border => {
                    doc.set_node_border_color(&id, hex);
                }
            }
        }
    }

    renderer.rebuild_color_picker_overlay_buffers(app_scene, None);
    rebuild_all(doc, mindmap_tree, app_scene, renderer);
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
    picker_dirty: &mut bool,
) {
    use crate::application::color_picker::{ColorPickerState, PickerHandle};
    use crate::application::document::ColorPickerPreview;
    use baumhard::util::color::hsv_to_hex;

    let (handle, hue_deg, sat, val) = match state {
        ColorPickerState::Open {
            mode,
            hue_deg,
            sat,
            val,
            ..
        } => {
            let handle = match mode {
                crate::application::color_picker::PickerMode::Contextual { handle } => {
                    Some(handle.clone())
                }
                // Standalone mode has no bound target — nothing to
                // preview on the scene. The ࿕ glyph in the wheel
                // still shows the current HSV (rendered by the picker
                // overlay itself), so the user gets immediate
                // feedback without needing doc.color_picker_preview.
                crate::application::color_picker::PickerMode::Standalone => None,
            };
            (handle, *hue_deg, *sat, *val)
        }
        ColorPickerState::Closed => return,
    };
    let hex = hsv_to_hex(hue_deg, sat, val);
    if let Some(handle) = handle {
        match handle {
            PickerHandle::Edge(index) => {
                if let Some(edge) = doc.mindmap.edges.get(index) {
                    let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
                    doc.color_picker_preview =
                        Some(ColorPickerPreview::Edge { key, color: hex });
                }
            }
            PickerHandle::Portal(index) => {
                if let Some(portal) = doc.mindmap.portals.get(index) {
                    let key = baumhard::mindmap::scene_builder::PortalRefKey::from_portal(portal);
                    doc.color_picker_preview =
                        Some(ColorPickerPreview::Portal { key, color: hex });
                }
            }
            PickerHandle::Node { .. } => {
                // Node preview lives on the tree pipeline, not the
                // scene pipeline — not yet wired. Commit-only for v1.
            }
        }
    }
    // Scene + picker rebuilds are deferred to the `AboutToWait`
    // drain via `picker_dirty`. Mouse moves come in at ~120Hz on
    // modern hardware; without this gate every event would
    // re-shape every border / connection / portal on the map
    // plus the picker overlay. The drain is gated by
    // `picker_throttle` (the same `MutationFrequencyThrottle`
    // type the drag path uses), which self-tunes to keep the
    // per-frame work under the refresh budget.
    *picker_dirty = true;
}

/// Route a keystroke to the picker. Esc cancels (contextual only;
/// ignored in standalone), Enter commits, h/H ±15° hue, s/S ±0.1
/// sat, v/V ±0.1 val. Any other key falls through to normal
/// keybind dispatch.
#[cfg(not(target_arch = "wasm32"))]
fn handle_color_picker_key(
    key_name: &Option<String>,
    logical_key: &Key,
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    picker_dirty: &mut bool,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) -> bool {
    use crate::application::color_picker::ColorPickerState;

    let name = key_name.as_deref();
    let is_standalone = state.is_standalone();
    match name {
        Some("escape") => {
            if is_standalone {
                // Standalone mode ignores Escape — the persistent
                // palette only closes via `color picker off` from
                // the console. Don't consume the key — let it
                // flow through to normal keybind dispatch so the
                // user can e.g. close the console if they've
                // summoned it.
                return false;
            }
            cancel_color_picker(state, doc, mindmap_tree, app_scene, renderer);
            return true;
        }
        Some("enter") => {
            if is_standalone {
                // Standalone: Enter behaves like clicking ࿕ —
                // applies the current HSV to the document
                // selection, stays open.
                commit_color_picker_to_selection(
                    state,
                    doc,
                    mindmap_tree,
                    app_scene,
                    renderer,
                );
                return true;
            }
            commit_color_picker(state, doc, mindmap_tree, app_scene, renderer);
            return true;
        }
        _ => {}
    }
    // Character keys: h/s/v nudges. Use logical_key to keep this
    // case-sensitive (uppercase = bigger nudge). Non-matching
    // characters fall through so the user can e.g. press `/` to
    // open the console while the Standalone palette is active.
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
            apply_picker_preview(state, doc, picker_dirty);
            return true;
        }
        // Character key but not one of ours — fall through.
        return false;
    }
    // Any non-character key that didn't match an explicit arm
    // above (arrow keys, function keys, modifier-only, etc.) —
    // let it pass through to normal keybind dispatch.
    false
}

/// Mouse-move handler for the picker. Branches on active-drag vs
/// hover:
///
/// - **Drag active**: translate the wheel so
///   `center = cursor + grab_offset`. Every layout position (ring,
///   bars, chips, backdrop) rebuilds against the new center via
///   `center_override`.
/// - **Hover**: hit-test the cursor, update HSV / chip focus to match
///   the hovered glyph (live preview), and record
///   `hovered_hit` for the renderer's hover-grow effect.
///
/// Returns `true` when the picker consumed the move and the caller
/// should stop dispatching it. Returns `false` when the move
/// should fall through to normal canvas hover — the Standalone
/// palette with no active gesture and the cursor outside its
/// backdrop is the one case today, so the user can still see
/// button-node cursor changes on the canvas while the palette
/// floats above it.
#[cfg(not(target_arch = "wasm32"))]
fn handle_color_picker_mouse_move(
    cursor_pos: (f64, f64),
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    picker_dirty: &mut bool,
) -> bool {
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

    // Active gesture takes priority: while the wheel is being
    // dragged or resized, every cursor move feeds the gesture
    // instead of hit-testing for hover. The two gestures are
    // mutually exclusive — `gesture` holds at most one variant.
    if let ColorPickerState::Open {
        gesture: Some(g),
        center_override,
        size_scale,
        ..
    } = state
    {
        match *g {
            crate::application::color_picker::PickerGesture::Move { grab_offset } => {
                let new_center = (cursor.0 + grab_offset.0, cursor.1 + grab_offset.1);
                *center_override = Some(new_center);
            }
            crate::application::color_picker::PickerGesture::Resize {
                anchor_radius,
                anchor_scale,
                anchor_center,
            } => {
                // Multiplicative scale change: new_scale =
                // anchor_scale * (current_radius / anchor_radius),
                // floored on the input side at the same `font * 3`
                // anchor_radius cap so the ratio stays well-behaved
                // throughout the gesture. Clamps from the spec.
                let dx = cursor.0 - anchor_center.0;
                let dy = cursor.1 - anchor_center.1;
                let raw_r = (dx * dx + dy * dy).sqrt();
                let r_now = raw_r.max(anchor_radius * 0.1);
                let geom = &crate::application::widgets::color_picker_widget::load_spec()
                    .geometry;
                *size_scale = (anchor_scale * (r_now / anchor_radius))
                    .clamp(geom.resize_scale_min, geom.resize_scale_max);
            }
        }
        *picker_dirty = true;
        return true;
    }

    let hit = if let ColorPickerState::Open { layout: Some(layout), .. } = state {
        hit_test_picker(layout, cursor.0, cursor.1)
    } else {
        // Picker closed, or open but the first rebuild hasn't happened
        // yet — no cached layout to hit-test against. The open path
        // always rebuilds before releasing control, so this branch is
        // only reachable during the ~1-line window between construction
        // and the first rebuild call.
        return false;
    };

    // Standalone mode + cursor outside the backdrop: don't consume
    // the move. The canvas underneath should still update its own
    // hover state (button-node cursor, etc.) — the persistent
    // palette is meant to coexist with ordinary canvas work, not
    // block it.
    if state.is_standalone() && matches!(hit, PickerHit::Outside) {
        return false;
    }

    // Only mark dirty when the picker's interactive state
    // actually moved. Mouse events arrive at ~120 Hz and the
    // user can drag many cursor pixels within the same hue
    // slot or sat/val cell; without this gate the throttle
    // runs full-canvas rebuilds for cursor jiggle that has no
    // visible effect, and the cross feels laggier than the
    // wheel because cells are smaller (more boundary crossings
    // per visit).
    let mut state_changed = false;
    if let ColorPickerState::Open {
        hue_deg,
        sat,
        val,
        hovered_hit,
        ..
    } = state
    {
        // Track hover changes for hover-grow. Any change in the
        // hit region (e.g. moving from hue slot 3 to slot 4, or
        // from a ring glyph onto the empty backdrop) flips the
        // hovered_hit and triggers a rebuild.
        let new_hover = match hit {
            PickerHit::Hue(_)
            | PickerHit::SatCell(_)
            | PickerHit::ValCell(_)
            | PickerHit::Commit => Some(hit),
            // DragAnchor / Outside are not hoverable targets —
            // they don't grow on hover.
            PickerHit::DragAnchor | PickerHit::Outside => None,
        };
        if *hovered_hit != new_hover {
            *hovered_hit = new_hover;
            state_changed = true;
        }

        match hit {
            PickerHit::Hue(slot) => {
                let new_hue = hue_slot_to_degrees(slot);
                if (*hue_deg - new_hue).abs() > f32::EPSILON {
                    *hue_deg = new_hue;
                    state_changed = true;
                }
            }
            PickerHit::SatCell(i) => {
                let new_sat = sat_cell_to_value(i);
                if (*sat - new_sat).abs() > f32::EPSILON {
                    *sat = new_sat;
                    state_changed = true;
                }
            }
            PickerHit::ValCell(i) => {
                let new_val = val_cell_to_value(i);
                if (*val - new_val).abs() > f32::EPSILON {
                    *val = new_val;
                    state_changed = true;
                }
            }
            PickerHit::Commit | PickerHit::DragAnchor | PickerHit::Outside => {}
        }
    }

    // The hex readout's visibility depends on cursor position
    // crossing the backdrop boundary. We always update
    // `last_cursor_pos` above, so a subsequent `state_changed`
    // event will pick up the right `hex_visible` value. Pure
    // cursor wiggles inside the same cell don't redraw the hex
    // — which is fine: the readout was already showing the
    // current value.
    if state_changed {
        *picker_dirty = true;
        // Preview the updated HSV onto the (possibly contextual)
        // target so the map reflects the hover color live. No-op
        // in Standalone mode — no bound target — but the ࿕ glyph
        // in the wheel still shows the current HSV so the user
        // gets immediate color feedback on the wheel itself.
        apply_picker_preview(state, doc, picker_dirty);
    }
    true
}

/// Click handler for the picker. Semantics:
///
/// - **Hue / SatCell / ValCell / Chip** — preview only. The
///   mouse-move handler already updated HSV on hover, so a click on
///   a glyph is effectively a no-op at the model layer; it's the
///   user affirming the current selection. Clicks here **do not**
///   commit and **do not** close the wheel — users can click around
///   freely and watch the preview update.
/// - **Commit** (࿕) —
///   - Contextual: commit current HSV to the bound target, close.
///   - Standalone: apply current HSV to each item in the document
///     selection; stay open. If the selection is empty, trigger the
///     error-flash animation hook.
/// - **DragAnchor** —
///   - LMB → start a wheel-move gesture (translates `center_override`).
///   - RMB → start a wheel-resize gesture (mutates `size_scale`).
///   The mouse-up event ends either gesture via
///   `end_color_picker_gesture`.
/// - **Outside** —
///   - Contextual: cancel (restore original), close.
///   - Standalone: ignored (the persistent palette only closes via
///     `color picker off`).
///
/// `button` is `MouseButton::Left` or `MouseButton::Right`. The
/// caller (the `WindowEvent::MouseInput` branch) filters out other
/// buttons before reaching here.
///
/// Returns `true` if the click was consumed by the picker and the
/// caller should stop dispatching it. Returns `false` when the
/// click should fall through to normal canvas dispatch — the only
/// such case today is a Standalone-mode outside-backdrop click,
/// where the persistent palette needs to coexist with the user
/// interacting with the canvas underneath it.
#[cfg(not(target_arch = "wasm32"))]
fn handle_color_picker_click(
    cursor_pos: (f64, f64),
    button: MouseButton,
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) -> bool {
    use crate::application::color_picker::{
        hit_test_picker, ColorPickerState, PickerGesture, PickerHit,
    };

    let hit = if let ColorPickerState::Open { layout: Some(layout), .. } = state {
        hit_test_picker(layout, cursor_pos.0 as f32, cursor_pos.1 as f32)
    } else {
        return false;
    };

    // RMB outside the DragAnchor region is a no-op for now — only
    // the empty backdrop area acts as a resize handle. That keeps
    // the gesture predictable: RMB on a hue/sat/val cell or a chip
    // doesn't accidentally resize while the user is also reading
    // the live preview. In Standalone mode we return `false` so
    // the RMB can reach any future right-click menu on the canvas.
    if button == MouseButton::Right && !matches!(hit, PickerHit::DragAnchor) {
        return !state.is_standalone();
    }

    let is_standalone = state.is_standalone();

    match hit {
        PickerHit::Outside => {
            if is_standalone {
                // Standalone mode: the persistent palette only
                // closes via `color picker off`. Don't consume the
                // click — let it flow through to the canvas so the
                // user can still select nodes, create edges, etc.
                return false;
            }
            // Contextual mode: click outside cancels.
            cancel_color_picker(state, doc, mindmap_tree, app_scene, renderer);
        }
        PickerHit::Hue(_) | PickerHit::SatCell(_) | PickerHit::ValCell(_) => {
            // Preview-only: the mouse-move handler already updated
            // HSV as the cursor moved over the glyph, so clicking is
            // a no-op at the model layer. Users can click freely to
            // experiment without the picker closing.
        }
        PickerHit::Commit => {
            if is_standalone {
                // Standalone mode: apply the current HSV to each
                // item in the selection. Stay open.
                commit_color_picker_to_selection(
                    state,
                    doc,
                    mindmap_tree,
                    app_scene,
                    renderer,
                );
            } else {
                // Contextual mode: commit to the bound target,
                // close.
                commit_color_picker(state, doc, mindmap_tree, app_scene, renderer);
            }
        }
        PickerHit::DragAnchor => {
            // Start a gesture from anywhere inside the backdrop
            // that's not on an interactive glyph. LMB → move
            // (translate center_override); RMB → resize (mutate
            // size_scale). The two gestures are mutually exclusive
            // by construction — `gesture` only holds one variant.
            if let ColorPickerState::Open {
                layout: Some(layout),
                gesture,
                size_scale,
                ..
            } = state
            {
                let cursor = (cursor_pos.0 as f32, cursor_pos.1 as f32);
                *gesture = Some(match button {
                    MouseButton::Left => PickerGesture::Move {
                        grab_offset: (
                            layout.center.0 - cursor.0,
                            layout.center.1 - cursor.1,
                        ),
                    },
                    MouseButton::Right => {
                        // Floor the anchor radius so a grab very
                        // near the wheel center doesn't make a 1px
                        // cursor move into a 100% scale change.
                        // `font_size * 3.0` is comfortably outside
                        // the central ࿕ commit button's hit
                        // radius (`preview_size * 0.45`), so the
                        // floor is rarely hit in practice anyway.
                        let dx = cursor.0 - layout.center.0;
                        let dy = cursor.1 - layout.center.1;
                        let raw_r = (dx * dx + dy * dy).sqrt();
                        let anchor_radius = raw_r.max(layout.font_size * 3.0);
                        PickerGesture::Resize {
                            anchor_radius,
                            anchor_scale: *size_scale,
                            anchor_center: layout.center,
                        }
                    }
                    // Other buttons can't reach here — caller
                    // filters to Left/Right before dispatching.
                    _ => return false,
                });
            }
        }
    }
    true
}

/// End an active picker gesture. Called on mouse-up while the
/// picker is open. Returns `true` if a gesture was active and the
/// caller should treat the release as consumed. Returns `false`
/// when no gesture was active (e.g. Standalone-mode press that
/// fell through to the canvas) so the release also falls through.
#[cfg(not(target_arch = "wasm32"))]
fn end_color_picker_gesture(
    state: &mut crate::application::color_picker::ColorPickerState,
) -> bool {
    use crate::application::color_picker::ColorPickerState;
    if let ColorPickerState::Open { gesture, .. } = state {
        let was_active = gesture.is_some();
        *gesture = None;
        was_active
    } else {
        false
    }
}

/// Commit the picker's current HSV to every colorable item in the
/// document's current selection. Standalone mode's core gesture.
///
/// Dispatches through the [`AcceptsWheelColor`] trait: each component
/// type declares its own default color channel (nodes → bg, edges →
/// their single color field). The picker doesn't decide — the
/// component does. Empty selection → fire the error-flash animation
/// hook and do nothing.
///
/// Multi-select applies in a single pass — one undo entry per item
/// (grouped undo is a future refinement when `UndoAction::Group`
/// lands in the document layer).
#[cfg(not(target_arch = "wasm32"))]
fn commit_color_picker_to_selection(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{request_error_flash, ColorPickerState, FlashKind};
    use crate::application::console::traits::{
        selection_targets, view_for, AcceptsWheelColor, ColorValue, Outcome,
    };
    use baumhard::util::color::hsv_to_hex;

    let (hue_deg, sat, val) = match state {
        ColorPickerState::Open {
            hue_deg, sat, val, ..
        } => (*hue_deg, *sat, *val),
        ColorPickerState::Closed => return,
    };
    let color = ColorValue::Hex(hsv_to_hex(hue_deg, sat, val));

    let targets = selection_targets(&doc.selection);
    if targets.is_empty() {
        // The user pressed ࿕ with nothing selected. Fire the
        // animation hook (no-op stub today; picks up when the
        // animation pipeline lands) so the wheel flashes red.
        request_error_flash(state, FlashKind::Error);
        return;
    }

    // Fan out across the selection, letting each component decide
    // which channel the wheel color lands on. A fresh `TargetView`
    // per iteration so no two views alias the doc borrow.
    let mut any_accepted = false;
    for tid in &targets {
        let mut view = view_for(doc, tid);
        match view.apply_wheel_color(color.clone()) {
            Outcome::Applied | Outcome::Unchanged => any_accepted = true,
            Outcome::NotApplicable | Outcome::Invalid(_) => {}
        }
    }

    if any_accepted {
        // Rebuild the whole scene so the newly-colored items repaint
        // next frame. The picker itself stays open — no state change
        // needed on `state`.
        rebuild_all(doc, mindmap_tree, app_scene, renderer);
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
                if cm.timing.as_ref().map_or(false, |t| t.duration_ms > 0) {
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

    // Label-edit key routing (Phase 2.1 surface introduced by the
    // label-edit grapheme-cursor commit). The routing-to-operation
    // layer was previously untested — these pin backspace / delete
    // / arrow / home / end / printable-char behaviour without
    // needing a winit event loop.

    #[test]
    fn test_route_label_edit_backspace_deletes_grapheme_before_cursor() {
        let mut buf = String::from("café");
        // 4 graphemes: c a f é. Cursor at end; backspace removes é.
        let mut cursor = 4;
        let changed = route_label_edit_key(Some("backspace"), None, &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "caf");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_route_label_edit_backspace_at_zero_is_noop() {
        let mut buf = String::from("abc");
        let mut cursor = 0;
        let changed = route_label_edit_key(Some("backspace"), None, &mut buf, &mut cursor);
        assert!(!changed);
        assert_eq!(buf, "abc");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_route_label_edit_delete_at_end_is_noop() {
        let mut buf = String::from("abc");
        let mut cursor = 3;
        let changed = route_label_edit_key(Some("delete"), None, &mut buf, &mut cursor);
        assert!(!changed);
        assert_eq!(buf, "abc");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_route_label_edit_delete_removes_grapheme_at_cursor() {
        let mut buf = String::from("abc");
        let mut cursor = 1;
        let changed = route_label_edit_key(Some("delete"), None, &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "ac");
        assert_eq!(cursor, 1);
    }

    #[test]
    fn test_route_label_edit_arrow_left_right_walks_graphemes() {
        let mut buf = String::from("café");
        let mut cursor = 4;
        // Left past é, f, a — landing on the c boundary.
        assert!(route_label_edit_key(Some("arrowleft"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 3);
        assert!(route_label_edit_key(Some("arrowleft"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 2);
        // Right brings us back.
        assert!(route_label_edit_key(Some("arrowright"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_route_label_edit_arrow_left_at_zero_is_noop() {
        let mut buf = String::from("abc");
        let mut cursor = 0;
        assert!(!route_label_edit_key(Some("arrowleft"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_route_label_edit_home_end_jump_to_ends() {
        let mut buf = String::from("café");
        let mut cursor = 2;
        assert!(route_label_edit_key(Some("home"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 0);
        // Home again is a no-op.
        assert!(!route_label_edit_key(Some("home"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 0);
        assert!(route_label_edit_key(Some("end"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 4);
        // End again is a no-op.
        assert!(!route_label_edit_key(Some("end"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 4);
    }

    #[test]
    fn test_route_label_edit_printable_inserts_and_advances() {
        let mut buf = String::from("ab");
        let mut cursor = 1;
        let changed = route_label_edit_key(None, Some("X"), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "aXb");
        assert_eq!(cursor, 2);
    }

    /// IME / dead-key sequences can arrive as multi-char strings.
    /// Each non-control char inserts in order and the cursor
    /// advances past them.
    #[test]
    fn test_route_label_edit_multichar_typed_payload() {
        let mut buf = String::from("");
        let mut cursor = 0;
        let changed = route_label_edit_key(None, Some("né"), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "né");
        assert_eq!(cursor, 2);
    }

    /// Control characters in a typed payload are filtered out.
    /// Pins the regression where an IME sequence like `"a\t"`
    /// would otherwise insert a literal tab.
    #[test]
    fn test_route_label_edit_typed_control_chars_are_skipped() {
        let mut buf = String::from("");
        let mut cursor = 0;
        let changed = route_label_edit_key(None, Some("a\tb"), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "ab");
        assert_eq!(cursor, 2);
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

    // -----------------------------------------------------------------
    // Console completion acceptance
    // -----------------------------------------------------------------

    use crate::application::console::completion::Completion;

    fn open_state(input: &str, cursor: usize, candidates: &[&str]) -> ConsoleState {
        ConsoleState::Open {
            input: input.to_string(),
            cursor,
            history: Vec::new(),
            history_idx: None,
            scrollback: Vec::new(),
            completions: candidates
                .iter()
                .map(|c| Completion {
                    text: c.to_string(),
                    display: c.to_string(),
                    hint: None,
                })
                .collect(),
            completion_idx: if candidates.is_empty() { None } else { Some(0) },
        }
    }

    /// Accepting a command-name completion replaces the partial
    /// prefix and appends a trailing space so the user can type
    /// the next token immediately.
    #[test]
    fn test_accept_completion_positional_appends_space() {
        let mut state = open_state("co", 2, &["color"]);
        accept_console_completion(&mut state);
        if let ConsoleState::Open { input, cursor, .. } = state {
            assert_eq!(input, "color ");
            assert_eq!(cursor, 6);
        } else {
            panic!("state closed");
        }
    }

    /// Accepting a kv-key completion (text ends in `=`) adds no
    /// trailing space — the value comes next.
    #[test]
    fn test_accept_completion_kv_key_no_trailing_space() {
        let mut state = open_state("color b", 7, &["bg="]);
        accept_console_completion(&mut state);
        if let ConsoleState::Open { input, cursor, .. } = state {
            assert_eq!(input, "color bg=");
            assert_eq!(cursor, 9);
        } else {
            panic!("state closed");
        }
    }

    /// Accepting a kv-value completion replaces only the value slot
    /// (not the key=) and adds no trailing space.
    #[test]
    fn test_accept_completion_kv_value_replaces_only_value_slot() {
        let mut state = open_state("color bg=ac", 11, &["accent"]);
        accept_console_completion(&mut state);
        if let ConsoleState::Open { input, cursor, .. } = state {
            assert_eq!(input, "color bg=accent");
            assert_eq!(cursor, 15);
        } else {
            panic!("state closed");
        }
    }

    /// Accepting a kv-value with no partial typed (cursor right
    /// after `=`) inserts at the value slot and keeps the cursor
    /// after the value — no trailing space.
    #[test]
    fn test_accept_completion_kv_value_empty_partial() {
        let mut state = open_state("color bg=", 9, &["accent"]);
        accept_console_completion(&mut state);
        if let ConsoleState::Open { input, cursor, .. } = state {
            assert_eq!(input, "color bg=accent");
            assert_eq!(cursor, 15);
        } else {
            panic!("state closed");
        }
    }

    /// Accepting when the popup is empty is a no-op.
    #[test]
    fn test_accept_completion_no_popup_is_noop() {
        let mut state = open_state("color bg=", 9, &[]);
        accept_console_completion(&mut state);
        if let ConsoleState::Open { input, cursor, .. } = state {
            assert_eq!(input, "color bg=");
            assert_eq!(cursor, 9);
        } else {
            panic!("state closed");
        }
    }
}
