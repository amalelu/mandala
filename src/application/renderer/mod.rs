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
    build_console_border_strings, compute_console_frame_layout, ConsoleFrameLayout,
    ConsoleOverlayCompletion, ConsoleOverlayGeometry, ConsoleOverlayLine, ConsoleOverlayLineKind,
    MAX_CONSOLE_COMPLETION_ROWS, MAX_CONSOLE_SCROLLBACK_ROWS,
};
// These imports are referenced only from the test block; the
// non-test build flags them as unused. Gate to keep cargo check
// clean while leaving the test build self-contained.
#[cfg(test)]
use borders::{create_border_buffer, parse_hex_color};
#[cfg(test)]
use console_pass::{
    build_console_overlay_mutator, build_console_overlay_tree, console_overlay_areas,
    console_overlay_signature,
};
#[cfg(test)]
use render::glyph_position_in_viewport;
#[cfg(test)]
use tree_walker::walk_tree_into_buffers;

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;
use web_time::Instant;

use cosmic_text::{Attrs, AttrsList, Buffer, FontSystem};
use cosmic_text::{Family, Style};
use glyphon::{Cache, Resolution, SwashCache, TextAtlas, TextRenderer, Viewport};
use log::{error, info, warn};

use rustc_hash::FxHashMap;

use wgpu::{
    Adapter, Color, Device, Instance, MultisampleState, Queue, RenderPipeline,
    ShaderModule, Surface, SurfaceCapabilities, SurfaceConfiguration, TextureFormat,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::application::common::{FpsDisplayMode, PollTimer, RedrawMode, RenderDecree, StopWatch};
use baumhard::font::fonts;
use baumhard::font::fonts::AppFont;
#[cfg(test)]
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::mindmap::scene_cache::EdgeKey;
use baumhard::shaders::shaders::SHADER_APPLICATION;
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
/// place. Keep in sync with the attribute list in
/// `create_render_pipeline` and with the per-vertex push in
/// `push_rect_ndc`.
const RECT_VERTEX_SIZE: u64 = 36;

/// How many frames `FpsDisplayMode::Snapshot` waits between readout
/// refreshes, and how many frames `FpsDisplayMode::Debug` averages
/// over. 200 at 60 fps ≈ 3.3 s — short enough to react to sustained
/// perf changes, long enough to smooth out per-frame jitter.
const FPS_WINDOW: usize = 200;

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
    instance: Instance,
    surface: Surface<'static>,
    window: Arc<Window>,
    config: SurfaceConfiguration,
    adapter: Adapter,
    device: Device,
    queue: Queue,
    viewport: Viewport,
    swash_cache: SwashCache,
    glyphon_cache: Cache,
    atlas: TextAtlas,
    timer: PollTimer,
    target_duration_between_renders: Duration,
    last_render_time: Duration,
    shaders: FxHashMap<&'static str, ShaderModule>,
    render_pipeline: RenderPipeline,
    text_renderer: TextRenderer,
    /// Second glyphon TextRenderer dedicated to the command
    /// palette overlay. Shares `self.atlas` with `text_renderer`
    /// so glyph caching is unified, but keeps its own internal
    /// vertex/index buffers — which is what lets us issue a rect
    /// draw BETWEEN the two text renders inside one render pass
    /// (otherwise re-preparing the single text renderer would
    /// race with the pass's already-recorded draw commands).
    console_text_renderer: TextRenderer,
    texture_format: TextureFormat,
    surface_capabilities: SurfaceCapabilities,
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

    camera: Camera2D,
    mindmap_buffers: FxHashMap<String, MindMapTextBuffer>,
    /// Per-node border glyph buffers, keyed by `node_id`. Each entry is a
    /// `Vec` of 4 buffers (top/bottom/left/right) matching the layout in
    /// `rebuild_border_buffers_keyed`. Keyed so unchanged borders survive
    /// across drag frames without re-shaping — cosmic-text shaping is
    /// the dominant cost here, skipping it for unmoved borders is what
    /// keeps drag interactive.
    border_buffers: FxHashMap<String, Vec<MindMapTextBuffer>>,
    /// Per-edge connection glyph buffers, keyed by `(from_id, to_id,
    /// edge_type)`. Each entry is the `Vec` of already-shaped glyph
    /// buffers for that edge. Keyed so unchanged edges survive across
    /// drag frames — the big win for the "long cross-link, dragging
    /// something else" scenario.
    connection_buffers: FxHashMap<EdgeKey, Vec<MindMapTextBuffer>>,
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
    /// Set whenever the camera's viewport rect changes (pan, zoom,
    /// resize) and `connection_buffers` was cleared as a result.
    /// Consumed once per frame by the event loop in `AboutToWait` to
    /// rebuild the connection buffers against the new viewport.
    /// Without this flag, clearing the map on camera change would leave
    /// it empty until the next structural change, which is why
    /// connections used to vanish on pan.
    connection_viewport_dirty: bool,
    /// Set whenever the camera *zoom* changes. The document-side
    /// `SceneConnectionCache` stores pre-clip samples whose spacing
    /// depends on `GlyphConnectionConfig::effective_font_size_pt`, which
    /// is a function of zoom — so on zoom the cache must be flushed
    /// before the next scene build re-samples. `SceneConnectionCache`
    /// enforces this internally via `ensure_zoom`, but we still raise
    /// this flag so the event loop can explicitly clear the cache and
    /// order the rebuild readably alongside the viewport-dirty path.
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
}

impl NodeBackgroundRect {
    /// Should this rect render at the current camera state?
    /// Combines the spatial AABB cull (`Camera2D::is_visible`)
    /// with the zoom-window cull
    /// (`ZoomVisibility::contains`). Pure, no allocation; the
    /// render loop calls this once per rect per frame.
    pub(super) fn visible_at(
        &self,
        camera: &baumhard::gfx_structs::camera::Camera2D,
    ) -> bool {
        camera.is_visible(self.position, self.size)
            && self.zoom_visibility.contains(camera.zoom)
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
pub(crate) fn clamp_surface_size_to_gpu_limit(
    width: u32,
    height: u32,
    max_dim: u32,
) -> (u32, u32) {
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
    pub async fn new(
        instance: Instance,
        surface: Surface<'static>,
        window: Arc<Window>,
    ) -> Renderer {
        let adapter = Self::get_adapter(&instance, &surface).await;
        let (device, queue) = Self::get_device(&adapter).await;
        let mut shaders = FxHashMap::default();
        Self::load_shaders(&device, &mut shaders);
        assert!(shaders.len() > 0, "No shaders found!");
        let shader = shaders
            .get(SHADER_APPLICATION)
            .expect(&*format!("Shader not found {}", SHADER_APPLICATION));
        let swapchain_format = TextureFormat::Bgra8UnormSrgb;
        let pipeline_layout = Self::create_pipeline_layout(&device);
        let surface_capabilities = surface.get_capabilities(&adapter);
        let texture_format = surface_capabilities.formats[0];

        let render_pipeline = Self::create_render_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            texture_format.clone(),
        );
        let size = window.inner_size();
        let config = Self::create_surface_config(
            texture_format.clone(),
            &surface_capabilities,
            PhysicalSize::new(size.width, size.height),
        );
        let glyphon_cache = Cache::new(&device);

        let mut atlas = TextAtlas::new(&device, &queue, &glyphon_cache, swapchain_format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let console_text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let viewport = Viewport::new(&device, &glyphon_cache);
        let camera = Camera2D::new(size.width, size.height);

        // Rect pipeline: colored quads for node backgrounds and the
        // palette backdrop. Uses the swapchain (not capability[0])
        // format so the pipeline matches the LoadOp target, and
        // enables standard alpha blending so semi-transparent fills
        // compose cleanly with whatever's beneath them.
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
                    format: swapchain_format,
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
            instance,
            surface,
            window,
            config,
            adapter,
            device,
            queue,
            atlas,
            swash_cache: SwashCache::new(),
            timer: PollTimer::new(Duration::from_millis(16)),
            target_duration_between_renders: Duration::from_millis(10),
            last_render_time: Duration::from_millis(16),
            shaders,
            render_pipeline,
            text_renderer,
            console_text_renderer,
            texture_format,
            surface_capabilities,
            should_render: false,
            fps: None,
            redraw_mode: RedrawMode::NoLimit,
            run: true,
            fps_display_mode: FpsDisplayMode::Off,
            fps_overlay_buffers: Vec::new(),
            last_fps_shaped: None,
            last_frame_instant: None,
            fps_clock: 0,
            fps_ring: FrameIntervalRing::new(),
            glyphon_cache,
            viewport,
            camera,
            mindmap_buffers: Default::default(),
            border_buffers: FxHashMap::default(),
            connection_buffers: FxHashMap::default(),
            edge_handle_buffers: Vec::new(),
            connection_label_buffers: FxHashMap::default(),
            connection_label_hitboxes: FxHashMap::default(),
            portal_icon_hitboxes: FxHashMap::default(),
            portal_text_hitboxes: FxHashMap::default(),
            console_overlay_buffers: Vec::new(),
            color_picker_backdrop: None,
            overlay_buffers: Vec::new(),
            overlay_scene_buffers: Vec::new(),
            canvas_scene_buffers: Vec::new(),
            canvas_scene_background_rects: Vec::new(),
            connection_viewport_dirty: false,
            connection_geometry_dirty: false,
            rect_pipeline,
            rect_vertex_buffer,
            rect_vertex_buffer_capacity: RECT_VBUF_INITIAL_CAPACITY,
            node_background_rects: Vec::new(),
            main_rect_vertices: Vec::new(),
            console_rect_vertices: Vec::new(),
            console_backdrop: None,
            clear_color: Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 },
        }
    }

    /// Current camera zoom level, used by the event loop when it needs
    /// to pass the active zoom into `Document::build_scene*` (the scene
    /// builder consumes it via
    /// `GlyphConnectionConfig::effective_font_size_pt`).
    pub fn camera_zoom(&self) -> f32 {
        self.camera.zoom
    }

    /// Width of the swapchain surface in pixels. Used by overlay
    /// modal layouts (palette, glyph-wheel color picker) so they can
    /// position themselves in screen space without poking the wgpu
    /// config directly.
    pub fn surface_width(&self) -> u32 {
        self.config.width
    }

    /// Height of the swapchain surface in pixels. Counterpart to
    /// `surface_width`.
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


    /// Returns and resets the connection viewport-dirty flag. Called by
    /// the event loop once per frame in `AboutToWait`; a `true` return
    /// means the viewport rect changed since the last frame and the
    /// per-glyph viewport cull needs to run again.
    pub fn take_connection_viewport_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.connection_viewport_dirty, false)
    }

    /// Returns and resets the connection geometry-dirty flag. Called by
    /// the event loop once per frame; a `true` return means the zoom
    /// changed, so the document-side scene cache must be flushed before
    /// the next scene build.
    pub fn take_connection_geometry_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.connection_geometry_dirty, false)
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
                    let delta_duration =
                        self.target_duration_between_renders - self.last_render_time;
                    if delta_duration.le(&Self::ZERO_DURATION) {
                        self.timer.expire_in(Duration::from(Self::ZERO_DURATION));
                    } else {
                        self.timer.expire_in(delta_duration);
                    }
                    self.tick_fps();
                    self.rebuild_fps_overlay_if_needed();
                    let sw = StopWatch::new_start();
                    self.render();
                    self.last_render_time = sw.stop();
                }
            }
            RedrawMode::NoLimit => {
                self.tick_fps();
                self.rebuild_fps_overlay_if_needed();
                let sw = StopWatch::new_start();
                self.render();
                self.last_render_time = sw.stop();
            }
        }
        self.run
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
        let text = format!("FPS: {}", self.fps.unwrap_or(0));
        let attrs = Attrs::new().color(cosmic_text::Color::rgba(255, 235, 0, 255));
        let buf = borders::create_border_buffer(
            &mut font_system,
            &text,
            &attrs,
            16.0,
            (8.0, 8.0),
            (200.0, 24.0),
        );
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
    #[inline]
    fn tick_fps(&mut self) {
        let now = Instant::now();
        let frame_micros = self
            .last_frame_instant
            .map(|prev| now.duration_since(prev).as_micros())
            .unwrap_or(0);
        self.last_frame_instant = Some(now);

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

    #[inline]
    fn get_size(&self) -> PhysicalSize<u32> {
        self.window.inner_size()
    }


    #[inline]
    fn update_surface_size(&mut self, width: u32, height: u32) {
        if width <= 0 {
            error!("Width has to be higher than 0 but was {}", width);
            return;
        }
        if height <= 0 {
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
        // whether each buffer falls inside the new bounds. No
        // rebuild is needed.
    }


}


pub fn example_attrib(font_system: &mut FontSystem) -> AttrsList {
    let evilz_font = fonts::COMPILED_FONT_ID_MAP.get(&AppFont::Evilz).unwrap();
    let evilz_face = font_system.db().face(evilz_font[0]).unwrap();
    let mut attr_list = AttrsList::new(&Attrs::new());
    attr_list.add_span(
        Range { start: 0, end: 10 },
        &Attrs::new()
            .style(Style::Normal)
            .color(cosmic_text::Color::rgba(102, 51, 51, 255))
            .family(Family::Name(evilz_face.families[0].0.as_ref())),
    );
    let nightcrow_font = fonts::COMPILED_FONT_ID_MAP
        .get(&AppFont::NIGHTCROW)
        .unwrap();
    let nightcrow_face = font_system.db().face(nightcrow_font[0]).unwrap();
    attr_list.add_span(
        Range {
            start: 11,
            end: 500,
        },
        &Attrs::new()
            .style(Style::Normal)
            .color(cosmic_text::Color::rgba(0, 153, 51, 255))
            .family(Family::Name(nightcrow_face.families[0].0.as_ref())),
    );

    attr_list
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
    pub(super) fn visible_at(
        &self,
        camera: &baumhard::gfx_structs::camera::Camera2D,
    ) -> bool {
        let pos = Vec2::new(self.pos.0, self.pos.1);
        let size = Vec2::new(self.bounds.0, self.bounds.1);
        camera.is_visible(pos, size) && self.zoom_visibility.contains(camera.zoom)
    }
}




#[cfg(test)]
mod tests;
