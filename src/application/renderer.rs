use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use cosmic_text::{Attrs, AttrsList, Buffer, BufferRef, Edit, Editor, FontSystem};
use glam::{Mat4, Quat, Vec3};
use cosmic_text::{Family, Style};
use glyphon::{Cache, Resolution, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport};
use indextree::Arena;
use log::{debug, error, info};
use rustc_hash::{FxHashMap, FxHasher};

use wgpu::{
    Adapter, Color, Device, Instance, MultisampleState, PipelineLayout, Queue, RenderPipeline,
    ShaderModule, StoreOp, Surface, SurfaceCapabilities, SurfaceConfiguration, TextureFormat,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::application::common::{PollTimer, RedrawMode, RenderDecree, StopWatch};
use baumhard::font::fonts;
use baumhard::font::fonts::AppFont;
use baumhard::util::grapheme_chad;
use baumhard::core::primitives::{
    ColorFontRegion, ColorFontRegions, Range as ColorFontRange,
};
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::area::{GlyphArea, OutlineStyle};
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use baumhard::shaders::shaders::{SHADERS, SHADER_APPLICATION};
use baumhard::font::attrs::attrs_list_from_regions;
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::loader;
use baumhard::mindmap::border::BorderStyle;
use baumhard::mindmap::scene_builder::{RenderScene, BorderElement, ConnectionElement, PortalElement, PortalRefKey};
use baumhard::mindmap::scene_cache::EdgeKey;
use glam::Vec2;
use std::path::Path;

/// Pre-layout console data handed from the app event loop to the
/// renderer every time the console state changes. The renderer turns
/// it into cosmic-text buffers in `rebuild_console_overlay_buffers`.
/// Kept as a plain struct (no rendering primitives) so unit tests
/// can construct one trivially.
///
/// Layout shape: a bottom-anchored strip with (bottom → top)
/// prompt line → completion popup → scrollback region. The
/// scrollback shows the most recent N output lines; the completion
/// popup is empty unless the user pressed Tab.
///
/// Styling (`font_family`, `font_size`) is threaded in from the
/// user config. The renderer stays dumb about where those values
/// came from — it just draws what the geometry says.
#[derive(Clone, Debug)]
pub struct ConsoleOverlayGeometry {
    /// Input buffer text, rendered after the `❯ ` prompt glyph.
    pub input: String,
    /// Grapheme-cluster index of the cursor. The renderer converts
    /// this to a byte offset via
    /// `baumhard::util::grapheme_chad::find_byte_index_of_grapheme`
    /// so the prompt-line `split_at` lands on a grapheme boundary
    /// even for ZWJ emoji / combining marks.
    pub cursor_grapheme: usize,
    /// Scrollback lines, oldest first. Only the trailing
    /// `MAX_CONSOLE_SCROLLBACK_ROWS` are drawn; anything above scrolls
    /// off the top.
    pub scrollback: Vec<ConsoleOverlayLine>,
    /// Completion candidates. Empty when the popup is closed.
    pub completions: Vec<ConsoleOverlayCompletion>,
    /// Which completion is highlighted. `None` when `completions` is
    /// empty. Index into `completions` otherwise.
    pub selected_completion: Option<usize>,
    /// Font family name passed to cosmic-text via
    /// `Attrs::new().family(Family::Name(..))`. Empty string means
    /// "use cosmic-text's default family", which lets cosmic-text's
    /// own fallback chain resolve it.
    pub font_family: String,
    /// Font size in pixels. The whole overlay scales with this value;
    /// row height, frame extents, and border repetition counts are
    /// all derived from it.
    pub font_size: f32,
}

/// One line in the scrollback, carrying its kind so the renderer can
/// color input echoes, normal output, and errors differently.
#[derive(Clone, Debug)]
pub struct ConsoleOverlayLine {
    pub text: String,
    pub kind: ConsoleOverlayLineKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleOverlayLineKind {
    /// Echo of a user-entered command (e.g. `> anchor set from top`).
    Input,
    /// Normal output line from a successful command.
    Output,
    /// Error output from a failed command.
    Error,
}

/// One completion candidate: the replacement text plus an optional
/// dim hint printed to the right (e.g. the command's summary).
#[derive(Clone, Debug)]
pub struct ConsoleOverlayCompletion {
    pub text: String,
    pub hint: Option<String>,
}

/// Pure-function output of the console-overlay layout pass. Holds
/// the derived screen-space dimensions for the console frame so the
/// backdrop rectangle and the border-glyph positions agree exactly.
/// Extracted to a plain struct so unit tests can verify the
/// alignment invariant without constructing a full `Renderer`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConsoleFrameLayout {
    pub left: f32,
    pub top: f32,
    pub frame_width: f32,
    pub frame_height: f32,
    pub font_size: f32,
    pub char_width: f32,
    pub row_height: f32,
    pub inner_padding: f32,
    /// How many scrollback rows fit inside the frame — clamped to
    /// `MAX_CONSOLE_SCROLLBACK_ROWS` and the available vertical
    /// space.
    pub scrollback_rows: usize,
    /// How many completion rows are drawn. 0 when the popup is
    /// closed. Completions sit directly above the prompt line.
    pub completion_rows: usize,
}

/// Maximum number of scrollback lines rendered. The scrollback
/// vector itself can grow unboundedly in memory, but only the
/// trailing N lines ever reach the screen.
pub const MAX_CONSOLE_SCROLLBACK_ROWS: usize = 12;

/// Maximum number of completion candidates drawn in the popup above
/// the prompt.
pub const MAX_CONSOLE_COMPLETION_ROWS: usize = 8;

// Prompt / cursor / completion-marker glyphs live in
// `console::visuals` with the rest of the palette. Bring them into
// this module's scope via `use` from the renderer body.
use crate::application::console::visuals::{CURSOR_GLYPH, PROMPT_GLYPH};

/// How many rows of `│` belong in each side column of the console
/// frame. The side column sits between the top border (at
/// `y = top`, height `font_size`) and the bottom border (at
/// `y = top + frame_height`), so it spans `frame_height - font_size`
/// pixels at `row_height` per row. Rounded up so the column always
/// reaches the bottom corner.
fn side_row_count(frame_height: f32, font_size: f32, row_height: f32) -> usize {
    let span = (frame_height - font_size).max(0.0);
    (span / row_height).ceil() as usize
}

/// Scale alpha linearly between `min` and `max` by `t in [0, 1]`.
/// Used to dim older scrollback rows.
fn lerp_alpha(min: u8, max: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    let v = min as f32 + (max as f32 - min as f32) * t;
    v.round().clamp(0.0, 255.0) as u8
}

/// Rebuild a `cosmic_text::Color` with a new alpha byte, keeping
/// RGB. Cosmic-text's `Color` is `[R, G, B, A]` packed into a u32,
/// so we unpack via the getter accessors and re-pack.
fn with_alpha(c: cosmic_text::Color, a: u8) -> cosmic_text::Color {
    cosmic_text::Color::rgba(c.r(), c.g(), c.b(), a)
}

impl ConsoleFrameLayout {
    /// Screen-space rectangle covered by the opaque backdrop. Matches
    /// the border-glyph bounds exactly: the top border row sits at
    /// `y = top`, the bottom border row extends to
    /// `y = top + frame_height + font_size`, and the left / right
    /// columns span `[left, left + frame_width]` horizontally.
    pub fn backdrop_rect(&self) -> (f32, f32, f32, f32) {
        (
            self.left,
            self.top,
            self.frame_width,
            self.frame_height + self.font_size,
        )
    }

    /// Y offset of the prompt line's baseline. Sits directly below
    /// the scrollback and completion regions. Kept consistent with
    /// the scrollback / completion placement in
    /// `rebuild_console_overlay_buffers` so rows never overlap.
    pub fn prompt_y(&self) -> f32 {
        self.top
            + self.font_size
            + self.inner_padding
            + self.row_height * (self.scrollback_rows + self.completion_rows) as f32
    }
}

/// Build the four border strings (top, bottom, left_column,
/// right_column) for the console frame using the rounded
/// `BorderGlyphSet` preset: `╭─...─╮`, `│` stacked, `╰─...─╯`.
///
/// `cols` is the total width of the top/bottom rows in monospace
/// character cells, including both corners — so `cols >= 2` is
/// required for the corners to render. `rows` is the height of each
/// side column in rows, exclusive of the corner rows (which belong
/// to the top/bottom border strings).
///
/// Returns `(top, bottom, left, right)`. The box-drawing presets
/// have `left == right`, so both side strings are the same — the
/// caller positions them at different x offsets.
pub fn build_console_border_strings(
    cols: usize,
    rows: usize,
) -> (String, String, String, String) {
    let glyphs = baumhard::mindmap::border::BorderGlyphSet::box_drawing_rounded();
    let top = glyphs.top_border(cols);
    let bottom = glyphs.bottom_border(cols);
    let side = glyphs.side_border(rows);
    (top, bottom, side.clone(), side)
}

/// Compute the screen-space layout for the console overlay from a
/// `ConsoleOverlayGeometry` and the current screen dimensions. Pure
/// function — no GPU or font-system access. Called by
/// `rebuild_console_overlay_buffers` to derive positions for the
/// backdrop rect, border glyphs, prompt, scrollback, and completion
/// popup. Unit tests use it directly to assert the backdrop-vs-border
/// alignment invariant and the scrollback/completion row math.
///
/// The console is a bottom-anchored strip: rows run (bottom → top)
/// **prompt → completion popup (if any) → scrollback region**. The
/// frame grows upward from the bottom of the window as scrollback or
/// completions accumulate, up to the built-in caps.
pub fn compute_console_frame_layout(
    geometry: &ConsoleOverlayGeometry,
    screen_width: f32,
    screen_height: f32,
) -> ConsoleFrameLayout {
    let font_size = geometry.font_size.max(4.0);
    // `0.6` is a conservative monospace advance — cosmic-text's
    // fallback chain lands on a proportional font by default, but the
    // characters we render (`╭ ─ ╮ │ ╰ ╯ ❯ ▌ ▸ ▏`) all advance by
    // roughly font_size * 0.6. Tweaking this value visibly shifts
    // the column count; keep it in sync with the real advance if
    // you swap the default font.
    let char_width = font_size * 0.6;
    let inner_padding: f32 = 8.0;
    let row_height = font_size + 2.0;

    let scrollback_rows = geometry
        .scrollback
        .len()
        .min(MAX_CONSOLE_SCROLLBACK_ROWS);
    let completion_rows = geometry
        .completions
        .len()
        .min(MAX_CONSOLE_COMPLETION_ROWS);

    let prompt_budget = font_size * 1.4;
    // Frame vertical budget: top border + inner pad + scrollback +
    // completions + prompt row + inner pad. Bottom border sits
    // outside `frame_height`; see `backdrop_rect`.
    let frame_height = font_size
        + inner_padding * 2.0
        + row_height * scrollback_rows as f32
        + row_height * completion_rows as f32
        + prompt_budget;

    // Full-width strip at the bottom of the window. No horizontal
    // clamp: the overlay tracks the window width. An inner margin
    // keeps the border from kissing the screen edge.
    let horizontal_margin = char_width;
    let frame_width = (screen_width - horizontal_margin * 2.0).max(char_width * 4.0);
    let left = horizontal_margin;
    let top = (screen_height - frame_height - inner_padding - font_size)
        .max(inner_padding);

    ConsoleFrameLayout {
        left,
        top,
        frame_width,
        frame_height,
        font_size,
        char_width,
        row_height,
        inner_padding,
        scrollback_rows,
        completion_rows,
    }
}

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
    graphics_arena: Arc<RwLock<Arena<GfxElement>>>,
    buffer_cache: FxHashMap<usize, TextBuffer>,
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
    /// Glyph-wheel color picker static overlay buffers. Shaped once
    /// when the picker opens (and rebuilt on window resize) and left
    /// alone thereafter — they cover the parts of the modal whose
    /// positions and colors don't change per hover: the title bar,
    /// the hint footer, and the 24 hue-ring glyphs (each colored at
    /// its own fixed slot hue). Populated only when the picker is
    /// open; cleared otherwise.
    color_picker_static_buffers: Vec<MindMapTextBuffer>,
    /// Glyph-wheel color picker dynamic overlay buffers. Rebuilt on
    /// every hover / Tab / h-s-v keystroke because their content
    /// depends on the current HSV or chip focus: sat bar cells
    /// (re-colored at current hue+val), val bar cells (re-colored at
    /// current hue+sat), the center preview glyph, the hex readout,
    /// the chip row (focus arrow moves), and a selection indicator
    /// ring around the currently-picked hue slot.
    color_picker_dynamic_buffers: Vec<MindMapTextBuffer>,
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
struct NodeBackgroundRect {
    position: Vec2,
    size: Vec2,
    color: [u8; 4],
}

impl Renderer {
    pub async fn new(
        instance: Instance,
        surface: Surface<'static>,
        window: Arc<Window>,
        arena: Arc<RwLock<Arena<GfxElement>>>,
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
            graphics_arena: arena,
            buffer_cache: Default::default(),
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
            color_picker_static_buffers: Vec::new(),
            color_picker_dynamic_buffers: Vec::new(),
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

    /// Checks if the block exists in the buffer_cache already, and if so, is the cached version up to date?
    /// Updates the cache as necessary
    fn prepare_glyph_block(
        block: &GlyphArea,
        unique_id: &usize,
        buffer_cache: &mut FxHashMap<usize, TextBuffer>,
    ) {
        let mut hasher = FxHasher::default();
        block.hash(&mut hasher);
        let block_hash = hasher.finish();

        let mut contains_id = false;
        let mut existing_hash: u64 = 0;
        if let Some(k) = buffer_cache.get(unique_id) {
            contains_id = true;
            existing_hash = k.block_hash;
        }
        if !contains_id || existing_hash != block_hash {
            let mut editor = fonts::create_cosmic_editor(
               block.scale.0,
               block.line_height.0,
               block.render_bounds.x.0,
               block.render_bounds.y.0,
            );
            // Interactive path: a contended font-system lock skips
            // this node's buffer update — the next frame will retry.
            let Ok(mut font_system) = fonts::FONT_SYSTEM.try_write() else {
                return;
            };
            editor.insert_string(
                block.text.as_str(),
                Some(attrs_list_from_regions(&block.regions, &mut font_system)),
            );
            editor.shape_as_needed(&mut font_system, false);
            let text_buffer = TextBuffer::new(
               editor,
               block_hash,
               (block.render_bounds.x.0, block.render_bounds.y.0),
               (block.position.x.0, block.position.y.0),
            );
            buffer_cache.insert(*unique_id, text_buffer);
        }
    }

    fn update_buffer_cache(&mut self) {
        // Interactive path: a contended arena lock or a node that has
        // shed its glyph_area mid-mutation must not abort the frame.
        let Ok(arena) = self.graphics_arena.try_read() else {
            return;
        };
        for node in arena.iter() {
            if node.is_removed() {
                continue;
            }
            let element = node.get();
            let Some(area) = element.glyph_area() else {
                continue;
            };
            Self::prepare_glyph_block(area, &element.unique_id(), &mut self.buffer_cache);
        }
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
        let mut main_text_areas: Vec<TextArea> = self.mindmap_buffers.values()
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

        // GfxElement arena-based buffers (no camera transform)
        for text_buffer in self.buffer_cache.values() {
            main_text_areas.push(TextArea {
                buffer: text_buffer.buffer(),
                left: text_buffer.pos.0,
                top: text_buffer.pos.1,
                scale: 1.0,
                bounds: vp_bounds,
                default_color,
                custom_glyphs: &[],
            });
        }

        // Palette overlay: screen-space text, drawn in its own
        // glyphon pass so the rect-pipeline backdrop can be
        // interleaved between the main text and this one. The
        // glyph-wheel color picker shares this pass — it's a
        // mutually exclusive screen-space modal. The picker is
        // split into static (hue ring + title + hint, shaped once
        // per open/resize) and dynamic (sat/val bars, preview,
        // hex, chips, selection indicator — shaped every hover)
        // buffer lists; both chain in here so a single render
        // pass handles them.
        let palette_text_areas: Vec<TextArea> = self.console_overlay_buffers.iter()
            .chain(self.color_picker_static_buffers.iter())
            .chain(self.color_picker_dynamic_buffers.iter())
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
    /// has registered into [`AppScene`]. Walks the scene in layer
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
    /// has registered into [`AppScene`]'s canvas sub-scene
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
        geometry: Option<&crate::application::color_picker::ColorPickerOverlayGeometry>,
    ) {
        use crate::application::color_picker::compute_color_picker_layout;
        use crate::application::scene_host::OverlayRole;

        let Some(g) = geometry else {
            self.color_picker_backdrop = None;
            app_scene.unregister_overlay(OverlayRole::ColorPicker);
            self.rebuild_overlay_scene_buffers(app_scene);
            return;
        };

        let layout = compute_color_picker_layout(
            g,
            self.config.width as f32,
            self.config.height as f32,
        );
        // Spec-gated transparent backdrop. When enabled, the picker
        // skips emitting an opaque rect — canvas content shows
        // through the gaps between glyphs, and per-glyph black
        // halos (added in `build_color_picker_overlay_tree`) keep
        // them legible against any background. The hit-test
        // `layout.backdrop` rectangle is the *semantic* boundary,
        // independent of whether the rect is drawn.
        let spec = crate::application::widgets::color_picker_widget::load_spec();
        self.color_picker_backdrop = if spec.geometry.transparent_backdrop {
            None
        } else {
            Some(layout.backdrop)
        };

        let tree = build_color_picker_overlay_tree(g, &layout);
        app_scene.register_overlay(OverlayRole::ColorPicker, tree, glam::Vec2::ZERO);
        self.rebuild_overlay_scene_buffers(app_scene);
    }

    /// §B2 mutation path — apply an in-place delta to the picker
    /// overlay tree without rebuilding it. Pairs with
    /// [`build_color_picker_overlay_mutator`]: every variable
    /// field on every picker GlyphArea is overwritten via an
    /// `Assign` `DeltaGlyphArea` keyed by stable channel.
    ///
    /// Use this for hover, HSV, chip-focus, and drag-Move /
    /// drag-Resize updates (anything that doesn't change the
    /// picker's element set). Open / close still use
    /// [`Self::rebuild_color_picker_overlay_buffers`] because the
    /// arena needs to be created or torn down. Calls
    /// `rebuild_overlay_scene_buffers` afterward to refresh the
    /// shaped buffers — the cosmic-text shape pass is still per-
    /// element, which is the §B1 perf gap tracked in `ROADMAP.md`.
    pub fn apply_color_picker_overlay_mutator(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    ) {
        use crate::application::color_picker::compute_color_picker_layout;
        use crate::application::scene_host::OverlayRole;
        let layout = compute_color_picker_layout(
            geometry,
            self.config.width as f32,
            self.config.height as f32,
        );
        let mutator = build_color_picker_overlay_mutator(geometry, &layout);
        app_scene.apply_overlay_mutator(OverlayRole::ColorPicker, &mutator);
        self.rebuild_overlay_scene_buffers(app_scene);
    }


    /// Keyed connection rebuild. See [`rebuild_border_buffers_keyed`] for
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
            RenderDecree::ArenaUpdate => {
                self.update_buffer_cache();
            }
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

pub struct TextBuffer {
    pub block_hash: u64,
    pub editor: Editor<'static>,
    pub pos: (f32, f32),
    pub bounds: (f32, f32),
}

impl TextBuffer {
    pub fn new(editor: Editor<'static>, block_hash: u64, bounds: (f32, f32), pos: (f32, f32)) -> Self {
        TextBuffer {
            block_hash,
            editor,
            pos,
            bounds,
        }
    }

    pub fn buffer(&self) -> &Buffer {
        match self.editor.buffer_ref() {
            BufferRef::Owned(buffer) => {buffer},
            BufferRef::Borrowed(buffer) => {*buffer},
            BufferRef::Arc(buffer) => {buffer.as_ref()},
        }
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
}

/// Build the color-picker overlay tree from a geometry +
/// pre-computed layout. Mirrors what
/// `Renderer::rebuild_color_picker_overlay_buffers_legacy` did
/// across its static + dynamic halves, but as one
/// `Tree<GfxElement, GfxMutator>` instead of two parallel buffer
/// lists.
///
/// Tree shape (flat under root, per-element ordering preserved):
///
/// ```text
/// Void (root)
/// ├── GlyphArea title bar
/// ├── GlyphArea hue ring slot 0
/// │   ...
/// ├── GlyphArea hue ring slot 23
/// ├── GlyphArea hint footer
/// ├── GlyphArea sat-bar cell 0..N (skipping centre)
/// ├── GlyphArea val-bar cell 0..N (skipping centre)
/// ├── GlyphArea selected-hue indicator (cyan ring)
/// ├── GlyphArea preview glyph (࿕ at 2× font size)
/// ├── GlyphArea hex readout (when geometry.hex_visible)
/// └── GlyphArea theme chip 0..N
/// ```
///
/// **Performance note**: this rebuilds every glyph on every
/// `rebuild_color_picker_overlay_buffers` call, which is the hover
/// hot path. The legacy split skipped the hue-ring shape on hover.
/// A follow-up will introduce a `MutatorTree`-based incremental
/// path (per §B2 of `lib/baumhard/CONVENTIONS.md`) that mutates
/// only the cells whose colors changed and the indicator's
/// position, leaving the static hue ring alone. The user
/// explicitly asked to land the migration first and address
/// picker sluggishness afterwards.
fn build_color_picker_overlay_tree(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    for (channel, area) in picker_glyph_areas(geometry, layout) {
        let element = GfxElement::new_area_non_indexed_with_id(area, channel, channel);
        let leaf = tree.arena.new_node(element);
        tree.root.append(leaf, &mut tree.arena);
    }
    tree
}

/// Build a [`MutatorTree`] that updates an already-registered picker
/// tree to the current `(geometry, layout)` state without rebuilding
/// the arena. Pairs with [`build_color_picker_overlay_tree`] —
/// channels are stable across both, so the walker's
/// `align_child_walks` matches each mutator child against the
/// existing GlyphArea at the same channel.
///
/// Every entry is an `Assign` `DeltaGlyphArea` carrying the full set
/// of variable fields (text, position, bounds, scale, line_height,
/// regions, outline). `align_center` stays at whatever the initial
/// tree build set; it's never mutated through this path because the
/// picker's per-element alignment is constant.
///
/// This is the §B2 "mutation, not rebuild" path for picker hover /
/// HSV / chip / drag updates. The arena is reused; only field values
/// change. The walker still re-shapes every cell — that's the
/// remaining §B1 perf gap, tracked in `ROADMAP.md` as the
/// hash-keyed shape cache follow-up.
fn build_color_picker_overlay_mutator(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> baumhard::gfx_structs::tree::MutatorTree<GfxMutator> {
    use baumhard::core::primitives::ApplyOperation;
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::gfx_structs::tree::MutatorTree;

    let mut mt = MutatorTree::new_with(GfxMutator::new_void(0));
    for (channel, area) in picker_glyph_areas(geometry, layout) {
        let delta = DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(area.text),
            GlyphAreaField::position(area.position.x.0, area.position.y.0),
            GlyphAreaField::bounds(area.render_bounds.x.0, area.render_bounds.y.0),
            GlyphAreaField::scale(area.scale.0),
            GlyphAreaField::line_height(area.line_height.0),
            GlyphAreaField::ColorFontRegions(area.regions),
            GlyphAreaField::Outline(area.outline),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]);
        let mutator = GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), channel);
        let id = mt.arena.new_node(mutator);
        mt.root.append(id, &mut mt.arena);
    }
    mt
}

/// Single source of truth for the picker's GlyphArea content, keyed
/// by stable channels. Both [`build_color_picker_overlay_tree`] (the
/// initial-build path) and [`build_color_picker_overlay_mutator`]
/// (the in-place update path) consume this so they can never drift.
///
/// **Channel ordering invariant**: the returned vec must be sorted
/// by ascending channel — Baumhard's `align_child_walks` pairs
/// mutator children against target children by ascending channel
/// and breaks alignment if the order is violated. The constants in
/// `color_picker.rs` (PICKER_CHANNEL_*) are already chosen to
/// preserve this invariant in the natural insertion order
/// (title → hue ring → hint → sat → val → preview → hex → chips).
///
/// **Stable element count**: hex is always emitted (with empty
/// text when invisible) so the channel set doesn't shift when the
/// cursor crosses the backdrop boundary. Empty-text areas are
/// skipped by the walker without shaping.
fn picker_glyph_areas(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> Vec<(usize, GlyphArea)> {
    use crate::application::color_picker::{
        arm_bottom_font, arm_bottom_glyphs, arm_left_glyphs, arm_right_glyphs, arm_top_glyphs,
        center_preview_glyph, hue_ring_glyphs, hue_slot_to_degrees, sat_cell_to_value,
        val_cell_to_value, CROSSHAIR_CENTER_CELL, PickerHit, PICKER_CHANNEL_HEX,
        PICKER_CHANNEL_HINT, PICKER_CHANNEL_HUE_RING_BASE, PICKER_CHANNEL_PREVIEW,
        PICKER_CHANNEL_SAT_BASE, PICKER_CHANNEL_TITLE, PICKER_CHANNEL_VAL_BASE,
        SAT_CELL_COUNT, VAL_CELL_COUNT,
    };
    use crate::application::widgets::color_picker_widget::load_spec;
    use baumhard::util::color::{hsv_to_hex, hsv_to_rgb};

    let spec = load_spec();
    let hover_scale: f32 = spec.geometry.hover_scale;

    // Outline style for every picker glyph. Sized at the spec's
    // `font_max` baseline and scaled linearly to the actual layout
    // `font_size` so a shrunk picker gets a proportionally thinner
    // outline. The walker (`walk_tree_into_buffers`) reads
    // `area.outline` and stamps 8 copies at the offsets yielded by
    // `OutlineStyle::offsets` — the stamp count is canonical inside
    // baumhard, so there's no `samples` knob here.
    let outline = if spec.geometry.outline_px > 0.0 {
        Some(OutlineStyle {
            color: [0, 0, 0, 255],
            px: spec.geometry.outline_px * (layout.font_size / spec.geometry.font_max),
        })
    } else {
        None
    };

    // Local `make_area` helper — equivalent to the prior `push_area`
    // but returns the GlyphArea rather than appending to a tree, so
    // both the tree- and mutator-building paths can route the same
    // value through their respective wrappers. `centered = true`
    // shapes the text with `Align::Center` so cross-script glyphs
    // (Devanagari / Hebrew / Tibetan in the hue ring, mixed sat/val
    // cells) sit on the same visual radius.
    //
    // `font` pins a specific `AppFont` for this area's region span
    // when cosmic-text's default fallback won't pick a covering
    // face — the SMP-range Egyptian hieroglyphs in particular.
    fn make_area(
        text: &str,
        color: cosmic_text::Color,
        font_size: f32,
        line_height: f32,
        pos: (f32, f32),
        bounds: (f32, f32),
        centered: bool,
        font: Option<baumhard::font::fonts::AppFont>,
        outline: Option<OutlineStyle>,
    ) -> GlyphArea {
        let mut area = GlyphArea::new_with_str(
            text,
            font_size,
            line_height,
            Vec2::new(pos.0, pos.1),
            Vec2::new(bounds.0, bounds.1),
        );
        area.align_center = centered;
        area.outline = outline;
        let cluster_count = text.chars().count();
        if cluster_count > 0 {
            let rgba = [
                color.r() as f32 / 255.0,
                color.g() as f32 / 255.0,
                color.b() as f32 / 255.0,
                color.a() as f32 / 255.0,
            ];
            let mut regions = ColorFontRegions::new_empty();
            regions.submit_region(ColorFontRegion::new(
                ColorFontRange::new(0, cluster_count),
                font,
                Some(rgba),
            ));
            area.regions = regions;
        }
        area
    }

    let font_size = layout.font_size;
    let ring_font_size = layout.ring_font_size;
    let cell_font_size = layout.cell_font_size;
    // Widen box reservations past the base glyph so hover-grow has
    // room to render at HOVER_SCALE without clipping neighbors, and
    // SMP glyphs (Egyptian hieroglyphs especially) shape without
    // hitting the right bound.
    let ring_box_w = ring_font_size * spec.geometry.ring_box_scale;
    let cell_box_w =
        (layout.cell_advance * spec.geometry.cell_box_scale).max(cell_font_size * 1.5);

    // Non-wheel chrome (title, hint, hex readout) tracks the
    // picker's current HSV preview color. This means the text
    // "carries" the selected color out of the wheel and into the
    // surrounding copy — confirming at a glance what the user is
    // about to commit. Halo contrast handles legibility.
    let preview_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, geometry.val);
    let preview_color = rgb_to_cosmic_color(preview_rgb);

    let mut out: Vec<(usize, GlyphArea)> = Vec::with_capacity(80);

    // Title.
    let is_standalone = geometry.target_label.is_empty();
    let title_text = if is_standalone {
        spec.title_template_standalone.clone()
    } else {
        spec.title_template_contextual
            .replace("{target_label}", geometry.target_label)
    };
    out.push((
        PICKER_CHANNEL_TITLE,
        make_area(
            &title_text,
            preview_color,
            font_size,
            font_size,
            layout.title_pos,
            (font_size * 24.0, font_size * 1.5),
            false,
            None,
            outline,
        ),
    ));

    // Hue ring.
    for (i, &ring_glyph) in hue_ring_glyphs().iter().enumerate() {
        let hue = hue_slot_to_degrees(i);
        let rgb = hsv_to_rgb(hue, 1.0, 1.0);
        let is_hovered = matches!(geometry.hovered_hit, Some(PickerHit::Hue(h)) if h == i);
        let color = if is_hovered {
            highlight_hovered_cell_color(rgb)
        } else {
            rgb_to_cosmic_color(rgb)
        };
        let scale = if is_hovered { hover_scale } else { 1.0 };
        let pos = layout.hue_slot_positions[i];
        let fs = ring_font_size * scale;
        let bw = ring_box_w * scale;
        out.push((
            PICKER_CHANNEL_HUE_RING_BASE + i,
            make_area(
                ring_glyph,
                color,
                fs,
                fs,
                (pos.0 - bw * 0.5, pos.1 - fs * 0.5),
                (bw, fs * 1.5),
                true,
                None,
                outline,
            ),
        ));
    }

    // Hint footer. Contextual mode includes "Esc cancel" because
    // Esc exits the modal picker; Standalone mode omits it because
    // the persistent palette only closes via `color picker off`
    // from the console, and showing a dead affordance is worse
    // than hiding it.
    let hint_text = if is_standalone {
        spec.hint_text_standalone.as_str()
    } else {
        spec.hint_text_contextual.as_str()
    };
    out.push((
        PICKER_CHANNEL_HINT,
        make_area(
            hint_text,
            preview_color,
            font_size * 0.85,
            font_size * 0.85,
            layout.hint_pos,
            (font_size * 30.0, font_size * 1.5),
            false,
            None,
            outline,
        ),
    ));

    // Sat / val bars (skip centre cell — that's the preview glyph slot).
    let current_sat_cell = (geometry.sat * (SAT_CELL_COUNT as f32 - 1.0))
        .round()
        .clamp(0.0, (SAT_CELL_COUNT - 1) as f32) as usize;
    let current_val_cell = ((1.0 - geometry.val) * (VAL_CELL_COUNT as f32 - 1.0))
        .round()
        .clamp(0.0, (VAL_CELL_COUNT - 1) as f32) as usize;

    for i in 0..SAT_CELL_COUNT {
        if i == CROSSHAIR_CENTER_CELL {
            continue;
        }
        let cell_sat = sat_cell_to_value(i);
        let base_rgb = hsv_to_rgb(geometry.hue_deg, cell_sat, geometry.val);
        let is_hovered = matches!(geometry.hovered_hit, Some(PickerHit::SatCell(h)) if h == i);
        let color = if is_hovered {
            highlight_hovered_cell_color(base_rgb)
        } else if i == current_sat_cell {
            highlight_selected_cell_color(base_rgb)
        } else {
            rgb_to_cosmic_color(base_rgb)
        };
        let glyph = if i < CROSSHAIR_CENTER_CELL {
            arm_left_glyphs()[i]
        } else {
            arm_right_glyphs()[i - CROSSHAIR_CENTER_CELL - 1]
        };
        let scale = if is_hovered { hover_scale } else { 1.0 };
        let (cx, cy) = layout.sat_cell_positions[i];
        let fs = cell_font_size * scale;
        let bw = cell_box_w * scale;
        out.push((
            PICKER_CHANNEL_SAT_BASE + i,
            make_area(
                glyph,
                color,
                fs,
                fs,
                (cx - bw * 0.5, cy - fs * 0.5),
                (bw, fs * 1.5),
                true,
                None,
                outline,
            ),
        ));
    }
    for i in 0..VAL_CELL_COUNT {
        if i == CROSSHAIR_CENTER_CELL {
            continue;
        }
        let cell_val = val_cell_to_value(i);
        let base_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, cell_val);
        let is_hovered = matches!(geometry.hovered_hit, Some(PickerHit::ValCell(h)) if h == i);
        let color = if is_hovered {
            highlight_hovered_cell_color(base_rgb)
        } else if i == current_val_cell {
            highlight_selected_cell_color(base_rgb)
        } else {
            rgb_to_cosmic_color(base_rgb)
        };
        // Pin Egyptian hieroglyph font on the bottom arm — see the
        // walker's family-name fix for context.
        let (glyph, font) = if i < CROSSHAIR_CENTER_CELL {
            (arm_top_glyphs()[i], None)
        } else {
            (
                arm_bottom_glyphs()[i - CROSSHAIR_CENTER_CELL - 1],
                arm_bottom_font(),
            )
        };
        let scale = if is_hovered { hover_scale } else { 1.0 };
        let (cx, cy) = layout.val_cell_positions[i];
        let fs = cell_font_size * scale;
        let bw = cell_box_w * scale;
        out.push((
            PICKER_CHANNEL_VAL_BASE + i,
            make_area(
                glyph,
                color,
                fs,
                fs,
                (cx - bw * 0.5, cy - fs * 0.5),
                (bw, fs * 1.5),
                true,
                font,
                outline,
            ),
        ));
    }

    // Centre preview glyph ࿕ (right-facing Tibetan svasti — the
    // spiritual "four roads meeting" symbol). Acts as the commit
    // button; hovering brightens it.
    let preview_size = layout.preview_size;
    let commit_hovered = matches!(geometry.hovered_hit, Some(PickerHit::Commit));
    let commit_color = if commit_hovered {
        highlight_hovered_cell_color(preview_rgb)
    } else {
        preview_color
    };
    let preview_scale_f = if commit_hovered { hover_scale } else { 1.0 };
    let scaled_preview = preview_size * preview_scale_f;
    // Pin the Tibetan font for the ࿕ glyph (U+0FD5) — cosmic-text's
    // default fallback isn't reliable for it, and we already pin
    // specific fonts for the Egyptian arm via the same pattern.
    let center_font = Some(baumhard::font::fonts::AppFont::NotoSerifTibetanRegular);
    out.push((
        PICKER_CHANNEL_PREVIEW,
        make_area(
            center_preview_glyph(),
            commit_color,
            scaled_preview,
            scaled_preview,
            (
                layout.preview_pos.0 - (scaled_preview - preview_size) * 0.5,
                layout.preview_pos.1 - (scaled_preview - preview_size) * 0.5,
            ),
            (scaled_preview * 1.5, scaled_preview * 1.5),
            true,
            center_font,
            outline,
        ),
    ));

    // Hex readout — always emitted at a stable channel so the
    // mutator path doesn't have to handle a flickering element.
    // Empty text when invisible; the walker shapes nothing.
    let (hex_text, hex_pos, hex_bounds) = match layout.hex_pos {
        Some(anchor) => (
            hsv_to_hex(geometry.hue_deg, geometry.sat, geometry.val),
            anchor,
            (font_size * 8.0, font_size * 1.5),
        ),
        None => (String::new(), (0.0, 0.0), (0.0, 0.0)),
    };
    out.push((
        PICKER_CHANNEL_HEX,
        make_area(
            &hex_text,
            preview_color,
            font_size,
            font_size,
            hex_pos,
            hex_bounds,
            false,
            None,
            outline,
        ),
    ));

    out
}

// =============================================================
// Stable channel scheme for the console overlay tree
// =============================================================
//
// Mirrors the picker's stable-channel discipline (commit
// `ceaeeb4`): every console GlyphArea sits at a deterministic
// channel so the §B2 in-place mutator path can target it across
// keystrokes. Bands are wide enough to add new sub-rows without
// renumbering. **Order matters** — the values must be strictly
// ascending in tree-insertion order, otherwise Baumhard's
// `align_child_walks` breaks alignment and the mutator path
// silently misses elements.
//
// Layout-wise: 4 borders → `scrollback_rows` × (gutter + text)
// always-emitted slots → `completion_rows` always-emitted slots
// → prompt line. Slots beyond what the geometry currently
// populates carry empty `""` text, which the walker shapes as
// nothing — a stable element set even when scrollback is short.

const CONSOLE_CHANNEL_TOP_BORDER: usize = 1;
const CONSOLE_CHANNEL_BOTTOM_BORDER: usize = 2;
const CONSOLE_CHANNEL_LEFT_COL: usize = 3;
const CONSOLE_CHANNEL_RIGHT_COL: usize = 4;
const CONSOLE_CHANNEL_SCROLLBACK_GUTTER_BASE: usize = 100;
const CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE: usize = 1_000;
const CONSOLE_CHANNEL_COMPLETION_BASE: usize = 10_000;
const CONSOLE_CHANNEL_PROMPT: usize = 100_000;

/// Single source of truth for the console overlay's GlyphArea
/// content, keyed by stable channel. Both
/// [`build_console_overlay_tree`] (the initial-build path) and
/// [`build_console_overlay_mutator`] (the in-place §B2 update path)
/// consume this so the two paths cannot drift.
///
/// Every `scrollback_rows × 2` scrollback slot and every
/// `completion_rows` completion slot is emitted, padding with empty
/// `""` text when the geometry has fewer items than the layout
/// allows. The walker shapes nothing for empty text, so the cost
/// of an empty slot is one allocation-free leaf visit.
///
/// **Channel ordering invariant**: returned in strictly ascending
/// channel order — the constants above are deliberately spaced so
/// that strict order is preserved even when the per-row counts
/// grow.
fn console_overlay_areas(
    geometry: &ConsoleOverlayGeometry,
    layout: &ConsoleFrameLayout,
    font_system: &mut FontSystem,
) -> Vec<(usize, GlyphArea)> {
    use crate::application::console::visuals::{
        ACCENT_COLOR, BORDER_COLOR, ERROR_COLOR, GUTTER_GLYPH, INPUT_ECHO_COLOR,
        SCROLLBACK_MIN_ALPHA, SELECTED_COMPLETION_MARKER, TEXT_COLOR,
        UNSELECTED_COMPLETION_MARKER,
    };

    let &ConsoleFrameLayout {
        left,
        top,
        frame_width,
        frame_height,
        font_size,
        char_width,
        row_height,
        inner_padding,
        scrollback_rows,
        completion_rows,
    } = layout;

    let mk_area = |text: &str,
                   color: cosmic_text::Color,
                   font_size: f32,
                   line_height: f32,
                   pos: (f32, f32),
                   bounds: (f32, f32)|
     -> GlyphArea {
        let mut area = GlyphArea::new_with_str(
            text,
            font_size,
            line_height,
            Vec2::new(pos.0, pos.1),
            Vec2::new(bounds.0, bounds.1),
        );
        let cluster_count = text.chars().count();
        if cluster_count > 0 {
            let rgba = [
                color.r() as f32 / 255.0,
                color.g() as f32 / 255.0,
                color.b() as f32 / 255.0,
                color.a() as f32 / 255.0,
            ];
            let mut regions = ColorFontRegions::new_empty();
            regions.submit_region(ColorFontRegion::new(
                ColorFontRange::new(0, cluster_count),
                None,
                Some(rgba),
            ));
            area.regions = regions;
        }
        area
    };

    let measured_char_width =
        measure_max_glyph_advance(font_system, &["\u{2500}", "\u{2502}"], font_size);
    let cols = ((frame_width / measured_char_width).floor() as usize).max(2);
    let side_rows = side_row_count(frame_height, font_size, row_height);
    let (top_border, bottom_border, left_col, right_col) =
        build_console_border_strings(cols, side_rows);

    let mut out: Vec<(usize, GlyphArea)> = Vec::new();

    // Borders (always present).
    out.push((
        CONSOLE_CHANNEL_TOP_BORDER,
        mk_area(
            &top_border,
            BORDER_COLOR,
            font_size,
            font_size,
            (left, top),
            (frame_width, font_size * 1.5),
        ),
    ));
    out.push((
        CONSOLE_CHANNEL_BOTTOM_BORDER,
        mk_area(
            &bottom_border,
            BORDER_COLOR,
            font_size,
            font_size,
            (left, top + frame_height),
            (frame_width, font_size * 1.5),
        ),
    ));
    out.push((
        CONSOLE_CHANNEL_LEFT_COL,
        mk_area(
            &left_col,
            BORDER_COLOR,
            font_size,
            row_height,
            (left, top + font_size),
            (measured_char_width, frame_height),
        ),
    ));
    let right_col_x = left + (cols.saturating_sub(1) as f32) * measured_char_width;
    out.push((
        CONSOLE_CHANNEL_RIGHT_COL,
        mk_area(
            &right_col,
            BORDER_COLOR,
            font_size,
            row_height,
            (right_col_x, top + font_size),
            (measured_char_width, frame_height),
        ),
    ));

    // Scrollback rows: always emit `scrollback_rows` slots,
    // padding with empty text when the geometry has fewer items.
    // Stable structure is what lets the §B2 mutator path target
    // the same channel across calls when the visible count
    // shifts under it.
    let gutter_x = left + measured_char_width;
    let content_left = gutter_x + measured_char_width + inner_padding;
    let content_width = right_col_x - content_left - inner_padding;
    let content_top = top + font_size + inner_padding;
    let content_cols = (content_width / measured_char_width).floor() as usize;

    let skip = geometry.scrollback.len().saturating_sub(scrollback_rows);
    let visible_count = scrollback_rows.max(1);
    for slot in 0..scrollback_rows {
        let line_opt = geometry.scrollback.get(skip + slot);
        let y = content_top + row_height * slot as f32;
        let (gutter_text, gutter_color, text_str, text_color) = match line_opt {
            None => (String::new(), BORDER_COLOR, String::new(), TEXT_COLOR),
            Some(line) => {
                let newness = if visible_count <= 1 {
                    1.0
                } else {
                    slot as f32 / (visible_count - 1) as f32
                };
                let alpha = lerp_alpha(SCROLLBACK_MIN_ALPHA, 0xff, newness);
                let (text_color, gutter_color, gutter_glyph) = match line.kind {
                    ConsoleOverlayLineKind::Input => (
                        with_alpha(INPUT_ECHO_COLOR, alpha),
                        with_alpha(INPUT_ECHO_COLOR, alpha),
                        " ",
                    ),
                    ConsoleOverlayLineKind::Output => (
                        with_alpha(TEXT_COLOR, alpha),
                        with_alpha(ACCENT_COLOR, alpha),
                        GUTTER_GLYPH,
                    ),
                    ConsoleOverlayLineKind::Error => (
                        with_alpha(ERROR_COLOR, alpha),
                        with_alpha(ERROR_COLOR, alpha),
                        GUTTER_GLYPH,
                    ),
                };
                let clipped = baumhard::util::grapheme_chad::truncate_to_display_width(
                    &line.text,
                    content_cols,
                );
                let gutter = if gutter_glyph == " " {
                    String::new()
                } else {
                    gutter_glyph.to_string()
                };
                (gutter, gutter_color, clipped.to_string(), text_color)
            }
        };
        out.push((
            CONSOLE_CHANNEL_SCROLLBACK_GUTTER_BASE + slot,
            mk_area(
                &gutter_text,
                gutter_color,
                font_size,
                row_height,
                (gutter_x, y),
                (char_width, row_height),
            ),
        ));
        out.push((
            CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE + slot,
            mk_area(
                &text_str,
                text_color,
                font_size,
                row_height,
                (content_left, y),
                (content_width, row_height),
            ),
        ));
    }

    // Completion popup rows: same always-emit-N pattern.
    let completion_top = content_top + row_height * scrollback_rows as f32;
    for slot in 0..completion_rows {
        let comp_opt = geometry.completions.get(slot);
        let y = completion_top + row_height * slot as f32;
        let (text_str, color) = match comp_opt {
            None => (String::new(), TEXT_COLOR),
            Some(c) => {
                let is_selected = geometry.selected_completion == Some(slot);
                let color = if is_selected { ACCENT_COLOR } else { TEXT_COLOR };
                let prefix = if is_selected {
                    SELECTED_COMPLETION_MARKER
                } else {
                    UNSELECTED_COMPLETION_MARKER
                };
                let line = match &c.hint {
                    Some(hint) => format!("{prefix}{}    {}", c.text, hint),
                    None => format!("{prefix}{}", c.text),
                };
                let clipped = baumhard::util::grapheme_chad::truncate_to_display_width(
                    &line,
                    content_cols,
                );
                (clipped.to_string(), color)
            }
        };
        out.push((
            CONSOLE_CHANNEL_COMPLETION_BASE + slot,
            mk_area(
                &text_str,
                color,
                font_size,
                row_height,
                (content_left, y),
                (content_width, row_height),
            ),
        ));
    }

    // Prompt line — single GlyphArea with two ColorFontRegions so
    // the prompt and the input share one shaped run, and the
    // input's first glyph lands at the prompt's actual shaped
    // advance.
    let prompt_budget = font_size * 1.4;
    let y = layout.prompt_y();
    let cursor_byte = baumhard::util::grapheme_chad::find_byte_index_of_grapheme(
        &geometry.input,
        geometry.cursor_grapheme,
    )
    .unwrap_or(geometry.input.len());
    let (pre, post) = geometry.input.split_at(cursor_byte);
    let input_with_cursor = format!("{pre}{CURSOR_GLYPH}{post}");
    let input_clipped = baumhard::util::grapheme_chad::truncate_to_display_width(
        &input_with_cursor,
        content_cols.saturating_sub(2),
    );
    let prompt_text = "\u{276F} ";
    let combined = format!("{prompt_text}{input_clipped}");
    let prompt_chars = prompt_text.chars().count();
    let input_chars = input_clipped.chars().count();

    let mut prompt_area = GlyphArea::new_with_str(
        &combined,
        font_size,
        font_size,
        Vec2::new(content_left, y),
        Vec2::new(content_width, prompt_budget),
    );
    let mut regions = ColorFontRegions::new_empty();
    let to_rgba = |c: cosmic_text::Color| -> [f32; 4] {
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            c.a() as f32 / 255.0,
        ]
    };
    regions.submit_region(ColorFontRegion::new(
        ColorFontRange::new(0, prompt_chars),
        None,
        Some(to_rgba(ACCENT_COLOR)),
    ));
    if input_chars > 0 {
        regions.submit_region(ColorFontRegion::new(
            ColorFontRange::new(prompt_chars, prompt_chars + input_chars),
            None,
            Some(to_rgba(TEXT_COLOR)),
        ));
    }
    prompt_area.regions = regions;
    out.push((CONSOLE_CHANNEL_PROMPT, prompt_area));

    out
}

/// Build the console overlay tree from a geometry + pre-computed
/// layout. One Void root with one GlyphArea per stable-channel
/// slot: 4 borders, `scrollback_rows × 2` scrollback slots,
/// `completion_rows` completion slots, and 1 prompt line.
/// Empty slots carry empty text so the structure is constant
/// across keystrokes — the prerequisite for the in-place
/// [`build_console_overlay_mutator`] path.
///
/// Used by [`Renderer::rebuild_console_overlay_buffers`] which
/// then registers the tree under
/// [`crate::application::scene_host::OverlayRole::Console`] and
/// walks it through the standard overlay-scene pipeline.
///
/// `font_system` is needed only for `measure_max_glyph_advance` —
/// no shaping happens here. The returned tree's GlyphArea
/// positions are absolute screen coordinates so the walker's
/// per-tree offset can be `Vec2::ZERO`.
fn build_console_overlay_tree(
    geometry: &ConsoleOverlayGeometry,
    layout: &ConsoleFrameLayout,
    font_system: &mut FontSystem,
) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    for (channel, area) in console_overlay_areas(geometry, layout, font_system) {
        let element = GfxElement::new_area_non_indexed_with_id(area, channel, channel);
        let leaf = tree.arena.new_node(element);
        tree.root.append(leaf, &mut tree.arena);
    }
    tree
}

/// Build a [`MutatorTree`] that updates an already-registered
/// console overlay tree to the current `(geometry, layout)` state
/// without rebuilding the arena. Pairs with
/// [`build_console_overlay_tree`] — both consume
/// [`console_overlay_areas`] so channels and slot counts match.
///
/// Use this for the keystroke hot path: input mutation moves only
/// the prompt line's text and cursor region; the borders /
/// scrollback / completion slots stay stable in shape, the
/// mutator overwrites their fields with the same values, and the
/// arena is reused. Open / close still use the full rebuild path
/// because the arena needs to be created or torn down. A change
/// in `scrollback_rows` or `completion_rows` (window resize)
/// shifts the structural signature and the dispatcher in
/// [`Renderer::rebuild_console_overlay_buffers`] falls back to a
/// rebuild.
fn build_console_overlay_mutator(
    geometry: &ConsoleOverlayGeometry,
    layout: &ConsoleFrameLayout,
    font_system: &mut FontSystem,
) -> baumhard::gfx_structs::tree::MutatorTree<GfxMutator> {
    use baumhard::core::primitives::ApplyOperation;
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::gfx_structs::tree::MutatorTree;

    let mut mt = MutatorTree::new_with(GfxMutator::new_void(0));
    for (channel, area) in console_overlay_areas(geometry, layout, font_system) {
        let delta = DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(area.text),
            GlyphAreaField::position(area.position.x.0, area.position.y.0),
            GlyphAreaField::bounds(area.render_bounds.x.0, area.render_bounds.y.0),
            GlyphAreaField::scale(area.scale.0),
            GlyphAreaField::line_height(area.line_height.0),
            GlyphAreaField::ColorFontRegions(area.regions),
            GlyphAreaField::Outline(area.outline),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]);
        let mutator = GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), channel);
        let id = mt.arena.new_node(mutator);
        mt.root.append(id, &mut mt.arena);
    }
    mt
}

/// Structural signature for the console overlay tree.
/// `(scrollback_rows, completion_rows)` from the layout. Two
/// calls share a signature iff the slot counts match — the
/// precondition for the in-place
/// [`build_console_overlay_mutator`] path. Window resize is the
/// only typical event that shifts these, so the signature stays
/// stable across keystroke / scrollback-grow / completion-update
/// frames and the §B2 path runs on those.
fn console_overlay_signature(layout: &ConsoleFrameLayout) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    layout.scrollback_rows.hash(&mut h);
    layout.completion_rows.hash(&mut h);
    h.finish()
}

/// Shared tree → cosmic-text buffer walker.
///
/// Iterates every `GlyphArea` descendant of `tree`, shapes a
/// `cosmic_text::Buffer` for each one, and hands the result to
/// `yield_buffer` together with the element's `unique_id` (raw
/// `usize`, not stringified — keying is the caller's choice).
/// Background fills (if any) are forwarded to `yield_background`
/// before the buffer is built so rects attached to text-empty
/// areas still land.
///
/// `offset` is added to every `position` — callers pass
/// `Vec2::ZERO` whenever the tree's areas are already in the
/// destination coordinate space (e.g. the mindmap, whose nodes
/// hold canvas-space positions); pass the registered tree offset
/// for scene trees that lay out in their own local frame.
///
/// # Costs
///
/// O(descendants). One `cosmic_text::Buffer` allocated per
/// non-empty-text area; background rect yields are trivial. No
/// per-area `String` allocation — the `unique_id` flows as a raw
/// integer and only the mindmap closure stringifies it for its
/// `FxHashMap` key. Holds the provided `font_system` write guard
/// for the duration of the walk — keep the call site's own guard
/// scope tight.
fn walk_tree_into_buffers(
    tree: &Tree<GfxElement, GfxMutator>,
    offset: Vec2,
    font_system: &mut FontSystem,
    mut yield_buffer: impl FnMut(usize, MindMapTextBuffer),
    mut yield_background: impl FnMut(NodeBackgroundRect),
) {
    for descendant_id in tree.root().descendants(&tree.arena) {
        let node = match tree.arena.get(descendant_id) {
            Some(n) => n,
            None => continue,
        };
        let element = node.get();
        let area = match element.glyph_area() {
            Some(a) => a,
            None => continue, // Void and GlyphModel nodes carry no text.
        };

        if let Some(color) = area.background_color {
            yield_background(NodeBackgroundRect {
                position: Vec2::new(area.position.x.0, area.position.y.0) + offset,
                size: Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0),
                color,
            });
        }

        if area.text.is_empty() {
            continue;
        }

        let scale = area.scale.0;
        let line_height = area.line_height.0;
        let bound_x = area.render_bounds.x.0;
        let bound_y = area.render_bounds.y.0;

        // Pre-compute font family names per region. The walker had a
        // long-standing bug where `region.font` was stored on the
        // GlyphArea but never threaded into the cosmic-text `Attrs`,
        // so SMP-range glyphs that needed an explicit face (Egyptian
        // hieroglyphs in the color-picker bottom arm in particular)
        // silently rendered as tofu — cosmic-text's default fallback
        // doesn't pick the Noto Sans Egyptian Hieroglyphs face.
        //
        // The family lookup borrows `font_system.db()` immutably,
        // while `set_rich_text` below needs `&mut font_system`. We
        // collect the family strings into an owned `Vec<Option<String>>`
        // here so the immutable borrow ends before the mutable one
        // begins, and the owned strings outlive each spans Vec that
        // borrows them via `Family::Name`. The same names are reused
        // across the main buffer and every halo copy.
        let family_names: Vec<Option<String>> = if area.regions.num_regions() == 0 {
            vec![None]
        } else {
            area.regions
                .all_regions()
                .iter()
                .map(|region| {
                    region.font.and_then(|f| {
                        fonts::COMPILED_FONT_ID_MAP.get(&f).and_then(|ids| {
                            font_system
                                .db()
                                .face(ids[0])
                                .map(|face| face.families[0].0.clone())
                        })
                    })
                })
                .collect()
        };

        let text = &area.text;
        let alignment = if area.align_center {
            Some(cosmic_text::Align::Center)
        } else {
            None
        };

        // Build a `Vec<(&str, Attrs)>` for shaping, with an optional
        // color override that recolors *every* span to the given
        // color (used by the halo loop below). `None` keeps each
        // region's own color. Per-region font pinning is preserved
        // either way, so a halo behind an Egyptian hieroglyph still
        // shapes through the Noto Egyptian Hieroglyphs face.
        let build_spans = |color_override: Option<cosmic_text::Color>| -> Vec<(&str, Attrs)> {
            if area.regions.num_regions() == 0 {
                let mut attrs = Attrs::new();
                if let Some(c) = color_override {
                    attrs = attrs.color(c);
                }
                attrs = attrs.metrics(cosmic_text::Metrics::new(scale, line_height));
                vec![(text.as_str(), attrs)]
            } else {
                area.regions
                    .all_regions()
                    .iter()
                    .enumerate()
                    .filter_map(|(i, region)| {
                        let start =
                            grapheme_chad::find_byte_index_of_char(text, region.range.start)
                                .unwrap_or(text.len());
                        let end = grapheme_chad::find_byte_index_of_char(text, region.range.end)
                            .unwrap_or(text.len());
                        if start >= end {
                            return None;
                        }
                        let slice = &text[start..end];
                        let mut attrs = Attrs::new();
                        let color = color_override.or_else(|| {
                            region.color.map(|rgba| {
                                let u8c = baumhard::util::color::convert_f32_to_u8(&rgba);
                                cosmic_text::Color::rgba(u8c[0], u8c[1], u8c[2], u8c[3])
                            })
                        });
                        if let Some(c) = color {
                            attrs = attrs.color(c);
                        }
                        // Pin the per-region font when the GlyphArea
                        // specified one. The family name is owned by
                        // `family_names`; `Family::Name` borrows it
                        // for the lifetime of `attrs`. Iterators have
                        // identical length by construction (both run
                        // over `area.regions.all_regions()`), so
                        // direct indexing is safe.
                        if let Some(family) = family_names[i].as_deref() {
                            attrs = attrs.family(Family::Name(family));
                        }
                        attrs = attrs.metrics(cosmic_text::Metrics::new(scale, line_height));
                        Some((slice, attrs))
                    })
                    .collect()
            }
        };

        // Helper to shape one buffer at an offset and yield it. The
        // wrap mode stays at cosmic-text's default `Wrap::WordOrGlyph`
        // — `Word` mode silently dropped supplementary-plane glyphs
        // (e.g. picker Egyptian hieroglyphs) whose shaped advance
        // exceeded the cell box.
        let mut shape_and_yield =
            |spans: Vec<(&str, Attrs)>, x_off: f32, y_off: f32, fs: &mut FontSystem| {
                let mut buffer = cosmic_text::Buffer::new(
                    fs,
                    cosmic_text::Metrics::new(scale, line_height),
                );
                buffer.set_size(fs, Some(bound_x), Some(bound_y));
                buffer.set_rich_text(
                    fs,
                    spans,
                    &Attrs::new(),
                    cosmic_text::Shaping::Advanced,
                    alignment,
                );
                buffer.shape_until_scroll(fs, false);
                let text_buffer = MindMapTextBuffer {
                    buffer,
                    pos: (
                        area.position.x.0 + x_off + offset.x,
                        area.position.y.0 + y_off + offset.y,
                    ),
                    bounds: (bound_x, bound_y),
                };
                yield_buffer(element.unique_id(), text_buffer);
            };

        // Halos first — DFS yield order means later buffers render on
        // top, so emitting halos before the main glyph puts them
        // visually behind. The stamp geometry is canonical in
        // baumhard (`OutlineStyle::offsets`) — we just recolor every
        // span to `outline.color` and shape one buffer per offset.
        if let Some(outline) = area.outline {
            if outline.px > 0.0 {
                let halo_color = cosmic_text::Color::rgba(
                    outline.color[0],
                    outline.color[1],
                    outline.color[2],
                    outline.color[3],
                );
                for (dx, dy) in outline.offsets() {
                    let halo_spans = build_spans(Some(halo_color));
                    shape_and_yield(halo_spans, dx, dy, font_system);
                }
            }
        }

        // Main glyph. Always emitted last so it sits on top of any
        // halos.
        let main_spans = build_spans(None);
        shape_and_yield(main_spans, 0.0, 0.0, font_system);
    }
}

/// Convert a normalized `[0, 1]` RGB triple into an opaque
/// `cosmic_text::Color`. Used by the glyph-wheel color picker render
/// path to paint each hue-ring slot, sat/val cell, and preview glyph
/// at its own HSV coordinate without per-frame closure allocation.
#[inline]
fn rgb_to_cosmic_color(rgb: [f32; 3]) -> cosmic_text::Color {
    cosmic_text::Color::rgba(
        (rgb[0] * 255.0).round() as u8,
        (rgb[1] * 255.0).round() as u8,
        (rgb[2] * 255.0).round() as u8,
        255,
    )
}

/// Highlight a crosshair-arm cell's color to mark it as "currently
/// selected". The picker used to swap glyphs (■ → ◆) to indicate
/// selection, but with sacred-script glyphs that approach would lose
/// the per-cell script identity. Instead we brighten the cell toward
/// white, which reads as a subtle glow on top of the hue-saturated
/// base color.
#[inline]
fn highlight_selected_cell_color(rgb: [f32; 3]) -> cosmic_text::Color {
    // Mix 60% toward white.
    let mix = |c: f32| (c + (1.0 - c) * 0.6).clamp(0.0, 1.0);
    rgb_to_cosmic_color([mix(rgb[0]), mix(rgb[1]), mix(rgb[2])])
}

/// Highlight a cell under the cursor. Distinct from the selected-
/// cell mix (which marks the HSV-current cell) so the hovered + the
/// already-selected cell can both be visually distinguishable — the
/// hovered one reads "whitest" because of the scale bump AND this
/// deeper mix, while the selected one stays subtly glowing behind
/// the hover cursor. A 40% mix toward white is enough to pop against
/// the hue-saturated background but not so saturated that the glyph
/// character becomes hard to read.
#[inline]
fn highlight_hovered_cell_color(rgb: [f32; 3]) -> cosmic_text::Color {
    let mix = |c: f32| (c + (1.0 - c) * 0.4).clamp(0.0, 1.0);
    rgb_to_cosmic_color([mix(rgb[0]), mix(rgb[1]), mix(rgb[2])])
}

/// Measure the widest shaped advance across a set of glyph strings
/// at the given font size, via cosmic-text. Used by the color picker
/// to pick a cell-spacing unit that accommodates the actual shaped
/// width of sacred-script glyphs — Devanagari clusters, Tibetan
/// stacks, and especially Egyptian hieroglyphs shape meaningfully
/// wider than the Latin `font_size * 0.6` baseline.
///
/// Returns the max `glyph.w` (advance in pixels) seen across every
/// glyph string passed in. Falls back to `font_size * 0.6` if every
/// glyph somehow shapes to zero width (e.g., tofu + missing fallback).
pub fn measure_max_glyph_advance(
    font_system: &mut cosmic_text::FontSystem,
    glyphs: &[&str],
    font_size: f32,
) -> f32 {
    let mut buffer = cosmic_text::Buffer::new(
        font_system,
        cosmic_text::Metrics::new(font_size, font_size),
    );
    let attrs = Attrs::new();
    let mut max_w: f32 = 0.0;
    for g in glyphs {
        buffer.set_text(
            font_system,
            g,
            &attrs,
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(font_system, false);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                if glyph.w > max_w {
                    max_w = glyph.w;
                }
            }
        }
    }
    if max_w <= 0.0 {
        font_size * 0.6
    } else {
        max_w
    }
}

fn create_border_buffer(
    font_system: &mut FontSystem,
    text: &str,
    attrs: &Attrs,
    font_size: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    create_border_buffer_lh(font_system, text, attrs, font_size, font_size, pos, bounds)
}

/// Like [`create_border_buffer`] but sets an explicit line-height on
/// the buffer metrics. Needed for multi-line console side columns,
/// where the vertical stack of `│` glyphs has to advance at the
/// content's `row_height` (font_size + 2px breathing room) — not the
/// default `font_size`, which would drift the side column short by
/// 2px per row.
fn create_border_buffer_lh(
    font_system: &mut FontSystem,
    text: &str,
    attrs: &Attrs,
    font_size: f32,
    line_height: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    let mut buf = cosmic_text::Buffer::new(
        font_system,
        cosmic_text::Metrics::new(font_size, line_height),
    );
    buf.set_size(font_system, Some(bounds.0), Some(bounds.1));
    buf.set_rich_text(
        font_system,
        vec![(text, attrs.clone())],
        &Attrs::new(),
        cosmic_text::Shaping::Advanced,
        None,
    );
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer { buffer: buf, pos, bounds }
}

/// Multi-span variant of [`create_border_buffer`] — hands cosmic-text
/// a sequence of `(text, attrs)` pairs in one buffer so adjacent
/// spans with different colors (e.g. accent-colored prompt glyph +
/// text-colored input) lay out as one line without the caller having
/// to position them separately.
fn create_border_buffer_spans(
    font_system: &mut FontSystem,
    spans: &[(&str, Attrs)],
    font_size: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    let mut buf = cosmic_text::Buffer::new(
        font_system,
        cosmic_text::Metrics::new(font_size, font_size),
    );
    buf.set_size(font_system, Some(bounds.0), Some(bounds.1));
    let span_refs: Vec<(&str, Attrs)> =
        spans.iter().map(|(t, a)| (*t, a.clone())).collect();
    buf.set_rich_text(
        font_system,
        span_refs,
        &Attrs::new(),
        cosmic_text::Shaping::Advanced,
        None,
    );
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer { buffer: buf, pos, bounds }
}

/// Like `create_border_buffer` but center-aligns the text within its
/// box via `cosmic_text::Align::Center`. Used for the color picker's
/// crosshair-arm glyphs and hue-ring glyphs: with sacred-script
/// glyphs varying significantly in shaped width (~5 px for Hebrew,
/// ~20 px for Egyptian hieroglyphs at base `font_size`), flush-left
/// positioning would produce a visibly drifting cross and a ring
/// thrown out of round. Center alignment pins each glyph's visual
/// center to the middle of its box, independent of advance width.
fn create_centered_cell_buffer(
    font_system: &mut FontSystem,
    text: &str,
    attrs: &Attrs,
    font_size: f32,
    pos: (f32, f32),
    bounds: (f32, f32),
) -> MindMapTextBuffer {
    let mut buf = cosmic_text::Buffer::new(
        font_system,
        cosmic_text::Metrics::new(font_size, font_size),
    );
    buf.set_size(font_system, Some(bounds.0), Some(bounds.1));
    buf.set_rich_text(
        font_system,
        vec![(text, attrs.clone())],
        &Attrs::new(),
        cosmic_text::Shaping::Advanced,
        Some(cosmic_text::Align::Center),
    );
    buf.shape_until_scroll(font_system, false);
    MindMapTextBuffer { buffer: buf, pos, bounds }
}

fn parse_hex_color(hex: &str) -> Option<cosmic_text::Color> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    Some(cosmic_text::Color::rgba(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
        255,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cull_accepts_center_of_viewport() {
        let vp_min = Vec2::new(0.0, 0.0);
        let vp_max = Vec2::new(100.0, 100.0);
        assert!(glyph_position_in_viewport(50.0, 50.0, vp_min, vp_max, 12.0));
    }

    #[test]
    fn cull_accepts_glyph_just_inside_edge() {
        let vp_min = Vec2::new(0.0, 0.0);
        let vp_max = Vec2::new(100.0, 100.0);
        // Right on the boundary — inclusive on both sides.
        assert!(glyph_position_in_viewport(0.0, 0.0, vp_min, vp_max, 0.0));
        assert!(glyph_position_in_viewport(100.0, 100.0, vp_min, vp_max, 0.0));
    }

    #[test]
    fn cull_rejects_far_off_screen() {
        let vp_min = Vec2::new(0.0, 0.0);
        let vp_max = Vec2::new(100.0, 100.0);
        // Way off to the right, far beyond any reasonable margin.
        assert!(!glyph_position_in_viewport(10_000.0, 50.0, vp_min, vp_max, 12.0));
        assert!(!glyph_position_in_viewport(50.0, 10_000.0, vp_min, vp_max, 12.0));
        assert!(!glyph_position_in_viewport(-10_000.0, 50.0, vp_min, vp_max, 12.0));
        assert!(!glyph_position_in_viewport(50.0, -10_000.0, vp_min, vp_max, 12.0));
    }

    #[test]
    fn cull_margin_extends_visible_rect() {
        let vp_min = Vec2::new(0.0, 0.0);
        let vp_max = Vec2::new(100.0, 100.0);
        // Just outside the rect but within the margin — should be included
        // so there's no visible popping at viewport edges during pan.
        assert!(glyph_position_in_viewport(-10.0, 50.0, vp_min, vp_max, 12.0));
        assert!(glyph_position_in_viewport(110.0, 50.0, vp_min, vp_max, 12.0));
        assert!(glyph_position_in_viewport(50.0, -10.0, vp_min, vp_max, 12.0));
        assert!(glyph_position_in_viewport(50.0, 110.0, vp_min, vp_max, 12.0));
    }

    #[test]
    fn cull_rejects_just_beyond_margin() {
        let vp_min = Vec2::new(0.0, 0.0);
        let vp_max = Vec2::new(100.0, 100.0);
        let margin = 12.0;
        // One epsilon past the padded boundary → excluded.
        assert!(!glyph_position_in_viewport(
            vp_max.x + margin + 0.001,
            50.0,
            vp_min,
            vp_max,
            margin
        ));
        assert!(!glyph_position_in_viewport(
            vp_min.x - margin - 0.001,
            50.0,
            vp_min,
            vp_max,
            margin
        ));
    }

    #[test]
    fn cull_handles_non_origin_viewport() {
        // Viewport not at origin (pan offset).
        let vp_min = Vec2::new(500.0, 1000.0);
        let vp_max = Vec2::new(700.0, 1200.0);
        assert!(glyph_position_in_viewport(600.0, 1100.0, vp_min, vp_max, 12.0));
        assert!(!glyph_position_in_viewport(100.0, 100.0, vp_min, vp_max, 12.0));
    }

    #[test]
    fn cull_kills_most_glyphs_on_a_very_long_edge() {
        // Simulate a Phase 4(A) scenario: a 20,000 canvas-unit connection,
        // sampled every 15 units (default spacing), one endpoint at origin,
        // the other at (20000, 0). Viewport is the first 400x400 canvas
        // units. With font_size=12 margin, we should keep glyphs whose x
        // is in [-12, 412] — roughly 28 of ~1334 samples.
        let vp_min = Vec2::new(0.0, 0.0);
        let vp_max = Vec2::new(400.0, 400.0);
        let margin = 12.0;
        let total = 1334;
        let kept = (0..total)
            .filter(|&i| {
                let x = i as f32 * 15.0;
                glyph_position_in_viewport(x, 0.0, vp_min, vp_max, margin)
            })
            .count();
        // Expect well under 5% retained.
        assert!(kept < total / 20, "kept {} of {}, expected < {}", kept, total, total / 20);
        // And at least a few — it's not zero.
        assert!(kept > 10, "kept {} of {}, expected at least 10", kept, total);
    }

    // ====================================================================
    // Console overlay layout
    // ====================================================================

    fn empty_console_geometry() -> ConsoleOverlayGeometry {
        ConsoleOverlayGeometry {
            input: String::new(),
            cursor_grapheme: 0,
            scrollback: Vec::new(),
            completions: Vec::new(),
            selected_completion: None,
            font_family: String::new(),
            font_size: 16.0,
        }
    }

    fn sample_console_geometry() -> ConsoleOverlayGeometry {
        ConsoleOverlayGeometry {
            input: "anchor set from t".to_string(),
            cursor_grapheme: 17,
            scrollback: vec![
                ConsoleOverlayLine {
                    text: "> help".to_string(),
                    kind: ConsoleOverlayLineKind::Input,
                },
                ConsoleOverlayLine {
                    text: "commands:".to_string(),
                    kind: ConsoleOverlayLineKind::Output,
                },
            ],
            completions: vec![
                ConsoleOverlayCompletion {
                    text: "top".to_string(),
                    hint: None,
                },
            ],
            selected_completion: Some(0),
            font_family: String::new(),
            font_size: 16.0,
        }
    }

    #[test]
    fn test_console_backdrop_matches_border_bounds_exactly() {
        let geometry = sample_console_geometry();
        let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
        let (bd_left, bd_top, bd_w, bd_h) = layout.backdrop_rect();
        assert_eq!(bd_left, layout.left);
        assert_eq!(bd_top, layout.top);
        assert_eq!(bd_w, layout.frame_width);
        assert_eq!(bd_h, layout.frame_height + layout.font_size);
    }

    #[test]
    fn test_console_backdrop_has_no_horizontal_overhang() {
        let geometry = sample_console_geometry();
        let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
        let (bd_left, _, bd_w, _) = layout.backdrop_rect();
        let bd_right = bd_left + bd_w;
        let border_right = layout.left + layout.frame_width;
        assert!(bd_right <= border_right + 0.001);
        assert!(bd_left >= layout.left - 0.001);
    }

    #[test]
    fn test_console_frame_is_bottom_anchored() {
        let geometry = sample_console_geometry();
        let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
        // Bottom border glyph row extends `font_size` below frame_height.
        // Its bottom edge should sit within `inner_padding` of the
        // screen bottom.
        let frame_bottom = layout.top + layout.frame_height + layout.font_size;
        let gap = 1080.0 - frame_bottom;
        assert!(
            gap <= layout.inner_padding + 0.5 && gap >= 0.0,
            "frame not bottom-anchored: gap={gap}"
        );
    }

    #[test]
    fn test_console_frame_height_linear_in_scrollback_rows() {
        let g_empty = empty_console_geometry();
        let mut g_one = empty_console_geometry();
        g_one.scrollback.push(ConsoleOverlayLine {
            text: "one".into(),
            kind: ConsoleOverlayLineKind::Output,
        });
        let mut g_two = g_one.clone();
        g_two.scrollback.push(ConsoleOverlayLine {
            text: "two".into(),
            kind: ConsoleOverlayLineKind::Output,
        });
        let h0 = compute_console_frame_layout(&g_empty, 1920.0, 1080.0).frame_height;
        let h1 = compute_console_frame_layout(&g_one, 1920.0, 1080.0).frame_height;
        let h2 = compute_console_frame_layout(&g_two, 1920.0, 1080.0).frame_height;
        let delta1 = h1 - h0;
        let delta2 = h2 - h1;
        assert!((delta1 - delta2).abs() < 0.01);
    }

    #[test]
    fn test_console_scrollback_clamped_to_max_rows() {
        let mut geometry = empty_console_geometry();
        for i in 0..100 {
            geometry.scrollback.push(ConsoleOverlayLine {
                text: format!("line {i}"),
                kind: ConsoleOverlayLineKind::Output,
            });
        }
        let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
        assert_eq!(layout.scrollback_rows, MAX_CONSOLE_SCROLLBACK_ROWS);
    }

    #[test]
    fn test_console_completions_clamped_to_max_rows() {
        let mut geometry = empty_console_geometry();
        for i in 0..100 {
            geometry.completions.push(ConsoleOverlayCompletion {
                text: format!("cmd_{i}"),
                hint: None,
            });
        }
        let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
        assert_eq!(layout.completion_rows, MAX_CONSOLE_COMPLETION_ROWS);
    }

    #[test]
    fn test_console_frame_is_full_window_width() {
        // The console is a bottom-anchored full-width strip with a
        // small horizontal margin on each side. Frame width + 2 ×
        // margin should sum to roughly the screen width.
        let layout = compute_console_frame_layout(&empty_console_geometry(), 1920.0, 1080.0);
        let total = layout.left * 2.0 + layout.frame_width;
        assert!((total - 1920.0).abs() < 1.0, "frame doesn't span full width");
    }

    #[test]
    fn test_console_frame_width_independent_of_scrollback_len() {
        // With the full-width layout, a long scrollback line cannot
        // push the frame wider — it's clipped by the content area.
        let short = compute_console_frame_layout(&empty_console_geometry(), 1920.0, 1080.0).frame_width;
        let mut huge = empty_console_geometry();
        huge.scrollback.push(ConsoleOverlayLine {
            text: "x".repeat(500),
            kind: ConsoleOverlayLineKind::Output,
        });
        let long = compute_console_frame_layout(&huge, 1920.0, 1080.0).frame_width;
        assert_eq!(short, long);
    }

    #[test]
    fn test_console_frame_width_stable_for_wide_char_scrollback() {
        // Backdrop-vs-border alignment with a wide-char line — the
        // content is truncated by baumhard's `truncate_to_display_width`
        // so it can't blow past the right border, and the frame
        // itself is still the full window width.
        let mut g = empty_console_geometry();
        g.scrollback.push(ConsoleOverlayLine {
            text: "日本語".repeat(200),
            kind: ConsoleOverlayLineKind::Output,
        });
        let layout = compute_console_frame_layout(&g, 1920.0, 1080.0);
        let (bd_left, _, bd_w, _) = layout.backdrop_rect();
        assert_eq!(bd_left, layout.left);
        assert_eq!(bd_w, layout.frame_width);
    }

    // -----------------------------------------------------------------
    // Console border source-string tests
    //
    // The border draw uses baumhard's `BorderGlyphSet::box_drawing_rounded`
    // via `build_console_border_strings(cols, rows)`.
    // -----------------------------------------------------------------

    #[test]
    fn test_console_border_uses_rounded_corners() {
        let (top, bottom, _, _) = build_console_border_strings(10, 4);
        let top_chars: Vec<char> = top.chars().collect();
        let bot_chars: Vec<char> = bottom.chars().collect();
        assert_eq!(top_chars[0], '\u{256D}'); // ╭
        assert_eq!(*top_chars.last().unwrap(), '\u{256E}'); // ╮
        assert_eq!(bot_chars[0], '\u{2570}'); // ╰
        assert_eq!(*bot_chars.last().unwrap(), '\u{256F}'); // ╯
        // Middle chars of the top border are `─`.
        for c in &top_chars[1..top_chars.len() - 1] {
            assert_eq!(*c, '\u{2500}');
        }
    }

    #[test]
    fn test_console_border_top_row_length_matches_cols() {
        // `cols` = total border length including both corners.
        let (top, bottom, _, _) = build_console_border_strings(20, 4);
        assert_eq!(top.chars().count(), 20);
        assert_eq!(bottom.chars().count(), 20);
    }

    #[test]
    fn test_console_border_sides_one_char_per_line() {
        let (_, _, left, right) = build_console_border_strings(10, 5);
        // One `│` per line, newline-separated; 5 lines total.
        assert_eq!(left.lines().count(), 5);
        assert_eq!(right.lines().count(), 5);
        for line in left.lines() {
            assert_eq!(line.chars().count(), 1);
            assert_eq!(line.chars().next().unwrap(), '\u{2502}');
        }
    }

    #[test]
    fn test_console_border_scales_with_cols_and_rows() {
        let (top_narrow, _, left_short, _) = build_console_border_strings(10, 3);
        let (top_wide, _, left_tall, _) = build_console_border_strings(40, 10);
        assert!(top_wide.chars().count() > top_narrow.chars().count());
        assert!(left_tall.lines().count() > left_short.lines().count());
    }

    #[test]
    fn test_console_prompt_y_sits_below_scrollback_and_completions() {
        // Regression guard for the overlap bug where `prompt_y`
        // floated at `frame_height - inner_padding - font_size`,
        // landing ~0.6 · font_size *above* the last completion row
        // instead of below it.
        let mut g = empty_console_geometry();
        g.scrollback = vec![
            ConsoleOverlayLine {
                text: "one".into(),
                kind: ConsoleOverlayLineKind::Output,
            },
            ConsoleOverlayLine {
                text: "two".into(),
                kind: ConsoleOverlayLineKind::Output,
            },
        ];
        g.completions = vec![ConsoleOverlayCompletion {
            text: "help".into(),
            hint: None,
        }];
        g.selected_completion = Some(0);
        let layout = compute_console_frame_layout(&g, 1920.0, 1080.0);

        let content_top = layout.top + layout.font_size + layout.inner_padding;
        let last_completion_end = content_top
            + layout.row_height * (layout.scrollback_rows + layout.completion_rows) as f32;
        assert!(
            layout.prompt_y() >= last_completion_end - 0.01,
            "prompt_y {} overlaps last completion row ending at {}",
            layout.prompt_y(),
            last_completion_end
        );
    }

    #[test]
    fn test_console_prompt_y_fits_inside_frame() {
        // The prompt row plus its padded budget must stay inside
        // `frame_height`; otherwise it renders outside the border.
        let geometry = sample_console_geometry();
        let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
        let prompt_bottom = layout.prompt_y() + layout.font_size * 1.4;
        let frame_bottom = layout.top + layout.frame_height;
        assert!(
            prompt_bottom <= frame_bottom + 0.01,
            "prompt bottom {} overruns frame bottom {}",
            prompt_bottom,
            frame_bottom
        );
    }

    #[test]
    fn test_console_border_fills_full_frame_cols() {
        // The renderer picks `cols = floor(frame_width / char_width)`
        // and calls `build_console_border_strings(cols, rows)`, so
        // the top string always has exactly `cols` glyphs — one per
        // char-width cell.
        let geometry = sample_console_geometry();
        let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
        let cols = (layout.frame_width / layout.char_width).floor() as usize;
        let (top, _, _, _) = build_console_border_strings(cols, 4);
        assert_eq!(top.chars().count(), cols);
    }

    #[test]
    fn test_console_frame_layout_scales_with_font_size() {
        let mut g = empty_console_geometry();
        g.font_size = 8.0;
        let small = compute_console_frame_layout(&g, 1920.0, 1080.0);
        g.font_size = 32.0;
        let large = compute_console_frame_layout(&g, 1920.0, 1080.0);
        assert!(large.font_size > small.font_size);
        assert!(large.row_height > small.row_height);
        assert!(large.frame_height > small.frame_height);
    }

    /// Helpers for the picker mutator tests below.
    fn picker_sample_geometry(
    ) -> crate::application::color_picker::ColorPickerOverlayGeometry {
        crate::application::color_picker::ColorPickerOverlayGeometry {
            target_label: "edge",
            hue_deg: 0.0,
            sat: 1.0,
            val: 1.0,
            preview_hex: "#ff0000".to_string(),
            hex_visible: false,
            max_cell_advance: 16.0,
            max_ring_advance: 24.0,
            measurement_font_size: 16.0,
            size_scale: 1.0,
            center_override: None,
            hovered_hit: None,
        }
    }

    fn picker_glyph_areas_for(
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    ) -> Vec<(usize, GlyphArea)> {
        use crate::application::color_picker::compute_color_picker_layout;
        let layout = compute_color_picker_layout(geometry, 1280.0, 720.0);
        super::picker_glyph_areas(geometry, &layout)
    }

    /// `picker_glyph_areas` must emit channels in strictly
    /// ascending order — Baumhard's `align_child_walks` relies on
    /// this for the §B2 mutator path. Regression guard for any
    /// future band reordering or skipped insertion.
    #[test]
    fn picker_glyph_areas_ascending_channels() {
        let g = picker_sample_geometry();
        let areas = picker_glyph_areas_for(&g);
        for window in areas.windows(2) {
            assert!(
                window[1].0 > window[0].0,
                "channel {} should follow {} strictly, got {} → {}",
                window[0].0,
                window[0].0,
                window[0].0,
                window[1].0,
            );
        }
    }

    /// Hex visibility flips on cursor enter/exit of the backdrop.
    /// The element set must stay stable across that flip — same
    /// channels, same count — so the mutator path can keep using
    /// the same registered tree without unregistering / rebuilding.
    /// When invisible, the hex emits empty text (walker shapes
    /// nothing).
    #[test]
    fn picker_glyph_areas_hex_channel_stable_when_visibility_flips() {
        let mut g = picker_sample_geometry();
        g.hex_visible = false;
        let invisible = picker_glyph_areas_for(&g);
        g.hex_visible = true;
        let visible = picker_glyph_areas_for(&g);
        assert_eq!(
            invisible.len(),
            visible.len(),
            "element count must stay stable across hex visibility"
        );
        let invisible_channels: Vec<usize> =
            invisible.iter().map(|(c, _)| *c).collect();
        let visible_channels: Vec<usize> = visible.iter().map(|(c, _)| *c).collect();
        assert_eq!(invisible_channels, visible_channels);
        // Hex itself: invisible → empty text, visible → hex string.
        let hex_invisible = invisible
            .iter()
            .find(|(c, _)| *c == crate::application::color_picker::PICKER_CHANNEL_HEX)
            .expect("hex channel present");
        assert!(hex_invisible.1.text.is_empty());
        let hex_visible = visible
            .iter()
            .find(|(c, _)| *c == crate::application::color_picker::PICKER_CHANNEL_HEX)
            .expect("hex channel present");
        assert!(hex_visible.1.text.starts_with('#'));
    }

    /// Console round-trip: applying the mutator to a tree built
    /// at state A leaves it byte-identical (per variable field) to
    /// a fresh `build_console_overlay_tree(B)`. Pins the §B2
    /// in-place update path for the keystroke hot path: the
    /// dispatcher in `rebuild_console_overlay_buffers` takes this
    /// branch on every input change frame.
    #[test]
    fn console_mutator_round_trips_to_fresh_build() {
        use baumhard::core::primitives::Applicable;
        use baumhard::gfx_structs::tree::BranchChannel;
        baumhard::font::fonts::init();

        let mut g_a = sample_console_geometry();
        g_a.input = "anchor".into();
        g_a.cursor_grapheme = 6;
        let layout_a = compute_console_frame_layout(&g_a, 1280.0, 720.0);

        let mut g_b = sample_console_geometry();
        g_b.input = "anchor set".into();
        g_b.cursor_grapheme = 10;
        let layout_b = compute_console_frame_layout(&g_b, 1280.0, 720.0);

        // Same scrollback_rows / completion_rows means the
        // structural signature matches and the mutator is sound.
        assert_eq!(layout_a.scrollback_rows, layout_b.scrollback_rows);
        assert_eq!(layout_a.completion_rows, layout_b.completion_rows);

        let mut tree = {
            let mut fs = baumhard::font::fonts::FONT_SYSTEM.write().unwrap();
            build_console_overlay_tree(&g_a, &layout_a, &mut fs)
        };
        let mutator = {
            let mut fs = baumhard::font::fonts::FONT_SYSTEM.write().unwrap();
            build_console_overlay_mutator(&g_b, &layout_b, &mut fs)
        };
        mutator.apply_to(&mut tree);

        let expected = {
            let mut fs = baumhard::font::fonts::FONT_SYSTEM.write().unwrap();
            console_overlay_areas(&g_b, &layout_b, &mut fs)
        };

        let mut got: Vec<(usize, GlyphArea)> = Vec::new();
        for descendant_id in tree.root().descendants(&tree.arena) {
            let node = tree.arena.get(descendant_id).expect("arena node");
            let element = node.get();
            if let Some(area) = element.glyph_area() {
                got.push((element.channel(), area.clone()));
            }
        }

        assert_eq!(got.len(), expected.len(), "post-mutation element count");
        for ((c_got, a_got), (c_exp, a_exp)) in got.iter().zip(expected.iter()) {
            assert_eq!(c_got, c_exp, "channel mismatch");
            assert_eq!(a_got.text, a_exp.text, "text on ch {c_got}");
            assert_eq!(a_got.position, a_exp.position, "position on ch {c_got}");
            assert_eq!(a_got.regions, a_exp.regions, "regions on ch {c_got}");
        }

        // The signature itself must agree across the two layouts
        // (otherwise the dispatcher wouldn't take the mutator
        // branch in the first place).
        assert_eq!(
            console_overlay_signature(&layout_a),
            console_overlay_signature(&layout_b)
        );
    }

    /// Round-trip: applying the mutator to a freshly-built tree
    /// should leave every GlyphArea's variable state matching what
    /// a fresh `picker_glyph_areas` call would emit. Pins the
    /// promise that the §B2 in-place update path produces the same
    /// observable state as a from-scratch rebuild.
    ///
    /// Strategy: build a tree with state A, build a mutator from
    /// state B, apply the mutator, then verify the tree's
    /// per-channel GlyphAreas equal what `picker_glyph_areas(B)`
    /// would have produced.
    #[test]
    fn picker_mutator_round_trips_to_fresh_build() {
        use crate::application::color_picker::{compute_color_picker_layout, PickerHit};
        use baumhard::core::primitives::Applicable;
        use baumhard::gfx_structs::tree::BranchChannel;

        let g_a = picker_sample_geometry();
        let mut g_b = picker_sample_geometry();
        g_b.hue_deg = 120.0;
        g_b.sat = 0.5;
        g_b.val = 0.7;
        g_b.hovered_hit = Some(PickerHit::Hue(3));

        let layout_a = compute_color_picker_layout(&g_a, 1280.0, 720.0);
        let layout_b = compute_color_picker_layout(&g_b, 1280.0, 720.0);

        // Build the picker tree at state A, then apply the mutator
        // computed from state B.
        let mut tree = build_color_picker_overlay_tree(&g_a, &layout_a);
        let mutator = build_color_picker_overlay_mutator(&g_b, &layout_b);
        mutator.apply_to(&mut tree);

        // Fresh build at state B, for comparison.
        let expected = picker_glyph_areas(&g_b, &layout_b);

        // Walk the mutated tree, gather (channel, area) pairs, and
        // compare to `expected`. Since the mutator uses Assign on
        // every variable field, the pairs should match.
        let mut got: Vec<(usize, GlyphArea)> = Vec::new();
        for descendant_id in tree.root().descendants(&tree.arena) {
            let node = tree.arena.get(descendant_id).expect("arena node");
            let element = node.get();
            if let Some(area) = element.glyph_area() {
                got.push((element.channel(), area.clone()));
            }
        }

        assert_eq!(
            got.len(),
            expected.len(),
            "post-mutation tree element count mismatch"
        );
        for ((c_got, a_got), (c_exp, a_exp)) in got.iter().zip(expected.iter()) {
            assert_eq!(c_got, c_exp, "channel mismatch");
            assert_eq!(
                a_got.text, a_exp.text,
                "text mismatch on channel {c_got}"
            );
            assert_eq!(
                a_got.position, a_exp.position,
                "position mismatch on channel {c_got}"
            );
            assert_eq!(
                a_got.render_bounds, a_exp.render_bounds,
                "bounds mismatch on channel {c_got}"
            );
            assert_eq!(
                a_got.scale, a_exp.scale,
                "scale mismatch on channel {c_got}"
            );
            assert_eq!(
                a_got.line_height, a_exp.line_height,
                "line_height mismatch on channel {c_got}"
            );
            assert_eq!(
                a_got.outline, a_exp.outline,
                "outline mismatch on channel {c_got}"
            );
            // Regions equality compares the inner Vec — a single
            // mismatch on any region field (range, font, color)
            // surfaces here.
            assert_eq!(
                a_got.regions, a_exp.regions,
                "regions mismatch on channel {c_got}"
            );
        }
    }
}
