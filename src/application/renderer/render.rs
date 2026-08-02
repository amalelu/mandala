// SPDX-License-Identifier: MPL-2.0

//! Frame rendering: the `Renderer::render()` body and its geometry
//! / vertex helpers.

use glam::Vec2;
use glyphon::{TextArea, TextBounds};
use log::debug;
use wgpu::StoreOp;

use baumhard::font::{fonts, COLOR_WHITE};
use baumhard::util::color_conversion::convert_u8_to_f32;

use super::{Renderer, RECT_VBUF_INITIAL_CAPACITY, RECT_VERTEX_FLOATS};

use baumhard::gfx_structs::shape::SHAPE_ID_RECTANGLE;

impl Renderer {
    /// Push the six vertices (two triangles) of a filled axis-aligned
    /// rectangle into `out`. Coords are already in NDC — the caller is
    /// responsible for any camera or screen→NDC transform. `color` is
    /// the flat RGBA written to every vertex; `shape_id` selects the
    /// fragment-shader path (`0` = rectangle, `1` = ellipse, …). The
    /// per-vertex `uv` is hard-wired to the quad's local `[0, 1]²`
    /// frame so the fragment shader can evaluate any SDF in the shape
    /// table without extra uniforms.
    ///
    /// Layout per vertex: `[x, y, u, v, r, g, b, a, shape_id]`
    /// (9 × 4 bytes = 36 bytes; must match `RECT_VERTEX_SIZE`).
    /// `shape_id` rides the stream as a plain `f32` (`shape_id as f32`)
    /// because wgpu's WebGL2 backend doesn't support integer vertex
    /// attributes on every browser — the WGSL vertex stage rounds and
    /// casts to `u32` before flat-interpolating. The round-trip is
    /// lossless for the small integer range we use.
    fn push_rect_ndc(out: &mut Vec<f32>, ndc_min: Vec2, ndc_max: Vec2, color: [f32; 4], shape_id: u32) {
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
        // `shape_id` is encoded as `Float32` in the vertex buffer;
        // WGSL rounds + casts back to `u32` before the switch. See
        // the type-level doc on this function for the rationale.
        let sid = shape_id as f32;
        // UVs match the quad's local frame: TL = (0, 0), TR = (1, 0),
        // BR = (1, 1), BL = (0, 1). The SDF cases in the fragment
        // shader assume exactly this parameterization.
        let push = |out: &mut Vec<f32>, x: f32, y: f32, u: f32, v: f32| {
            out.extend_from_slice(&[x, y, u, v, r, g, b, a, sid]);
        };
        // Triangle 1: TL, BL, BR
        push(out, lx, ry, 0.0, 0.0);
        push(out, lx, ly, 0.0, 1.0);
        push(out, rx, ly, 1.0, 1.0);
        // Triangle 2: TL, BR, TR
        push(out, lx, ry, 0.0, 0.0);
        push(out, rx, ly, 1.0, 1.0);
        push(out, rx, ry, 1.0, 0.0);
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
    #[inline]
    pub(super) fn render(&mut self) {
        if !self.should_render {
            return;
        }
        let vp_w_px = self.config.width as f32;
        let vp_h_px = self.config.height as f32;

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
            if !rect.visible_at(&self.camera) {
                continue;
            }
            let screen_tl = self.camera.canvas_to_screen(rect.position);
            let screen_size = rect.size * self.camera.zoom;
            let (ndc_min, ndc_max) = Self::screen_rect_to_ndc_bounds(
                screen_tl.x,
                screen_tl.y,
                screen_size.x,
                screen_size.y,
                vp_w_px,
                vp_h_px,
            );
            let color = convert_u8_to_f32(&rect.color);
            Self::push_rect_ndc(
                &mut self.main_rect_vertices,
                ndc_min,
                ndc_max,
                color,
                rect.shape_id,
            );
        }

        // Rebuild the "palette" rect batch: one opaque backdrop
        // behind the command palette and/or the glyph-wheel color
        // picker, in screen space. The two modals are mutually
        // exclusive but a single batch handles either case (or
        // both, if a future variant ever overlaps them).
        self.console_rect_vertices.clear();
        if let Some((left, top, w, h)) = self.console_backdrop {
            let (ndc_min, ndc_max) = Self::screen_rect_to_ndc_bounds(left, top, w, h, vp_w_px, vp_h_px);
            // Pitch black. Sits cleanly against the cyan frame and
            // any canvas background without tinting the palette's
            // cyan foreground.
            let bg_color = [0.0, 0.0, 0.0, 1.0];
            Self::push_rect_ndc(
                &mut self.console_rect_vertices,
                ndc_min,
                ndc_max,
                bg_color,
                SHAPE_ID_RECTANGLE,
            );
        }
        if let Some((left, top, w, h)) = self.color_picker_backdrop {
            let (ndc_min, ndc_max) = Self::screen_rect_to_ndc_bounds(left, top, w, h, vp_w_px, vp_h_px);
            // Same pitch black as the palette — the picker's hue
            // ring glyphs and crosshair cells are saturated colors
            // that pop against true black with no tinting.
            let bg_color = [0.0, 0.0, 0.0, 1.0];
            Self::push_rect_ndc(
                &mut self.console_rect_vertices,
                ndc_min,
                ndc_max,
                bg_color,
                SHAPE_ID_RECTANGLE,
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
            self.queue.write_buffer(
                &self.rect_vertex_buffer,
                0,
                bytemuck::cast_slice(&self.main_rect_vertices),
            );
        }
        if palette_bytes_len > 0 {
            self.queue.write_buffer(
                &self.rect_vertex_buffer,
                main_bytes_len as u64,
                bytemuck::cast_slice(&self.console_rect_vertices),
            );
        }
        let main_vertex_count = (self.main_rect_vertices.len() / RECT_VERTEX_FLOATS) as u32;
        let palette_vertex_count = (self.console_rect_vertices.len() / RECT_VERTEX_FLOATS) as u32;

        // Collect text areas + run both glyphon prepares against
        // the atlas. Shared with `prewarm()` so the warm-up sees
        // exactly the same glyph set that the next real frame will
        // draw — no risk of warming a different set than gets
        // rasterised. On lock contention or prepare failure, skip
        // the rest of this frame.
        if !self.prepare_text_for_pass() {
            return;
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                debug!("Failed to get the surface texture ({other:?}), can't render.");
                return;
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
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
                pass.set_vertex_buffer(0, self.rect_vertex_buffer.slice(0..main_bytes_len as u64));
                pass.draw(0..main_vertex_count, 0..1);
            }

            // 2. Main text pass — node text, borders, connections,
            //    edge handles, camera-transformed and screen-space
            //    overlays, all drawn on top of the node backgrounds.
            //    Interactive path: log and continue on render failure
            //    so a single bad atlas frame doesn't crash the editor.
            //    Unreachable as of glyphon 0.11.0 — `TextRenderer::
            //    render` returns `Ok(())` on every path and neither
            //    `RenderError` variant is ever constructed — so this
            //    is forward-compat cover, not a live warn, and it
            //    carries no frame-rate spam risk today. Kept because
            //    the signature is fallible and a future glyphon may
            //    start using it.
            if let Err(e) = self.text_renderer.render(&self.atlas, &self.viewport, &mut pass) {
                log::warn!("text_renderer.render failed: {e}");
            }

            // 3. Palette backdrop (rect pipeline, screen-space).
            //    Drawn AFTER the main text pass so node text
            //    sitting behind the palette is fully occluded.
            if palette_vertex_count > 0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_vertex_buffer(
                    0,
                    self.rect_vertex_buffer
                        .slice(main_bytes_len as u64..(main_bytes_len + palette_bytes_len) as u64),
                );
                pass.draw(0..palette_vertex_count, 0..1);
            }

            // 4. Palette text pass — cyan border, query line,
            //    filtered action rows. Drawn on top of the palette
            //    backdrop so every glyph sits cleanly on solid fill.
            //    Interactive path: log and continue on render failure.
            //    Unreachable in glyphon 0.11.0 for the same reason as
            //    the main pass above; kept as forward-compat cover.
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

    /// Collect every visible text-area from the renderer's buffer
    /// maps (mindmap nodes, overlays, canvas-scene — which carries
    /// borders, connections, labels, and handles through the tree
    /// pipeline — console, color picker, FPS
    /// overlay) and run both `text_renderer.prepare()` and
    /// `console_text_renderer.prepare()` against the atlas. Returns
    /// `false` (and skips the rest of the caller's frame) on
    /// font-system lock contention or on either prepare failing —
    /// the same degrade path the inline render() code had.
    ///
    /// Shared between `render()` (steady-state) and `prewarm()`
    /// (one-shot at load) so the warm-up sees exactly the same
    /// glyph set the next real frame will draw.
    /// Note: this performs disjoint mutable borrows of `text_renderer`
    /// + `atlas` + `swash_cache` while holding immutable borrows of
    /// the buffer-map fields; the borrow checker permits this within
    /// a single `&mut self` body but would reject it across a method
    /// boundary, which is why the prepare call lives in this method
    /// rather than splitting into a `collect_text_areas` helper.
    fn prepare_text_for_pass(&mut self) -> bool {
        let vp_w = self.config.width as i32;
        let vp_h = self.config.height as i32;
        let vp_bounds = TextBounds {
            left: 0,
            top: 0,
            right: vp_w,
            bottom: vp_h,
        };
        let default_color = COLOR_WHITE;

        // Collect "main" text areas: the mindmap node buffers +
        // overlays + canvas-scene arena buffers (borders,
        // connections, labels, portals, handles all arrive here
        // through the tree walker).
        // Palette buffers go into a separate list so they render
        // in a second glyphon pass (with the backdrop rect
        // between them, hence the split).
        // Upper-bound capacity so the per-frame `Vec` doesn't grow
        // through several reallocs. Visibility-culling reduces the
        // realized count below this; allocating once at the ceiling
        // is still cheaper than `push` reallocations.
        let main_capacity = self.mindmap_buffers.values().map(|v| v.len()).sum::<usize>()
            + self.overlay_buffers.len()
            + self.canvas_scene_buffers.len();
        let mut main_text_areas: Vec<TextArea> = Vec::with_capacity(main_capacity);
        main_text_areas.extend(
            self.mindmap_buffers
                .values()
                .flat_map(|v| v.iter())
                .chain(self.overlay_buffers.iter())
                .chain(self.canvas_scene_buffers.iter())
                .filter_map(|tb| {
                    if !tb.visible_at(&self.camera) {
                        return None;
                    }
                    let canvas_pos = Vec2::new(tb.pos.0, tb.pos.1);
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
                }),
        );

        // Palette overlay: screen-space text, drawn in its own
        // glyphon pass so the rect-pipeline backdrop can be
        // interleaved between the main text and this one. The
        // glyph-wheel color picker's glyph buffers flow through
        // `overlay_scene_buffers` (populated by
        // `rebuild_overlay_scene_buffers` from the picker's overlay
        // tree in `AppScene`) — it's a mutually exclusive
        // screen-space modal that shares this pass with the
        // console.
        let palette_capacity = self.console_overlay_buffers.len()
            + self.overlay_scene_buffers.len()
            + self.fps_overlay_buffers.len()
            + self.mode_status_overlay_buffers.len();
        let mut palette_text_areas: Vec<TextArea> = Vec::with_capacity(palette_capacity);
        palette_text_areas.extend(
            self.console_overlay_buffers
                .iter()
                .chain(self.overlay_scene_buffers.iter())
                .chain(self.fps_overlay_buffers.iter())
                .chain(self.mode_status_overlay_buffers.iter())
                .map(|tb| TextArea {
                    buffer: &tb.buffer,
                    left: tb.pos.0,
                    top: tb.pos.1,
                    scale: 1.0,
                    bounds: vp_bounds,
                    default_color,
                    custom_glyphs: &[],
                }),
        );

        // Interactive path: a contended font-system lock must skip
        // the frame, not abort the process. `debug!`, not `warn!`,
        // per CODE_CONVENTIONS §9: this runs once per frame, the
        // skip is a designed transient that the next frame retries,
        // and there is nothing a user could act on — a `warn!` here
        // would flood stderr at frame rate for no diagnostic gain.
        // `rebuild_mode_status_overlay_if_needed` takes the same
        // contention path silently for the same reason.
        let Ok(mut font_system) = fonts::FONT_SYSTEM.try_write() else {
            log::debug!("renderer: font_system lock contended in prepare_text_for_pass, skipping");
            return false;
        };

        // Interactive path: a glyphon prepare failure must degrade the
        // frame, not abort the process. Skip the whole render so we
        // don't run a half-prepared atlas through the GPU.
        //
        // Logged once per fault episode, not once per frame. Unlike
        // the `render()` failures above, `PrepareError::AtlasFull` is
        // reachable and can be *permanent*: glyphon only reports it
        // after `TextAtlas::grow()` returns `false`, which it does
        // unconditionally once the atlas has reached
        // `max_texture_dimension_2d` — commonly 4096 on WebGL2, a
        // first-class target under CODE_CONVENTIONS §4. Returning
        // `false` here makes the caller bail before
        // `get_current_texture()` / `present()`, so the skipped frame
        // takes no vsync backpressure either: under
        // `RedrawMode::NoLimit` with `ControlFlow::Poll` the loop
        // spins at CPU rate during a drag or animation and an
        // unguarded `warn!` would write a line per spin. The flag
        // clears on the next successful prepare, so a transient fault
        // logs again if it returns.
        if let Err(e) = self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut font_system,
            &mut self.atlas,
            &self.viewport,
            main_text_areas,
            &mut self.swash_cache,
        ) {
            if !self.prepare_fault_logged {
                self.prepare_fault_logged = true;
                log::warn!("text_renderer.prepare failed, skipping frame: {e}");
            }
            return false;
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
            if !self.prepare_fault_logged {
                self.prepare_fault_logged = true;
                log::warn!("console_text_renderer.prepare failed, skipping frame: {e}");
            }
            return false;
        }
        drop(font_system);
        self.prepare_fault_logged = false;
        true
    }

    /// Pre-warm the render pipeline during map load: run one full
    /// `render()` cycle (rect-vertex prep → glyph atlas upload →
    /// `pass.set_pipeline(rect_pipeline)` → text pipeline bind →
    /// `surface.get_current_texture()` → `frame.present()`). Forces
    /// the wgpu driver to compile pipeline shaders and the
    /// swapchain to allocate its first backing image — both are
    /// commonly 50-300ms costs (Vulkan/Metal/D3D12 lazily compile
    /// SPIR-V→GPU-ISA on first pipeline bind, Mesa especially) that
    /// would otherwise land on the user's first interaction.
    ///
    /// The original `prewarm_atlas` skipped the render pass to
    /// avoid `atlas.trim()` evicting freshly warmed glyphs. The
    /// concern was unfounded but for a reason worth stating
    /// precisely: glyphon's `TextAtlas::trim()` doesn't evict at
    /// all — it just clears the per-frame `glyphs_in_use` set
    /// (text_atlas.rs `mask_atlas.trim` / `color_atlas.trim`
    /// are `self.glyphs_in_use.clear()`). The glyph cache itself
    /// only evicts under packer pressure inside `try_allocate`,
    /// and the `glyphs_in_use` set is populated by
    /// `text_renderer.prepare()` — not `render()`. So warmed
    /// glyphs sit in the LRU cache regardless of whether a draw
    /// happened, and the prior atlas-only path was already safe;
    /// what the full render adds is shader compile and the
    /// swapchain allocation, which `prepare` alone doesn't reach.
    ///
    /// Caller must have already populated the canvas-scene buffers
    /// (`flush_canvas_scene_buffers`) and dispatched
    /// `RenderDecree::StartRender`. If buffers are empty (doc-load
    /// failure path) this is a cheap no-op (one empty draw + one
    /// blank present).
    ///
    /// **Failure handling.** The whole call is wrapped in
    /// `catch_unwind`: prewarm runs *before the window is
    /// visible*, so if the first render's pipeline compile or
    /// `frame.present()` panics on a flaky driver, we'd otherwise
    /// abort startup with no UI feedback. `render()`'s own surface-
    /// acquisition failures already early-return cleanly (see
    /// `render()`'s `get_current_texture` match), but anything
    /// deeper in wgpu/glyphon that panics would propagate. Catching
    /// the panic and continuing means a degraded first frame
    /// instead of a dead app — the next real `process()` cycle
    /// retries from a known state.
    pub fn prewarm(&mut self) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.render()));
        if result.is_err() {
            log::warn!(
                "renderer.prewarm: first render panicked; continuing with cold pipelines. \
                 First user interaction may show a one-frame stutter."
            );
        }
    }
}
