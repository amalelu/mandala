//! Render-decree dispatch: the single entry point the event loop
//! uses to push commands (`SetSurfaceSize`, `SetClearColor`, and
//! the zoom-related cache invalidations) into the renderer without
//! reaching for a named method per variant.

use glam::Vec2;

use crate::application::common::{FpsDisplayMode, RedrawMode, RenderDecree};

use super::Renderer;

impl Renderer {
    /// Process a single decree directly
    pub fn process_decree(&mut self, decree: RenderDecree) {
        self.handle_render_decree(decree);
    }

    fn handle_render_decree(&mut self, decree: RenderDecree) {
        match decree {
            RenderDecree::SetFpsDisplay(mode) => {
                self.fps_display_mode = mode;
                // Reset every per-mode bit on every transition so a
                // prior mode's state can't bleed into the new one:
                //  - `last_frame_instant` so the first delta in the new
                //    mode isn't measured against a stale timestamp from
                //    seconds (or longer) ago, which would yield a one-
                //    frame FPS of ~0 right after toggling.
                //  - `fps_clock` so Snapshot's first sample fires on the
                //    next frame rather than after a full window.
                //  - the debug ring so a prior debug run's samples
                //    don't seed a fresh window.
                //  - `last_fps_shaped` so the overlay re-shapes with
                //    the new mode's first reading even if it happens
                //    to round to the same integer the previous mode
                //    last displayed.
                self.last_frame_instant = None;
                self.fps_clock = 0;
                self.fps_ring.clear();
                self.last_fps_shaped = None;
                if matches!(mode, FpsDisplayMode::Off) {
                    self.fps_overlay_buffers.clear();
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
                // Pan is a pure camera-matrix update. Canvas-space
                // glyph positions and shaped buffers do not change;
                // the shader applies the transform at draw time and
                // the per-frame `MindMapTextBuffer::visible_at`
                // check in `render.rs` handles viewport containment
                // cheaply.
            }
            RenderDecree::CameraZoom { screen_x, screen_y, factor } => {
                self.camera.apply_mutation(
                    &baumhard::gfx_structs::camera::CameraMutation::ZoomAt {
                        screen_focus: Vec2::new(screen_x, screen_y),
                        factor,
                    },
                );
                // Zoom invalidates the document-side sample cache:
                // the effective font size — and therefore sample
                // spacing along connection paths — is a function of
                // zoom via
                // `GlyphConnectionConfig::effective_font_size_pt`.
                self.connection_geometry_dirty = true;
            }
        }
    }
}
