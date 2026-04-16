//! Hit-testing + camera-fit methods. Grouped together because
//! they share an idiom (canvas-space or screen-space coordinate
//! math against a cached bounding rect or scene extent) and
//! because they all operate against state the other impls
//! build up — `connection_label_hitboxes`, `portal_hitboxes`,
//! `camera`.

use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use baumhard::mindmap::scene_builder::{PortalRefKey, RenderScene};
use baumhard::mindmap::scene_cache::EdgeKey;
use glam::Vec2;

use super::Renderer;

impl Renderer {

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
}
