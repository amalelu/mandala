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
use baumhard::mindmap::scene_builder::{RenderScene, BorderElement, ConnectionElement};
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
    /// Session 6C: command palette overlay buffers. Rendered above
    /// everything else in screen coordinates. Populated only when
    /// the palette is open; cleared otherwise.
    palette_overlay_buffers: Vec<MindMapTextBuffer>,
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
        let viewport = Viewport::new(&device, &glyphon_cache);
        let camera = Camera2D::new(size.width, size.height);
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
            palette_overlay_buffers: Vec::new(),
            overlay_buffers: Vec::new(),
            connection_viewport_dirty: false,
            connection_geometry_dirty: false,
        }
    }

    /// Current camera zoom level, used by the event loop when it needs
    /// to pass the active zoom into `Document::build_scene*` (the scene
    /// builder consumes it via
    /// `GlyphConnectionConfig::effective_font_size_pt`).
    pub fn camera_zoom(&self) -> f32 {
        self.camera.zoom
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
        let vp_w = self.config.width as i32;
        let vp_h = self.config.height as i32;
        let vp_bounds = TextBounds { left: 0, top: 0, right: vp_w, bottom: vp_h };
        let default_color = cosmic_text::Color::rgba(255, 255, 255, 255);

        // Collect all camera-transformed mindmap + border + connection buffers with viewport culling
        let mut text_areas: Vec<TextArea> = self.mindmap_buffers.values()
            .chain(self.border_buffers.values().flat_map(|v| v.iter()))
            .chain(self.connection_buffers.values().flat_map(|v| v.iter()))
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
            text_areas.push(TextArea {
                buffer: text_buffer.buffer(),
                left: text_buffer.pos.0,
                top: text_buffer.pos.1,
                scale: 1.0,
                bounds: vp_bounds,
                default_color,
                custom_glyphs: &[],
            });
        }

        // Palette overlay (screen-space, no camera transform) —
        // drawn last so it sits on top of the mindmap. Skipped
        // when the palette is closed and the buffer list is empty.
        for text_buffer in self.palette_overlay_buffers.iter() {
            text_areas.push(TextArea {
                buffer: &text_buffer.buffer,
                left: text_buffer.pos.0,
                top: text_buffer.pos.1,
                scale: 1.0,
                bounds: vp_bounds,
                default_color,
                custom_glyphs: &[],
            });
        }
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
                text_areas,
                &mut self.swash_cache,
            )
            .unwrap();
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
                        load: wgpu::LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.text_renderer.render(&self.atlas, &self.viewport, &mut pass).unwrap();
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
        let geometry = match geometry {
            Some(g) => g,
            None => return,
        };

        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        // Layout constants, in screen-space pixels. Sized to be
        // legible at a typical desktop DPI without overlaying too
        // much of the map.
        let font_size: f32 = 16.0;
        let char_width = font_size * 0.6;
        let row_height = font_size * 1.5;
        let frame_width: f32 = 560.0;
        let inner_padding: f32 = 12.0;
        let max_rows_shown: usize = 10;
        let shown_rows = geometry.rows.len().min(max_rows_shown);
        let frame_height = row_height * (2.0 + shown_rows as f32) + inner_padding * 2.0;

        // Center horizontally, fixed offset from the top.
        let screen_w = self.config.width as f32;
        let left = ((screen_w - frame_width) * 0.5).max(0.0);
        let top: f32 = 80.0;

        let palette_color = cosmic_text::Color::rgba(0, 229, 255, 255); // cyan
        let text_color = cosmic_text::Color::rgba(235, 235, 235, 255);
        let dim_color = cosmic_text::Color::rgba(150, 150, 160, 255);
        let selected_color = cosmic_text::Color::rgba(0, 229, 255, 255);

        // Box-drawing border around the frame.
        let inner_cols = ((frame_width - char_width * 2.0) / char_width).max(1.0) as usize;
        let top_border = format!(
            "\u{256D}{}\u{256E}",
            "\u{2500}".repeat(inner_cols),
        );
        let bottom_border = format!(
            "\u{2570}{}\u{256F}",
            "\u{2500}".repeat(inner_cols),
        );
        let side = "\u{2502}";

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
        let inner_rows = ((frame_height / font_size).max(1.0) as usize).saturating_sub(1);
        let side_text: String = std::iter::repeat_n(format!("{side}\n"), inner_rows).collect();
        self.palette_overlay_buffers.push(create_border_buffer(
            &mut font_system,
            &side_text,
            &border_attrs,
            font_size,
            (left, top + font_size),
            (char_width, frame_height),
        ));
        self.palette_overlay_buffers.push(create_border_buffer(
            &mut font_system,
            &side_text,
            &border_attrs,
            font_size,
            (left + frame_width - char_width, top + font_size),
            (char_width, frame_height),
        ));

        // Query line: "/query|" where "|" is a cursor glyph.
        let query_line = format!("/{}\u{258C}", geometry.query_text);
        let query_attrs = Attrs::new()
            .color(text_color)
            .metrics(cosmic_text::Metrics::new(font_size, font_size));
        self.palette_overlay_buffers.push(create_border_buffer(
            &mut font_system,
            &query_line,
            &query_attrs,
            font_size,
            (left + inner_padding + char_width, top + inner_padding),
            (frame_width - inner_padding * 2.0 - char_width * 2.0, row_height),
        ));

        // Filtered rows, each labelled and dim-described. Selected
        // row gets a cyan prefix glyph and cyan label color.
        let row_base_y = top + inner_padding + row_height;
        for (i, row) in geometry.rows.iter().take(shown_rows).enumerate() {
            let is_selected = i == geometry.selected_row;
            let prefix = if is_selected { "\u{25B8} " } else { "  " };
            let row_attrs = Attrs::new()
                .color(if is_selected { selected_color } else { text_color })
                .metrics(cosmic_text::Metrics::new(font_size, font_size));
            let label_line = format!("{prefix}{}", row.label);
            let row_y = row_base_y + row_height * (i as f32 + 1.0);
            self.palette_overlay_buffers.push(create_border_buffer(
                &mut font_system,
                &label_line,
                &row_attrs,
                font_size,
                (left + inner_padding + char_width, row_y),
                (frame_width - inner_padding * 2.0 - char_width * 2.0, row_height),
            ));
            let desc_attrs = Attrs::new()
                .color(dim_color)
                .metrics(cosmic_text::Metrics::new(font_size * 0.8, font_size * 0.8));
            let desc_line = format!("    {}", row.description);
            self.palette_overlay_buffers.push(create_border_buffer(
                &mut font_system,
                &desc_line,
                &desc_attrs,
                font_size * 0.8,
                (
                    left + inner_padding + char_width,
                    row_y + font_size * 0.9,
                ),
                (frame_width - inner_padding * 2.0 - char_width * 2.0, row_height),
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
}
