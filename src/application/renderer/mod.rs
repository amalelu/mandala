// SPDX-License-Identifier: MPL-2.0

//! GPU-side presentation: every wgpu device, every cosmic-text
//! rasterization, every text/rect/border buffer Mandala paints
//! lives under [`Renderer`]. `Renderer` reads two intermediate
//! representations the document layer hands it
//! (`Tree<GfxElement, GfxMutator>` for canvas content,
//! `Scene` for connection / portal / label overlays); it never
//! reaches into the document directly, and the document never
//! holds GPU resources (CODE_CONVENTIONS §3 "Model / view
//! separation").
//!
//! The submodule split corresponds to wgpu pipeline boundaries:
//!
//! - [`pipeline`] — the small `RenderPipeline` factory shared
//!   across every text + rect pass.
//! - [`render`] — the per-frame `RenderPass` driver
//!   (`Renderer::process`); composes the buffer layers in
//!   draw order.
//! - [`tree_buffers`] / [`tree_walker`] — `GfxElement` tree
//!   → text-buffer + rect-buffer projection. The tree walker
//!   is where the bulk of canvas-content shaping happens.
//! - [`scene_buffers`] — connection paths, edge handles,
//!   portal markers, edge labels. Scene-graph projection.
//! - [`borders`] — node-frame buffers (the box-drawing
//!   glyph runs around each node).
//! - [`console_pass`] / [`console_geometry`] — the console
//!   overlay's glyph-tree pass + pure-function layout math.
//! - [`color_picker`] — the glyph-wheel picker overlay.
//! - [`hit`] — screen-space → canvas-space hit math
//!   (`screen_to_canvas`, `canvas_to_screen`, AABB resolution).
//! - [`decree`] — the `RenderDecree` queue the event loop
//!   feeds the renderer (resize, zoom, camera-pan, etc.).
//! - [`overlay_dispatch`] — overlay-vs-canvas slot routing
//!   for the [`crate::application::scene_host::AppScene`]
//!   tree handles.

mod borders;
mod color_picker;
mod console_geometry;
mod console_pass;
mod decree;
mod hit;
mod overlay_dispatch;
mod pipeline;
mod render;
mod scene_buffers;
mod tree_buffers;
mod tree_walker;

pub use borders::measure_max_glyph_advance;
// `ConsoleFrameLayout` / `MAX_*` / `build_console_border_strings` are
// part of the renderer's public surface and consumed by the test
// block at the bottom of this file plus external callers (the app
// crate threads `ConsoleFrameLayout` through the rebuild path).
// cargo check (without `--tests`) doesn't see those usages.
#[allow(unused_imports)]
pub use console_geometry::{
    build_console_border_strings, compute_console_frame_layout, ConsoleFrameLayout, ConsoleOverlayCompletion,
    ConsoleOverlayGeometry, ConsoleOverlayLine, ConsoleOverlayLineKind, MAX_CONSOLE_COMPLETION_ROWS,
    MAX_CONSOLE_SCROLLBACK_ROWS,
};
#[cfg(test)]
use console_pass::{
    build_console_overlay_mutator, build_console_overlay_tree, console_overlay_areas,
    console_overlay_signature,
};

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;
use web_time::Instant;

use baumhard::font::{Attrs, Buffer};
use glyphon::{Cache, Resolution, SwashCache, TextAtlas, TextRenderer, Viewport};
use log::{error, info, warn};

use rustc_hash::FxHashMap;

use wgpu::{
    Color, Device, Instance, MultisampleState, Queue, RenderPipeline, Surface, SurfaceConfiguration,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::application::common::{FpsDisplayMode, PollTimer, RedrawMode, RenderDecree, StopWatch};
use baumhard::font::fonts;
#[cfg(test)]
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::mindmap::scene_cache::EdgeKey;
use glam::Vec2;

/// Inline WGSL shader for the colored-rectangle pipeline. Draws a
/// stream of NDC-space vertices, each carrying its own RGBA color,
/// a local-space `uv` in `[0, 1]`, and a `shape_id` that selects
/// how the fragment shader treats the fill. Kept inline (rather
/// than in the baumhard shader table) because it's 100%
/// renderer-local — no tree data, no camera uniforms; the CPU
/// bakes the camera transform into each vertex before upload.
///
/// Extending with a new shape: add a `SHAPE_*` constant and a
/// `case` arm in `fs_main`. The shape id comes from
/// `NodeShape::shader_id` on the baumhard side; the two must stay
/// in lock-step.
///
/// `shape_id` rides the vertex stream as a plain `f32` (written
/// with `SHAPE_ID_* as f32`, read with `u32(round(id))`) rather
/// than a `Uint32` vertex attribute, because integer vertex
/// attributes are a wgpu WebGL2 feature gate on some browsers and
/// the per-shape branch only needs a handful of discrete values.
/// The round-trip through `f32` is lossless for the small integer
/// range we use; see `NodeShape::shader_id` for the allocation.
const RECT_SHADER_WGSL: &str = r#"
const SHAPE_RECT: u32 = 0u;
const SHAPE_ELLIPSE: u32 = 1u;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) shape_id: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) shape_id: u32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(in.pos, 0.0, 1.0);
    out.color = in.color;
    out.uv = in.uv;
    // `round` then cast — the CPU writes exact integers, so the
    // round is belt-and-braces against any driver-side rasterisation
    // of the attribute. Flat-interpolated onto VsOut as `u32` so
    // the fragment `switch` is a plain integer compare.
    out.shape_id = u32(round(in.shape_id));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    switch (in.shape_id) {
        case SHAPE_ELLIPSE: {
            // Local-space ellipse SDF: bounds map to uv in [0, 1]
            // so the inscribed unit circle lives at |uv - 0.5| <= 0.5.
            // Remap to [-1, 1] so the test is `dot(p, p) <= 1`.
            let p = (in.uv - vec2<f32>(0.5, 0.5)) * 2.0;
            let d = dot(p, p);
            if (d > 1.0) {
                discard;
            }
            return in.color;
        }
        default: {
            // SHAPE_RECT (and the safe fallback for unknown ids):
            // the whole quad is the fill.
            return in.color;
        }
    }
}
"#;

/// Bytes-per-vertex for the rect pipeline: `vec2<f32> pos +
/// vec2<f32> uv + vec4<f32> color + u32 shape_id = 9 × 4 = 36 bytes`.
/// Used when sizing / offsetting the vertex buffer. Declared as a
/// compile-time const so the layout math is grep-able from a single
/// place. Keep in sync with the inline `wgpu::VertexAttribute`
/// table in `Renderer::new` and the per-vertex push in
/// `push_rect_ndc`.
const RECT_VERTEX_SIZE: u64 = 36;

/// How many frames `FpsDisplayMode::Snapshot` waits between readout
/// refreshes, and how many frames `FpsDisplayMode::Debug` averages
/// over. 200 at 60 fps ≈ 3.3 s — short enough to react to sustained
/// perf changes, long enough to smooth out per-frame jitter.
const FPS_WINDOW: usize = 200;

/// Mode-status overlay layout: font size, screen-space position
/// (top-left corner, in pixels), and shaping bounds (max width ×
/// height in pixels). The position sits below the FPS overlay's
/// row (`(8.0, 8.0)` + ~24 px line height). Width is generous so
/// long node ids + section counts fit on one line; height matches
/// the FPS overlay so vertical alignment is predictable when both
/// are visible. Promoted to constants so future overlays can
/// stack predictably below the mode-status row.
const MODE_STATUS_FONT_SIZE_PT: f32 = 14.0;
const MODE_STATUS_OVERLAY_POS: (f32, f32) = (8.0, 32.0);
const MODE_STATUS_OVERLAY_BOUNDS: (f32, f32) = (640.0, 24.0);

/// Fixed-size ring buffer of frame intervals (microseconds) with an
/// O(1) running sum. Backs `FpsDisplayMode::Debug`'s rolling-average
/// readout. Encapsulates the sum invariant — `sum` is always
/// consistent with `samples[..filled.min(FPS_WINDOW)]` — so the
/// four-field state can never drift out of sync via direct access.
/// Private to this module.
pub(super) struct FrameIntervalRing {
    samples: [u128; FPS_WINDOW],
    idx: usize,
    sum: u128,
    filled: usize,
}

impl FrameIntervalRing {
    pub(super) fn new() -> Self {
        Self {
            samples: [0u128; FPS_WINDOW],
            idx: 0,
            sum: 0,
            filled: 0,
        }
    }

    pub(super) fn clear(&mut self) {
        self.samples = [0u128; FPS_WINDOW];
        self.idx = 0;
        self.sum = 0;
        self.filled = 0;
    }

    pub(super) fn push(&mut self, micros: u128) {
        let old = self.samples[self.idx];
        self.sum = self.sum - old + micros;
        self.samples[self.idx] = micros;
        self.idx = (self.idx + 1) % FPS_WINDOW;
        if self.filled < FPS_WINDOW {
            self.filled += 1;
        }
    }

    pub(super) fn avg_micros(&self) -> Option<u128> {
        if self.filled == 0 {
            None
        } else {
            Some(self.sum / self.filled as u128)
        }
    }
}

/// Number of `f32`-sized slots per vertex. The CPU accumulates
/// packed floats into `main_rect_vertices` / `console_rect_vertices`;
/// `shape_id` is stored as an `f32` holding the `u32` bit pattern
/// via `f32::from_bits` so the whole stream stays a single `Vec<f32>`.
pub(super) const RECT_VERTEX_FLOATS: usize = 9;

/// Starting capacity (in bytes) for the rect vertex buffer. Big
/// enough for a modest map with several hundred node backgrounds
/// without an immediate grow; doubling-on-overflow handles anything
/// larger. 8192 bytes ÷ 36 bytes/vertex ≈ 227 vertices ≈ 37 rects.
/// Deliberately small since most maps will have a handful of colored
/// nodes and the grow path is exercised rarely.
pub(super) const RECT_VBUF_INITIAL_CAPACITY: u64 = 8192;

pub struct Renderer {
    surface: Surface<'static>,
    window: Arc<Window>,
    config: SurfaceConfiguration,
    device: Device,
    queue: Queue,
    viewport: Viewport,
    swash_cache: SwashCache,
    glyphon_cache: Cache,
    atlas: TextAtlas,
    timer: PollTimer,
    target_duration_between_renders: Duration,
    last_render_time: Duration,
    text_renderer: TextRenderer,
    /// Second glyphon TextRenderer dedicated to the command
    /// palette overlay. Shares `self.atlas` with `text_renderer`
    /// so glyph caching is unified, but keeps its own internal
    /// vertex/index buffers — which is what lets us issue a rect
    /// draw BETWEEN the two text renders inside one render pass
    /// (otherwise re-preparing the single text renderer would
    /// race with the pass's already-recorded draw commands).
    console_text_renderer: TextRenderer,
    redraw_mode: RedrawMode,
    run: bool,
    should_render: bool,
    fps: Option<usize>,
    /// Which FPS readout to display, if any. `Snapshot` samples one
    /// frame's interval every `FPS_WINDOW` frames; `Debug` averages the
    /// last `FPS_WINDOW` frame intervals and updates every frame.
    /// Toggled via `fps on` / `fps debug` / `fps off`.
    fps_display_mode: FpsDisplayMode,
    /// Screen-space text buffer(s) carrying the yellow FPS readout.
    /// Chained into `palette_text_areas` at render time so the readout
    /// draws at `scale: 1.0` with no camera transform. Empty whenever
    /// `fps_display_mode` is `Off`.
    fps_overlay_buffers: Vec<MindMapTextBuffer>,
    /// The `self.fps` value that was shaped into `fps_overlay_buffers`
    /// last. Used to skip re-shaping when the integer value hasn't
    /// changed since the last rebuild.
    last_fps_shaped: Option<usize>,
    /// Pending mode-status overlay text, set by the app's
    /// scene-rebuild path on every mode-affecting action and consumed
    /// by [`Self::rebuild_mode_status_overlay_if_needed`] at the next
    /// frame. `None` clears the overlay (Default mode); `Some(text)`
    /// shows it. Computing the string in `scene_rebuild.rs` (rather
    /// than the renderer) keeps the renderer model-agnostic and lets
    /// the source of truth — `(mode, selection, doc)` — stay on the
    /// app side.
    mode_status_text: Option<String>,
    /// Screen-space text buffer(s) carrying the mode-status line
    /// (e.g. `editing: <node-id> — section [N of M]`). Sibling of
    /// `fps_overlay_buffers`; same render path. Empty when
    /// `mode_status_text` is `None`.
    mode_status_overlay_buffers: Vec<MindMapTextBuffer>,
    /// The `self.mode_status_text` value that was shaped into
    /// `mode_status_overlay_buffers` last. Used to skip re-shaping
    /// when the text hasn't changed.
    last_mode_status_shaped: Option<String>,
    /// Wall-clock timestamp of the previous rendered frame. The
    /// difference between consecutive values is the actual frame
    /// interval, which is what FPS is derived from. Measuring
    /// wall-clock here rather than `last_render_time` is load-bearing:
    /// `render()` can early-return on font-system lock contention
    /// under heavy interaction, which would otherwise make
    /// `last_render_time` shrink to near-zero and inflate FPS to a
    /// false huge value.
    last_frame_instant: Option<Instant>,
    /// Frame counter used by `FpsDisplayMode::Snapshot` to refresh the
    /// displayed value only every `FPS_WINDOW` frames. Increments
    /// every frame regardless of mode; meaningful only in Snapshot.
    fps_clock: usize,
    /// Rolling window of the last `FPS_WINDOW` frame intervals,
    /// consumed by `FpsDisplayMode::Debug` to compute a rolling
    /// average. The sum / divisor invariant is enforced by the
    /// `FrameIntervalRing` wrapper — no direct field access here.
    fps_ring: FrameIntervalRing,
    /// Set by [`Self::set_fps_idle`] when the event loop transitions
    /// from active rendering to `ControlFlow::Wait`. Consumed by
    /// the next [`Self::tick_fps`] call, which short-circuits to
    /// `fps = None` so the transitional render paints "FPS: -"
    /// regardless of what the rolling average or snapshot
    /// alignment would otherwise compute.
    fps_pending_idle_paint: bool,

    camera: Camera2D,
    /// Mindmap text buffers keyed by `GfxElement::unique_id`
    /// (stringified for use as a `FxHashMap` key alongside the
    /// edit / undo paths' Dewey-decimal addressing). The value is
    /// a `Vec<MindMapTextBuffer>` because the tree walker emits
    /// **multiple buffers per element** when an outline halo is
    /// configured: one buffer per halo offset emitted before the
    /// main glyph, all sharing the same `unique_id`. Pre-vec the
    /// store collapsed every halo onto the main glyph (last-write-
    /// wins via `insert`); the vec preserves emission order so
    /// halos stay behind the main glyph at render time.
    mindmap_buffers: FxHashMap<String, Vec<MindMapTextBuffer>>,
    /// Per-node border glyph buffers, keyed by `node_id`. Each entry is
    /// a `Vec` of 4 buffers (top/bottom/left/right) emitted by
    /// `rebuild_border_buffers`.
    border_buffers: FxHashMap<String, Vec<MindMapTextBuffer>>,
    /// Edge grab-handle buffers for the connection reshape surface.
    /// Populated only when an edge is selected; rebuilt fresh every
    /// time the scene is rebuilt with a selected edge. Bounded cost
    /// (≤ 5 glyph buffers per selected edge) so no keyed cache is
    /// warranted.
    edge_handle_buffers: Vec<MindMapTextBuffer>,
    /// Per-edge label buffers, keyed by `EdgeKey`. Each entry is the
    /// shaped cosmic-text buffer for that edge's label (if any).
    /// Labels are ≤ 1 per edge and rebuilt every scene build — no
    /// incremental-reuse cache is warranted.
    connection_label_buffers: FxHashMap<EdgeKey, MindMapTextBuffer>,
    /// AABB hitbox for each rendered label, keyed by `EdgeKey`.
    /// Populated alongside `connection_label_buffers`; consulted by
    /// `hit_test_edge_label` when the app dispatches inline
    /// click-to-edit. Stored as `(min, max)` canvas-space corners so
    /// the hit test is a pair of comparisons per edge.
    connection_label_hitboxes: FxHashMap<EdgeKey, (Vec2, Vec2)>,
    /// AABB hitbox for each rendered portal marker, keyed by
    /// `(edge_key, endpoint_node_id)`. Portal glyph buffers
    /// themselves flow through `canvas_scene_buffers` via the
    /// tree pipeline (see `tree_builder::portal`); this map
    /// carries only the hit-test rectangles the event loop
    /// needs. Consulted by `hit_test_portal` when
    /// `handle_click` resolves a click on a portal glyph to an
    /// `EdgeKey` + the endpoint the marker sits above (the
    /// double-click jump target is the *other* endpoint).
    /// Split between the icon's AABB and the text's AABB so the
    /// event loop can route clicks on text to
    /// `SelectionState::PortalText` and clicks on the icon to
    /// `SelectionState::PortalLabel`. Text entries are absent
    /// when the endpoint has no visible text (see
    /// `tree_builder::portal` for the load-bearing phantom-hot-
    /// zone invariant).
    portal_icon_hitboxes: FxHashMap<(EdgeKey, String), (Vec2, Vec2)>,
    portal_text_hitboxes: FxHashMap<(EdgeKey, String), (Vec2, Vec2)>,
    /// Command palette / console overlay buffers. Rendered above
    /// everything else in screen coordinates. Populated only when
    /// the console is open; cleared otherwise.
    console_overlay_buffers: Vec<MindMapTextBuffer>,
    /// Screen-space geometry of the color picker's opaque backdrop.
    /// Captured inside `rebuild_color_picker_overlay_buffers`; the
    /// `render()` rect-pipeline pass appends a black fill rect for
    /// this region alongside the palette backdrop. `None` whenever
    /// the picker is closed.
    color_picker_backdrop: Option<(f32, f32, f32, f32)>,
    /// Temporary overlay buffers (e.g., selection rectangle). Camera-transformed.
    overlay_buffers: Vec<MindMapTextBuffer>,
    /// `(char_count, row_count)` of the most recent selection-rect
    /// shape held in [`Self::overlay_buffers`]. Per-tick rebuilds
    /// reuse the existing shaped buffers (just update positions)
    /// when these counts match, avoiding 4 fresh `cosmic_text`
    /// shapings per drag tick. `None` whenever the overlay is
    /// cleared or holds a non-selection-rect shape.
    selection_rect_shape_cache: Option<(usize, usize)>,
    /// Screen-space buffers produced by walking the app's
    /// [`AppScene`](crate::application::scene_host::AppScene).
    /// Populated by [`Self::rebuild_overlay_scene_buffers`] and
    /// drawn alongside the existing console/color-picker overlay
    /// buffer lists. Empty until an overlay migrates to a tree.
    overlay_scene_buffers: Vec<MindMapTextBuffer>,
    /// Canvas-space buffers for the app's
    /// [`AppScene`](crate::application::scene_host::AppScene)'s
    /// canvas sub-scene (borders, connections, portals, etc.).
    /// Populated by [`Self::rebuild_canvas_scene_buffers`]. Drawn
    /// in the main camera-transformed pass. Empty until a canvas
    /// component migrates to a tree.
    canvas_scene_buffers: Vec<MindMapTextBuffer>,
    /// Background-rect instances collected while walking the
    /// canvas sub-scene — forwarded to the camera-transformed
    /// rect pipeline so GlyphArea fills on migrated components
    /// render beneath their glyphs.
    canvas_scene_background_rects: Vec<NodeBackgroundRect>,
    /// Set whenever the camera *zoom* changes. The document-side
    /// `SceneConnectionCache` stores pre-clip samples whose spacing
    /// depends on `GlyphConnectionConfig::effective_font_size_pt`, which
    /// is a function of zoom — so on zoom the cache must be flushed
    /// before the next scene build re-samples. `SceneConnectionCache`
    /// enforces this internally via `ensure_zoom`, but we still raise
    /// this flag so the event loop can explicitly clear the cache and
    /// re-run the connection rebuild.
    connection_geometry_dirty: bool,
    /// Filled-rectangle rendering pipeline. Used to draw node
    /// backgrounds (from `GlyphArea.background_color`), the command
    /// palette backdrop, and any other solid-color fill that needs
    /// to sit in the render pipeline alongside text. See the
    /// `RECT_SHADER_WGSL` const above for the shader, and
    /// `push_canvas_rect` / `push_screen_rect` for the CPU-side
    /// vertex layout.
    rect_pipeline: RenderPipeline,
    /// Persistent vertex buffer for the rect pipeline. Grows
    /// (doubling) on overflow, never shrinks. Re-uploaded each
    /// frame with the concatenation of `main_rect_vertices` and
    /// `console_rect_vertices`; the two batches draw separately
    /// using offset + count so a single buffer keeps the code
    /// simple.
    rect_vertex_buffer: wgpu::Buffer,
    /// Current allocated capacity of `rect_vertex_buffer`, in
    /// bytes.
    rect_vertex_buffer_capacity: u64,
    /// Canvas-space node background rects (pos, size, rgba u8)
    /// collected from `GlyphArea.background_color` during
    /// `rebuild_buffers_from_tree`. Camera-transformed to NDC in
    /// `render` each frame so a camera pan/zoom is a pure CPU
    /// rebuild — no tree rewalk required.
    node_background_rects: Vec<NodeBackgroundRect>,
    /// Packed vertex floats for the "main" (node background) rect
    /// batch, rebuilt every frame from `node_background_rects` +
    /// current camera. 6 floats per vertex, 6 vertices per rect.
    main_rect_vertices: Vec<f32>,
    /// Packed vertex floats for the "overlay" (palette backdrop)
    /// rect batch, rebuilt whenever the palette opens/closes or
    /// the viewport resizes. Stays empty when the palette is shut.
    console_rect_vertices: Vec<f32>,
    /// Screen-space geometry of the palette's opaque backdrop.
    /// Captured inside `rebuild_console_overlay_buffers` so
    /// `render()` can turn it into NDC vertices against the
    /// current viewport size without re-running the layout.
    /// `None` whenever the palette is closed.
    console_backdrop: Option<(f32, f32, f32, f32)>, // (left, top, width, height)
    /// Clear color for the render pass, driven by the map's
    /// `Canvas.background_color`. Starts as opaque black so the
    /// app looks sensible before a map loads; the event loop
    /// calls `set_clear_color` right after load.
    clear_color: Color,
}

/// Canvas-space record of a background fill drawn behind a node's
/// text. The CPU always uploads an axis-aligned quad covering
/// `(position, size)`; the fragment shader then discards pixels
/// outside the shape described by `shape_id` (rectangle keeps the
/// whole quad, ellipse clips to the inscribed conic, future shapes
/// add one more case). Captured from `GlyphArea.background_color`
/// during the tree walk in `rebuild_buffers_from_tree`;
/// camera-transformed to NDC in `render` each frame.
#[derive(Clone, Debug)]
pub(super) struct NodeBackgroundRect {
    pub position: Vec2,
    pub size: Vec2,
    pub color: [u8; 4],
    /// Stable shape id from [`baumhard::gfx_structs::shape::NodeShape::shader_id`].
    /// Flat-interpolated to the fragment shader's `switch`.
    pub shape_id: u32,
    /// Per-`GlyphArea` zoom window. The main render loop skips this
    /// rect whenever `camera.zoom` falls outside the window. Default
    /// (both bounds `None`) renders at every zoom — existing nodes
    /// pay nothing.
    pub zoom_visibility: baumhard::gfx_structs::zoom_visibility::ZoomVisibility,
    /// `GfxElement::unique_id` of the source element. Lets keyed
    /// reshape paths ([`Renderer::reshape_buffer_for`]) drop the
    /// stale rect for a single element before re-collecting it
    /// — otherwise repeated keystrokes leak duplicate rects per
    /// edit. Always populated by the tree walker; tests synthesise
    /// rects with any sentinel value (matching by `unique_id`
    /// during reshape is the only consumer today).
    pub unique_id: usize,
}

impl NodeBackgroundRect {
    /// Should this rect render at the current camera state?
    /// Combines the spatial AABB cull (`Camera2D::is_visible`)
    /// with the zoom-window cull
    /// (`ZoomVisibility::contains`). Pure, no allocation; the
    /// render loop calls this once per rect per frame.
    pub(super) fn visible_at(&self, camera: &baumhard::gfx_structs::camera::Camera2D) -> bool {
        camera.is_visible(self.position, self.size) && self.zoom_visibility.contains(camera.zoom)
    }
}

/// Clamp a requested surface (width, height) to the GPU's
/// `max_texture_dimension_2d`. Pure function so the clamp logic is
/// testable without a live GPU device.
///
/// # Why this exists
///
/// `surface.configure` on dimensions beyond the GPU's 2D texture
/// limit can leave the surface in a bad state on some wgpu
/// backends — subsequent `get_current_texture()` calls may then
/// block indefinitely rather than returning an error. Clamping
/// proactively trades a letterboxed frame for a non-hung UI. The
/// scenario is realistic on ultra-wide displays or multi-monitor-
/// maxed windows.
pub(crate) fn clamp_surface_size_to_gpu_limit(width: u32, height: u32, max_dim: u32) -> (u32, u32) {
    let clamped_width = if width > max_dim {
        warn!(
            "Requested surface width {} exceeds GPU max_texture_dimension_2d {}; clamping",
            width, max_dim
        );
        max_dim
    } else {
        width
    };
    let clamped_height = if height > max_dim {
        warn!(
            "Requested surface height {} exceeds GPU max_texture_dimension_2d {}; clamping",
            height, max_dim
        );
        max_dim
    } else {
        height
    };
    (clamped_width, clamped_height)
}

impl Renderer {
    /// Native bootstrap: own the `wgpu::Instance` + `Surface`
    /// construction so the caller doesn't need to import `wgpu`.
    /// Hand wgpu the owned `Arc<Window>` rather than pre-snapshotting
    /// raw handles via `SurfaceTargetUnsafe::from_window`: under
    /// wgpu 29 + winit 0.30 the latter blew up with
    /// `Hal(MissingDisplayHandle)` on EGL/GL Linux because the GL
    /// surface ctor re-queries the display handle and won't accept a
    /// captured raw struct.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn bootstrap_native(window: Arc<Window>) -> Renderer {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("failed to create wgpu surface for window");
        Self::new(instance, surface, window).await
    }

    /// WASM bootstrap: same as `bootstrap_native` but binds the
    /// surface to the supplied `<canvas>` element. The browser's
    /// adapter/device init is Promise-backed so this stays async
    /// like the native form.
    #[cfg(target_arch = "wasm32")]
    pub async fn bootstrap_wasm(
        window: Arc<Window>,
        canvas: web_sys::HtmlCanvasElement,
    ) -> Renderer {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .expect("failed to create wgpu surface for canvas");
        Self::new(instance, surface, window).await
    }

    pub(crate) async fn new(instance: Instance, surface: Surface<'static>, window: Arc<Window>) -> Renderer {
        let adapter = Self::get_adapter(&instance, &surface).await;
        let (device, queue) = Self::get_device(&adapter).await;
        let surface_capabilities = surface.get_capabilities(&adapter);
        let surface_format = surface_capabilities
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_capabilities.formats[0]);
        let size = window.inner_size();
        let config = Self::create_surface_config(
            surface_format,
            &surface_capabilities,
            PhysicalSize::new(size.width, size.height),
        );
        let glyphon_cache = Cache::new(&device);

        let mut atlas = TextAtlas::new(&device, &queue, &glyphon_cache, surface_format);
        let text_renderer = TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let console_text_renderer = TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let viewport = Viewport::new(&device, &glyphon_cache);
        let camera = Camera2D::new(size.width, size.height);

        // Rect pipeline: colored quads for node backgrounds and the
        // palette backdrop. Uses the same surface format as the
        // render-pass attachment so the pipeline matches, and enables
        // standard alpha blending so semi-transparent fills compose
        // cleanly with whatever's beneath them.
        let rect_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rect_shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(RECT_SHADER_WGSL)),
        });
        let rect_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rect_pipeline_layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let rect_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rect_pipeline"),
            layout: Some(&rect_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &rect_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: RECT_VERTEX_SIZE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    // Layout: pos (8B) | uv (8B) | color (16B) | shape_id (4B)
                    //         = 36B total, must match `RECT_VERTEX_SIZE`.
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 16,
                            shader_location: 2,
                        },
                        // `shape_id` as `Float32`, not `Uint32`: wgpu's
                        // WebGL2 backend doesn't support integer vertex
                        // attributes on every browser, and we only need
                        // a handful of discrete ids. The WGSL vertex
                        // stage rounds + casts to `u32` before
                        // flat-interpolating.
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: 32,
                            shader_location: 3,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &rect_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let rect_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rect_vertex_buffer"),
            size: RECT_VBUF_INITIAL_CAPACITY,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Renderer {
            surface,
            window,
            config,
            device,
            queue,
            atlas,
            swash_cache: SwashCache::new(),
            timer: PollTimer::new(Duration::from_millis(16)),
            target_duration_between_renders: Duration::from_millis(10),
            last_render_time: Duration::from_millis(16),
            text_renderer,
            console_text_renderer,
            should_render: false,
            fps: None,
            redraw_mode: RedrawMode::NoLimit,
            run: true,
            fps_display_mode: FpsDisplayMode::Off,
            fps_overlay_buffers: Vec::new(),
            last_fps_shaped: None,
            mode_status_text: None,
            mode_status_overlay_buffers: Vec::new(),
            last_mode_status_shaped: None,
            last_frame_instant: None,
            fps_clock: 0,
            fps_ring: FrameIntervalRing::new(),
            fps_pending_idle_paint: false,
            glyphon_cache,
            viewport,
            camera,
            mindmap_buffers: Default::default(),
            border_buffers: FxHashMap::default(),
            edge_handle_buffers: Vec::new(),
            connection_label_buffers: FxHashMap::default(),
            connection_label_hitboxes: FxHashMap::default(),
            portal_icon_hitboxes: FxHashMap::default(),
            portal_text_hitboxes: FxHashMap::default(),
            console_overlay_buffers: Vec::new(),
            color_picker_backdrop: None,
            overlay_buffers: Vec::new(),
            selection_rect_shape_cache: None,
            overlay_scene_buffers: Vec::new(),
            canvas_scene_buffers: Vec::new(),
            canvas_scene_background_rects: Vec::new(),
            connection_geometry_dirty: false,
            rect_pipeline,
            rect_vertex_buffer,
            rect_vertex_buffer_capacity: RECT_VBUF_INITIAL_CAPACITY,
            node_background_rects: Vec::new(),
            main_rect_vertices: Vec::new(),
            console_rect_vertices: Vec::new(),
            console_backdrop: None,
            clear_color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        }
    }

    /// Current camera zoom level, used by the event loop when it needs
    /// to pass the active zoom into `Document::build_scene*` (the scene
    /// builder consumes it via
    /// `GlyphConnectionConfig::effective_font_size_pt`).
    pub fn camera_zoom(&self) -> f32 {
        self.camera.zoom
    }

    /// Swapchain surface width in pixels.
    pub fn surface_width(&self) -> u32 {
        self.config.width
    }

    /// Swapchain surface height in pixels.
    pub fn surface_height(&self) -> u32 {
        self.config.height
    }

    /// Set the render-pass clear color from a hex string like
    /// `#141820`. Called by the event loop after a map loads so the
    /// canvas matches `Canvas.background_color`. Bad hex degrades
    /// to opaque black via `hex_to_rgba_safe`, so a typo in a
    /// theme file can't leave the app with a glitched background.
    pub fn set_clear_color_from_hex(&mut self, hex: &str) {
        let rgba = baumhard::util::color::hex_to_rgba_safe(hex, [0.0, 0.0, 0.0, 1.0]);
        self.clear_color = Color {
            r: rgba[0] as f64,
            g: rgba[1] as f64,
            b: rgba[2] as f64,
            a: rgba[3] as f64,
        };
    }

    /// Set the screen-space FPS readout mode. Routes through the
    /// decree bus so `should_render` / `StartRender` / `StopRender`
    /// and the FPS toggle share a single in-renderer mutation point.
    pub fn set_fps_display(&mut self, mode: FpsDisplayMode) {
        self.process_decree(RenderDecree::SetFpsDisplay(mode));
    }

    /// Current FPS readout mode. Used by `ToggleFps` / `ToggleFpsDebug`
    /// dispatch arms to compute the next state.
    pub fn fps_display_mode(&self) -> FpsDisplayMode {
        self.fps_display_mode
    }

    /// Returns and resets the connection geometry-dirty flag. Called by
    /// the event loop once per frame; a `true` return means the zoom
    /// changed, so the document-side scene cache must be flushed before
    /// the next scene build.
    pub fn take_connection_geometry_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.connection_geometry_dirty, false)
    }

    /// Non-consuming peek of [`Self::take_connection_geometry_dirty`].
    /// Used by the idle-CPU `needs_continuation` predicate to
    /// decide whether the loop should keep iterating without
    /// burning the flag — `take` would consume it before the
    /// next `drain_camera_geometry_rebuild` got a chance to react.
    pub fn connection_geometry_dirty(&self) -> bool {
        self.connection_geometry_dirty
    }

    /// Forward a redraw request to the underlying winit window. On
    /// native this queues a `WindowEvent::RedrawRequested` for the
    /// next event-loop iteration; on web (winit-web) it schedules
    /// an internal `requestAnimationFrame`. Multiple calls in one
    /// event chain coalesce to a single delivery — safe to call
    /// from any handler that mutated visual state.
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    const ZERO_DURATION: Duration = Duration::new(0, 0);

    #[inline]
    pub fn process(&mut self) -> bool {
        match self.redraw_mode {
            RedrawMode::OnRequest => {
                self.fps = Some(0);
            }
            RedrawMode::FpsLimit(_) => {
                if self.timer.is_expired() {
                    let delta_duration = self.target_duration_between_renders - self.last_render_time;
                    if delta_duration.le(&Self::ZERO_DURATION) {
                        self.timer.expire_in(Duration::from(Self::ZERO_DURATION));
                    } else {
                        self.timer.expire_in(delta_duration);
                    }
                    if self.fps_display_mode != FpsDisplayMode::Off {
                        self.tick_fps();
                        self.rebuild_fps_overlay_if_needed();
                    }
                    self.rebuild_mode_status_overlay_if_needed();
                    let sw = StopWatch::new_start();
                    self.render();
                    self.last_render_time = sw.stop();
                }
            }
            RedrawMode::NoLimit => {
                if self.fps_display_mode != FpsDisplayMode::Off {
                    self.tick_fps();
                    self.rebuild_fps_overlay_if_needed();
                }
                self.rebuild_mode_status_overlay_if_needed();
                let sw = StopWatch::new_start();
                self.render();
                self.last_render_time = sw.stop();
            }
        }
        self.run
    }

    /// Set the mode-status overlay text. `None` clears the overlay
    /// (Default mode); `Some(text)` shows the line on the next
    /// frame. Called from the app's scene-rebuild paths on every
    /// mode-affecting action — the renderer trusts the app to
    /// recompute the string when (mode, selection, doc) changes.
    pub fn set_mode_status_text(&mut self, text: Option<String>) {
        self.mode_status_text = text;
    }

    /// Re-shape the cyan mode-status line when `self.mode_status_text`
    /// has changed since the last shape. Sibling of
    /// [`Self::rebuild_fps_overlay_if_needed`]; same caching
    /// discipline (skip when nothing changed). Silent on font-system
    /// lock contention — the cache key advance is gated on a
    /// successful shaping, so a missed lock retries on the next
    /// process() cycle without permanently dropping the overlay.
    #[inline]
    fn rebuild_mode_status_overlay_if_needed(&mut self) {
        // Same-string short-circuit — no parse / no allocation when
        // the active mode hasn't changed (every drag-drain frame
        // hits this path).
        if self.mode_status_text.as_deref() == self.last_mode_status_shaped.as_deref() {
            return;
        }
        let Ok(mut font_system) = fonts::FONT_SYSTEM.try_write() else {
            // Lock contention — leave the cache state alone so the
            // next process() cycle retries shaping. Pre-fix this
            // advanced `last_mode_status_shaped` before the lock
            // check, which permanently lost the overlay until the
            // text changed again.
            return;
        };
        self.mode_status_overlay_buffers.clear();
        if let Some(text) = self.mode_status_text.as_deref() {
            // Same cyan as `HIGHLIGHT_COLOR` (the selection-tint
            // constant in `document::types`) so the status bar
            // visually pairs with the selection highlight: both are
            // the canonical "active" affordance.
            // `HIGHLIGHT_COLOR = [0.0, 0.9, 1.0, 1.0]` → (0, 230, 255).
            let attrs = Attrs::new().color(baumhard::font::Color::rgba(0, 230, 255, 255));
            let buf = borders::create_border_buffer(
                &mut font_system,
                text,
                &attrs,
                MODE_STATUS_FONT_SIZE_PT,
                MODE_STATUS_OVERLAY_POS,
                MODE_STATUS_OVERLAY_BOUNDS,
            );
            self.mode_status_overlay_buffers.push(buf);
        }
        // Cache key advances only after a successful shape (or an
        // empty-text path that explicitly cleared the buffers).
        self.last_mode_status_shaped = self.mode_status_text.clone();
    }

    /// Re-shape the yellow "FPS: N" screen-space overlay when the
    /// integer `self.fps` value has changed since the last shape.
    /// Called from `process()` after `tick_fps`. In Snapshot mode
    /// the value only changes every `FPS_WINDOW` frames, so most
    /// rebuilds early-return; in Debug mode the value can change
    /// every frame, but cosmic-text shaping a 6-glyph string is
    /// cheap and only fires when the rounded integer actually
    /// shifts. Silent on font-system lock contention — the next
    /// process() cycle retries.
    ///
    /// `self.fps == None` is rendered as `"FPS: -"` to signal idle:
    /// since the overlay no longer forces continuous rendering,
    /// the readout reflects the app's actual workload — when no
    /// frames are being drawn, the dash makes that explicit
    /// instead of leaving a stale numeric value frozen on screen.
    #[inline]
    fn rebuild_fps_overlay_if_needed(&mut self) {
        if matches!(self.fps_display_mode, FpsDisplayMode::Off) {
            return;
        }
        if self.fps == self.last_fps_shaped && !self.fps_overlay_buffers.is_empty() {
            return;
        }
        let Ok(mut font_system) = fonts::FONT_SYSTEM.try_write() else {
            return;
        };
        let text = match self.fps {
            Some(n) => format!("FPS: {}", n),
            None => "FPS: -".to_string(),
        };
        let attrs = Attrs::new().color(baumhard::font::Color::rgba(255, 235, 0, 255));
        let buf =
            borders::create_border_buffer(&mut font_system, &text, &attrs, 16.0, (8.0, 8.0), (200.0, 24.0));
        self.fps_overlay_buffers.clear();
        self.fps_overlay_buffers.push(buf);
        self.last_fps_shaped = self.fps;
    }

    /// Capture the wall-clock interval since the previous frame and
    /// update `self.fps` according to the active display mode.
    /// Wall-clock (rather than `last_render_time`) is load-bearing:
    /// `render()` can early-return on a contended font-system lock
    /// under heavy drag / scene-rebuild load, which would otherwise
    /// shrink `last_render_time` to a near-zero early-return cost and
    /// inflate the reported FPS into the hundreds of thousands.
    ///
    /// A frame interval longer than [`Self::IDLE_FRAME_THRESHOLD_US`]
    /// indicates the previous "frame" was actually idle wall-clock,
    /// not a render — under event-driven rendering the loop parks
    /// between user actions. Resuming from such a gap discards the
    /// spurious huge interval (folding it into a real FPS reading
    /// would compute "FPS: 1") and resets the readout to idle so
    /// the next genuine frame interval lands fresh.
    #[inline]
    fn tick_fps(&mut self) {
        let now = Instant::now();
        let frame_micros = self
            .last_frame_instant
            .map(|prev| now.duration_since(prev).as_micros())
            .unwrap_or(0);
        self.last_frame_instant = Some(now);

        // Honour a pending idle paint queued by `set_fps_idle`: this
        // transition render must show "-" even if the rolling avg
        // would compute a value from prior active samples. Clear
        // the rolling window so the next active session starts
        // fresh instead of inheriting stale samples.
        if self.fps_pending_idle_paint {
            self.fps_pending_idle_paint = false;
            self.fps = None;
            self.fps_ring.clear();
            return;
        }

        if frame_micros > Self::IDLE_FRAME_THRESHOLD_US {
            // Resuming from idle. Don't fold the huge gap into a
            // FPS sample; just reset to the idle marker. The next
            // real frame's interval lands in a clean state.
            self.fps = None;
            return;
        }

        match self.fps_display_mode {
            FpsDisplayMode::Off => {}
            FpsDisplayMode::Snapshot => {
                if self.fps_clock % FPS_WINDOW == 0 && frame_micros > 0 {
                    self.fps = Some((1_000_000u128 / frame_micros) as usize);
                }
                self.fps_clock = self.fps_clock.wrapping_add(1);
            }
            FpsDisplayMode::Debug => {
                if frame_micros > 0 {
                    self.fps_ring.push(frame_micros);
                }
                if let Some(avg) = self.fps_ring.avg_micros() {
                    if avg > 0 {
                        self.fps = Some((1_000_000u128 / avg) as usize);
                    }
                }
            }
        }
    }

    /// A frame interval longer than this is treated as a wall-clock
    /// idle gap rather than a render — see [`Self::tick_fps`]. 500ms
    /// is comfortably longer than any genuine frame at refresh rates
    /// down to 4Hz and short enough that a brief lull during typing
    /// shows as idle in the overlay.
    const IDLE_FRAME_THRESHOLD_US: u128 = 500_000;

    /// True iff the FPS overlay currently displays a numeric reading
    /// (i.e., the renderer recently sampled a frame interval). False
    /// if the overlay is at the idle marker or has never been
    /// populated. Used by the event loop to decide whether the
    /// overlay needs one more redraw to flip to "-" before parking.
    pub fn has_live_fps(&self) -> bool {
        self.fps.is_some()
    }

    /// Decide whether the active→idle FPS transition should be
    /// deferred. Returns `Some(deadline)` if the last rendered
    /// frame is more recent than `grace`, meaning the user could
    /// still be reading the live reading and an immediate flip to
    /// "-" would flicker. The caller pairs this with
    /// `ControlFlow::WaitUntil(deadline)` so the loop wakes after
    /// the grace period to commit the transition. Returns `None`
    /// when the FPS is already idle, when no frame has been
    /// rendered yet, or when the grace period has already elapsed
    /// (transition can fire immediately).
    ///
    /// Without this, an active throttled drag whose `should_drain`
    /// gates produce momentary `needs_continuation == false` gaps
    /// between drain frames would flash "FPS: -" between every
    /// drain — making the readout unusable as a diagnostic.
    pub fn fps_idle_defer_deadline(&self, grace: Duration) -> Option<Instant> {
        if !self.has_live_fps() {
            return None;
        }
        let last = self.last_frame_instant?;
        let age = last.elapsed();
        if age >= grace {
            None
        } else {
            Some(last + grace)
        }
    }

    /// Force the FPS overlay into the idle state so the next render
    /// shows "-" instead of a stale numeric value. Called when the
    /// event loop transitions from active rendering to
    /// `ControlFlow::Wait`. Pairs with a `request_redraw` so the
    /// transition lands one final frame before the loop parks.
    /// The arm-and-consume flag protects the transitional frame
    /// from `tick_fps` re-computing a numeric reading from the
    /// pre-idle rolling-avg samples.
    pub fn set_fps_idle(&mut self) {
        self.fps = None;
        self.fps_pending_idle_paint = true;
    }

    #[inline]
    fn get_size(&self) -> PhysicalSize<u32> {
        self.window.inner_size()
    }

    #[inline]
    fn update_surface_size(&mut self, width: u32, height: u32) {
        if width == 0 {
            error!("Width has to be higher than 0 but was {}", width);
            return;
        }
        if height == 0 {
            error!("Height has to be higher than 0 but was {}", height);
            return;
        }
        let max_dim = self.device.limits().max_texture_dimension_2d;
        let (width, height) = clamp_surface_size_to_gpu_limit(width, height, max_dim);
        info!("Updating surface size");
        self.config.width = width;
        self.config.height = height;

        self.surface.configure(&self.device, &self.config);
        self.viewport.update(&self.queue, Resolution { width, height });
        self.camera.set_viewport_size(width, height);
        // Canvas-space glyph positions and shaped buffers survive a
        // viewport resize; the per-frame `visible_at` cull handles
        // whether each buffer falls inside the new bounds.
    }
}

pub struct MindMapTextBuffer {
    pub buffer: Buffer,
    pub pos: (f32, f32),
    pub bounds: (f32, f32),
    /// Per-`GlyphArea` zoom window copied in at buffer-build time.
    /// The main render loop skips this buffer whenever
    /// `camera.zoom` falls outside the window. Default (both
    /// bounds `None`) renders at every zoom — existing buffers pay
    /// nothing.
    pub zoom_visibility: baumhard::gfx_structs::zoom_visibility::ZoomVisibility,
}

impl MindMapTextBuffer {
    /// Should this text buffer render at the current camera
    /// state? Combines the spatial AABB cull
    /// (`Camera2D::is_visible`) with the zoom-window cull
    /// (`ZoomVisibility::contains`). Pure, no allocation; the
    /// render loop calls this once per buffer per frame in the
    /// `main_text_areas` collector.
    pub(super) fn visible_at(&self, camera: &baumhard::gfx_structs::camera::Camera2D) -> bool {
        let pos = Vec2::new(self.pos.0, self.pos.1);
        let size = Vec2::new(self.bounds.0, self.bounds.1);
        camera.is_visible(pos, size) && self.zoom_visibility.contains(camera.zoom)
    }
}

#[cfg(test)]
mod tests;
