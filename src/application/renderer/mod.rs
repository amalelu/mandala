// SPDX-License-Identifier: MPL-2.0

//! GPU-side presentation: every wgpu device, every cosmic-text
//! rasterization, every text/rect/border buffer Mandala paints
//! lives under [`Renderer`]. `Renderer` reads two intermediate
//! representation the document layer hands it
//! (`Tree<GfxElement, GfxMutator>`, one per canvas role — nodes,
//! borders, connections, labels, portals, handles); it never
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
//! - [`selection_overlay`] — the rubber-band selection
//!   rectangle, the one canvas visual with no model behind it.
//! - [`borders`] — node-frame buffers (the box-drawing
//!   glyph runs around each node).
//! - [`console_pass`] / [`console_geometry`] — the console
//!   overlay's glyph-tree pass + pure-function layout math.
//! - [`color_picker`] — the glyph-wheel picker overlay.
//! - [`camera`] — camera framing and the screen-space ↔
//!   canvas-space mapping (`screen_to_canvas`,
//!   `canvas_per_pixel`, `fit_camera_to_tree`).
//! - [`decree`] — the `RenderDecree` queue the event loop
//!   feeds the renderer (resize, zoom, camera-pan, etc.).
//! - [`overlay_dispatch`] — overlay-vs-canvas slot routing
//!   for the [`crate::application::scene_host::AppScene`]
//!   tree handles.

// The overlay-render modules below carry a wasm32-only
// `allow(dead_code)`: each renders a modal whose *shell* is
// native-gated (CLAUDE.md "Dual-target status"), so on wasm32 the
// builders compile with no caller. Scoped to wasm32 so the host
// lint, which can see the shell, stays armed.
mod borders;
mod camera;
// Color-picker overlay tree + mutators. Shell: the native-gated
// `app::color_picker_flow`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod color_picker;
// Console overlay geometry. Shell: the native-gated
// `app::console_input`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod console_geometry;
// Console overlay glyph areas + mutators. Same shell as
// `console_geometry`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod console_pass;
mod decree;
// Renderer entry points for the two overlay rebuilds. Same shells
// as `console_pass` and `color_picker`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod overlay_dispatch;
mod pipeline;
mod render;
mod scene_shape_cache;
// Test-only reach-through to the shared scene shape pass. The pass
// takes a `&Scene` rather than a `&mut Renderer` precisely so its
// granularity can be asserted without a live device (§T8), and that
// only helps if the assertion can name it: the canvas half's fixture
// is a real `AppScene` projected by `app::scene_rebuild`'s
// `CanvasFrame`, which lives outside this module. The module stays
// private in a shipped build, so nothing here widens the renderer's
// production surface.
#[cfg(test)]
pub(crate) use scene_shape_cache::{refresh, ScenePassKind, ShapedSceneElement};
// Rubber-band selection rectangle. Driven by the native-gated
// drag state machine.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod selection_overlay;
mod tree_buffers;
mod tree_walker;

pub use borders::measure_max_glyph_advance;
// The console overlay's geometry vocabulary, re-exported for the
// app-side console shell (`app::console_input`) that fills it in.
// That shell is native-gated, so on wasm32 nothing consumes these —
// the same reason the overlay modules above carry a wasm32-only
// `allow`, scoped the same way so the host lint stays armed.
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
pub use console_geometry::{
    ConsoleOverlayCompletion, ConsoleOverlayGeometry, ConsoleOverlayLine, ConsoleOverlayLineKind,
    MAX_CONSOLE_SCROLLBACK_ROWS,
};
// Layout math exercised only by this module's test block; the
// production callers inside `renderer/` reach it through
// `super::console_geometry::` directly.
#[cfg(test)]
use console_geometry::{
    build_console_border_strings, compute_console_frame_layout, MAX_CONSOLE_COMPLETION_ROWS,
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

use wgpu::{Color, Device, Instance, MultisampleState, Queue, RenderPipeline, Surface, SurfaceConfiguration};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::application::common::{FpsDisplayMode, RenderDecree};
use baumhard::font::fonts;
#[cfg(test)]
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::camera::Camera2D;
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
/// in lock-step, and
/// `test_every_node_shape_has_a_matching_wgsl_constant_and_case_arm`
/// is what holds them there — it reads this string as text, so no
/// GPU is involved and §T8 is untouched. That it is *this* string
/// `Renderer::new` compiles, rather than another const beside it, is
/// pinned by
/// `test_every_rect_vertex_attribute_lands_in_the_vs_in_field_at_its_location`:
/// a const read by value can be orphaned, and a decoy handed to
/// `create_shader_module` would leave both tests holding over a
/// shader the GPU never sees.
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
    // round is belt-and-braces against any driver-side rasterization
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
/// vec2<f32> uv + vec4<f32> color + f32 shape_id = 9 × 4 = 36 bytes`.
/// Used when sizing / offsetting the vertex buffer. Declared as a
/// compile-time const so the layout math is grep-able from a single
/// place.
///
/// Not kept in step by hand: it is the running sum of
/// [`RECT_VERTEX_ATTRIBUTES`]' formats, and
/// `test_every_rect_vertex_attribute_lands_in_the_vs_in_field_at_its_location`
/// adds them up and compares. The same test ties it to
/// [`RECT_VERTEX_FLOATS`] and to the slots `push_rect_ndc` writes,
/// and pins that this constant — not a literal beside it — is the
/// `array_stride` `Renderer::new` gives the pipeline. wgpu asks only
/// that a stride be a multiple of four and leave room for every
/// attribute, so an over-large one builds cleanly and reads every
/// vertex after the first from the wrong byte.
const RECT_VERTEX_SIZE: u64 = 36;

/// The rect pipeline's vertex-attribute table: which run of bytes in
/// each [`RECT_VERTEX_SIZE`]-byte vertex feeds which `VsIn` field of
/// [`RECT_SHADER_WGSL`].
///
/// Layout: `pos` (8 B) | `uv` (8 B) | `color` (16 B) | `shape_id`
/// (4 B), the offsets being the running sum of the formats' sizes and
/// the order being the order `push_rect_ndc` writes them.
///
/// **The names in that sentence are the shader's, not this table's.**
/// An entry here carries a `shader_location` and nothing else; which
/// field it lands in is decided by the `@location(N)` in `VsIn`. Two
/// attributes of the same format — `pos` and `uv` both being
/// `Float32x2` — differ in nothing a pipeline can validate, so
/// swapping their `shader_location`s builds a byte-identical
/// pipeline that draws every quad into the `[0, 1]²` corner of NDC.
/// What holds the two sides together is
/// `test_every_rect_vertex_attribute_lands_in_the_vs_in_field_at_its_location`,
/// which reads this table slot for slot against `VsIn`.
///
/// A module-level `const` rather than an array literal inline in
/// `Renderer::new` for that test's sake: nothing here is a
/// source-level property, so the test reads it as a *value* instead
/// of re-parsing what the compiler has already parsed. What the test
/// does read as text is the expression `Renderer::new` writes after
/// `attributes:`, compared whole against `&RECT_VERTEX_ATTRIBUTES`.
/// An inline array grown back alongside, a `RECT_VERTEX_ATTRIBUTES_2`
/// beside this one, or a `&RECT_VERTEX_ATTRIBUTES[..3]` would each
/// leave this table a decoy the GPU never sees, and the first name in
/// that list is why the comparison is an equality rather than a
/// substring search.
///
/// `shape_id` rides the stream as `Float32`, not `Uint32`: wgpu's
/// WebGL2 backend doesn't support integer vertex attributes on every
/// browser, and we only need a handful of discrete ids. The WGSL
/// vertex stage rounds + casts to `u32` before flat-interpolating,
/// so `VsIn.shape_id` is an `f32` on purpose.
const RECT_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] = [
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
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 32,
        shader_location: 3,
    },
];

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

/// Mode-status text color. The status row and the selection tint
/// are the same "this is active" affordance, so they are the same
/// color — derived from the one definition of it rather than
/// re-quantised here, which is how this used to read
/// `Color::rgba(0, 230, 255, 255)` against a `#00E5FF` whose green
/// channel is 229.
pub(crate) const MODE_STATUS_COLOR: baumhard::font::Color =
    baumhard::font::color::cosmic_color_from_color(baumhard::mindmap::SELECTION_HIGHLIGHT);

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
/// packed floats into `main_rect_vertices` / `console_rect_vertices`,
/// and the draw divides their length by this to get a vertex count,
/// so it is four bytes times this that has to equal
/// [`RECT_VERTEX_SIZE`].
///
/// `shape_id` keeps the stream a single `Vec<f32>` by riding it as
/// its own **numeric value** — `shape_id as f32` in `push_rect_ndc`,
/// read back as `u32(round(in.shape_id))` in the vertex stage. It is
/// not a bit pattern: this doc claimed `f32::from_bits` until #147,
/// and nothing in the tree has ever done that. It could not — the
/// bit pattern of `1u32` is a denormal that rounds straight back to
/// zero, so every ellipse would draw as a rectangle.
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
    /// The window the surface was created from. Held, not read: the
    /// `Surface<'static>` lifetime is bought by handing wgpu an
    /// `Arc<Window>` clone at `create_surface`, and this is the
    /// renderer's own share of that ownership — dropping the field
    /// would make the renderer's lifetime depend on whoever else
    /// still holds the window. `should_render` gating and
    /// `ControlFlow::Poll` mean nothing on native asks the window to
    /// redraw itself; the browser's loop does, through
    /// [`Self::request_redraw`].
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    window: Arc<Window>,
    config: SurfaceConfiguration,
    device: Device,
    queue: Queue,
    viewport: Viewport,
    swash_cache: SwashCache,
    /// glyphon's shader / bind-group cache. Held, not read: it is
    /// borrowed once each by `TextAtlas::new` and `Viewport::new` at
    /// construction, and both keep their own handle to it. Dropping
    /// the field would end the renderer's share of a resource its
    /// two consumers still use.
    #[allow(dead_code)]
    glyphon_cache: Cache,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    /// Second glyphon TextRenderer dedicated to the command
    /// palette overlay. Shares `self.atlas` with `text_renderer`
    /// so glyph caching is unified, but keeps its own internal
    /// vertex/index buffers — which is what lets us issue a rect
    /// draw BETWEEN the two text renders inside one render pass
    /// (otherwise re-preparing the single text renderer would
    /// race with the pass's already-recorded draw commands).
    console_text_renderer: TextRenderer,
    /// Whether the current glyphon-`prepare` fault episode has already
    /// been reported. Set when either `prepare()` fails, cleared on
    /// the next fully successful prepare, so a *persistent* fault —
    /// `PrepareError::AtlasFull` is permanent once the atlas hits
    /// `max_texture_dimension_2d` — logs one line instead of one per
    /// frame. A failed prepare skips `present()` too, so the frame
    /// takes no vsync backpressure and the loop can spin at CPU rate.
    prepare_fault_logged: bool,
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
    /// wall-clock here rather than the duration of the `render()`
    /// call is load-bearing: `render()` can early-return on
    /// font-system lock contention under heavy interaction, and
    /// timing the call would then shrink to near-zero and inflate FPS
    /// to a false huge value.
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
    /// Mindmap text buffers keyed by `GfxElement::unique_id`.
    ///
    /// The key is the raw `usize`. It used to be that `usize`
    /// stringified, justified as fitting "the edit / undo paths'
    /// Dewey-decimal addressing" — but no Dewey-decimal string ever
    /// reaches this map: every writer and every reader has a
    /// `unique_id` integer in hand and was allocating a `String` to
    /// spell it, once per element per rebuild and once per moved
    /// node per drained drag frame.
    ///
    /// The value is a `Vec<MindMapTextBuffer>` because the tree
    /// walker emits **multiple buffers per element** when an
    /// outline halo is configured: one buffer per halo offset
    /// emitted before the main glyph, all sharing the same
    /// `unique_id`. Pre-vec the store collapsed every halo onto the
    /// main glyph (last-write-wins via `insert`); the vec preserves
    /// emission order so halos stay behind the main glyph at render
    /// time.
    mindmap_buffers: FxHashMap<usize, Vec<MindMapTextBuffer>>,
    /// Screen-space geometry of the color picker's opaque backdrop.
    /// Captured inside `rebuild_color_picker_overlay_buffers`; the
    /// `render()` rect-pipeline pass appends a black fill rect for
    /// this region alongside the palette backdrop. `None` whenever
    /// the picker is closed.
    color_picker_backdrop: Option<(f32, f32, f32, f32)>,
    /// The rubber-band selection rectangle's four shaped edges, and
    /// nothing else. Camera-transformed. Written only by
    /// [`Self::rebuild_selection_rect_overlay`] and dropped only by
    /// [`Self::clear_overlay_buffers`]; the color picker, the FPS
    /// readout and the mode-status line each have their own store.
    ///
    /// The exclusivity is load-bearing rather than incidental:
    /// `app::drain_frame::drain_rect_select` — a plain code span
    /// because `app` is a private module and rustdoc cannot resolve
    /// a link into one — clears this on every frame no rubber band
    /// is live, so anything else parked here would be wiped. The
    /// doc used to read "temporary overlay buffers (e.g., selection
    /// rectangle)", which invited exactly that.
    overlay_buffers: Vec<MindMapTextBuffer>,
    /// `(char_count, row_count)` of the most recent selection-rect
    /// shape held in [`Self::overlay_buffers`]. Per-tick rebuilds
    /// reuse the existing shaped buffers (just update positions)
    /// when these counts match, avoiding 4 fresh `cosmic_text`
    /// shapings per drag tick. `None` whenever the overlay is
    /// cleared or holds a non-selection-rect shape.
    selection_rect_shape_cache: Option<(usize, usize)>,
    /// Screen-space output produced by walking the overlay sub-scene
    /// of the app's
    /// [`AppScene`](crate::application::scene_host::AppScene), in
    /// draw order, one entry per walked element. Populated by
    /// [`Self::rebuild_overlay_scene_buffers`] and drawn alongside
    /// the existing console/color-picker overlay buffer lists. Empty
    /// until an overlay migrates to a tree.
    ///
    /// Each entry carries the shaping inputs its buffers came from
    /// so the next pass can reuse them unchanged; see
    /// [`scene_shape_cache`] for the reuse rule. That is also why
    /// this is a `Vec` of per-element groups rather than one flat
    /// buffer list — the grouping is what a walk position can be
    /// checked against.
    overlay_scene_elements: Vec<scene_shape_cache::ShapedSceneElement>,
    /// Canvas-space output for the same
    /// [`AppScene`](crate::application::scene_host::AppScene)'s
    /// canvas sub-scene (borders, connections, portals, etc.),
    /// grouped and reused by the same rule. Populated by
    /// [`Self::rebuild_canvas_scene_buffers`]. Drawn in the main
    /// camera-transformed pass. Empty until a canvas component
    /// migrates to a tree.
    ///
    /// Unlike the overlay half, these entries carry the walker's
    /// background fills as well as its buffers: canvas fills *are*
    /// wired to the camera-transformed rect pipeline. Holding each
    /// element's fill inside the element's own entry is what keeps
    /// that index-ordered draw stream stable while only part of the
    /// scene re-shapes.
    canvas_scene_elements: Vec<scene_shape_cache::ShapedSceneElement>,
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
    /// edit. Always populated by the tree walker; tests synthesize
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
    pub async fn bootstrap_wasm(window: Arc<Window>, canvas: web_sys::HtmlCanvasElement) -> Renderer {
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
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: RECT_VERTEX_SIZE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    // The layout, its offsets and why `shape_id` is a
                    // float are all documented on the table itself.
                    attributes: &RECT_VERTEX_ATTRIBUTES,
                })],
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
            text_renderer,
            console_text_renderer,
            prepare_fault_logged: false,
            should_render: false,
            fps: None,
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
            color_picker_backdrop: None,
            overlay_buffers: Vec::new(),
            selection_rect_shape_cache: None,
            overlay_scene_elements: Vec::new(),
            canvas_scene_elements: Vec::new(),
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

    /// Current camera zoom level, used by the event loop when it
    /// needs to pass the active zoom into `CanvasFrame::new` (the
    /// connection, label, and portal passes consume it via
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
    /// decree bus so `should_render` / `StartRender` and the FPS
    /// toggle share a single in-renderer mutation point.
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
    // Native-driver-only: read by the native idle-CPU
    // `needs_continuation` predicate; the browser's rAF loop has no
    // idle state to decide about.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn connection_geometry_dirty(&self) -> bool {
        self.connection_geometry_dirty
    }

    /// Forward a redraw request to the underlying winit window. On
    /// web (winit-web) this schedules an internal
    /// `requestAnimationFrame`; multiple calls in one event chain
    /// coalesce to a single delivery, so it is safe to call from any
    /// handler that mutated visual state.
    ///
    /// Browser-only consumer: `run_wasm` drives its frames off winit
    /// redraw requests, while native's loop runs `ControlFlow::Poll`
    /// and never asks. Lint armed on wasm32 so the day that changes,
    /// wasm32 says so.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// Draw one frame and report whether the event loop should keep
    /// going. The loop paces itself — `ControlFlow::Poll` plus the
    /// surface's own vsync backpressure — so there is no rate gate
    /// here.
    ///
    /// There used to be one: a `RedrawMode` with `OnRequest` /
    /// `FpsLimit(n)` / `NoLimit` arms. The field was written exactly
    /// once, to `NoLimit`, so the other two were unreachable, and the
    /// `FpsLimit` arm subtracted one `Duration` from another with no
    /// saturation — with its seeded values (10 ms target, 16 ms last
    /// render) the very first tick would have underflowed and
    /// panicked. Wiring the mode would not have enabled a feature; it
    /// would have crashed the frame. Reinstating a frame cap means
    /// writing the arithmetic, not reviving this.
    #[inline]
    pub fn process(&mut self) -> bool {
        if self.fps_display_mode != FpsDisplayMode::Off {
            self.tick_fps();
            self.rebuild_fps_overlay_if_needed();
        }
        self.rebuild_mode_status_overlay_if_needed();
        self.render();
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
        let Some(mut font_system) = fonts::try_acquire_font_system_write("renderer::mode_status_overlay")
        else {
            // Lock contention — leave the cache state alone so the
            // next process() cycle retries shaping. Pre-fix this
            // advanced `last_mode_status_shaped` before the lock
            // check, which permanently lost the overlay until the
            // text changed again.
            return;
        };
        self.mode_status_overlay_buffers.clear();
        if let Some(text) = self.mode_status_text.as_deref() {
            let attrs = Attrs::new().color(MODE_STATUS_COLOR);
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
        let Some(mut font_system) = fonts::try_acquire_font_system_write("renderer::fps_overlay") else {
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
    /// Wall-clock (rather than the duration of the `render()` call)
    /// is load-bearing: `render()` can early-return on a contended
    /// font-system lock under heavy drag / scene-rebuild load, which
    /// would otherwise shrink the measured cost to a near-zero
    /// early-return and inflate the reported FPS into the hundreds of
    /// thousands.
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

        // Honor a pending idle paint queued by `set_fps_idle`: this
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
                if self.fps_clock.is_multiple_of(FPS_WINDOW) && frame_micros > 0 {
                    self.fps = Some((1_000_000u128 / frame_micros) as usize);
                }
                self.fps_clock = self.fps_clock.wrapping_add(1);
            }
            FpsDisplayMode::Debug => {
                if frame_micros > 0 {
                    self.fps_ring.push(frame_micros);
                }
                // `checked_div`: a zero average leaves `self.fps` as it was.
                if let Some(fps) = self
                    .fps_ring
                    .avg_micros()
                    .and_then(|avg| 1_000_000u128.checked_div(avg))
                {
                    self.fps = Some(fps as usize);
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
    // Native-driver-only, with the two below: the active-to-idle FPS
    // transition belongs to `ControlFlow::Wait`, which only native's
    // loop enters.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
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
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
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
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn set_fps_idle(&mut self) {
        self.fps = None;
        self.fps_pending_idle_paint = true;
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
    /// The `(dx, dy)` this buffer was emitted at relative to its
    /// authoring `GlyphArea`'s anchor — `(0, 0)` for a main glyph,
    /// one of [`OutlineStyle::offsets`](baumhard::gfx_structs::area_fields::OutlineStyle::offsets)
    /// for an outline-halo stamp. Kept so a position-only patch
    /// (`Renderer::patch_drag_positions`) can re-derive
    /// `pos = anchor + emission_offset` instead of collapsing every
    /// halo of a dragged element onto its main glyph.
    pub emission_offset: (f32, f32),
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
