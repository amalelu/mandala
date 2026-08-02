// SPDX-License-Identifier: MPL-2.0

//! Camera methods on `Renderer`: framing (`set_camera_center`,
//! `fit_camera_to_tree`) and the screen-space ↔ canvas-space
//! mapping every input handler converts through.
//!
//! Hit-testing does **not** live here. Canvas content is
//! hit-tested against the very trees that produced the pixels,
//! through
//! [`AppScene`](crate::application::scene_host::AppScene) and
//! Baumhard's per-tree BVH; the renderer owns no spatial index.

use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use glam::Vec2;

use super::Renderer;

impl Renderer {
    /// Pan the camera so `target` (canvas coordinates) is centered
    /// on the viewport at the current zoom. Used by the portal
    /// double-click handler to jump to the other side of a portal
    /// edge. Pure pan — no dirty flag raised; the shader transform
    /// plus render-time `visible_at` handle the new view.
    pub fn set_camera_center(&mut self, target: Vec2) {
        self.camera
            .apply_mutation(&baumhard::gfx_structs::camera::CameraMutation::SetPosition {
                canvas_pos: target,
            });
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
            self.camera
                .apply_mutation(&baumhard::gfx_structs::camera::CameraMutation::FitToBounds {
                    min: Vec2::new(min_x, min_y),
                    max: Vec2::new(max_x, max_y),
                    padding_fraction: 0.05,
                });
            // The fit typically changes both pan and zoom. Today this
            // is only called from `load_mindmap`, which follows up
            // with a full connection rebuild against the new zoom —
            // but raise `geometry_dirty` so any future caller (e.g.
            // a "fit to selection" command) automatically gets a
            // scene-cache flush + rebuild on the next frame instead
            // of silently leaving stale samples behind.
            self.connection_geometry_dirty = true;
        }
    }

    pub fn screen_to_canvas(&self, screen_x: f32, screen_y: f32) -> Vec2 {
        self.camera.screen_to_canvas(Vec2::new(screen_x, screen_y))
    }

    /// Size of one screen pixel in canvas units — used to convert
    /// screen-space tolerances (e.g. click tolerance) to canvas-space
    /// distances that stay visually consistent across zoom.
    pub fn canvas_per_pixel(&self) -> f32 {
        if self.camera.zoom > f32::EPSILON {
            1.0 / self.camera.zoom
        } else {
            1.0
        }
    }
}
