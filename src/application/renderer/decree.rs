// SPDX-License-Identifier: MPL-2.0

//! Render-decree dispatch: the single entry point the event loop
//! uses to push commands (`SetSurfaceSize`, `SetClearColor`, and
//! the zoom-related cache invalidations) into the renderer without
//! reaching for a named method per variant.

use glam::Vec2;

use crate::application::common::{FpsDisplayMode, RenderDecree};

use super::Renderer;

impl Renderer {
    pub fn process_decree(&mut self, decree: RenderDecree) {
        self.handle_render_decree(decree);
    }

    fn handle_render_decree(&mut self, decree: RenderDecree) {
        match decree {
            RenderDecree::SetFpsDisplay(mode) => {
                self.fps_display_mode = mode;
                // Reset all per-mode FPS bookkeeping so prior-mode
                // samples can't bleed in (a stale `last_frame_instant`
                // would yield a ~0 FPS for the first frame after
                // toggling). The active→idle transition flag is also
                // cleared — it's tied to a specific transition that
                // no longer applies once the user explicitly changed
                // the mode.
                self.last_frame_instant = None;
                self.fps_clock = 0;
                self.fps_ring.clear();
                self.last_fps_shaped = None;
                self.fps_pending_idle_paint = false;
                if matches!(mode, FpsDisplayMode::Off) {
                    self.fps_overlay_buffers.clear();
                }
            }
            RenderDecree::StartRender => {
                self.should_render = true;
            }
            RenderDecree::SetSurfaceSize(x, y) => {
                self.update_surface_size(x, y);
            }
            RenderDecree::Terminate => {
                self.run = false;
            }
            RenderDecree::Noop => {}
            RenderDecree::CameraPan(dx, dy) => {
                self.camera
                    .apply_mutation(&baumhard::gfx_structs::camera::CameraMutation::Pan {
                        screen_delta: Vec2::new(dx, dy),
                    });
            }
            RenderDecree::CameraZoom {
                screen_x,
                screen_y,
                factor,
            } => {
                self.camera
                    .apply_mutation(&baumhard::gfx_structs::camera::CameraMutation::ZoomAt {
                        screen_focus: Vec2::new(screen_x, screen_y),
                        factor,
                    });
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
