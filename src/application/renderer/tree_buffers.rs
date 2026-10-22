// SPDX-License-Identifier: MPL-2.0

//! Tree-to-buffer pipeline — walks a Baumhard `Tree`, shapes cosmic-text
//! buffers for each `GlyphArea`, and stores them (with optional background
//! rects) into the `Renderer`'s per-role buffer maps.

use baumhard::font::fonts;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use glam::Vec2;

use super::scene_shape_cache::{self, ScenePassKind};
use super::tree_walker::{shape_one_element_into_buffers, walk_tree_into_buffers};
use super::{NodeBackgroundRect, Renderer};

/// Draw-order-preserving write-back cursor for the background fills
/// of a single re-shaped element.
///
/// `node_background_rects` is drawn in index order — `render.rs`
/// pushes one quad per entry into a single vertex stream, so a later
/// entry paints over an earlier one — and
/// [`Renderer::rebuild_buffers_from_tree`] fills it in tree-walk
/// order. A keyed re-shape that dropped an element's fill and
/// re-pushed the fresh one at the end would therefore lift that
/// element's fill above every other node's, for as long as the
/// caller keeps re-shaping it: the whole of a text-edit or a
/// section-resize drag, snapping back on the next full rebuild.
/// Writing the fresh fills into the slot the old ones held keeps the
/// order stable instead, without a per-frame re-walk of the tree.
pub(super) struct BackgroundRectSlot<'a> {
    rects: &'a mut Vec<NodeBackgroundRect>,
    next: usize,
}

impl<'a> BackgroundRectSlot<'a> {
    /// Remove every fill authored by `unique_id` and remember where
    /// the first of them sat, so [`Self::push`] writes the fresh
    /// ones back into the same place.
    ///
    /// An element with no fill in the list yet — one that just
    /// gained a `background_color` — has no slot to preserve, so it
    /// appends. Its tree-order position is derivable, but only by
    /// walking the tree, which is the per-frame cost this type
    /// exists to avoid; no live caller reaches that case today.
    ///
    /// # Costs
    ///
    /// O(len) for the scan and the retain, plus O(len) per
    /// [`Self::push`] for the shift. Same order as the `retain` +
    /// `push` it replaces.
    pub(super) fn take_over(rects: &'a mut Vec<NodeBackgroundRect>, unique_id: usize) -> Self {
        let next = rects
            .iter()
            .position(|rect| rect.unique_id == unique_id)
            .unwrap_or(rects.len());
        rects.retain(|rect| rect.unique_id != unique_id);
        BackgroundRectSlot { rects, next }
    }

    /// Write one fresh fill into the slot, advancing so a second
    /// fill from the same element lands after it rather than before.
    pub(super) fn push(&mut self, rect: NodeBackgroundRect) {
        self.rects.insert(self.next, rect);
        self.next += 1;
    }
}

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
                self.mindmap_buffers.entry(unique_id).or_default().push(buffer);
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
    /// the fresh element so a section whose background color just
    /// changed reflects the new fill *without* leaking duplicate
    /// stale rects per keystroke. Other elements' rects are
    /// untouched — the filter compares `NodeBackgroundRect.unique_id`
    /// directly — and the fresh fill goes back into the slot the old
    /// one held, because that list is drawn in index order and
    /// re-appending would float this element's fill over every other
    /// node's for the duration of the edit. See
    /// [`BackgroundRectSlot`].
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
        // so other elements' rects survive the filter, and the
        // draw-order slot is remembered across the removal so the
        // fresh fill goes back where the old one was.
        self.mindmap_buffers.remove(&unique_id);
        let mut slot = BackgroundRectSlot::take_over(&mut self.node_background_rects, unique_id);

        let mut font_system = fonts::acquire_font_system_write("reshape_buffer_for");
        shape_one_element_into_buffers(
            element,
            Vec2::ZERO,
            &mut font_system,
            &mut |uid, buffer| {
                self.mindmap_buffers.entry(uid).or_default().push(buffer);
            },
            &mut |rect| slot.push(rect),
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
    /// **Halos.** When an element has an outline halo, the walker
    /// emits one buffer per halo offset (positioned at
    /// `area.position + (dx, dy)` per `OutlineStyle::offsets()`)
    /// then the main glyph, and each of them records the `(dx, dy)`
    /// it was stamped at in
    /// [`MindMapTextBuffer::emission_offset`](super::MindMapTextBuffer#structfield.emission_offset).
    /// The patch re-derives `pos = new_pos + emission_offset`, so a
    /// dragged element's halos keep their ring around the main
    /// glyph. Writing `new_pos` flat — which this path used to do —
    /// collapsed every halo onto the main glyph; that was latent
    /// only because no mindmap-tree element sets `area.outline`
    /// today, and a latent landmine is still the finding (§5).
    ///
    /// # Costs
    ///
    /// O(patch_set_size × halos+1) — no text shaping, no font-system
    /// lock, no allocation. Halos count is 0 for mindmap nodes
    /// today, so the constant collapses on the common path.
    // Native-driver-only, with `rebuild_node_backgrounds_from_tree`:
    // both are drag-preview fast paths driven by the native drag
    // state machine.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn patch_drag_positions(&mut self, patches: &[(usize, (f32, f32))]) {
        for &(unique_id, new_pos) in patches {
            if let Some(bufs) = self.mindmap_buffers.get_mut(&unique_id) {
                for buf in bufs.iter_mut() {
                    buf.pos = (
                        new_pos.0 + buf.emission_offset.0,
                        new_pos.1 + buf.emission_offset.1,
                    );
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
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
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

    /// Refresh the screen-space shaped output for every tree the app
    /// has registered into [`crate::application::scene_host::AppScene`]'s
    /// overlay sub-scene. Walks the scene in layer order and produces
    /// one entry per walked element in draw order; callers do not need
    /// to know about individual overlays.
    ///
    /// **Only elements whose shaping inputs changed are re-shaped.**
    /// Every overlay mutator apply routes here — picker hover, picker
    /// HSV drag, console keystroke, console scrollback — and each of
    /// those writes every slot's fields whether or not the value
    /// moved, so before the reuse check the pass re-shaped the
    /// picker's ~58 outlined cells (9 buffers each) or the console's
    /// ~50 rows on every event. See [`super::scene_shape_cache`] for
    /// the reuse rule and why it needs no cooperation from the
    /// writers.
    ///
    /// # Costs
    ///
    /// [`scene_shape_cache::refresh`]'s, over the overlay sub-scene.
    pub fn rebuild_overlay_scene_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        let ids = app_scene.overlay_ids_in_layer_order();
        scene_shape_cache::refresh(
            app_scene.overlay_scene(),
            &ids,
            &mut self.overlay_scene_elements,
            ScenePassKind::Overlay,
        );
    }

    /// Refresh the canvas-space shaped output for every tree the app
    /// has registered into
    /// [`crate::application::scene_host::AppScene`]'s canvas
    /// sub-scene — borders, connections, portals, edge handles,
    /// connection labels, section frames and the two resize-handle
    /// roles. These buffers and fills feed the camera-transformed
    /// main pass alongside the mindmap's own.
    ///
    /// **Only elements whose shaping inputs changed are re-shaped**,
    /// by the same rule and the same body the overlay pass above
    /// uses. That is what makes the ten
    /// `flush_canvas_scene_buffers` call sites cost what they
    /// changed rather than what the canvas holds: each of them
    /// re-projects the roles its interaction can move and then
    /// flushes, and a role no `update_*` touched presents the walk
    /// with the elements it already shaped. An edge-label drag
    /// re-projects one role of the eight; a camera move re-projects
    /// four.
    ///
    /// # Costs
    ///
    /// [`scene_shape_cache::refresh`]'s, over the canvas sub-scene.
    pub fn rebuild_canvas_scene_buffers(&mut self, app_scene: &mut crate::application::scene_host::AppScene) {
        let ids = app_scene.canvas_ids_in_layer_order();
        scene_shape_cache::refresh(
            app_scene.canvas_scene(),
            &ids,
            &mut self.canvas_scene_elements,
            ScenePassKind::Canvas,
        );
    }
}
