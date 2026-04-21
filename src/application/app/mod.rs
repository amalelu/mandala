use std::collections::HashMap;
use std::sync::Arc;

// Cross-platform submodules: compile for both native and WASM.
mod scene_rebuild;
mod text_edit;

// Native-only submodules: the interactive modal state machines
// (click routing, console, color picker, label edit, edge drag) live
// here and are entirely absent from the WASM build. Per
// `CODE_CONVENTIONS.md §2`, cross-platform `cfg` discipline puts the
// split at the module boundary rather than per-item; the one-line
// status of what's native-only vs. cross-platform lives in
// `CLAUDE.md`'s "Dual-target status" section.
#[cfg(not(target_arch = "wasm32"))]
mod click;
#[cfg(not(target_arch = "wasm32"))]
mod color_picker_flow;
#[cfg(not(target_arch = "wasm32"))]
mod console_input;
#[cfg(not(target_arch = "wasm32"))]
mod drain_frame;
#[cfg(not(target_arch = "wasm32"))]
mod edge_drag;
#[cfg(not(target_arch = "wasm32"))]
mod edge_label_drag;
#[cfg(not(target_arch = "wasm32"))]
mod event_cursor_moved;
#[cfg(not(target_arch = "wasm32"))]
mod event_keyboard;
#[cfg(not(target_arch = "wasm32"))]
mod event_mouse_click;
#[cfg(not(target_arch = "wasm32"))]
mod freeze_watchdog;
#[cfg(not(target_arch = "wasm32"))]
mod portal_label_drag;
#[cfg(not(target_arch = "wasm32"))]
mod label_edit;
#[cfg(not(target_arch = "wasm32"))]
mod run_native;
#[cfg(not(target_arch = "wasm32"))]
mod run_native_init;
#[cfg(target_arch = "wasm32")]
mod run_wasm;

// Cross-platform imports.
use scene_rebuild::{
    flush_canvas_scene_buffers, rebuild_all, rebuild_scene_only, update_border_tree_static,
    update_border_tree_with_offsets, update_connection_label_tree, update_connection_tree,
    update_edge_handle_tree, update_portal_tree,
};
use text_edit::{
    close_text_edit, delete_at_cursor, delete_before_cursor, handle_text_edit_key,
    insert_at_cursor, open_text_edit, TextEditState,
};

// Native-only imports: every name below is only referenced from
// `run_native` or native-only helpers in this file.
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use pollster::block_on;
#[cfg(not(target_arch = "wasm32"))]
use click::{
    handle_click, handle_connect_target_click, handle_reparent_target_click,
    rebuild_all_with_mode,
};
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
#[cfg(not(target_arch = "wasm32"))]
use edge_drag::apply_edge_handle_drag;
#[cfg(not(target_arch = "wasm32"))]
use portal_label_drag::apply_portal_label_drag;
#[cfg(not(target_arch = "wasm32"))]
use label_edit::{
    handle_label_edit_key, handle_portal_text_edit_key, open_label_edit,
    open_portal_text_edit, LabelEditState, PortalTextEditState,
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
    apply_drag_delta, apply_drag_delta_and_collect_patches,
    apply_tree_highlights,
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
/// testing. Slightly larger than the edge-path tolerance above
/// because handles are point-like and need a bit more grab-area
/// to feel forgiving.
#[cfg(not(target_arch = "wasm32"))]
const EDGE_HANDLE_HIT_TOLERANCE_PX: f32 = 12.0;


/// What a single click targeted. Used by [`LastClick`] + the
/// double-click detector so a portal-marker double-click (navigate)
/// is distinguishable from a node double-click (edit text) and from
/// empty-space double-click (create orphan). Two clicks "match" as
/// a double-click only when they have the same `ClickHit`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ClickHit {
    /// No node and no portal marker under the cursor. Empty-canvas
    /// double-click creates a new orphan unless an edge is selected.
    Empty,
    /// Cursor is inside node `id`'s AABB.
    Node(String),
    /// Cursor is inside a portal **icon** marker. `edge` identifies
    /// the owning portal-mode edge; `endpoint` is the node the
    /// hit marker sits above (the double-click pan target is the
    /// *other* endpoint).
    PortalMarker {
        edge: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint: String,
    },
    /// Cursor is inside a portal **text** label — the glyph area
    /// sitting alongside a portal icon. Routes to
    /// `SelectionState::PortalText`, distinct from the icon so
    /// per-channel operations (color / font) target only the
    /// clicked sub-part. Double-click inherits the same
    /// pan-to-partner behaviour as `PortalMarker` — the
    /// endpoint identity is shared between icon and text.
    PortalText {
        edge: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint: String,
    },
    /// Cursor is inside a line-mode edge's **label** AABB.
    /// Routes to `SelectionState::EdgeLabel` on single click so
    /// color / font / copy operations target the label instead
    /// of the edge body; double-click opens the inline label
    /// editor, matching the "click to select, dbl to edit"
    /// idiom the `Node` variant already follows.
    EdgeLabel(baumhard::mindmap::scene_cache::EdgeKey),
}

/// Records the previous left-click's time, screen position, and hit
/// target so a second click within a short time + distance window
/// is recognized as a double-click. Double-click fires on the second
/// `Pressed` event, not the second release. `time` is `f64`
/// milliseconds from the cross-platform `now_ms()` helper.
#[derive(Debug, Clone)]
struct LastClick {
    time: f64,
    screen_pos: (f64, f64),
    /// What the first click landed on. Two clicks whose `hit`
    /// values are equal (see [`ClickHit::PartialEq`]) qualify as a
    /// double-click.
    hit: ClickHit,
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
    new_hit: &ClickHit,
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
        /// If the cursor landed on a portal marker at mouse-down,
        /// this records `(edge_key, endpoint_node_id)` so a drag
        /// past threshold transitions to `DraggingPortalLabel`.
        /// Takes precedence over `hit_node` — the marker sits
        /// above a node, but clicking the marker is "grab this
        /// label," not "move this node." Independent of
        /// `hit_edge_handle` because portal-mode edges don't
        /// expose edge-handles in the first place.
        hit_portal_label: Option<(
            baumhard::mindmap::scene_cache::EdgeKey,
            String,
        )>,
        /// If the cursor landed on an edge-label AABB at
        /// mouse-down, this records the owning edge key so a
        /// drag past threshold transitions to
        /// `DraggingEdgeLabel`. Takes precedence over
        /// `hit_node` — a label hovering over a node behind
        /// it should move as a label, not a node.
        hit_edge_label: Option<baumhard::mindmap::scene_cache::EdgeKey>,
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
    /// Handles come in four kinds (see
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
    /// Dragging a portal label along its owning node's border.
    /// The cursor drags in free canvas space and each drain frame
    /// snaps that position to the nearest border point, mutating
    /// the edge's `portal_from` / `portal_to.border_t` in place.
    /// On release a single `UndoAction::EditEdge` is pushed
    /// carrying the pre-drag edge snapshot, mirroring the
    /// `DraggingEdgeHandle` commit path.
    DraggingPortalLabel {
        edge_ref: EdgeRef,
        endpoint_node_id: String,
        /// Full pre-drag `MindEdge` snapshot, used both for
        /// `UndoAction::EditEdge` at release and to skip undo
        /// entries when the drag didn't actually move `border_t`.
        original: baumhard::mindmap::model::MindEdge,
    },
    /// Dragging a line-mode edge's text label along its
    /// connection path. The cursor drags in free canvas space
    /// and each drain frame projects that position onto the
    /// edge's path via
    /// [`baumhard::mindmap::connection::closest_point_on_path`],
    /// writing the resulting
    /// `(position_t, perpendicular_offset)` into the edge's
    /// `label_config`. On release a single
    /// `UndoAction::EditEdge` is pushed carrying the pre-drag
    /// snapshot.
    DraggingEdgeLabel {
        edge_ref: EdgeRef,
        /// Full pre-drag `MindEdge` snapshot — used both for
        /// the undo entry at release and to skip pushing an
        /// entry when the drag didn't actually move the label.
        original: baumhard::mindmap::model::MindEdge,
    },
}

/**
Represents the root container of the application
Manages the winit window and event_loop, and launches the rendering pipeline
 **/
#[cfg(target_arch = "wasm32")]
pub struct Application {
    options: Options,
    event_loop: EventLoop<()>,
    window: Arc<Window>,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Application {
    options: Options,
}

impl Application {
    #[cfg(target_arch = "wasm32")]
    pub fn new(options: Options) -> Self {
        let event_loop = EventLoop::new().expect("Could not create an EventLoop");

        // Pre-creating the window here on winit 0.30 is deprecated in
        // favour of `ActiveEventLoop::create_window` inside
        // `ApplicationHandler::resumed`. The native path takes that
        // route; the WASM path still pre-creates because
        // `run_wasm::run` attaches the canvas and installs DOM event
        // listeners before the event loop starts.
        #[allow(deprecated)]
        let window = event_loop
            .create_window(Window::default_attributes())
            .expect("Failed to create application window");

        Application {
            options,
            event_loop,
            window: Arc::new(window),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(options: Options) -> Self {
        Application { options }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(self) {
        run_native::run(self)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn run(self) {
        run_wasm::run(self)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn into_options(self) -> Options {
        self.options
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

// Unit tests for pure helpers (cursor math, caret insertion,
// double-click detection, Baumhard mutation round-trip). Event-loop
// integration is verified manually via `cargo run`.

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;
