//! Render-decree dispatch: the single entry point the event loop
//! uses to push commands (`SetSurfaceSize`, `SetClearColor`, and
//! the zoom-related cache invalidations) into the renderer without
//! reaching for a named method per variant.

use glam::Vec2;

use crate::application::common::{FpsDisplayMode, RedrawMode, RenderDecree};

use super::{Renderer, FPS_WINDOW};

impl Renderer {
    /// Process a single decree directly
    pub fn process_decree(&mut self, decree: RenderDecree) {
        self.handle_render_decree(decree);
    }

    fn handle_render_decree(&mut self, decree: RenderDecree) {
        match decree {
            RenderDecree::DisplayFps(mode) => {
                self.fps_display_mode = mode;
                // Reset per-mode state on every transition: snapshot
                // clock zeroes so the first readout appears on the
                // next frame rather than after a full window, and the
                // debug ring resets so a prior debug run's samples
                // don't bleed into a fresh window.
                self.fps_clock = 0;
                self.fps_ring = [0u128; FPS_WINDOW];
                self.fps_ring_idx = 0;
                self.fps_ring_sum = 0;
                self.fps_ring_filled = 0;
                if matches!(mode, FpsDisplayMode::Off) {
                    self.fps_overlay_buffers.clear();
                    self.last_fps_shaped = None;
                }
            }
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
                // The per-edge off-screen glyph cull is a function of
                // the camera, so moving the camera invalidates the
                // cached per-edge visible-glyph layout. Clear the
                // renderer-side connection cache so the next rebuild
                // re-runs the cull from scratch, and raise the
                // viewport-dirty flag so the event loop actually
                // triggers the rebuild. The document-side
                // `SceneConnectionCache` holds canvas-space samples
                // whose spacing doesn't depend on pan, so it is NOT
                // cleared here — geometry stays cached across pans.
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
