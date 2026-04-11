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
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use baumhard::shaders::shaders::{SHADERS, SHADER_APPLICATION};
use crate::application::baumhard_adapter::to_cosmic_text;
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::loader;
use baumhard::mindmap::border::BorderStyle;
use baumhard::mindmap::scene_builder::{RenderScene, BorderElement, ConnectionElement, PortalElement, PortalRefKey};
use baumhard::mindmap::scene_cache::EdgeKey;
use glam::Vec2;
use std::path::Path;

/// Session 6C: pre-layout palette data handed from the app event
/// loop to the renderer every time the palette state changes. The
/// renderer turns it into cosmic-text buffers in
/// `rebuild_palette_overlay_buffers`. Kept as a plain struct (no
/// rendering primitives) so unit tests can construct one trivially.
pub struct PaletteOverlayGeometry {
    /// Current query text, shown after the `/` prefix on the input
    /// line. Empty when the palette just opened.
    pub query_text: String,
    /// One entry per currently-filtered action, in display order.
    pub rows: Vec<PaletteOverlayRow>,
    /// Which row is highlighted. Index into `rows`.
    pub selected_row: usize,
}

/// One row in the palette overlay — label + description, matching
/// the fields on `PaletteAction`.
pub struct PaletteOverlayRow {
    pub label: String,
    pub description: String,
}

/// Pure-function output of the palette-overlay layout pass. Holds
/// the derived screen-space dimensions for the palette frame so the
/// backdrop rectangle and the border-glyph positions agree exactly.
/// Extracted to a plain struct so unit tests can verify the
/// alignment invariant without constructing a full `Renderer`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaletteFrameLayout {
    pub left: f32,
    pub top: f32,
    pub frame_width: f32,
    pub frame_height: f32,
    pub font_size: f32,
    pub char_width: f32,
    pub row_height: f32,
    pub inner_padding: f32,
    /// How many filtered rows are rendered inside the frame. Clamped
    /// to `MAX_PALETTE_VISIBLE_ROWS`.
    pub shown_rows: usize,
    /// Index of the first row in `PaletteOverlayGeometry.rows` that
    /// is visible inside the frame — scroll-window origin. Always
    /// zero when the total filtered row count fits inside
    /// `shown_rows`; otherwise chosen so `selected_row` stays
    /// roughly centered (see `compute_palette_frame_layout`).
    pub first_visible: usize,
}

/// Maximum number of filtered rows drawn inside the palette frame.
/// With the new ~34 px row height this keeps the modal from
/// dominating the canvas while still showing a useful slice of a
/// long filtered list.
pub const MAX_PALETTE_VISIBLE_ROWS: usize = 8;

impl PaletteFrameLayout {
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
}

/// Selects between Unicode and ASCII rendering for the palette
/// "sacred border" — the decorative border made of repeating words
/// for God in Sanskrit (top: ॐ, the syllable Om) and ancient Hebrew
/// (bottom: אל, the word "El"), replacing the old Unicode box-
/// drawing border. Dual mode because fonts covering Devanagari /
/// Hebrew are not guaranteed (CI images, stripped-down desktops,
/// etc.) and the ASCII path produces a useful fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SacredBorderStyle {
    /// Devanagari + Hebrew. Native default.
    Unicode,
    /// ASCII fallback. Used when `MANDALA_PALETTE_ASCII_BORDER=1`
    /// is set in the environment, and in unit tests that need a
    /// font-independent baseline.
    Ascii,
}

impl SacredBorderStyle {
    /// Read the style from the environment. Defaults to `Unicode`;
    /// set `MANDALA_PALETTE_ASCII_BORDER=1` to force `Ascii`.
    pub fn from_env() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if std::env::var("MANDALA_PALETTE_ASCII_BORDER")
                .map(|v| v == "1")
                .unwrap_or(false)
            {
                return SacredBorderStyle::Ascii;
            }
        }
        SacredBorderStyle::Unicode
    }

    fn top_unit(self) -> &'static str {
        match self {
            SacredBorderStyle::Unicode => "ॐ ",
            SacredBorderStyle::Ascii => " GOD ",
        }
    }

    fn bottom_unit(self) -> &'static str {
        match self {
            SacredBorderStyle::Unicode => "אל ",
            SacredBorderStyle::Ascii => " AUM ",
        }
    }

    fn side_unit(self) -> &'static str {
        match self {
            SacredBorderStyle::Unicode => "·\n",
            SacredBorderStyle::Ascii => ".\n",
        }
    }
}

/// Build the four border strings (top, bottom, left_column,
/// right_column) for the command palette "sacred border". The
/// strings are sized to approximately fill the given frame extent
/// — small over/underfill is cosmetically fine because the opaque
/// palette backdrop rect masks any overshoot and the Devanagari /
/// Hebrew glyphs are decorative rather than load-bearing grid.
///
/// Returns `(top, bottom, left, right)`. Left and right columns
/// are identical strings (same per-line repetition count); the
/// caller places them at different x positions.
pub fn build_sacred_border_strings(
    inner_width_px: f32,
    inner_height_px: f32,
    font_size: f32,
    style: SacredBorderStyle,
) -> (String, String, String, String) {
    // Conservative average glyph width — wider than a typical
    // Latin character so the repetition count leans slightly low
    // (under-fill, masked by the backdrop) rather than high
    // (over-fill, wraps onto a second line and visually doubles
    // up).
    let avg_glyph_width = (font_size * 0.7).max(1.0);
    let top_unit = style.top_unit();
    let bottom_unit = style.bottom_unit();
    let side_unit = style.side_unit();

    let top_unit_chars = top_unit.chars().count().max(1) as f32;
    let bottom_unit_chars = bottom_unit.chars().count().max(1) as f32;

    let top_repetitions =
        ((inner_width_px / (avg_glyph_width * top_unit_chars)).ceil() as usize).max(1);
    let bottom_repetitions =
        ((inner_width_px / (avg_glyph_width * bottom_unit_chars)).ceil() as usize).max(1);

    // Side columns are laid out in `font_size`-tall line-height
    // slots, *not* in ROW_HEIGHT slots — the side buffer is a
    // single cosmic-text buffer with `\n`-separated lines, and
    // each line takes `font_size` of vertical space. Deriving the
    // count from the palette's row height would leave the rail
    // short under the new 34 px ROW_HEIGHT.
    let line_count =
        ((inner_height_px / font_size).ceil() as usize).max(1);

    let top = top_unit.repeat(top_repetitions);
    let bottom = bottom_unit.repeat(bottom_repetitions);
    let side = side_unit.repeat(line_count);
    (top, bottom, side.clone(), side)
}

/// Compute the screen-space layout for the command-palette overlay
/// from a `PaletteOverlayGeometry` and the current screen width.
/// Pure function — no GPU or font-system access. Called by
/// `rebuild_palette_overlay_buffers` to derive positions for the
/// backdrop rect, border glyphs, and row text, and by unit tests to
/// assert the backdrop-vs-border alignment invariant.
pub fn compute_palette_frame_layout(
    geometry: &PaletteOverlayGeometry,
    screen_width: f32,
) -> PaletteFrameLayout {
    // Layout constants, in screen-space pixels. Sized to be
    // legible at a typical desktop DPI without overlaying too much
    // of the map. Kept in sync with the values used inside
    // `rebuild_palette_overlay_buffers`.
    let font_size: f32 = 16.0;
    let char_width = font_size * 0.6;
    let inner_padding: f32 = 8.0;

    // Per-row layout. The label and description are drawn in
    // separate, non-overlapping sub-regions inside each row slot,
    // with a gap below the description so successive rows have
    // clear visual rhythm — the pre-session-6D layout drew the
    // description inside the same `font_size * 1.5 = 24 px` cell
    // as the label, which caused a visible collision.
    //
    // LABEL_LINE   — vertical budget for the label text. Matches
    //                `font_size` since cosmic-text's line box is
    //                ~`font_size` tall at `Metrics::new(fs, fs)`.
    // DESC_SIZE    — font size for the description line.
    // DESC_Y_OFFSET — y offset from the row's top (label baseline
    //                anchor) to the description's top. Hard-coded
    //                rather than derived from `font_size` so the
    //                relationship between label and description is
    //                explicit in the code.
    // ROW_GAP      — empty space below the description before the
    //                next row's label starts.
    // ROW_HEIGHT   — total vertical cost of a single row, the sum
    //                of the three above.
    const LABEL_LINE: f32 = 16.0;
    const DESC_SIZE: f32 = 12.0;
    const DESC_Y_OFFSET: f32 = LABEL_LINE + 4.0;
    const ROW_GAP: f32 = 6.0;
    const ROW_HEIGHT: f32 = LABEL_LINE + DESC_SIZE + ROW_GAP;
    // Query line vertical budget — slightly taller than a label
    // line so the cursor glyph sits clear of the top border.
    let query_line = font_size * 1.4;
    let row_height = ROW_HEIGHT;

    let shown_rows = geometry.rows.len().min(MAX_PALETTE_VISIBLE_ROWS);
    // Frame height is strictly linear in `shown_rows` now — no
    // "+2 row premium" like the old `row_height * (2 + N) + 2*pad`
    // formula. The query line is accounted for exactly once, and
    // each visible filtered row contributes exactly ROW_HEIGHT.
    let frame_height =
        query_line + inner_padding * 2.0 + ROW_HEIGHT * shown_rows as f32;

    // Scroll window: recompute per rebuild from `selected_row` so
    // the highlight stays visible on Up/Down navigation through a
    // filtered list longer than `shown_rows`. Centered-window
    // policy: selection stays roughly mid-frame, pinning to the
    // top or bottom when it runs out of room. Stateless — safe
    // because the palette has no "scroll without moving the
    // selection" gesture (no PageUp/PageDown yet, and filter
    // changes always reset `selected` to 0).
    let total_rows = geometry.rows.len();
    let first_visible = if total_rows <= shown_rows {
        0
    } else {
        let half = shown_rows / 2;
        let sel = geometry.selected_row.min(total_rows.saturating_sub(1));
        if sel < half {
            0
        } else if sel + (shown_rows - half) >= total_rows {
            total_rows - shown_rows
        } else {
            sel - half
        }
    };

    // Adaptive frame width: wrap tightly around the longest
    // visible content row (query line, label, or description),
    // with a minimum so very short content doesn't produce a
    // postage stamp and a maximum so a stray long description
    // doesn't blow the palette across the whole window.
    let query_chars = geometry.query_text.chars().count() + 3; // "/…▌"
    let longest_label = geometry
        .rows
        .iter()
        .skip(first_visible)
        .take(shown_rows)
        .map(|r| r.label.chars().count() + 2) // "▸ "
        .max()
        .unwrap_or(0);
    let longest_desc = geometry
        .rows
        .iter()
        .skip(first_visible)
        .take(shown_rows)
        .map(|r| r.description.chars().count() + 4) // "    "
        .max()
        .unwrap_or(0);
    let content_chars = query_chars.max(longest_label).max(longest_desc);
    let frame_width = ((content_chars as f32 + 2.0) * char_width
        + inner_padding * 2.0
        + char_width * 2.0)
        .clamp(320.0, 720.0);

    // Center horizontally, fixed offset from the top.
    let left = ((screen_width - frame_width) * 0.5).max(0.0);
    let top: f32 = 80.0;

    PaletteFrameLayout {
        left,
        top,
        frame_width,
        frame_height,
        font_size,
        char_width,
        row_height,
        inner_padding,
        shown_rows,
        first_visible,
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
    palette_text_renderer: TextRenderer,
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
    /// Session 6D: when set to `Some((key, text))`, the renderer
    /// substitutes the given text (with a trailing caret glyph) for
    /// whichever label matches `key` during
    /// `rebuild_connection_label_buffers`. Used by inline label edit
    /// mode to preview uncommitted text without mutating the model.
    pub label_edit_override: Option<(EdgeKey, String)>,
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
    palette_overlay_buffers: Vec<MindMapTextBuffer>,
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
    /// `palette_rect_vertices`; the two batches draw separately
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
    palette_rect_vertices: Vec<f32>,
    /// Screen-space geometry of the palette's opaque backdrop.
    /// Captured inside `rebuild_palette_overlay_buffers` so
    /// `render()` can turn it into NDC vertices against the
    /// current viewport size without re-running the layout.
    /// `None` whenever the palette is closed.
    palette_backdrop: Option<(f32, f32, f32, f32)>, // (left, top, width, height)
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
        let palette_text_renderer =
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
            palette_text_renderer,
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
            label_edit_override: None,
            portal_buffers: FxHashMap::default(),
            portal_hitboxes: FxHashMap::default(),
            palette_overlay_buffers: Vec::new(),
            color_picker_static_buffers: Vec::new(),
            color_picker_dynamic_buffers: Vec::new(),
            color_picker_backdrop: None,
            overlay_buffers: Vec::new(),
            connection_viewport_dirty: false,
            connection_geometry_dirty: false,
            rect_pipeline,
            rect_vertex_buffer,
            rect_vertex_buffer_capacity: RECT_VBUF_INITIAL_CAPACITY,
            node_background_rects: Vec::new(),
            main_rect_vertices: Vec::new(),
            palette_rect_vertices: Vec::new(),
            palette_backdrop: None,
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
        self.fps = Some(
            usize::try_from(Duration::from_secs(1).as_micros()).unwrap()
                / usize::try_from(micros).unwrap(),
        );
    }

    #[inline]
    fn calculate_fps(&mut self, delta_time: Duration) {
        self.fps = Some(
            usize::try_from(Duration::from_secs(1).as_micros()).unwrap()
                / usize::try_from(
                    (self.last_render_time
                        + Duration::max(delta_time, Self::ZERO_DURATION.clone()))
                    .as_micros(),
                )
                .unwrap(),
        );
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
            let mut font_system = fonts::FONT_SYSTEM
                .try_write()
                .expect("Failed to acquire font-system write lock");
            editor.insert_string(
                block.text.as_str(),
                Some(to_cosmic_text(&block.regions, &mut font_system)),
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
        let arena_lock = self.graphics_arena.try_read();
        if arena_lock.is_ok() {
            for node in arena_lock.unwrap().iter() {
                if !node.is_removed() {
                    let element = node.get();
                    Self::prepare_glyph_block(
                        element.glyph_area().unwrap(),
                        &element.unique_id(),
                        &mut self.buffer_cache,
                    );
                }
            }
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
        for rect in &self.node_background_rects {
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
        self.palette_rect_vertices.clear();
        if let Some((left, top, w, h)) = self.palette_backdrop {
            let (ndc_min, ndc_max) = Self::screen_rect_to_ndc_bounds(
                left, top, w, h, vp_w_px, vp_h_px,
            );
            // Pitch black. Sits cleanly against the cyan frame and
            // any canvas background without tinting the palette's
            // cyan foreground.
            let bg_color = [0.0, 0.0, 0.0, 1.0];
            Self::push_rect_ndc(
                &mut self.palette_rect_vertices,
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
                &mut self.palette_rect_vertices,
                ndc_min,
                ndc_max,
                bg_color,
            );
        }

        // Upload both batches to the shared rect vertex buffer,
        // growing if the combined size exceeds the current
        // capacity. Layout: `[main_bytes | palette_bytes]`.
        let main_bytes_len = self.main_rect_vertices.len() * std::mem::size_of::<f32>();
        let palette_bytes_len = self.palette_rect_vertices.len() * std::mem::size_of::<f32>();
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
                    self.palette_rect_vertices.as_ptr() as *const u8,
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
        let palette_vertex_count = (self.palette_rect_vertices.len() / 6) as u32;

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
        let palette_text_areas: Vec<TextArea> = self.palette_overlay_buffers.iter()
            .chain(self.color_picker_static_buffers.iter())
            .chain(self.color_picker_dynamic_buffers.iter())
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

        let mut font_system = fonts::FONT_SYSTEM
            .try_write()
            .expect("Failed to acquire font_system lock");

        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut font_system,
                &mut self.atlas,
                &self.viewport,
                main_text_areas,
                &mut self.swash_cache,
            )
            .unwrap();
        self.palette_text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut font_system,
                &mut self.atlas,
                &self.viewport,
                palette_text_areas,
                &mut self.swash_cache,
            )
            .unwrap();
        drop(font_system);

        let frame_result = self.surface.get_current_texture();
        if frame_result.is_err() {
            debug!("Failed to get the surface texture, can't render.");
            return;
        }
        let frame = frame_result.unwrap();
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
            self.text_renderer.render(&self.atlas, &self.viewport, &mut pass).unwrap();

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
            self.palette_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .unwrap();
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

    fn load_mindmap(&mut self, path: &str) {
        match loader::load_from_file(Path::new(path)) {
            Ok(map) => {
                info!("Loaded mindmap '{}' with {} nodes", map.name, map.nodes.len());

                // Nodes via Baumhard tree
                let mindmap_tree = baumhard::mindmap::tree_builder::build_mindmap_tree(&map);
                self.rebuild_buffers_from_tree(&mindmap_tree.tree);
                self.fit_camera_to_tree(&mindmap_tree.tree);

                // Connections + borders via flat scene. `fit_camera_to_tree`
                // ran above, so `self.camera.zoom` is settled and
                // `effective_font_size_pt` will resolve against the final
                // zoom rather than whatever the zoom was before the load.
                let scene = baumhard::mindmap::scene_builder::build_scene(&map, self.camera.zoom);
                self.rebuild_connection_buffers(&scene.connection_elements);
                self.rebuild_border_buffers(&scene.border_elements);

                self.should_render = true;
            }
            Err(e) => {
                error!("Failed to load mindmap: {}", e);
            }
        }
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
        self.camera.fit_to_bounds(
            Vec2::new(min_x, min_y),
            Vec2::new(max_x, max_y),
            0.05,
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

        for descendant_id in tree.root().descendants(&tree.arena) {
            let node = match tree.arena.get(descendant_id) {
                Some(n) => n,
                None => continue,
            };
            let element = node.get();
            let area = match element.glyph_area() {
                Some(a) => a,
                None => continue, // Skip Void and GlyphModel nodes
            };

            // Background rects live on the GlyphArea itself. Even
            // text-empty elements can have a background (a blank
            // colored pad), so collect the rect before the text
            // skip below. Mutations on the tree can mutate this
            // directly via `glyph_area_mut().background_color`.
            if let Some(color) = area.background_color {
                self.node_background_rects.push(NodeBackgroundRect {
                    position: Vec2::new(area.position.x.0, area.position.y.0),
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

            let mut buffer = cosmic_text::Buffer::new(
                &mut font_system,
                cosmic_text::Metrics::new(scale, line_height),
            );
            buffer.set_size(&mut font_system, Some(bound_x), Some(bound_y));
            buffer.set_wrap(&mut font_system, cosmic_text::Wrap::Word);

            // Build spans from ColorFontRegions
            let text = &area.text;
            let spans: Vec<(&str, Attrs)> = if area.regions.num_regions() == 0 {
                vec![(text.as_str(), Attrs::new())]
            } else {
                area.regions.all_regions().iter().filter_map(|region| {
                    let start = grapheme_chad::find_byte_index_of_char(text, region.range.start)
                        .unwrap_or(text.len());
                    let end = grapheme_chad::find_byte_index_of_char(text, region.range.end)
                        .unwrap_or(text.len());
                    if start >= end {
                        return None;
                    }
                    let slice = &text[start..end];
                    let mut attrs = Attrs::new();
                    if let Some(rgba) = region.color {
                        let u8c = baumhard::util::color::convert_f32_to_u8(&rgba);
                        attrs = attrs.color(cosmic_text::Color::rgba(u8c[0], u8c[1], u8c[2], u8c[3]));
                    }
                    attrs = attrs.metrics(cosmic_text::Metrics::new(scale, line_height));
                    Some((slice, attrs))
                }).collect()
            };

            buffer.set_rich_text(
                &mut font_system,
                spans,
                &Attrs::new(),
                cosmic_text::Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut font_system, false);

            let text_buffer = MindMapTextBuffer {
                buffer,
                pos: (area.position.x.0, area.position.y.0),
                bounds: (bound_x, bound_y),
            };
            self.mindmap_buffers.insert(element.unique_id().to_string(), text_buffer);
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
        /// How far the top/bottom border line boxes are pulled inward (toward
        /// the node content) so their glyph visible extents overlap with the
        /// vertical columns' glyph visible extents. Empirically chosen for
        /// LiberationSans at typical border font sizes; larger values visibly
        /// encroach on the node content, smaller values leave gaps.
        const CORNER_OVERLAP_FRAC: f32 = 0.35;

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
            let approx_char_width = font_size * 0.6;
            let char_count = ((nw / approx_char_width) + 2.0)
                .ceil()
                .max(3.0) as usize;
            let right_corner_x =
                nx - approx_char_width + (char_count - 1) as f32 * approx_char_width;
            let corner_overlap = font_size * CORNER_OVERLAP_FRAC;
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

    /// Rebuild the command palette overlay buffers. When
    /// `geometry` is `None`, the palette is closed — clear the
    /// buffer list and return. When `Some`, lay out a glyph-rendered
    /// frame at a fixed screen position: a box-drawing border, a
    /// query line with a trailing cursor, and one row per filtered
    /// action.
    ///
    /// Everything is positioned in screen coordinates (the render
    /// pass draws `palette_overlay_buffers` with `scale = 1.0`), so
    /// the palette stays a fixed size regardless of canvas zoom.
    pub fn rebuild_palette_overlay_buffers(
        &mut self,
        geometry: Option<&PaletteOverlayGeometry>,
    ) {
        self.palette_overlay_buffers.clear();
        // Drop any previously-recorded backdrop — the render pass
        // emits a palette rect only when this is `Some`, so closing
        // the palette also clears the backdrop.
        self.palette_backdrop = None;
        let geometry = match geometry {
            Some(g) => g,
            None => return,
        };

        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        // Compute the screen-space layout via the pure helper so the
        // backdrop rect and the border-glyph positions come from the
        // same source of truth. See `compute_palette_frame_layout`.
        let layout = compute_palette_frame_layout(geometry, self.config.width as f32);
        let PaletteFrameLayout {
            left,
            top,
            frame_width,
            frame_height,
            font_size,
            char_width,
            row_height,
            inner_padding,
            shown_rows,
            first_visible,
        } = layout;

        let palette_color = cosmic_text::Color::rgba(0, 229, 255, 255); // cyan
        let text_color = cosmic_text::Color::rgba(235, 235, 235, 255);
        let dim_color = cosmic_text::Color::rgba(150, 150, 160, 255);
        let selected_color = cosmic_text::Color::rgba(0, 229, 255, 255);

        // Record the backdrop geometry for the rect pipeline.
        // `render()` rebuilds the palette rect batch from this each
        // frame (cheap — one rect) and draws it between the main
        // text pass and the palette text pass, so the fill is truly
        // opaque and no node text bleeds through. The rect matches
        // the border bounds exactly — top border glyphs sit at
        // `y = top`, bottom border glyphs extend down to
        // `y = top + frame_height + font_size`, and the left/right
        // columns span `[left, left + frame_width]` horizontally.
        self.palette_backdrop = Some(layout.backdrop_rect());

        // Sacred border: top row repeats ॐ (Sanskrit "Om"), bottom
        // row repeats אל (Hebrew "El"), left/right columns repeat a
        // neutral dot. Falls back to ASCII if the environment asks
        // for it — see `SacredBorderStyle::from_env`.
        let sacred_style = SacredBorderStyle::from_env();
        let (top_border, bottom_border, left_col, right_col) = build_sacred_border_strings(
            frame_width - char_width * 2.0,
            frame_height,
            font_size,
            sacred_style,
        );

        let border_attrs = Attrs::new()
            .color(palette_color)
            .metrics(cosmic_text::Metrics::new(font_size, font_size));

        self.palette_overlay_buffers.push(create_border_buffer(
            &mut font_system,
            &top_border,
            &border_attrs,
            font_size,
            (left, top),
            (frame_width, font_size * 1.5),
        ));
        self.palette_overlay_buffers.push(create_border_buffer(
            &mut font_system,
            &bottom_border,
            &border_attrs,
            font_size,
            (left, top + frame_height),
            (frame_width, font_size * 1.5),
        ));
        self.palette_overlay_buffers.push(create_border_buffer(
            &mut font_system,
            &left_col,
            &border_attrs,
            font_size,
            (left, top + font_size),
            (char_width, frame_height),
        ));
        self.palette_overlay_buffers.push(create_border_buffer(
            &mut font_system,
            &right_col,
            &border_attrs,
            font_size,
            (left + frame_width - char_width, top + font_size),
            (char_width, frame_height),
        ));

        // Query line: "/query▌" where "▌" is a cursor glyph.
        let query_line_text = format!("/{}\u{258C}", geometry.query_text);
        let query_attrs = Attrs::new()
            .color(text_color)
            .metrics(cosmic_text::Metrics::new(font_size, font_size));
        let query_line_budget = font_size * 1.4;
        self.palette_overlay_buffers.push(create_border_buffer(
            &mut font_system,
            &query_line_text,
            &query_attrs,
            font_size,
            (left + inner_padding + char_width, top + inner_padding),
            (
                frame_width - inner_padding * 2.0 - char_width * 2.0,
                query_line_budget,
            ),
        ));

        // Filtered rows. Iterate over the scroll window (not the
        // full filtered list), so a long list doesn't leave the
        // highlight off-screen. The selected-row check compares
        // against the absolute index into `geometry.rows`, not
        // `i` — the visible index `i` would reset to 0 at
        // `first_visible`, misrepresenting selection on a scrolled
        // list.
        //
        // Row layout per `compute_palette_frame_layout`:
        //
        //     row_y(i) = top + inner_padding + query_line_budget
        //                + ROW_HEIGHT * i
        //     label    drawn at (x, row_y)
        //     description drawn at (x, row_y + DESC_Y_OFFSET)
        //
        // where ROW_HEIGHT = 34 px and DESC_Y_OFFSET = 20 px.
        const DESC_SIZE: f32 = 12.0;
        const DESC_Y_OFFSET: f32 = 20.0;
        let rows_top = top + inner_padding + query_line_budget;
        for (i, row) in geometry
            .rows
            .iter()
            .skip(first_visible)
            .take(shown_rows)
            .enumerate()
        {
            let absolute_index = first_visible + i;
            let is_selected = absolute_index == geometry.selected_row;
            let prefix = if is_selected { "\u{25B8} " } else { "  " };
            let row_attrs = Attrs::new()
                .color(if is_selected { selected_color } else { text_color })
                .metrics(cosmic_text::Metrics::new(font_size, font_size));
            let label_line = format!("{prefix}{}", row.label);
            let row_y = rows_top + row_height * i as f32;
            self.palette_overlay_buffers.push(create_border_buffer(
                &mut font_system,
                &label_line,
                &row_attrs,
                font_size,
                (left + inner_padding + char_width, row_y),
                (
                    frame_width - inner_padding * 2.0 - char_width * 2.0,
                    row_height,
                ),
            ));
            let desc_attrs = Attrs::new()
                .color(dim_color)
                .metrics(cosmic_text::Metrics::new(DESC_SIZE, DESC_SIZE));
            let desc_line = format!("    {}", row.description);
            self.palette_overlay_buffers.push(create_border_buffer(
                &mut font_system,
                &desc_line,
                &desc_attrs,
                DESC_SIZE,
                (
                    left + inner_padding + char_width,
                    row_y + DESC_Y_OFFSET,
                ),
                (
                    frame_width - inner_padding * 2.0 - char_width * 2.0,
                    row_height,
                ),
            ));
        }
    }

    /// Rebuild the glyph-wheel color picker's full overlay —
    /// BOTH the static buffer list (title, hint, hue ring) AND the
    /// dynamic buffer list (sat/val bars, preview, hex, chips,
    /// selection indicator). Called by `open_color_picker` and by
    /// the `Resized` handler; closing the picker with `None` clears
    /// both lists.
    ///
    /// For per-hover updates — where only the dynamic parts change
    /// — call `rebuild_color_picker_dynamic_buffers` instead. That
    /// path skips the static buffers entirely and is the reason
    /// this method exists as a separate entry point at all.
    pub fn rebuild_color_picker_overlay_buffers(
        &mut self,
        geometry: Option<&crate::application::color_picker::ColorPickerOverlayGeometry>,
    ) {
        // Close path clears both halves + drops the backdrop.
        let g = match geometry {
            Some(g) => g,
            None => {
                self.color_picker_static_buffers.clear();
                self.color_picker_dynamic_buffers.clear();
                self.color_picker_backdrop = None;
                return;
            }
        };
        self.rebuild_color_picker_static_buffers(g);
        self.rebuild_color_picker_dynamic_buffers(g);
    }

    /// Rebuild just the static part of the picker overlay: the
    /// title bar, the hint footer, and the 24-slot hue ring. These
    /// don't depend on the current HSV or chip focus, so they only
    /// need to be reshaped on open (with the current window
    /// dimensions) and on resize. Every hover reuses these buffers
    /// untouched, saving ~26 shape calls per frame of hover.
    ///
    /// Also captures the backdrop rect — the layout pass is
    /// deterministic, so the backdrop ends up at the same
    /// coordinates whether the static or the dynamic pass computes
    /// it. Callers that invoke this method always follow with
    /// `rebuild_color_picker_dynamic_buffers` using the same
    /// geometry.
    fn rebuild_color_picker_static_buffers(
        &mut self,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    ) {
        use crate::application::color_picker::{
            compute_color_picker_layout, hue_slot_to_degrees, HUE_RING_GLYPHS, HUE_SLOT_COUNT,
        };
        use baumhard::util::color::hsv_to_rgb;

        self.color_picker_static_buffers.clear();

        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        let layout = compute_color_picker_layout(
            geometry,
            self.config.width as f32,
            self.config.height as f32,
        );
        self.color_picker_backdrop = Some(layout.backdrop);

        let font_size = layout.font_size;
        let ring_font_size = layout.ring_font_size;
        let ring_glyph_box = (ring_font_size * 1.5, ring_font_size * 1.5);

        // ---- Title bar ----
        let title_text = format!("\u{0950} {} color", geometry.target_label);
        let title_attrs = Attrs::new()
            .color(cosmic_text::Color::rgba(0, 229, 255, 255))
            .metrics(cosmic_text::Metrics::new(font_size, font_size));
        self.color_picker_static_buffers.push(create_border_buffer(
            &mut font_system,
            &title_text,
            &title_attrs,
            font_size,
            layout.title_pos,
            (font_size * 24.0, font_size * 1.5),
        ));

        // ---- Hue ring (24 sacred-script glyphs) — three 8-glyph
        // arcs clockwise from 12 o'clock: Devanagari, Hebrew, Tibetan.
        // Each slot renders at HUE_RING_FONT_SCALE × base font size
        // so the ring reads as the dominant visual element. The
        // dynamic pass later overlays a cyan outline ring ◯ on top
        // of the currently-selected slot, so we don't need to
        // rebuild the hue ring when the selection moves.
        for i in 0..HUE_SLOT_COUNT {
            let hue = hue_slot_to_degrees(i);
            let rgb = hsv_to_rgb(hue, 1.0, 1.0);
            let cosmic_color = rgb_to_cosmic_color(rgb);
            let attrs = Attrs::new()
                .color(cosmic_color)
                .metrics(cosmic_text::Metrics::new(ring_font_size, ring_font_size));
            let pos = layout.hue_slot_positions[i];
            self.color_picker_static_buffers.push(create_border_buffer(
                &mut font_system,
                HUE_RING_GLYPHS[i],
                &attrs,
                ring_font_size,
                (pos.0 - ring_font_size * 0.5, pos.1 - ring_font_size * 0.5),
                ring_glyph_box,
            ));
        }

        // ---- Hint footer ----
        let hint_text =
            "Esc cancel  \u{00B7}  Enter commit  \u{00B7}  h/s/v nudge  \u{00B7}  Tab chips";
        let hint_attrs = Attrs::new()
            .color(cosmic_text::Color::rgba(140, 140, 150, 255))
            .metrics(cosmic_text::Metrics::new(font_size * 0.85, font_size * 0.85));
        self.color_picker_static_buffers.push(create_border_buffer(
            &mut font_system,
            hint_text,
            &hint_attrs,
            font_size * 0.85,
            layout.hint_pos,
            (font_size * 30.0, font_size * 1.5),
        ));
    }

    /// Rebuild the dynamic (per-hover) part of the picker overlay:
    /// the crosshair sat and val bars (re-colored at the current
    /// hue+val and hue+sat respectively), the center preview glyph,
    /// the hex readout, the chip row (with the focus arrow), and
    /// the selected-hue-slot outline ring. Called on every cursor
    /// move inside the picker and on every h/s/v/Tab keystroke.
    ///
    /// Safe to call without `rebuild_color_picker_static_buffers`
    /// having been called first — cosmic-text buffers are
    /// independent and the render pass draws both lists in one
    /// glyphon pass. In practice callers always invoke the static
    /// rebuild first (via `rebuild_color_picker_overlay_buffers`
    /// from `open_color_picker`) so the two lists are populated in
    /// lockstep.
    pub fn rebuild_color_picker_dynamic_buffers(
        &mut self,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    ) {
        use crate::application::color_picker::{
            compute_color_picker_layout, sat_cell_to_value, val_cell_to_value,
            ARM_BOTTOM_GLYPHS, ARM_LEFT_GLYPHS, ARM_RIGHT_GLYPHS, ARM_TOP_GLYPHS,
            CENTER_PREVIEW_GLYPH, CROSSHAIR_CENTER_CELL, HUE_SLOT_COUNT, SAT_CELL_COUNT,
            THEME_CHIPS, VAL_CELL_COUNT,
        };
        use baumhard::util::color::{hsv_to_hex, hsv_to_rgb};

        self.color_picker_dynamic_buffers.clear();

        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        let layout = compute_color_picker_layout(
            geometry,
            self.config.width as f32,
            self.config.height as f32,
        );

        let font_size = layout.font_size;
        let char_width = layout.char_width;
        let ring_font_size = layout.ring_font_size;
        let glyph_box = (font_size * 1.5, font_size * 1.5);
        let ring_glyph_box = (ring_font_size * 1.5, ring_font_size * 1.5);

        // ---- Saturation crosshair bar (horizontal) ----
        // Each cell shows the color at (current_hue, cell_sat, current_val)
        // so the bar acts as a live "what does this saturation look
        // like for the chosen hue?" preview. Cell CROSSHAIR_CENTER_CELL
        // is the wheel center and is NOT rendered as a bar cell — the
        // ॐ glyph drawn below occupies that position.
        let current_sat_cell = (geometry.sat * (SAT_CELL_COUNT as f32 - 1.0)).round() as usize;
        let current_val_cell =
            ((1.0 - geometry.val) * (VAL_CELL_COUNT as f32 - 1.0)).round() as usize;
        for i in 0..SAT_CELL_COUNT {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            let cell_sat = sat_cell_to_value(i);
            let base_rgb = hsv_to_rgb(geometry.hue_deg, cell_sat, geometry.val);
            // Selected cell gets a tint-up toward cyan so it pops out
            // against its arm-mate cells — replaces the earlier
            // ■ → ◆ glyph-swap highlight which doesn't translate to
            // sacred-script glyphs (we'd lose the per-cell script
            // identity if we swapped glyphs).
            let cosmic_color = if i == current_sat_cell {
                highlight_selected_cell_color(base_rgb)
            } else {
                rgb_to_cosmic_color(base_rgb)
            };
            let glyph = if i < CROSSHAIR_CENTER_CELL {
                ARM_LEFT_GLYPHS[i]
            } else {
                ARM_RIGHT_GLYPHS[i - CROSSHAIR_CENTER_CELL - 1]
            };
            let attrs = Attrs::new()
                .color(cosmic_color)
                .metrics(cosmic_text::Metrics::new(font_size, font_size));
            let (cx, cy) = layout.sat_cell_positions[i];
            self.color_picker_dynamic_buffers.push(create_border_buffer(
                &mut font_system,
                glyph,
                &attrs,
                font_size,
                (cx - char_width * 0.5, cy - font_size * 0.5),
                glyph_box,
            ));
        }

        // ---- Value crosshair bar (vertical) ----
        for i in 0..VAL_CELL_COUNT {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            let cell_val = val_cell_to_value(i);
            let base_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, cell_val);
            let cosmic_color = if i == current_val_cell {
                highlight_selected_cell_color(base_rgb)
            } else {
                rgb_to_cosmic_color(base_rgb)
            };
            let glyph = if i < CROSSHAIR_CENTER_CELL {
                ARM_TOP_GLYPHS[i]
            } else {
                ARM_BOTTOM_GLYPHS[i - CROSSHAIR_CENTER_CELL - 1]
            };
            let attrs = Attrs::new()
                .color(cosmic_color)
                .metrics(cosmic_text::Metrics::new(font_size, font_size));
            let (cx, cy) = layout.val_cell_positions[i];
            self.color_picker_dynamic_buffers.push(create_border_buffer(
                &mut font_system,
                glyph,
                &attrs,
                font_size,
                (cx - char_width * 0.5, cy - font_size * 0.5),
                glyph_box,
            ));
        }

        // ---- Selected hue slot indicator ----
        // A cyan outline circle ◯ drawn on top of the static hue
        // ring's sacred-script glyph at the currently-picked slot.
        // ◯ is hollow, so the user can still read the hue-colored
        // letter through the indicator ring. Font size matches the
        // ring's HUE_RING_FONT_SCALE so the ring encircles the
        // glyph rather than sitting inside it.
        let current_hue_slot = ((geometry.hue_deg.rem_euclid(360.0) / 360.0)
            * HUE_SLOT_COUNT as f32)
            .round() as usize
            % HUE_SLOT_COUNT;
        let indicator_attrs = Attrs::new()
            .color(cosmic_text::Color::rgba(0, 229, 255, 255))
            .metrics(cosmic_text::Metrics::new(ring_font_size, ring_font_size));
        let slot_pos = layout.hue_slot_positions[current_hue_slot];
        self.color_picker_dynamic_buffers.push(create_border_buffer(
            &mut font_system,
            "\u{25EF}",
            &indicator_attrs,
            ring_font_size,
            (slot_pos.0 - ring_font_size * 0.5, slot_pos.1 - ring_font_size * 0.5),
            ring_glyph_box,
        ));

        // ---- Center preview glyph ॐ at 2× font size ----
        // The position and size both come from the layout — the
        // pre-render layout pass owns the centering math, so the
        // glyph anchors correctly even if we tweak preview size.
        let preview_size = layout.preview_size;
        let preview_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, geometry.val);
        let preview_color = rgb_to_cosmic_color(preview_rgb);
        let preview_attrs = Attrs::new()
            .color(preview_color)
            .metrics(cosmic_text::Metrics::new(preview_size, preview_size));
        self.color_picker_dynamic_buffers.push(create_border_buffer(
            &mut font_system,
            CENTER_PREVIEW_GLYPH,
            &preview_attrs,
            preview_size,
            layout.preview_pos,
            (preview_size * 1.5, preview_size * 1.5),
        ));

        // ---- Preview hex readout (small, below the chip row, only
        // when geometry.hex_visible) ----
        // The readout is hidden by default; the layout fn sets
        // `hex_pos` to `Some(..)` when `geometry.hex_visible` is true
        // (cursor inside backdrop or chip focused), `None` otherwise.
        // Anchored horizontally centered on the wheel center, below
        // the theme chip row.
        if let Some(hex_anchor) = layout.hex_pos {
            let hex_text = hsv_to_hex(geometry.hue_deg, geometry.sat, geometry.val);
            let hex_attrs = Attrs::new()
                .color(cosmic_text::Color::rgba(220, 220, 220, 255))
                .metrics(cosmic_text::Metrics::new(font_size, font_size));
            self.color_picker_dynamic_buffers.push(create_border_buffer(
                &mut font_system,
                &hex_text,
                &hex_attrs,
                font_size,
                hex_anchor,
                (font_size * 8.0, font_size * 1.5),
            ));
        }

        // ---- Theme chips row ----
        // Each chip looks like "▸ --accent" if focused, "  --accent"
        // otherwise. Focused chips render in cyan to signal
        // keyboard focus; unfocused chips use a dim text color.
        // (A per-chip swatch color preview — resolving the chip's
        // `var(--name)` reference through the canvas theme map — is
        // deferred. The center preview glyph already shows the
        // currently-picked color, which covers the common case.)
        for (i, chip) in THEME_CHIPS.iter().enumerate() {
            let focused = geometry.chip_focus == Some(i);
            let prefix = if focused { "\u{25B8} " } else { "  " };
            let label = format!("{prefix}{}", chip.label);
            let chip_color = if focused {
                cosmic_text::Color::rgba(0, 229, 255, 255)
            } else {
                cosmic_text::Color::rgba(200, 200, 200, 255)
            };
            let attrs = Attrs::new()
                .color(chip_color)
                .metrics(cosmic_text::Metrics::new(font_size, font_size));
            let (cx, cy, cw) = layout.chip_positions[i];
            self.color_picker_dynamic_buffers.push(create_border_buffer(
                &mut font_system,
                &label,
                &attrs,
                font_size,
                (cx, cy),
                (cw, layout.chip_height),
            ));
        }
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
    /// If `self.label_edit_override` is `Some((key, text))`, the
    /// matching edge's label is drawn from `text` (with a trailing
    /// caret) instead of the element's committed text — the inline
    /// edit preview. The override is consulted only by the buffer
    /// rebuild; the AABB is still computed from the scene element's
    /// bounds, so the hitbox stays stable across keystrokes.
    pub fn rebuild_connection_label_buffers(
        &mut self,
        label_elements: &[baumhard::mindmap::scene_builder::ConnectionLabelElement],
    ) {
        self.connection_label_buffers.clear();
        self.connection_label_hitboxes.clear();
        if label_elements.is_empty() && self.label_edit_override.is_none() {
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

            // Preview override (inline label editor): substitute the
            // edited buffer text + caret for whichever edge is being
            // edited right now.
            let rendered_text: String = match self.label_edit_override.as_ref() {
                Some((key, text)) if *key == elem.edge_key => {
                    format!("{text}\u{258C}")
                }
                _ => elem.text.clone(),
            };

            let buffer = create_border_buffer(
                &mut font_system,
                &rendered_text,
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

        // If an override points at an edge whose label element isn't
        // in the scene (e.g. the user is typing the very first
        // character of a brand-new label whose committed text is
        // still empty), synthesize a preview buffer at the edge
        // anchor so the caret is visible while typing. The app is
        // responsible for passing a scene that includes a
        // ConnectionLabelElement for the edited edge once the buffer
        // is non-empty; this branch is a belt-and-suspenders guard.
        if let Some((key, text)) = self.label_edit_override.as_ref() {
            if !self.connection_label_buffers.contains_key(key) {
                // No scene element to anchor to — do nothing. Callers
                // ensure the scene is rebuilt after opening the
                // editor so this branch is exercised only on the
                // first keystroke before the next scene build.
                let _ = (text,);
            }
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

    /// Session 6E: (re)build the per-portal-marker cosmic-text
    /// buffers from `scene.portal_elements`. Mirrors
    /// `rebuild_connection_label_buffers` byte-for-byte: clear both
    /// keyed maps, iterate the scene elements, build a buffer per
    /// marker at the element's position and color, and record an
    /// AABB hitbox keyed by `(portal_ref, endpoint_node_id)` so
    /// `hit_test_portal` can resolve clicks back to a
    /// `PortalRefKey`.
    ///
    /// Portals are cheap: ≤ 2 markers per pair and portal counts
    /// stay in the dozens, so there is no incremental-reuse cache
    /// and the full map is rebuilt on every scene build.
    pub fn rebuild_portal_buffers(
        &mut self,
        portal_elements: &[PortalElement],
    ) {
        self.portal_buffers.clear();
        self.portal_hitboxes.clear();
        if portal_elements.is_empty() {
            return;
        }
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        for elem in portal_elements {
            let cosmic_color = parse_hex_color(&elem.color)
                .unwrap_or(cosmic_text::Color::rgba(235, 235, 235, 255));
            let attrs = Attrs::new()
                .color(cosmic_color)
                .metrics(cosmic_text::Metrics::new(elem.font_size_pt, elem.font_size_pt));

            let buffer = create_border_buffer(
                &mut font_system,
                &elem.glyph,
                &attrs,
                elem.font_size_pt,
                elem.position,
                elem.bounds,
            );
            let key = (elem.portal_ref.clone(), elem.endpoint_node_id.clone());
            self.portal_buffers.insert(key.clone(), buffer);

            let min = Vec2::new(elem.position.0, elem.position.1);
            let max = Vec2::new(
                elem.position.0 + elem.bounds.0,
                elem.position.1 + elem.bounds.1,
            );
            self.portal_hitboxes.insert(key, (min, max));
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
            self.camera.fit_to_bounds(
                Vec2::new(min_x, min_y),
                Vec2::new(max_x, max_y),
                0.05,
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
                self.camera.pan(Vec2::new(dx, dy));
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
                self.camera.zoom_at(Vec2::new(screen_x, screen_y), factor);
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
            RenderDecree::LoadMindMap(path) => {
                self.load_mindmap(&path);
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
        None,
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
    // Phase 0 — Command palette overlay polish (Session 6D bug fixes)
    // ====================================================================

    fn sample_palette_geometry() -> PaletteOverlayGeometry {
        PaletteOverlayGeometry {
            query_text: "hello".to_string(),
            rows: vec![
                PaletteOverlayRow {
                    label: "Reset connection to straight".to_string(),
                    description: "Remove all control points".to_string(),
                },
                PaletteOverlayRow {
                    label: "Set from-anchor: Top".to_string(),
                    description: "Attach the source of the edge to the top".to_string(),
                },
            ],
            selected_row: 0,
        }
    }

    #[test]
    fn palette_backdrop_matches_border_bounds_exactly() {
        // Bug fix: before Session 6D, the backdrop rect was inflated
        // by `char_width` on each horizontal side and by ~(font_size - 2.0)
        // below the bottom border, so the opaque background leaked out
        // past the cyan border. After the fix the backdrop must match
        // the border bounds exactly — no horizontal overhang, no
        // vertical overhang.
        let geometry = sample_palette_geometry();
        let layout = compute_palette_frame_layout(&geometry, 1920.0);

        let (bd_left, bd_top, bd_w, bd_h) = layout.backdrop_rect();

        // Left edge aligned with the border's left column.
        assert_eq!(bd_left, layout.left);
        // Top edge aligned with the top border row.
        assert_eq!(bd_top, layout.top);
        // Width matches the border frame width exactly — no horizontal overhang.
        assert_eq!(bd_w, layout.frame_width);
        // Height covers the whole border box: frame_height for the
        // interior + one font_size row for the bottom border glyphs.
        assert_eq!(bd_h, layout.frame_height + layout.font_size);
    }

    #[test]
    fn palette_backdrop_has_no_horizontal_overhang() {
        // Explicit regression guard for the horizontal overhang bug.
        let geometry = sample_palette_geometry();
        let layout = compute_palette_frame_layout(&geometry, 1920.0);
        let (bd_left, _, bd_w, _) = layout.backdrop_rect();
        // Backdrop right edge.
        let bd_right = bd_left + bd_w;
        // Border right edge (rightmost column of border glyphs).
        let border_right = layout.left + layout.frame_width;
        assert!(
            bd_right <= border_right + 0.001,
            "backdrop right {} overhangs border right {}",
            bd_right,
            border_right
        );
        assert!(
            bd_left >= layout.left - 0.001,
            "backdrop left {} overhangs border left {}",
            bd_left,
            layout.left
        );
    }

    #[test]
    fn palette_backdrop_has_no_vertical_overhang() {
        // Explicit regression guard for the vertical overhang bug.
        let geometry = sample_palette_geometry();
        let layout = compute_palette_frame_layout(&geometry, 1920.0);
        let (_, bd_top, _, bd_h) = layout.backdrop_rect();
        let bd_bottom = bd_top + bd_h;
        // The bottom border is drawn at `y = top + frame_height` and its
        // glyphs extend down by one font_size row.
        let border_bottom = layout.top + layout.frame_height + layout.font_size;
        assert!(
            bd_bottom <= border_bottom + 0.001,
            "backdrop bottom {} overhangs border bottom {}",
            bd_bottom,
            border_bottom
        );
        assert!(
            bd_top >= layout.top - 0.001,
            "backdrop top {} overhangs border top {}",
            bd_top,
            layout.top
        );
    }

    #[test]
    fn palette_frame_layout_clamps_width_between_min_and_max() {
        // A tiny query and zero rows should still produce at least the
        // minimum frame width.
        let empty = PaletteOverlayGeometry {
            query_text: String::new(),
            rows: Vec::new(),
            selected_row: 0,
        };
        let min_layout = compute_palette_frame_layout(&empty, 1920.0);
        assert!(
            min_layout.frame_width >= 320.0,
            "frame_width {} below min 320",
            min_layout.frame_width
        );

        // A gigantic description should cap out at the max width.
        let huge = PaletteOverlayGeometry {
            query_text: String::new(),
            rows: vec![PaletteOverlayRow {
                label: "x".to_string(),
                description: "y".repeat(500),
            }],
            selected_row: 0,
        };
        let max_layout = compute_palette_frame_layout(&huge, 1920.0);
        assert!(
            max_layout.frame_width <= 720.0,
            "frame_width {} above max 720",
            max_layout.frame_width
        );
    }

    #[test]
    fn palette_frame_layout_centers_horizontally() {
        let geometry = sample_palette_geometry();
        let layout = compute_palette_frame_layout(&geometry, 1920.0);
        // The frame should be centered: distance from screen-left to
        // frame-left equals distance from frame-right to screen-right.
        let right_margin = 1920.0 - (layout.left + layout.frame_width);
        assert!(
            (layout.left - right_margin).abs() < 0.5,
            "frame not centered: left={} right_margin={}",
            layout.left,
            right_margin
        );
    }

    // -----------------------------------------------------------------
    // Scroll-window tests
    //
    // `compute_palette_frame_layout` is stateless — it derives
    // `first_visible` from `selected_row` every rebuild. These tests
    // lock in the centered-window clamp policy so a selection past
    // `MAX_PALETTE_VISIBLE_ROWS` doesn't leave the highlight off-
    // screen and so the window pins cleanly at both ends.
    // -----------------------------------------------------------------

    fn many_row_geometry(total: usize, selected: usize) -> PaletteOverlayGeometry {
        PaletteOverlayGeometry {
            query_text: String::new(),
            rows: (0..total)
                .map(|i| PaletteOverlayRow {
                    label: format!("label_{i}"),
                    description: format!("desc_{i}"),
                })
                .collect(),
            selected_row: selected,
        }
    }

    /// With more rows than `MAX_PALETTE_VISIBLE_ROWS` and a selection
    /// past the visible count, the scroll window must advance so the
    /// selected row is inside `[first_visible, first_visible +
    /// shown_rows)`.
    #[test]
    fn palette_scroll_window_follows_selection() {
        let geometry = many_row_geometry(15, 12);
        let layout = compute_palette_frame_layout(&geometry, 1920.0);
        assert_eq!(layout.shown_rows, MAX_PALETTE_VISIBLE_ROWS);
        let top = layout.first_visible;
        let bottom = top + layout.shown_rows;
        assert!(
            top <= 12 && 12 < bottom,
            "selection 12 not visible: window [{top}, {bottom})"
        );
    }

    /// With a selection near the top of a long filtered list, the
    /// window must pin to the top — `first_visible == 0`. Otherwise
    /// centering would leave empty rows above the content.
    #[test]
    fn palette_scroll_window_pins_to_top_when_selection_low() {
        let geometry = many_row_geometry(15, 0);
        let layout = compute_palette_frame_layout(&geometry, 1920.0);
        assert_eq!(layout.first_visible, 0);
        assert_eq!(layout.shown_rows, MAX_PALETTE_VISIBLE_ROWS);
    }

    /// With a selection at the bottom of a long filtered list, the
    /// window must pin to the bottom — `first_visible + shown_rows
    /// == total_rows`. Otherwise centering would overshoot and
    /// leave empty rows below the content.
    #[test]
    fn palette_scroll_window_pins_to_bottom_when_selection_high() {
        let geometry = many_row_geometry(15, 14);
        let layout = compute_palette_frame_layout(&geometry, 1920.0);
        assert_eq!(
            layout.first_visible + layout.shown_rows,
            15,
            "window did not pin to bottom: first_visible={}, shown_rows={}",
            layout.first_visible,
            layout.shown_rows
        );
    }

    /// With fewer rows than `shown_rows`, `first_visible` must be 0
    /// and `shown_rows == total_rows` (no scroll window needed).
    #[test]
    fn palette_scroll_window_no_clamp_when_total_fits() {
        let geometry = many_row_geometry(3, 2);
        let layout = compute_palette_frame_layout(&geometry, 1920.0);
        assert_eq!(layout.first_visible, 0);
        assert_eq!(layout.shown_rows, 3);
    }

    /// New frame height formula is strictly linear in
    /// `shown_rows` — no hidden "+2 row premium" like the old
    /// `row_height * (2.0 + shown_rows)` math. Guards against
    /// accidentally reintroducing the wasted bottom space.
    #[test]
    fn palette_frame_height_is_linear_in_visible_rows() {
        let one = many_row_geometry(1, 0);
        let two = many_row_geometry(2, 0);
        let three = many_row_geometry(3, 0);
        let h1 = compute_palette_frame_layout(&one, 1920.0).frame_height;
        let h2 = compute_palette_frame_layout(&two, 1920.0).frame_height;
        let h3 = compute_palette_frame_layout(&three, 1920.0).frame_height;
        // Each extra row adds exactly one ROW_HEIGHT (34 px).
        let delta_21 = h2 - h1;
        let delta_32 = h3 - h2;
        assert!(
            (delta_21 - delta_32).abs() < 0.01,
            "non-linear row delta: 2-1={delta_21}, 3-2={delta_32}"
        );
        assert!(
            (delta_21 - 34.0).abs() < 0.01,
            "expected row delta ~34 px, got {delta_21}"
        );
    }

    // -----------------------------------------------------------------
    // Sacred border source-string tests
    //
    // Assertions only check the *source strings* produced by the
    // helper, never the shaped glyph output. This keeps the tests
    // green in CI environments that lack Devanagari / Hebrew fonts —
    // the shaper will render .notdef tofu there, but the source
    // strings themselves are stable.
    // -----------------------------------------------------------------

    #[test]
    fn palette_sacred_border_unicode_contains_devanagari_and_hebrew() {
        let (top, bottom, left, right) = build_sacred_border_strings(
            400.0, 320.0, 16.0, SacredBorderStyle::Unicode,
        );
        // Top border contains the Om syllable (U+0950).
        assert!(
            top.contains('\u{0950}'),
            "top missing Devanagari Om (U+0950): {top:?}"
        );
        // Bottom border contains Hebrew aleph (U+05D0) and lamed (U+05DC).
        assert!(
            bottom.contains('\u{05D0}') && bottom.contains('\u{05DC}'),
            "bottom missing Hebrew El (אל): {bottom:?}"
        );
        // Side columns contain the neutral middle-dot rail.
        assert!(left.contains('\u{00B7}'), "left missing dot rail: {left:?}");
        assert!(right.contains('\u{00B7}'), "right missing dot rail: {right:?}");
        // Side columns are newline-separated, so their length should
        // scale with the inner height.
        let newline_count = left.chars().filter(|c| *c == '\n').count();
        assert!(
            newline_count >= 1,
            "side column has no newlines: {left:?}"
        );
    }

    #[test]
    fn palette_sacred_border_ascii_uses_ascii_only() {
        let (top, bottom, left, right) = build_sacred_border_strings(
            400.0, 320.0, 16.0, SacredBorderStyle::Ascii,
        );
        assert!(top.contains("GOD"), "ascii top missing GOD: {top:?}");
        assert!(bottom.contains("AUM"), "ascii bottom missing AUM: {bottom:?}");
        // All characters must be ASCII (no Devanagari / Hebrew bleed-
        // through from the wrong style).
        for c in top.chars().chain(bottom.chars()).chain(left.chars()).chain(right.chars()) {
            assert!(
                c.is_ascii() || c == '\n',
                "ascii border contains non-ASCII {c:?}"
            );
        }
    }

    #[test]
    fn palette_sacred_border_scales_with_width_and_height() {
        let (top_narrow, _, left_short, _) =
            build_sacred_border_strings(100.0, 100.0, 16.0, SacredBorderStyle::Unicode);
        let (top_wide, _, left_tall, _) =
            build_sacred_border_strings(800.0, 400.0, 16.0, SacredBorderStyle::Unicode);
        assert!(
            top_wide.chars().count() > top_narrow.chars().count(),
            "wide border not longer than narrow: {} vs {}",
            top_wide.chars().count(),
            top_narrow.chars().count()
        );
        let short_lines = left_short.chars().filter(|c| *c == '\n').count();
        let tall_lines = left_tall.chars().filter(|c| *c == '\n').count();
        assert!(
            tall_lines > short_lines,
            "tall side column not longer than short: {tall_lines} vs {short_lines}"
        );
    }
}
