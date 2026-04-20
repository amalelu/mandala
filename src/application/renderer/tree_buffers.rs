//! Tree-to-buffer pipeline — walks a Baumhard `Tree`, shapes cosmic-text
//! buffers for each `GlyphArea`, and stores them (with optional background
//! rects) into the `Renderer`'s per-role buffer maps.

use baumhard::font::fonts;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use glam::Vec2;

use super::tree_walker::walk_tree_into_buffers;
use super::Renderer;

impl Renderer {
    /// Rebuild text buffers from a Baumhard tree (nodes rendered from GlyphArea
    /// elements). This is the primary text-rendering path; borders and
    /// connections use their own `rebuild_*_buffers` methods alongside it.
    pub fn rebuild_buffers_from_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        self.mindmap_buffers.clear();
        // Node backgrounds live on GlyphArea and are collected
        // fresh alongside the text buffers. The render pipeline
        // reads them back out each frame to draw solid fills behind
        // the text, with the camera transform baked in at the last
        // moment. Clearing here (rather than on every render call)
        // keeps the collect cost aligned with the tree rebuild
        // cadence — i.e. only when something structural changed.
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
                // with the edit / undo paths that address nodes by
                // Dewey-decimal id). Stringifying here keeps the
                // allocation off the helper's critical path so
                // overlay / canvas-scene callers never pay it.
                self.mindmap_buffers.insert(unique_id.to_string(), buffer);
            },
            |rect| self.node_background_rects.push(rect),
        );
    }

    /// Patch the canvas-space position of moved nodes' buffers in
    /// place. Avoids reshaping text when only position changed (the
    /// common case during a drag).
    ///
    /// For each `(unique_id, new_pos)` pair, looks up the existing
    /// buffer by key and overwrites its `pos` field. Buffers for
    /// nodes not in the patch set are left untouched — their shaped
    /// text and position remain valid.
    ///
    /// # Costs
    ///
    /// O(patch_set_size) — no text shaping, no font-system lock, no
    /// allocation. Each patch is a single hash lookup + field write.
    pub fn patch_drag_positions(&mut self, patches: &[(usize, (f32, f32))]) {
        for &(unique_id, new_pos) in patches {
            let key = unique_id.to_string();
            if let Some(buf) = self.mindmap_buffers.get_mut(&key) {
                buf.pos = new_pos;
            }
        }
    }

    /// Rebuild only the `node_background_rects` from a tree, without
    /// reshaping any text buffers. Used during drag to keep background
    /// fills in sync with moved node positions.
    ///
    /// # Costs
    ///
    /// O(n) descendant walk, but no text shaping, no font-system
    /// lock — just position and color reads from the arena.
    pub fn rebuild_node_backgrounds_from_tree(
        &mut self,
        tree: &Tree<GfxElement, GfxMutator>,
    ) {
        self.node_background_rects.clear();
        for descendant_id in tree.root().descendants(&tree.arena) {
            let Some(node) = tree.arena.get(descendant_id) else { continue };
            let Some(area) = node.get().glyph_area() else { continue };
            if let Some(color) = area.background_color {
                self.node_background_rects.push(super::NodeBackgroundRect {
                    position: Vec2::new(area.position.x.0, area.position.y.0),
                    size: Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0),
                    color,
                });
            }
        }
    }

    /// Rebuild the screen-space buffer list for every tree the app
    /// has registered into [`crate::application::scene_host::AppScene`].
    /// Walks the scene in layer
    /// order and produces one flat list; callers do not need to
    /// know about individual overlays. The renderer composites the
    /// result into the palette pass alongside the per-overlay
    /// buffer stores that predate this refactor — once every
    /// overlay has migrated to a tree, those per-overlay stores go
    /// away.
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
                    // a screen-space rect pipeline yet. When a
                    // screen-space overlay actually needs background
                    // fills, add a dedicated
                    // `overlay_scene_background_rects` field and a
                    // screen-space draw pass.
                },
            );
        }
    }

    /// Rebuild the canvas-space buffer list for every tree the app
    /// has registered into
    /// [`crate::application::scene_host::AppScene`]'s canvas sub-scene
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
}
