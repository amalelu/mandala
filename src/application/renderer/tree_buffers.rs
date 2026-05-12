// SPDX-License-Identifier: MPL-2.0

//! Tree-to-buffer pipeline — walks a Baumhard `Tree`, shapes cosmic-text
//! buffers for each `GlyphArea`, and stores them (with optional background
//! rects) into the `Renderer`'s per-role buffer maps.

use baumhard::font::fonts;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use glam::Vec2;

use super::tree_walker::{shape_one_element_into_buffers, walk_tree_into_buffers};
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
        let mut font_system = fonts::acquire_font_system_write("rebuild_buffers_from_tree");
        walk_tree_into_buffers(
            tree,
            Vec2::ZERO,
            &mut font_system,
            |unique_id, buffer| {
                // The walker emits multiple buffers per element
                // when an outline halo is configured (one buffer
                // per halo offset, then the main glyph). Push onto
                // the entry's `Vec` so all of them survive in
                // emission order — `insert`-style replace would
                // collapse halos onto the main glyph silently.
                self.mindmap_buffers
                    .entry(unique_id.to_string())
                    .or_default()
                    .push(buffer);
            },
            |rect| self.node_background_rects.push(rect),
        );
    }

    /// Re-shape one element's text buffers in place, keyed by the
    /// element's arena `NodeId` and `unique_id`. Used by the per-
    /// keystroke text-edit path on a multi-section node — pre-fix
    /// the path called [`Self::rebuild_buffers_from_tree`] which
    /// walked the entire arena and re-shaped every section across
    /// every node, an O(N×sections) cost paid on every keypress.
    /// The keyed reshape drops this to O(halos+1) buffers per
    /// element.
    ///
    /// Drops every existing buffer keyed by `unique_id` (the main
    /// glyph plus any halos) before re-shaping; the re-shape pass
    /// writes the new buffers back into the same `Vec` entry.
    /// Background rects authored by this same `unique_id` are
    /// removed from `node_background_rects` and re-collected from
    /// the fresh element so a section whose background colour just
    /// changed reflects the new fill *without* leaking duplicate
    /// stale rects per keystroke. Other elements' rects are
    /// untouched — the filter compares `NodeBackgroundRect.unique_id`
    /// directly.
    ///
    /// Silent no-op when `arena_id` doesn't resolve (e.g. the tree
    /// was rebuilt between the caller's lookup and this call). The
    /// caller is expected to fall back to a full
    /// `rebuild_buffers_from_tree` if it cannot guarantee the
    /// arena id is fresh.
    pub fn reshape_buffer_for(&mut self, arena_id: indextree::NodeId, tree: &Tree<GfxElement, GfxMutator>) {
        let Some(element) = tree.arena.get(arena_id).map(|n| n.get()) else {
            return;
        };
        let unique_id = element.unique_id();

        // Drop the existing buffers + background entries authored
        // by this element so the re-shape pass can write fresh
        // ones. Background rects use direct `unique_id` equality
        // so other elements' rects survive the filter.
        let key = unique_id.to_string();
        self.mindmap_buffers.remove(&key);
        self.node_background_rects
            .retain(|rect| rect.unique_id != unique_id);

        let mut font_system = fonts::acquire_font_system_write("reshape_buffer_for");
        shape_one_element_into_buffers(
            element,
            Vec2::ZERO,
            &mut font_system,
            &mut |uid, buffer| {
                self.mindmap_buffers
                    .entry(uid.to_string())
                    .or_default()
                    .push(buffer);
            },
            &mut |rect| self.node_background_rects.push(rect),
        );
    }

    /// Patch the canvas-space position of moved nodes' buffers in
    /// place. Avoids reshaping text when only position changed (the
    /// common case during a drag).
    ///
    /// For each `(unique_id, new_pos)` pair, looks up the existing
    /// buffer entry by key and overwrites every buffer's `pos`
    /// field. Buffers for nodes not in the patch set are left
    /// untouched; their shaped text and position remain valid.
    ///
    /// **Halo limitation.** When an element has an outline halo,
    /// the walker emits one buffer per halo offset (positioned at
    /// `area.position + (dx, dy)` per `OutlineStyle::offsets()`)
    /// then the main glyph. This patch path overwrites every
    /// halo's `pos` with `new_pos` — collapsing halos onto the
    /// main glyph. Today this is latent because mindmap-tree
    /// elements (`mindnode_container_area`, `mindnode_section_area`)
    /// never set `area.outline`; halos live on the picker overlay
    /// path which doesn't go through `mindmap_buffers`. If a
    /// future feature sets `outline` on a mindmap element, drag
    /// will visibly collapse the halos. The fix at that point is
    /// to store per-buffer `(dx, dy)` offsets alongside `pos` and
    /// re-derive `pos = new_pos + offset` here.
    ///
    /// # Costs
    ///
    /// O(patch_set_size × halos+1) — no text shaping, no font-system
    /// lock, no allocation. Halos count is 0 for mindmap nodes
    /// today, so the constant collapses on the common path.
    pub fn patch_drag_positions(&mut self, patches: &[(usize, (f32, f32))]) {
        for &(unique_id, new_pos) in patches {
            let key = unique_id.to_string();
            if let Some(bufs) = self.mindmap_buffers.get_mut(&key) {
                for buf in bufs.iter_mut() {
                    buf.pos = new_pos;
                }
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
    pub fn rebuild_node_backgrounds_from_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        self.node_background_rects.clear();
        // Single source of background-rect math: routes through the
        // walker's `extract_background_rect` so the drag-fast-path
        // and the full text-shaping pass can't drift on padding /
        // shape-id / zoom-visibility resolution.
        for descendant_id in tree.root().descendants(&tree.arena) {
            let Some(node) = tree.arena.get(descendant_id) else { continue };
            let element = node.get();
            let Some(area) = element.glyph_area() else { continue };
            if let Some(rect) = super::tree_walker::extract_background_rect(element, area, Vec2::ZERO) {
                self.node_background_rects.push(rect);
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
        let mut font_system = fonts::acquire_font_system_write("rebuild_overlay_scene_buffers");
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
    pub fn rebuild_canvas_scene_buffers(&mut self, app_scene: &mut crate::application::scene_host::AppScene) {
        self.canvas_scene_buffers.clear();
        self.canvas_scene_background_rects.clear();
        let ids = app_scene.canvas_ids_in_layer_order();
        if ids.is_empty() {
            return;
        }
        let mut font_system = fonts::acquire_font_system_write("rebuild_canvas_scene_buffers");
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
