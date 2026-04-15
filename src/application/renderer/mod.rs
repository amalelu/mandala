mod borders;
mod color_picker;
mod console_geometry;
mod console_pass;
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
use borders::{
    create_border_buffer, parse_hex_color,
};
use console_pass::{
    build_console_overlay_mutator, build_console_overlay_tree, console_overlay_signature,
};
// `console_overlay_areas` is referenced only from the test block; the
// non-test build flags it as unused. Gate to keep cargo check clean
// while leaving the test build self-contained.
#[cfg(test)]
use console_pass::console_overlay_areas;
use tree_walker::walk_tree_into_buffers;

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use cosmic_text::{Attrs, AttrsList, Buffer, FontSystem};
use glam::{Mat4, Quat, Vec3};
use cosmic_text::{Family, Style};
use glyphon::{Cache, Resolution, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport};
use log::{debug, error, info};
use rustc_hash::FxHashMap;

use wgpu::{
    Adapter, Color, Device, Instance, MultisampleState, PipelineLayout, Queue, RenderPipeline,
    ShaderModule, StoreOp, Surface, SurfaceCapabilities, SurfaceConfiguration, TextureFormat,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::application::common::{PollTimer, RedrawMode, RenderDecree, StopWatch};
use baumhard::font::fonts;
use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::element::GfxElement;
#[cfg(test)]
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use baumhard::shaders::shaders::{SHADERS, SHADER_APPLICATION};
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::mindmap::scene_builder::{RenderScene, BorderElement, ConnectionElement, PortalRefKey};
use baumhard::mindmap::scene_cache::EdgeKey;
use glam::Vec2;


/// Inline WGSL shader for the colored-rectangle pipeline. Draws a
/// stream of NDC-space vertices, each carrying its own RGBA color.
/// Kept inline (rather than in the baumhard shader table) because
/// it's 100% renderer-local — no tree data, no camera uniforms; the
/// CPU bakes the camera transform into each vertex before upload.
const RECT_SHADER_WGSL: &str = r#"
struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(in.pos, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

/// Bytes-per-vertex for the rect pipeline: `vec2<f32> pos +
/// vec4<f32> color = 6 × f32 = 24 bytes`. Used when sizing /
/// offsetting the vertex buffer. Declared as a compile-time const so
/// the layout math is grep-able from a single place.
const RECT_VERTEX_SIZE: u64 = 24;

/// Starting capacity (in bytes) for the rect vertex buffer. Big
/// enough for a modest map with several hundred node backgrounds
/// without an immediate grow; doubling-on-overflow handles anything
/// larger. 8192 bytes = 341 vertices = ~56 rects. Deliberately small
/// since most maps will have a handful of colored nodes and the grow
/// path is exercised rarely.
const RECT_VBUF_INITIAL_CAPACITY: u64 = 8192;

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
    fps_clock: usize,

    camera: Camera2D,
    mindmap_buffers: FxHashMap<String, MindMapTextBuffer>,
    /// Per-node border glyph buffers, keyed by `node_id`. Each entry is a
    /// `Vec` of 4 buffers (top/bottom/left/right) matching the layout in
    /// `rebuild_border_buffers_keyed`. Keyed so unchanged borders survive
    /// across drag frames without re-shaping — Phase B of the
    /// connection-render cost work.
    border_buffers: FxHashMap<String, Vec<MindMapTextBuffer>>,
    /// Per-edge connection glyph buffers, keyed by `(from_id, to_id,
    /// edge_type)`. Each entry is the `Vec` of already-shaped glyph
    /// buffers for that edge. Keyed so unchanged edges survive across
    /// drag frames — the big win for the "long cross-link, dragging
    /// something else" scenario.
    connection_buffers: FxHashMap<EdgeKey, Vec<MindMapTextBuffer>>,
    /// Edge grab-handle buffers for Session 6C's connection reshape
    /// surface. Populated only when an edge is selected; rebuilt
    /// fresh every time the scene is rebuilt with a selected edge.
    /// Bounded cost (≤ 5 glyph buffers per selected edge) so no
    /// keyed cache is warranted.
    edge_handle_buffers: Vec<MindMapTextBuffer>,
    /// Session 6D: per-edge label buffers, keyed by `EdgeKey`. Each
    /// entry is the shaped cosmic-text buffer for that edge's label
    /// (if any). Labels are ≤ 1 per edge and rebuilt every scene
    /// build — no incremental-reuse cache is warranted.
    connection_label_buffers: FxHashMap<EdgeKey, MindMapTextBuffer>,
    /// Session 6D: AABB hitbox for each rendered label, keyed by
    /// `EdgeKey`. Populated alongside `connection_label_buffers`;
    /// consulted by `hit_test_edge_label` when the app dispatches
    /// inline click-to-edit. Stored as `(min, max)` canvas-space
    /// corners so the hit test is a pair of comparisons per edge.
    connection_label_hitboxes: FxHashMap<EdgeKey, (Vec2, Vec2)>,
    /// Session 6E: per-endpoint portal marker buffers, keyed by
    /// `(portal_ref, endpoint_node_id)` so each of the two marker
    /// glyphs of a pair is stored separately. Rebuilt every scene
    /// build from the `portal_elements` field of `RenderScene`.
    /// Portal counts stay in the dozens so a keyed cache is enough;
    /// no incremental rebuild path is warranted.
    portal_buffers: FxHashMap<(PortalRefKey, String), MindMapTextBuffer>,
    /// Session 6E: AABB hitbox for each rendered portal marker,
    /// keyed by `(portal_ref, endpoint_node_id)`. Populated alongside
    /// `portal_buffers`; consulted by `hit_test_portal` when the
    /// `handle_click` dispatcher needs to resolve a click on a
    /// portal glyph to a `PortalRefKey`.
    portal_hitboxes: FxHashMap<(PortalRefKey, String), (Vec2, Vec2)>,
    /// Session 6C: command palette overlay buffers. Rendered above
    /// everything else in screen coordinates. Populated only when
    /// the palette is open; cleared otherwise.
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

/// Canvas-space record of a filled rectangle drawn behind a node's
/// text. Captured from `GlyphArea.background_color` during the tree
/// walk in `rebuild_buffers_from_tree`; camera-transformed to NDC
/// in `render` each frame.
#[derive(Clone, Debug)]
pub(super) struct NodeBackgroundRect {
    pub position: Vec2,
    pub size: Vec2,
    pub color: [u8; 4],
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
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 8,
                            shader_location: 1,
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
            fps_clock: 0,
            glyphon_cache,
            viewport,
            camera,
            mindmap_buffers: Default::default(),
            border_buffers: FxHashMap::default(),
            connection_buffers: FxHashMap::default(),
            edge_handle_buffers: Vec::new(),
            connection_label_buffers: FxHashMap::default(),
            connection_label_hitboxes: FxHashMap::default(),
            portal_buffers: FxHashMap::default(),
            portal_hitboxes: FxHashMap::default(),
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

    /// Push the six vertices (two triangles) of a filled axis-aligned
    /// rectangle into `out`. Coords are already in NDC — the caller is
    /// responsible for any camera or screen→NDC transform. `color` is
    /// the flat RGBA written to every vertex.
    ///
    /// Layout: `[x_tl, y_tl, r, g, b, a, x_tr, y_tr, r, g, b, a, …]`
    /// (6 floats per vertex, 6 vertices per rect = 36 floats per rect).
    fn push_rect_ndc(
        out: &mut Vec<f32>,
        ndc_min: Vec2,
        ndc_max: Vec2,
        color: [f32; 4],
    ) {
        // Triangle 1: TL, BL, BR
        // Triangle 2: TL, BR, TR
        //
        // NDC y is UP, so "top" is the larger y. The caller computes
        // ndc_min / ndc_max from the canonical top-left + size by
        // flipping y during the screen-to-NDC transform, so here
        // ndc_min is bottom-left and ndc_max is top-right. Unpack:
        let (lx, ly) = (ndc_min.x, ndc_min.y); // bottom-left
        let (rx, ry) = (ndc_max.x, ndc_max.y); // top-right
        let [r, g, b, a] = color;
        // TL = (lx, ry), TR = (rx, ry), BR = (rx, ly), BL = (lx, ly)
        let push = |out: &mut Vec<f32>, x: f32, y: f32| {
            out.extend_from_slice(&[x, y, r, g, b, a]);
        };
        // Triangle 1: TL, BL, BR
        push(out, lx, ry);
        push(out, lx, ly);
        push(out, rx, ly);
        // Triangle 2: TL, BR, TR
        push(out, lx, ry);
        push(out, rx, ly);
        push(out, rx, ry);
    }

    /// Convert a screen-space rectangle (top-left + size in pixels)
    /// into a NDC bounding pair. Y is flipped so "top" (small y on
    /// screen) maps to "top" (large y in NDC).
    fn screen_rect_to_ndc_bounds(
        left: f32,
        top: f32,
        width: f32,
        height: f32,
        vp_w: f32,
        vp_h: f32,
    ) -> (Vec2, Vec2) {
        let x0 = left / vp_w * 2.0 - 1.0;
        let x1 = (left + width) / vp_w * 2.0 - 1.0;
        // Screen y grows down; NDC y grows up. Invert.
        let y_top = 1.0 - top / vp_h * 2.0;
        let y_bottom = 1.0 - (top + height) / vp_h * 2.0;
        // ndc_min = (x0, y_bottom), ndc_max = (x1, y_top)
        (Vec2::new(x0, y_bottom), Vec2::new(x1, y_top))
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

    #[inline]
    fn create_surface_config(
        texture_format: TextureFormat,
        surface_capabilities: &SurfaceCapabilities,
        surface_size: PhysicalSize<u32>,
    ) -> SurfaceConfiguration {
        SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: texture_format,
            width: surface_size.width,
            height: surface_size.height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: surface_capabilities.alpha_modes[0],
            view_formats: vec![],
        }
    }

    #[inline]
    fn create_render_pipeline(
        device: &Device,
        shader: &ShaderModule,
        pipeline_layout: &PipelineLayout,
        texture_format: TextureFormat,
    ) -> RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(texture_format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    #[inline]
    fn load_shaders(device: &Device, shaders: &mut FxHashMap<&'static str, ShaderModule>) {
        assert!(SHADERS.len() > 0, "No shaders defined!");
        for i in 0..SHADERS.len() {
            let (name, source) = SHADERS[i].clone();
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
            });
            shaders.insert(name, shader);
            debug!("Loaded a shader");
        }
    }

    #[inline]
    fn create_pipeline_layout(device: &Device) -> PipelineLayout {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[],
            immediate_size: 0,
        })
    }

    #[inline]
    async fn get_device(adapter: &Adapter) -> (Device, Queue) {
        adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults()
                        .using_resolution(adapter.limits()),
                    memory_hints: Default::default(),
                    trace: Default::default(),
                    experimental_features: Default::default(),
                },
            )
            .await
            .expect("Failed to create device")
    }

    #[inline]
    async fn get_adapter(instance: &Instance, surface: &Surface<'static>) -> Adapter {
        instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .expect("Failed to find an appropriate adapter")
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
                    if self.fps_clock % 100 == 0 {
                        self.calculate_fps(delta_duration);
                    }
                    self.fps_clock += 1;
                    let sw = StopWatch::new_start();
                    self.render();
                    self.last_render_time = sw.stop();
                }
            }
            RedrawMode::NoLimit => {
                if self.fps_clock % 100 == 0 {
                    self.calculate_no_limit_fps();
                }
                self.fps_clock += 1;
                let sw = StopWatch::new_start();
                self.render();
                self.last_render_time = sw.stop();
            }
        }
        self.run
    }

    #[inline]
    fn calculate_no_limit_fps(&mut self) {
        let micros = self.last_render_time.as_micros();
        if micros == 0 {
            return;
        }
        self.fps = Some((1_000_000u128 / micros) as usize);
    }

    #[inline]
    fn calculate_fps(&mut self, delta_time: Duration) {
        let frame_micros = (self.last_render_time
            + Duration::max(delta_time, Self::ZERO_DURATION.clone()))
        .as_micros();
        // Guard against divide-by-zero on the first frame when both
        // last_render_time and delta_time are zero.
        if frame_micros == 0 {
            return;
        }
        self.fps = Some((1_000_000u128 / frame_micros) as usize);
    }

    #[inline]
    fn get_size(&self) -> PhysicalSize<u32> {
        self.window.inner_size()
    }

    #[inline]
    fn render(&mut self) {
        if !self.should_render {
            return;
        }
        let vp_w_px = self.config.width as f32;
        let vp_h_px = self.config.height as f32;
        let vp_w = self.config.width as i32;
        let vp_h = self.config.height as i32;
        let vp_bounds = TextBounds { left: 0, top: 0, right: vp_w, bottom: vp_h };
        let default_color = cosmic_text::Color::rgba(255, 255, 255, 255);

        // Rebuild the "main" rect batch: canvas-space node
        // backgrounds transformed to NDC via the current camera.
        // Cheap: one push per visible node, no text shaping.
        // Visible-range test mirrors the text-area cull below so
        // clipped-offscreen nodes don't waste vertices either.
        self.main_rect_vertices.clear();
        for rect in self
            .node_background_rects
            .iter()
            .chain(self.canvas_scene_background_rects.iter())
        {
            if !self.camera.is_visible(rect.position, rect.size) {
                continue;
            }
            let screen_tl = self.camera.canvas_to_screen(rect.position);
            let screen_size = rect.size * self.camera.zoom;
            let (ndc_min, ndc_max) = Self::screen_rect_to_ndc_bounds(
                screen_tl.x, screen_tl.y,
                screen_size.x, screen_size.y,
                vp_w_px, vp_h_px,
            );
            let color = [
                rect.color[0] as f32 / 255.0,
                rect.color[1] as f32 / 255.0,
                rect.color[2] as f32 / 255.0,
                rect.color[3] as f32 / 255.0,
            ];
            Self::push_rect_ndc(&mut self.main_rect_vertices, ndc_min, ndc_max, color);
        }

        // Rebuild the "palette" rect batch: one opaque backdrop
        // behind the command palette and/or the glyph-wheel color
        // picker, in screen space. The two modals are mutually
        // exclusive but a single batch handles either case (or
        // both, if a future variant ever overlaps them).
        self.console_rect_vertices.clear();
        if let Some((left, top, w, h)) = self.console_backdrop {
            let (ndc_min, ndc_max) = Self::screen_rect_to_ndc_bounds(
                left, top, w, h, vp_w_px, vp_h_px,
            );
            // Pitch black. Sits cleanly against the cyan frame and
            // any canvas background without tinting the palette's
            // cyan foreground.
            let bg_color = [0.0, 0.0, 0.0, 1.0];
            Self::push_rect_ndc(
                &mut self.console_rect_vertices,
                ndc_min,
                ndc_max,
                bg_color,
            );
        }
        if let Some((left, top, w, h)) = self.color_picker_backdrop {
            let (ndc_min, ndc_max) = Self::screen_rect_to_ndc_bounds(
                left, top, w, h, vp_w_px, vp_h_px,
            );
            // Same pitch black as the palette — the picker's hue
            // ring glyphs and crosshair cells are saturated colors
            // that pop against true black with no tinting.
            let bg_color = [0.0, 0.0, 0.0, 1.0];
            Self::push_rect_ndc(
                &mut self.console_rect_vertices,
                ndc_min,
                ndc_max,
                bg_color,
            );
        }

        // Upload both batches to the shared rect vertex buffer,
        // growing if the combined size exceeds the current
        // capacity. Layout: `[main_bytes | palette_bytes]`.
        let main_bytes_len = self.main_rect_vertices.len() * std::mem::size_of::<f32>();
        let palette_bytes_len = self.console_rect_vertices.len() * std::mem::size_of::<f32>();
        let total_bytes = (main_bytes_len + palette_bytes_len) as u64;
        if total_bytes > self.rect_vertex_buffer_capacity {
            let mut new_cap = self.rect_vertex_buffer_capacity.max(RECT_VBUF_INITIAL_CAPACITY);
            while new_cap < total_bytes {
                new_cap *= 2;
            }
            self.rect_vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rect_vertex_buffer"),
                size: new_cap,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.rect_vertex_buffer_capacity = new_cap;
        }
        if main_bytes_len > 0 {
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    self.main_rect_vertices.as_ptr() as *const u8,
                    main_bytes_len,
                )
            };
            self.queue.write_buffer(&self.rect_vertex_buffer, 0, bytes);
        }
        if palette_bytes_len > 0 {
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    self.console_rect_vertices.as_ptr() as *const u8,
                    palette_bytes_len,
                )
            };
            self.queue.write_buffer(
                &self.rect_vertex_buffer,
                main_bytes_len as u64,
                bytes,
            );
        }
        let main_vertex_count = (self.main_rect_vertices.len() / 6) as u32;
        let palette_vertex_count = (self.console_rect_vertices.len() / 6) as u32;

        // Collect "main" text areas: the mindmap + borders +
        // connections + edge handles + overlays + arena buffers.
        // Palette buffers go into a separate list so they render
        // in a second glyphon pass (with the backdrop rect
        // between them, hence the split).
        let main_text_areas: Vec<TextArea> = self.mindmap_buffers.values()
            .chain(self.border_buffers.values().flat_map(|v| v.iter()))
            .chain(self.connection_buffers.values().flat_map(|v| v.iter()))
            .chain(self.connection_label_buffers.values())
            .chain(self.portal_buffers.values())
            .chain(self.edge_handle_buffers.iter())
            .chain(self.overlay_buffers.iter())
            .chain(self.canvas_scene_buffers.iter())
            .filter_map(|tb| {
                let canvas_pos = Vec2::new(tb.pos.0, tb.pos.1);
                let canvas_size = Vec2::new(tb.bounds.0, tb.bounds.1);
                if !self.camera.is_visible(canvas_pos, canvas_size) {
                    return None;
                }
                let screen_pos = self.camera.canvas_to_screen(canvas_pos);
                Some(TextArea {
                    buffer: &tb.buffer,
                    left: screen_pos.x,
                    top: screen_pos.y,
                    scale: self.camera.zoom,
                    bounds: vp_bounds,
                    default_color,
                    custom_glyphs: &[],
                })
            })
            .collect();


        // Palette overlay: screen-space text, drawn in its own
        // glyphon pass so the rect-pipeline backdrop can be
        // interleaved between the main text and this one. The
        // glyph-wheel color picker's glyph buffers flow through
        // `overlay_scene_buffers` (populated by
        // `rebuild_overlay_scene_buffers` from the picker's overlay
        // tree in `AppScene`) — it's a mutually exclusive
        // screen-space modal that shares this pass with the
        // console.
        let palette_text_areas: Vec<TextArea> = self.console_overlay_buffers.iter()
            .chain(self.overlay_scene_buffers.iter())
            .map(|tb| TextArea {
                buffer: &tb.buffer,
                left: tb.pos.0,
                top: tb.pos.1,
                scale: 1.0,
                bounds: vp_bounds,
                default_color,
                custom_glyphs: &[],
            })
            .collect();

        // Interactive path: a contended font-system lock must skip
        // the frame, not abort the process.
        let Ok(mut font_system) = fonts::FONT_SYSTEM.try_write() else {
            log::warn!("font_system lock contended in render(), skipping frame");
            return;
        };

        // Interactive path: a glyphon prepare failure must degrade the
        // frame, not abort the process. Skip the whole render so we
        // don't run a half-prepared atlas through the GPU.
        if let Err(e) = self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut font_system,
            &mut self.atlas,
            &self.viewport,
            main_text_areas,
            &mut self.swash_cache,
        ) {
            log::warn!("text_renderer.prepare failed, skipping frame: {e}");
            return;
        }
        if let Err(e) = self.console_text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut font_system,
            &mut self.atlas,
            &self.viewport,
            palette_text_areas,
            &mut self.swash_cache,
        ) {
            log::warn!("console_text_renderer.prepare failed, skipping frame: {e}");
            return;
        }
        drop(font_system);

        let Ok(frame) = self.surface.get_current_texture() else {
            debug!("Failed to get the surface texture, can't render.");
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // 1. Node backgrounds (rect pipeline, camera-transformed).
            if main_vertex_count > 0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_vertex_buffer(
                    0,
                    self.rect_vertex_buffer.slice(0..main_bytes_len as u64),
                );
                pass.draw(0..main_vertex_count, 0..1);
            }

            // 2. Main text pass — node text, borders, connections,
            //    edge handles, camera-transformed and screen-space
            //    overlays, all drawn on top of the node backgrounds.
            //    Interactive path: log and continue on render failure
            //    so a single bad atlas frame doesn't crash the editor.
            if let Err(e) =
                self.text_renderer.render(&self.atlas, &self.viewport, &mut pass)
            {
                log::warn!("text_renderer.render failed: {e}");
            }

            // 3. Palette backdrop (rect pipeline, screen-space).
            //    Drawn AFTER the main text pass so node text
            //    sitting behind the palette is fully occluded.
            if palette_vertex_count > 0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_vertex_buffer(
                    0,
                    self.rect_vertex_buffer.slice(
                        main_bytes_len as u64..(main_bytes_len + palette_bytes_len) as u64,
                    ),
                );
                pass.draw(0..palette_vertex_count, 0..1);
            }

            // 4. Palette text pass — cyan border, query line,
            //    filtered action rows. Drawn on top of the palette
            //    backdrop so every glyph sits cleanly on solid fill.
            //    Interactive path: log and continue on render failure.
            if let Err(e) = self
                .console_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
            {
                log::warn!("console_text_renderer.render failed: {e}");
            }
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        self.atlas.trim();
    }

    #[inline]
    fn create_transformation_matrix(rotx: f32, roty: f32, rotz: f32) -> [f32; 16] {
        let rotation = Quat::from_rotation_x(rotx)
            .mul_quat(Quat::from_rotation_y(roty))
            .mul_quat(Quat::from_rotation_z(rotz));

        let transform = Mat4::from_rotation_translation(rotation, Vec3::ZERO);
        transform.to_cols_array()
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
        info!("Updating surface size");
        self.config.width = width;
        self.config.height = height;

        self.surface.configure(&self.device, &self.config);
        self.viewport.update(&self.queue, Resolution { width, height });
        self.camera.set_viewport_size(width, height);
        // Viewport changed → Phase A's off-screen glyph cull needs to
        // re-run for every edge. Drop the keyed connection buffer cache
        // so the next rebuild rebuilds from a clean slate, and raise
        // the viewport-dirty flag so the event loop actually triggers
        // that rebuild.
        self.connection_buffers.clear();
        self.connection_viewport_dirty = true;
    }


    /// Fit the camera to show a RenderScene's content.
    pub fn fit_camera_to_scene(&mut self, scene: &RenderScene) {
        if scene.text_elements.is_empty() {
            return;
        }
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for elem in &scene.text_elements {
            let (x, y) = elem.position;
            let (w, h) = elem.size;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + w);
            max_y = max_y.max(y + h);
        }
        self.camera.apply_mutation(
            &baumhard::gfx_structs::camera::CameraMutation::FitToBounds {
                min: Vec2::new(min_x, min_y),
                max: Vec2::new(max_x, max_y),
                padding_fraction: 0.05,
            },
        );
    }

    /// Rebuild text buffers from a Baumhard tree (nodes rendered from GlyphArea
    /// elements). This is the primary text-rendering path; borders and
    /// connections use their own `rebuild_*_buffers` methods alongside it.
    pub fn rebuild_buffers_from_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        self.mindmap_buffers.clear();
        // Session 6C follow-up: node backgrounds live on GlyphArea
        // and are collected fresh alongside the text buffers. The
        // render pipeline reads them back out each frame to draw
        // solid fills behind the text, with the camera transform
        // baked in at the last moment. Clearing here (rather than
        // on every render call) keeps the collect cost aligned
        // with the tree rebuild cadence — i.e. only when something
        // structural changed.
        self.node_background_rects.clear();
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");
        walk_tree_into_buffers(
            tree,
            Vec2::ZERO,
            &mut font_system,
            |unique_id, buffer| {
                // Mindmap is the only buffer store that needs
                // string keys (its `FxHashMap<String, _>` is shared
                // with the legacy edit / undo paths). Stringifying
                // here keeps the allocation off the helper's
                // critical path so overlay / canvas-scene callers
                // never pay it.
                self.mindmap_buffers.insert(unique_id.to_string(), buffer);
            },
            |rect| self.node_background_rects.push(rect),
        );
    }

    /// Rebuild the screen-space buffer list for every tree the app
    /// has registered into [`crate::application::scene_host::AppScene`].
    /// Walks the scene in layer
    /// order and produces one flat list; callers do not need to
    /// know about individual overlays. The renderer composites the
    /// result into the palette pass alongside the per-overlay
    /// buffer stores that predate this refactor — once every
    /// overlay has migrated to a tree, those per-overlay stores go
    /// away (see Session 5 in the unified-rendering plan).
    ///
    /// # Costs
    ///
    /// O(sum of descendants) across every tree in the scene.
    /// Allocates a `cosmic_text::Buffer` per `GlyphArea` with
    /// non-empty text. Empty scenes short-circuit cheaply.
    pub fn rebuild_overlay_scene_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        self.overlay_scene_buffers.clear();
        let ids = app_scene.overlay_ids_in_layer_order();
        if ids.is_empty() {
            return;
        }
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");
        for id in ids {
            let Some(entry) = app_scene.overlay_scene().get(id) else {
                continue;
            };
            if !entry.visible() {
                continue;
            }
            walk_tree_into_buffers(
                entry.tree(),
                entry.offset(),
                &mut font_system,
                |_unique_id, buffer| {
                    self.overlay_scene_buffers.push(buffer);
                },
                |_rect| {
                    // Overlay-tree background fills aren't wired to
                    // a screen-space rect pipeline yet. When
                    // Sessions 3 / 4 need them they can add a
                    // dedicated `overlay_scene_background_rects`
                    // field and a screen-space draw pass.
                },
            );
        }
    }

    /// Rebuild the canvas-space buffer list for every tree the app
    /// has registered into
    /// [`crate::application::scene_host::AppScene`]'s canvas sub-scene
    /// (borders, connections, portals, edge handles, connection
    /// labels — whichever have migrated). These buffers feed the
    /// camera-transformed main pass alongside the mindmap's own
    /// buffer map.
    ///
    /// # Costs
    ///
    /// O(sum of descendants) across every canvas tree. Allocates a
    /// `cosmic_text::Buffer` per non-empty `GlyphArea`. Empty
    /// sub-scenes short-circuit cheaply.
    pub fn rebuild_canvas_scene_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        self.canvas_scene_buffers.clear();
        self.canvas_scene_background_rects.clear();
        let ids = app_scene.canvas_ids_in_layer_order();
        if ids.is_empty() {
            return;
        }
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");
        for id in ids {
            let Some(entry) = app_scene.canvas_scene().get(id) else {
                continue;
            };
            if !entry.visible() {
                continue;
            }
            walk_tree_into_buffers(
                entry.tree(),
                entry.offset(),
                &mut font_system,
                |_unique_id, buffer| {
                    self.canvas_scene_buffers.push(buffer);
                },
                |rect| {
                    self.canvas_scene_background_rects.push(rect);
                },
            );
        }
    }

    /// Rebuild border buffers from flat border elements (from RenderScene).
    ///
    /// Borders are a rectangle of box-drawing glyphs: the top and bottom
    /// edges are horizontal text runs, the left and right edges are columns
    /// of single-character lines.
    ///
    /// Two subtleties make this tricky:
    ///
    /// - The top/bottom runs share the same `approx_char_width`
    ///   approximation as the right column's x anchor — otherwise the right
    ///   column drifts away from the top-right corner as the node gets
    ///   wider (fixed in an earlier pass).
    /// - cosmic-text renders each glyph inside its line box with the
    ///   font's own ascent/descent, which is typically ~80% of the line
    ///   height for box-drawing characters in LiberationSans. So the `╮`
    ///   at the bottom of the top border's line box does NOT quite reach
    ///   the top of the right column's first `│` glyph. We close the gap
    ///   by overlapping the top/bottom runs' line boxes into the vertical
    ///   column's extent by `CORNER_OVERLAP_FRAC * font_size` on each side.
    /// Full (non-keyed) border rebuild — wipes the keyed cache and rebuilds
    /// every element from scratch. Used on map load, undo, reparent,
    /// selection change, and anywhere else the caller already knows every
    /// border may have changed.
    pub fn rebuild_border_buffers(&mut self, border_elements: &[BorderElement]) {
        self.border_buffers.clear();
        self.rebuild_border_buffers_keyed(border_elements, None);
    }

    /// Keyed border rebuild. If `dirty_node_ids` is `Some`, only entries
    /// whose `node_id` is in the set are re-shaped from scratch; clean
    /// entries have only their position patched in place on the existing
    /// cached buffers. Keys not present in `border_elements` are evicted
    /// at the end of the call. If `dirty_node_ids` is `None`, everything
    /// is treated as dirty (full re-shape).
    ///
    /// This is Phase B of the "Connection & border render cost" fix: on
    /// a drag frame that moves one node, only that node's border cache
    /// entry is re-shaped. All other visible borders reuse their shaped
    /// `cosmic_text::Buffer`s — cosmic-text shaping is the dominant cost
    /// here, so skipping it for unmoved borders is the point.
    pub fn rebuild_border_buffers_keyed(
        &mut self,
        border_elements: &[BorderElement],
        dirty_node_ids: Option<&std::collections::HashSet<String>>,
    ) {
        // Layout constants live on `baumhard::mindmap::border` so
        // this path and `tree_builder::build_border_tree` can't
        // drift on geometry. See the doc on the constants for the
        // empirical rationale.
        use baumhard::mindmap::border::{
            BORDER_APPROX_CHAR_WIDTH_FRAC, BORDER_CORNER_OVERLAP_FRAC,
        };

        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        let mut seen: std::collections::HashSet<String> =
            std::collections::HashSet::with_capacity(border_elements.len());

        for elem in border_elements {
            seen.insert(elem.node_id.clone());
            let is_dirty = dirty_node_ids
                .map(|set| set.contains(&elem.node_id))
                .unwrap_or(true);

            let font_size = elem.border_style.font_size_pt;
            let (nx, ny) = elem.node_position;
            let (nw, nh) = elem.node_size;

            // --- Horizontal math (identical to the classic path) ---
            let approx_char_width = font_size * BORDER_APPROX_CHAR_WIDTH_FRAC;
            let char_count = ((nw / approx_char_width) + 2.0)
                .ceil()
                .max(3.0) as usize;
            let right_corner_x =
                nx - approx_char_width + (char_count - 1) as f32 * approx_char_width;
            let corner_overlap = font_size * BORDER_CORNER_OVERLAP_FRAC;
            let top_y = ny - font_size + corner_overlap;
            let bottom_y = ny + nh - corner_overlap;

            // Fast path: cached, clean, matching glyph count. Patch positions
            // in place and skip re-shaping. We require the char_count to
            // match the cached entry's top run bounds so any node-resize
            // edits (rare outside drag) still force a re-shape.
            if !is_dirty {
                if let Some(existing) = self.border_buffers.get_mut(&elem.node_id) {
                    if existing.len() == 4 {
                        // Sanity: the top run's `bounds.0` is char_count-
                        // dependent. If it has diverged (e.g. the node was
                        // resized), re-shape.
                        let expected_h_width = (char_count as f32 + 1.0) * approx_char_width;
                        if (existing[0].bounds.0 - expected_h_width).abs() < 0.5 {
                            existing[0].pos = (nx - approx_char_width, top_y);
                            existing[1].pos = (nx - approx_char_width, bottom_y);
                            existing[2].pos = (nx - approx_char_width, ny);
                            existing[3].pos = (right_corner_x, ny);
                            continue;
                        }
                    }
                }
            }

            // Slow path: shape fresh. Compute the strings, allocate 4
            // buffers, insert into the cache (replacing any previous
            // entry).
            let border_color = parse_hex_color(&elem.border_style.color)
                .unwrap_or(cosmic_text::Color::rgba(255, 255, 255, 255));
            let glyph_set = &elem.border_style.glyph_set;
            let border_attrs = Attrs::new()
                .color(border_color)
                .metrics(cosmic_text::Metrics::new(font_size, font_size));

            let h_width = (char_count as f32 + 1.0) * approx_char_width;
            let v_width = approx_char_width * 2.0;

            let row_count = (nh / font_size).round().max(1.0) as usize;

            let top_text = glyph_set.top_border(char_count);
            let bottom_text = glyph_set.bottom_border(char_count);
            let left_text: String =
                std::iter::repeat_n(format!("{}\n", glyph_set.left_char()), row_count).collect();
            let right_text: String =
                std::iter::repeat_n(format!("{}\n", glyph_set.right_char()), row_count).collect();

            let entry = vec![
                create_border_buffer(
                    &mut font_system, &top_text, &border_attrs, font_size,
                    (nx - approx_char_width, top_y),
                    (h_width, font_size * 1.5),
                ),
                create_border_buffer(
                    &mut font_system, &bottom_text, &border_attrs, font_size,
                    (nx - approx_char_width, bottom_y),
                    (h_width, font_size * 1.5),
                ),
                create_border_buffer(
                    &mut font_system, &left_text, &border_attrs, font_size,
                    (nx - approx_char_width, ny),
                    (v_width, nh),
                ),
                create_border_buffer(
                    &mut font_system, &right_text, &border_attrs, font_size,
                    (right_corner_x, ny),
                    (v_width, nh),
                ),
            ];
            self.border_buffers.insert(elem.node_id.clone(), entry);
        }

        // Evict any cached entries whose node_id is no longer in the scene
        // (fold toggle, delete, show_frame = false, etc.).
        self.border_buffers.retain(|k, _| seen.contains(k));
    }

    /// Rebuild connection buffers from flat connection elements (from RenderScene).
    ///
    /// Per-glyph viewport culling is applied here. For each connection element,
    /// the visible canvas rect (expanded by `font_size` on each side as a
    /// margin) is tested against every glyph position; glyphs outside are
    /// skipped without creating a buffer. This is Phase 4(A) of the
    /// connection-render cost work: the dominant per-frame cost in this
    /// function is cosmic-text shaping, and a long cross-link during drag
    /// has thousands of sample positions the vast majority of which are
    /// off-screen. Skipping their buffer creation avoids the shaping cost
    /// entirely. The existing downstream cull in `render()` (line ~396)
    /// was only saving the rasterization of already-shaped buffers; the
    /// shaping had already happened.
    /// Full (non-keyed) connection rebuild — wipes the keyed cache and
    /// rebuilds every element from scratch. Used on map load, undo,
    /// reparent, edge CRUD, and anywhere else the caller already knows
    /// every connection may have changed.
    pub fn rebuild_connection_buffers(&mut self, connection_elements: &[ConnectionElement]) {
        self.connection_buffers.clear();
        self.rebuild_connection_buffers_keyed(connection_elements, None);
    }

    /// Rebuild the edge grab-handle overlay buffers. Called after every
    /// scene build — the handles are bounded (≤ 5 per selected edge)
    /// and always rebuilt from scratch, so no keyed cache is used.
    /// When `handles` is empty (nothing selected or selection is a
    /// node/None) this clears the buffer list and returns.
    pub fn rebuild_edge_handle_buffers(
        &mut self,
        handles: &[baumhard::mindmap::scene_builder::EdgeHandleElement],
    ) {
        self.edge_handle_buffers.clear();
        if handles.is_empty() {
            return;
        }
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");
        for handle in handles {
            let cosmic_color = parse_hex_color(&handle.color)
                .unwrap_or(cosmic_text::Color::rgba(0, 229, 255, 255));
            let attrs = Attrs::new()
                .color(cosmic_color)
                .metrics(cosmic_text::Metrics::new(handle.font_size_pt, handle.font_size_pt));

            // Center the glyph on the handle position. `approx_char_width`
            // keeps the math consistent with the connection glyph layout.
            let half_w = handle.font_size_pt * 0.3;
            let half_h = handle.font_size_pt * 0.5;
            let pos = (handle.position.0 - half_w, handle.position.1 - half_h);
            let bounds = (handle.font_size_pt, handle.font_size_pt);

            self.edge_handle_buffers.push(create_border_buffer(
                &mut font_system,
                &handle.glyph,
                &attrs,
                handle.font_size_pt,
                pos,
                bounds,
            ));
        }
    }

    /// Rebuild the console overlay buffers. When `geometry` is
    /// `None`, the console is closed — clear the buffer list and the
    /// backdrop, and return. When `Some`, lay out a bottom-anchored
    /// glyph-rendered strip: sacred border, scrollback region,
    /// optional completion popup, and the prompt line with cursor.
    ///
    /// Everything is positioned in screen coordinates (the render
    /// pass draws `console_overlay_buffers` with `scale = 1.0`), so
    /// the console stays a fixed size regardless of canvas zoom.
    pub fn rebuild_console_overlay_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: Option<&ConsoleOverlayGeometry>,
    ) {
        use crate::application::scene_host::{OverlayDispatch, OverlayRole};

        let Some(geometry) = geometry else {
            // Closed: drop the backdrop, drop the tree, refresh
            // overlay buffers so the console disappears. The
            // structural-signature cache lives on `AppScene` and
            // is cleared inside `unregister_overlay`, so the next
            // reopen starts from a clean slate.
            self.console_backdrop = None;
            app_scene.unregister_overlay(OverlayRole::Console);
            self.rebuild_overlay_scene_buffers(app_scene);
            return;
        };

        let layout = compute_console_frame_layout(
            geometry,
            self.config.width as f32,
            self.config.height as f32,
        );
        self.console_backdrop = Some(layout.backdrop_rect());
        let signature = console_overlay_signature(&layout);

        // §B2 dispatch: if the structural signature
        // (`scrollback_rows` × `completion_rows`) hasn't changed
        // since the last build, the existing tree's slot count
        // still matches and we apply an in-place mutator that
        // overwrites every slot's variable fields. Window resize
        // is the only typical event that shifts the signature, so
        // the mutator path covers every keystroke / scrollback-
        // grow / completion-update / Tab-cycle frame.
        match app_scene.overlay_dispatch(OverlayRole::Console, signature) {
            OverlayDispatch::InPlaceMutator => {
                let mutator = {
                    let mut font_system = fonts::FONT_SYSTEM
                        .write()
                        .expect("Failed to acquire font_system lock");
                    build_console_overlay_mutator(geometry, &layout, &mut font_system)
                };
                app_scene.apply_overlay_mutator(OverlayRole::Console, &mutator);
            }
            OverlayDispatch::FullRebuild => {
                // Build the tree under the FONT_SYSTEM lock — we
                // need it for `measure_max_glyph_advance` only.
                // Tree construction itself doesn't shape; that
                // happens during the overlay-scene walk below.
                let tree = {
                    let mut font_system = fonts::FONT_SYSTEM
                        .write()
                        .expect("Failed to acquire font_system lock");
                    build_console_overlay_tree(geometry, &layout, &mut font_system)
                };
                app_scene.register_overlay(OverlayRole::Console, tree, glam::Vec2::ZERO);
                app_scene.set_overlay_signature(OverlayRole::Console, signature);
            }
        }
        self.rebuild_overlay_scene_buffers(app_scene);
    }


    /// Build the picker's overlay tree from `geometry`, register
    /// it under [`OverlayRole::ColorPicker`](crate::application::scene_host::OverlayRole),
    /// and walk the overlay sub-scene into
    /// `overlay_scene_buffers`. `None` means the picker is closed
    /// — drops the backdrop, unregisters the tree, refreshes
    /// overlay buffers so it disappears.
    ///
    /// Called by `open_color_picker`, the `Resized` handler, and
    /// the hover / chip-focus / commit / cancel paths in
    /// `app::rebuild_color_picker_overlay`.
    ///
    /// **Performance note**: every invocation re-shapes every
    /// glyph in the picker (~64 cells). The legacy split that
    /// skipped re-shaping the static hue ring on hover is gone;
    /// the planned `MutatorTree`-based hover path will mutate
    /// only changed cell colors and the indicator's position
    /// per §B1 of `lib/baumhard/CONVENTIONS.md`.
    pub fn rebuild_color_picker_overlay_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry_and_layout: Option<(
            &crate::application::color_picker::ColorPickerOverlayGeometry,
            &crate::application::color_picker::ColorPickerLayout,
        )>,
    ) {
        self.color_picker_backdrop =
            color_picker::prepare_overlay_for_rebuild(app_scene, geometry_and_layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }

    /// §B2 mutation path — apply the **layout-phase** delta to the
    /// picker overlay tree without rebuilding the arena. Pairs with
    /// [`crate::application::color_picker_overlay::build_mutator`]:
    /// every variable field on every picker GlyphArea is overwritten
    /// via an `Assign` `DeltaGlyphArea` keyed by stable channel.
    ///
    /// Use this only when something the layout depends on actually
    /// changed (viewport resize, RMB size_scale drag, drag-move
    /// repositioning the wheel). Per-frame hover/HSV/chip updates
    /// should call [`Self::apply_color_picker_overlay_dynamic_mutator`]
    /// instead — same arena, slimmer per-cell delta. Open / close
    /// still use [`Self::rebuild_color_picker_overlay_buffers`]
    /// because the arena needs to be created or torn down. Calls
    /// `rebuild_overlay_scene_buffers` afterward to refresh the
    /// shaped buffers — the cosmic-text shape pass is still per-
    /// element, which is the §B1 perf gap tracked in `ROADMAP.md`.
    pub fn apply_color_picker_overlay_mutator(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
        layout: &crate::application::color_picker::ColorPickerLayout,
    ) {
        color_picker::apply_layout_mutator(app_scene, geometry, layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }

    /// §B2 mutation path — apply the **dynamic-phase** delta to the
    /// picker overlay tree. Pairs with
    /// [`crate::application::color_picker_overlay::build_dynamic_mutator`]:
    /// only the per-frame fields (color regions, hover scale, hex
    /// text) are written; layout-phase fields stay as the previous
    /// layout-mutator wrote them.
    ///
    /// This is the per-frame hot path for hover / HSV / chip-focus
    /// updates — the picker's element set, position, and bounds are
    /// unchanged. Calls `rebuild_overlay_scene_buffers` afterward to
    /// refresh the shaped buffers.
    pub fn apply_color_picker_overlay_dynamic_mutator(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
        layout: &crate::application::color_picker::ColorPickerLayout,
    ) {
        color_picker::apply_dynamic_mutator(app_scene, geometry, layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }


    /// Keyed connection rebuild. See [`Self::rebuild_border_buffers_keyed`] for
    /// the general pattern.
    ///
    /// If `dirty_edge_keys` is `Some`, clean edges (those whose
    /// `edge_key` is not in the set AND whose cached glyph count matches
    /// the current element's) only have their glyph *positions* patched
    /// in place — no cosmic-text shaping. Dirty edges are fully re-shaped.
    /// When `dirty_edge_keys` is `None`, everything is treated as dirty.
    ///
    /// **Interaction with Phase A viewport culling**: the off-screen glyph
    /// cull is a function of the camera, not the edge geometry, so on a
    /// camera pan the set of visible glyphs for a stable edge can change.
    /// The caller is responsible for clearing `self.connection_buffers`
    /// when the camera moves (see `process_decree` / `update_surface_size`)
    /// so that the dirty-set mechanism starts from a clean slate post-pan.
    /// Per-element the cull still runs here; clean fast-path kicks in
    /// only when the resulting visible-glyph count matches the cached
    /// entry's length.
    pub fn rebuild_connection_buffers_keyed(
        &mut self,
        connection_elements: &[ConnectionElement],
        dirty_edge_keys: Option<&std::collections::HashSet<EdgeKey>>,
    ) {
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        // Compute the visible canvas-space rectangle once.
        let vp_w = self.config.width as f32;
        let vp_h = self.config.height as f32;
        let corner_tl = self.camera.screen_to_canvas(Vec2::new(0.0, 0.0));
        let corner_br = self.camera.screen_to_canvas(Vec2::new(vp_w, vp_h));
        let vp_min = corner_tl.min(corner_br);
        let vp_max = corner_tl.max(corner_br);

        let mut seen: std::collections::HashSet<EdgeKey> =
            std::collections::HashSet::with_capacity(connection_elements.len());

        for elem in connection_elements {
            seen.insert(elem.edge_key.clone());
            let is_dirty = dirty_edge_keys
                .map(|set| set.contains(&elem.edge_key))
                .unwrap_or(true);

            let font_size = elem.font_size_pt;
            let half_glyph = font_size * 0.3;
            let half_height = font_size * 0.5;
            let glyph_bounds = (font_size, font_size);

            // Collect the positions that survive the viewport cull, in
            // order: cap_start, body glyphs, cap_end. Both the fast and
            // slow paths walk this same set.
            let in_view = |x: f32, y: f32| -> bool {
                glyph_position_in_viewport(x, y, vp_min, vp_max, font_size)
            };
            let mut visible_positions: Vec<(f32, f32)> =
                Vec::with_capacity(elem.glyph_positions.len() + 2);
            if let Some((_, cap_pos)) = &elem.cap_start {
                if in_view(cap_pos.0, cap_pos.1) {
                    visible_positions.push((cap_pos.0 - half_glyph, cap_pos.1 - half_height));
                }
            }
            for &pos in &elem.glyph_positions {
                if in_view(pos.0, pos.1) {
                    visible_positions.push((pos.0 - half_glyph, pos.1 - half_height));
                }
            }
            if let Some((_, cap_pos)) = &elem.cap_end {
                if in_view(cap_pos.0, cap_pos.1) {
                    visible_positions.push((cap_pos.0 - half_glyph, cap_pos.1 - half_height));
                }
            }

            if visible_positions.is_empty() {
                // Nothing visible — keep any existing cache entry? No, we
                // need to drop it so the buffer map doesn't hold stale
                // glyphs that would re-appear if `dirty_edge_keys`
                // bypassed this edge on the next frame. Just remove it.
                self.connection_buffers.remove(&elem.edge_key);
                continue;
            }

            // Fast path: clean + cached + same glyph count → patch
            // positions in place without re-shaping.
            if !is_dirty {
                if let Some(existing) = self.connection_buffers.get_mut(&elem.edge_key) {
                    if existing.len() == visible_positions.len() {
                        for (buf, new_pos) in existing.iter_mut().zip(visible_positions.iter()) {
                            buf.pos = *new_pos;
                        }
                        continue;
                    }
                }
            }

            // Slow path: re-shape. Build attrs once per element and emit
            // a fresh `Vec` of shaped buffers matching the same order as
            // `visible_positions`.
            let conn_color = parse_hex_color(&elem.color)
                .unwrap_or(cosmic_text::Color::rgba(200, 200, 200, 255));
            let conn_attrs = Attrs::new()
                .color(conn_color)
                .metrics(cosmic_text::Metrics::new(font_size, font_size));

            let mut new_entry: Vec<MindMapTextBuffer> =
                Vec::with_capacity(visible_positions.len());

            let cap_start_visible = elem
                .cap_start
                .as_ref()
                .map(|(_, p)| in_view(p.0, p.1))
                .unwrap_or(false);
            let cap_end_visible = elem
                .cap_end
                .as_ref()
                .map(|(_, p)| in_view(p.0, p.1))
                .unwrap_or(false);

            let mut idx = 0;
            if cap_start_visible {
                let cap_text = elem.cap_start.as_ref().map(|(t, _)| t.as_str()).unwrap_or("");
                new_entry.push(create_border_buffer(
                    &mut font_system, cap_text, &conn_attrs, font_size,
                    visible_positions[idx],
                    glyph_bounds,
                ));
                idx += 1;
            }
            for &pos in &elem.glyph_positions {
                if !in_view(pos.0, pos.1) {
                    continue;
                }
                new_entry.push(create_border_buffer(
                    &mut font_system, &elem.body_glyph, &conn_attrs, font_size,
                    visible_positions[idx],
                    glyph_bounds,
                ));
                idx += 1;
            }
            if cap_end_visible {
                let cap_text = elem.cap_end.as_ref().map(|(t, _)| t.as_str()).unwrap_or("");
                new_entry.push(create_border_buffer(
                    &mut font_system, cap_text, &conn_attrs, font_size,
                    visible_positions[idx],
                    glyph_bounds,
                ));
            }

            self.connection_buffers.insert(elem.edge_key.clone(), new_entry);
        }

        // Evict any cached entries whose edge key is no longer in the
        // scene — handles edge deletion / fold toggle.
        self.connection_buffers.retain(|k, _| seen.contains(k));
    }

    /// Session 6D: rebuild the per-edge label buffers from a freshly
    /// computed scene. Labels are rendered as individual cosmic-text
    /// buffers centered on their AABB, with a hitbox recorded so the
    /// app can detect clicks for inline label editing.
    ///
    /// The inline label-edit preview (buffer text + caret) is applied
    /// upstream in `scene_builder` via `MindMapDocument::label_edit_preview`,
    /// so the renderer can treat every label element as the final
    /// text to draw — no read-time override, no side channel.
    pub fn rebuild_connection_label_buffers(
        &mut self,
        label_elements: &[baumhard::mindmap::scene_builder::ConnectionLabelElement],
    ) {
        self.connection_label_buffers.clear();
        self.connection_label_hitboxes.clear();
        if label_elements.is_empty() {
            return;
        }
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        for elem in label_elements {
            let cosmic_color = parse_hex_color(&elem.color)
                .unwrap_or(cosmic_text::Color::rgba(235, 235, 235, 255));
            let attrs = Attrs::new()
                .color(cosmic_color)
                .metrics(cosmic_text::Metrics::new(elem.font_size_pt, elem.font_size_pt));

            let buffer = create_border_buffer(
                &mut font_system,
                &elem.text,
                &attrs,
                elem.font_size_pt,
                elem.position,
                elem.bounds,
            );
            self.connection_label_buffers
                .insert(elem.edge_key.clone(), buffer);

            let min = Vec2::new(elem.position.0, elem.position.1);
            let max = Vec2::new(
                elem.position.0 + elem.bounds.0,
                elem.position.1 + elem.bounds.1,
            );
            self.connection_label_hitboxes
                .insert(elem.edge_key.clone(), (min, max));
        }
    }

    /// Session 6D: AABB hit test against the rendered label hitboxes.
    /// Returns true when `canvas_pos` falls inside the hitbox of the
    /// given edge's label. Used by the app to dispatch inline
    /// click-to-edit when a selected edge's label is clicked.
    pub fn hit_test_edge_label(
        &self,
        canvas_pos: Vec2,
        edge_key: &EdgeKey,
    ) -> bool {
        if let Some((min, max)) = self.connection_label_hitboxes.get(edge_key) {
            canvas_pos.x >= min.x
                && canvas_pos.x <= max.x
                && canvas_pos.y >= min.y
                && canvas_pos.y <= max.y
        } else {
            false
        }
    }

    /// Replace the connection-label hitbox map wholesale.
    /// Used by `update_connection_label_tree` once labels render
    /// through the canvas-scene tree path; the tree builder owns
    /// the AABB computation and hands the map over via this
    /// setter so the legacy `hit_test_edge_label` keeps working.
    /// Will go away in Session 5.
    pub fn set_connection_label_hitboxes(
        &mut self,
        hitboxes: std::collections::HashMap<EdgeKey, (Vec2, Vec2)>,
    ) {
        self.connection_label_hitboxes.clear();
        for (k, v) in hitboxes {
            self.connection_label_hitboxes.insert(k, v);
        }
    }

    /// Replace the portal-hitbox map wholesale.
    ///
    /// Used by the `update_portal_tree` helper in `app.rs` once
    /// portal rendering migrated to the canvas-scene tree path:
    /// the tree builder owns geometry computation and emits both
    /// the tree and the AABBs, then hands the AABBs over via
    /// this setter so [`Self::hit_test_portal`] keeps working.
    /// Will go away in Session 5 when portal hit-testing routes
    /// through `Scene::component_at`.
    pub fn set_portal_hitboxes(
        &mut self,
        hitboxes: std::collections::HashMap<
            (
                (String, String, String),
                String,
            ),
            (Vec2, Vec2),
        >,
    ) {
        self.portal_hitboxes.clear();
        for (((label, endpoint_a, endpoint_b), endpoint_node_id), bbox) in hitboxes {
            let key = (
                PortalRefKey::new(label, endpoint_a, endpoint_b),
                endpoint_node_id,
            );
            self.portal_hitboxes.insert(key, bbox);
        }
    }

    /// Session 6E: hit-test portal markers at `canvas_pos`. Returns
    /// the `PortalRefKey` of the first marker whose AABB contains
    /// the point, or `None` if no marker is hit.
    ///
    /// Linear scan — portal counts stay in the dozens so a spatial
    /// index is not worth the maintenance cost. Consulted from
    /// `handle_click` as an alternate selection path, routed in
    /// before the edge hit test so clicks on a marker floating above
    /// a node's top-right corner don't accidentally fall through to
    /// an edge beneath.
    pub fn hit_test_portal(&self, canvas_pos: Vec2) -> Option<PortalRefKey> {
        for ((key, _endpoint), (min, max)) in &self.portal_hitboxes {
            if canvas_pos.x >= min.x
                && canvas_pos.x <= max.x
                && canvas_pos.y >= min.y
                && canvas_pos.y <= max.y
            {
                return Some(key.clone());
            }
        }
        None
    }

    /// Fit the camera to show a Baumhard tree's content.
    pub fn fit_camera_to_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        let mut found_any = false;

        for descendant_id in tree.root().descendants(&tree.arena) {
            let element = match tree.arena.get(descendant_id) {
                Some(n) => n.get(),
                None => continue,
            };
            let area = match element.glyph_area() {
                Some(a) => a,
                None => continue,
            };
            let x = area.position.x.0;
            let y = area.position.y.0;
            let w = area.render_bounds.x.0;
            let h = area.render_bounds.y.0;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + w);
            max_y = max_y.max(y + h);
            found_any = true;
        }
        if found_any {
            self.camera.apply_mutation(
                &baumhard::gfx_structs::camera::CameraMutation::FitToBounds {
                    min: Vec2::new(min_x, min_y),
                    max: Vec2::new(max_x, max_y),
                    padding_fraction: 0.05,
                },
            );
            // The fit typically changes both pan and zoom. Today this
            // is only called from `load_mindmap`, which follows up
            // with a full connection rebuild against the new zoom —
            // but raise both dirty flags so any future caller (e.g. a
            // "fit to selection" command) automatically gets a
            // rebuild on the next frame instead of silently leaving
            // stale buffers behind.
            self.connection_buffers.clear();
            self.connection_viewport_dirty = true;
            self.connection_geometry_dirty = true;
        }
    }

    /// Build overlay buffers for a selection rectangle using dashed box-drawing glyphs.
    /// Coordinates are in canvas space.
    pub fn rebuild_selection_rect_overlay(&mut self, min: Vec2, max: Vec2) {
        self.overlay_buffers.clear();
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        let font_size: f32 = 14.0;
        let approx_char_width = font_size * 0.6;
        let rect_color = cosmic_text::Color::rgba(0, 230, 255, 200); // Cyan, slightly transparent
        let attrs = Attrs::new()
            .color(rect_color)
            .metrics(cosmic_text::Metrics::new(font_size, font_size));

        let w = max.x - min.x;
        let h = max.y - min.y;
        let h_width = w + approx_char_width * 2.0;
        let v_width = approx_char_width * 2.0;

        // Top border
        let char_count = (w / approx_char_width).max(1.0) as usize;
        let top_text = format!("\u{256D}{}\u{256E}", "\u{2504}".repeat(char_count)); // ╭┄╮
        self.overlay_buffers.push(create_border_buffer(
            &mut font_system, &top_text, &attrs, font_size,
            (min.x - approx_char_width, min.y - font_size),
            (h_width, font_size * 1.5),
        ));

        // Bottom border
        let bottom_text = format!("\u{2570}{}\u{256F}", "\u{2504}".repeat(char_count)); // ╰┄╯
        self.overlay_buffers.push(create_border_buffer(
            &mut font_system, &bottom_text, &attrs, font_size,
            (min.x - approx_char_width, max.y),
            (h_width, font_size * 1.5),
        ));

        // Left border
        let row_count = (h / font_size).max(1.0) as usize;
        let left_text: String = std::iter::repeat_n("\u{2506}\n", row_count).collect(); // ┆
        self.overlay_buffers.push(create_border_buffer(
            &mut font_system, &left_text, &attrs, font_size,
            (min.x - approx_char_width, min.y),
            (v_width, h),
        ));

        // Right border
        let right_text: String = std::iter::repeat_n("\u{2506}\n", row_count).collect(); // ┆
        self.overlay_buffers.push(create_border_buffer(
            &mut font_system, &right_text, &attrs, font_size,
            (max.x, min.y),
            (v_width, h),
        ));
    }

    /// Clear all overlay buffers (e.g., after selection rect is finished).
    pub fn clear_overlay_buffers(&mut self) {
        self.overlay_buffers.clear();
    }

    /// Convert screen coordinates to canvas (world) coordinates using the camera transform.
    pub fn screen_to_canvas(&self, screen_x: f32, screen_y: f32) -> Vec2 {
        self.camera.screen_to_canvas(Vec2::new(screen_x, screen_y))
    }

    /// Returns the size of one screen pixel in canvas (world) units.
    /// Used to convert screen-space tolerances (e.g. click tolerance for
    /// edge hit testing) into canvas-space distances that stay visually
    /// consistent across zoom levels.
    pub fn canvas_per_pixel(&self) -> f32 {
        if self.camera.zoom > f32::EPSILON {
            1.0 / self.camera.zoom
        } else {
            1.0
        }
    }

    /// Process a single decree directly
    pub fn process_decree(&mut self, decree: RenderDecree) {
        self.handle_render_decree(decree);
    }

    fn handle_render_decree(&mut self, decree: RenderDecree) {
        match decree {
            RenderDecree::DisplayFps => {}
            RenderDecree::StartRender => {
                self.should_render = true;
            }
            RenderDecree::StopRender => {
                self.should_render = false;
            }
            RenderDecree::ReinitAdapter => {}
            RenderDecree::SetSurfaceSize(x, y) => {
                self.update_surface_size(x, y);
                if self.redraw_mode == RedrawMode::OnRequest {
                    self.render();
                }
            }
            RenderDecree::Terminate => {
                self.run = false;
            }
            RenderDecree::Noop => {}
            RenderDecree::CameraPan(dx, dy) => {
                self.camera.apply_mutation(
                    &baumhard::gfx_structs::camera::CameraMutation::Pan {
                        screen_delta: Vec2::new(dx, dy),
                    },
                );
                // Phase A's off-screen glyph cull is a function of the
                // camera, so moving the camera invalidates the cached
                // per-edge visible-glyph layout. Clear the renderer-side
                // connection cache so the next rebuild re-runs the cull
                // from scratch, and raise the viewport-dirty flag so the
                // event loop actually triggers the rebuild. The
                // document-side `SceneConnectionCache` holds canvas-space
                // samples whose spacing doesn't depend on pan, so it is
                // NOT cleared here — geometry stays cached across pans.
                self.connection_buffers.clear();
                self.connection_viewport_dirty = true;
            }
            RenderDecree::CameraZoom { screen_x, screen_y, factor } => {
                self.camera.apply_mutation(
                    &baumhard::gfx_structs::camera::CameraMutation::ZoomAt {
                        screen_focus: Vec2::new(screen_x, screen_y),
                        factor,
                    },
                );
                // Zoom invalidates both the renderer-side cull cache
                // (viewport-dirty) AND the document-side sample cache
                // (geometry-dirty), because the effective font size —
                // and therefore sample spacing along the path — is a
                // function of zoom via
                // `GlyphConnectionConfig::effective_font_size_pt`.
                self.connection_buffers.clear();
                self.connection_viewport_dirty = true;
                self.connection_geometry_dirty = true;
            }
        }
    }
}

/// Viewport containment test used by `rebuild_connection_buffers` to cull
/// off-screen connection glyphs before building cosmic-text buffers. The
/// visible canvas rect is padded by `margin` on every side so glyphs whose
/// anchor lands just outside the visible region still get drawn (avoiding
/// visible popping at the viewport edge during pan).
///
/// Extracted as a free function so Phase 4(A)'s core decision is unit-
/// testable without needing a real `Renderer` / wgpu context.
#[inline]
fn glyph_position_in_viewport(
    x: f32,
    y: f32,
    vp_min: Vec2,
    vp_max: Vec2,
    margin: f32,
) -> bool {
    x >= vp_min.x - margin
        && x <= vp_max.x + margin
        && y >= vp_min.y - margin
        && y <= vp_max.y + margin
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
}




#[cfg(test)]
mod tests;
