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
use glam::Vec2;
use std::path::Path;

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
    border_buffers: Vec<MindMapTextBuffer>,
    connection_buffers: Vec<MindMapTextBuffer>,
    /// Temporary overlay buffers (e.g., selection rectangle). Camera-transformed.
    overlay_buffers: Vec<MindMapTextBuffer>,
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
            border_buffers: Vec::new(),
            connection_buffers: Vec::new(),
            overlay_buffers: Vec::new(),
        }
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
            .chain(self.border_buffers.iter())
            .chain(self.connection_buffers.iter())
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
    }

    fn load_mindmap(&mut self, path: &str) {
        match loader::load_from_file(Path::new(path)) {
            Ok(map) => {
                info!("Loaded mindmap '{}' with {} nodes", map.name, map.nodes.len());

                // Nodes via Baumhard tree
                let mindmap_tree = baumhard::mindmap::tree_builder::build_mindmap_tree(&map);
                self.rebuild_buffers_from_tree(&mindmap_tree.tree);
                self.fit_camera_to_tree(&mindmap_tree.tree);

                // Connections + borders via flat scene
                let scene = baumhard::mindmap::scene_builder::build_scene(&map);
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
    pub fn rebuild_border_buffers(&mut self, border_elements: &[BorderElement]) {
        /// How far the top/bottom border line boxes are pulled inward (toward
        /// the node content) so their glyph visible extents overlap with the
        /// vertical columns' glyph visible extents. Empirically chosen for
        /// LiberationSans at typical border font sizes; larger values visibly
        /// encroach on the node content, smaller values leave gaps.
        const CORNER_OVERLAP_FRAC: f32 = 0.35;

        self.border_buffers.clear();
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        for elem in border_elements {
            let border_color = parse_hex_color(&elem.border_style.color)
                .unwrap_or(cosmic_text::Color::rgba(255, 255, 255, 255));
            let font_size = elem.border_style.font_size_pt;
            let glyph_set = &elem.border_style.glyph_set;
            let border_attrs = Attrs::new()
                .color(border_color)
                .metrics(cosmic_text::Metrics::new(font_size, font_size));

            let (nx, ny) = elem.node_position;
            let (nw, nh) = elem.node_size;

            // --- Horizontal math ---
            // The top/bottom runs include two corner cells plus enough body
            // cells to span the node's width. We round up so the border
            // always fully encloses the node horizontally.
            let approx_char_width = font_size * 0.6;
            let char_count = ((nw / approx_char_width) + 2.0)
                .ceil()
                .max(3.0) as usize;
            // Approximate x of the rightmost (corner) character within the
            // top run. The top run starts at `nx - approx_char_width`, and
            // the last character occupies positions
            // `(char_count - 1) * approx_char_width` further along.
            let right_corner_x =
                nx - approx_char_width + (char_count - 1) as f32 * approx_char_width;
            let h_width = (char_count as f32 + 1.0) * approx_char_width;
            let v_width = approx_char_width * 2.0;

            // Pull the horizontal runs inward so their glyph visible extents
            // meet the vertical columns at the corners.
            let corner_overlap = font_size * CORNER_OVERLAP_FRAC;
            let top_y = ny - font_size + corner_overlap;
            let bottom_y = ny + nh - corner_overlap;

            // Top run: `╭─ … ─╮`
            let top_text = glyph_set.top_border(char_count);
            self.border_buffers.push(create_border_buffer(
                &mut font_system, &top_text, &border_attrs, font_size,
                (nx - approx_char_width, top_y),
                (h_width, font_size * 1.5),
            ));

            // Bottom run: `╰─ … ─╯`
            let bottom_text = glyph_set.bottom_border(char_count);
            self.border_buffers.push(create_border_buffer(
                &mut font_system, &bottom_text, &border_attrs, font_size,
                (nx - approx_char_width, bottom_y),
                (h_width, font_size * 1.5),
            ));

            // --- Vertical math ---
            // The left/right columns use `row_count` line-height-tall cells
            // so the column spans `row_count * font_size` vertically. Round
            // so we don't overshoot or undershoot by more than half a row.
            let row_count = (nh / font_size).round().max(1.0) as usize;
            let left_text: String =
                std::iter::repeat_n(format!("{}\n", glyph_set.left_char()), row_count).collect();
            self.border_buffers.push(create_border_buffer(
                &mut font_system, &left_text, &border_attrs, font_size,
                (nx - approx_char_width, ny),
                (v_width, nh),
            ));

            // Right column is anchored to the *approximate* position of the
            // top run's right corner glyph, not to `nx + nw`. This keeps the
            // top/bottom corners and the right column visually aligned even
            // when `nw` isn't a whole multiple of `approx_char_width`.
            let right_text: String =
                std::iter::repeat_n(format!("{}\n", glyph_set.right_char()), row_count).collect();
            self.border_buffers.push(create_border_buffer(
                &mut font_system, &right_text, &border_attrs, font_size,
                (right_corner_x, ny),
                (v_width, nh),
            ));
        }
    }

    /// Rebuild connection buffers from flat connection elements (from RenderScene).
    pub fn rebuild_connection_buffers(&mut self, connection_elements: &[ConnectionElement]) {
        self.connection_buffers.clear();
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        for elem in connection_elements {
            let conn_color = parse_hex_color(&elem.color)
                .unwrap_or(cosmic_text::Color::rgba(200, 200, 200, 255));
            let font_size = elem.font_size_pt;
            let half_glyph = font_size * 0.3;
            let half_height = font_size * 0.5;
            let glyph_bounds = (font_size, font_size);
            let conn_attrs = Attrs::new()
                .color(conn_color)
                .metrics(cosmic_text::Metrics::new(font_size, font_size));

            if let Some((ref cap_text, cap_pos)) = elem.cap_start {
                self.connection_buffers.push(create_border_buffer(
                    &mut font_system, cap_text, &conn_attrs, font_size,
                    (cap_pos.0 - half_glyph, cap_pos.1 - half_height),
                    glyph_bounds,
                ));
            }

            for &pos in &elem.glyph_positions {
                self.connection_buffers.push(create_border_buffer(
                    &mut font_system, &elem.body_glyph, &conn_attrs, font_size,
                    (pos.0 - half_glyph, pos.1 - half_height),
                    glyph_bounds,
                ));
            }

            if let Some((ref cap_text, cap_pos)) = elem.cap_end {
                self.connection_buffers.push(create_border_buffer(
                    &mut font_system, cap_text, &conn_attrs, font_size,
                    (cap_pos.0 - half_glyph, cap_pos.1 - half_height),
                    glyph_bounds,
                ));
            }
        }
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
            }
            RenderDecree::CameraZoom { screen_x, screen_y, factor } => {
                self.camera.zoom_at(Vec2::new(screen_x, screen_y), factor);
            }
            RenderDecree::LoadMindMap(path) => {
                self.load_mindmap(&path);
            }
        }
    }
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
