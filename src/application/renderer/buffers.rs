//! All the `rebuild_*_buffers*` methods on `Renderer`, plus the two
//! picker-overlay `apply_*` methods that share the same plumbing.
//! Every method here walks a tree or scene, shapes into
//! [`MindMapTextBuffer`]s, and drops the result into a keyed map
//! (or a Vec, for the flat overlay surfaces).
//!
//! Lifted verbatim out of `renderer/mod.rs` so the frame-hot
//! `render()` path and the startup `new()` / pipeline setup aren't
//! crowded out. Every method retains its original `pub`/`pub(super)`
//! visibility; the `impl Renderer { ... }` block here is a second
//! `impl` on the same struct, which Rust allows.

use baumhard::font::fonts;
use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use baumhard::mindmap::scene_builder::{BorderElement, ConnectionElement, PortalRefKey, RenderScene};
use baumhard::mindmap::scene_cache::EdgeKey;
use cosmic_text::{Attrs, AttrsList, Buffer, Family, Style};
use glam::Vec2;
use glyphon::TextBounds;
use log::{debug, error};
use rustc_hash::FxHashMap;

use super::borders::{create_border_buffer, parse_hex_color};
use super::color_picker;
use super::console_geometry::{compute_console_frame_layout, ConsoleOverlayGeometry};
use super::console_pass::{
    build_console_overlay_mutator, build_console_overlay_tree, console_overlay_signature,
};
use super::render::glyph_position_in_viewport;
use super::tree_walker::walk_tree_into_buffers;
use super::{MindMapTextBuffer, NodeBackgroundRect, Renderer};

impl Renderer {
    /// Rebuild text buffers from a Baumhard tree (nodes rendered from GlyphArea
    /// elements). This is the primary text-rendering path; borders and
    /// connections use their own `rebuild_*_buffers` methods alongside it.
    pub fn rebuild_buffers_from_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        self.mindmap_buffers.clear();
        // Session 6C follow-up: node backgrounds live on GlyphArea
        // and are collected fresh alongside the text buffers. The
        // render pipeline reads them back out each frame to draw
        // solid fills behind the text, with the camera transform
        // baked in at the last moment. Clearing here (rather than
        // on every render call) keeps the collect cost aligned
        // with the tree rebuild cadence — i.e. only when something
        // structural changed.
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
                // with the legacy edit / undo paths). Stringifying
                // here keeps the allocation off the helper's
                // critical path so overlay / canvas-scene callers
                // never pay it.
                self.mindmap_buffers.insert(unique_id.to_string(), buffer);
            },
            |rect| self.node_background_rects.push(rect),
        );
    }

    /// Rebuild the screen-space buffer list for every tree the app
    /// has registered into [`crate::application::scene_host::AppScene`].
    /// Walks the scene in layer
    /// order and produces one flat list; callers do not need to
    /// know about individual overlays. The renderer composites the
    /// result into the palette pass alongside the per-overlay
    /// buffer stores that predate this refactor — once every
    /// overlay has migrated to a tree, those per-overlay stores go
    /// away (see Session 5 in the unified-rendering plan).
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
                    // a screen-space rect pipeline yet. When
                    // Sessions 3 / 4 need them they can add a
                    // dedicated `overlay_scene_background_rects`
                    // field and a screen-space draw pass.
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
    /// Full (non-keyed) border rebuild — wipes the keyed cache and rebuilds
    /// every element from scratch. Used on map load, undo, reparent,
    /// selection change, and anywhere else the caller already knows every
    /// border may have changed.
    pub fn rebuild_border_buffers(&mut self, border_elements: &[BorderElement]) {
        self.border_buffers.clear();
        self.rebuild_border_buffers_keyed(border_elements, None);
    }

    /// Keyed border rebuild. If `dirty_node_ids` is `Some`, only entries
    /// whose `node_id` is in the set are re-shaped from scratch; clean
    /// entries have only their position patched in place on the existing
    /// cached buffers. Keys not present in `border_elements` are evicted
    /// at the end of the call. If `dirty_node_ids` is `None`, everything
    /// is treated as dirty (full re-shape).
    ///
    /// This is Phase B of the "Connection & border render cost" fix: on
    /// a drag frame that moves one node, only that node's border cache
    /// entry is re-shaped. All other visible borders reuse their shaped
    /// `cosmic_text::Buffer`s — cosmic-text shaping is the dominant cost
    /// here, so skipping it for unmoved borders is the point.
    pub fn rebuild_border_buffers_keyed(
        &mut self,
        border_elements: &[BorderElement],
        dirty_node_ids: Option<&std::collections::HashSet<String>>,
    ) {
        // Layout constants live on `baumhard::mindmap::border` so
        // this path and `tree_builder::build_border_tree` can't
        // drift on geometry. See the doc on the constants for the
        // empirical rationale.
        use baumhard::mindmap::border::{
            BORDER_APPROX_CHAR_WIDTH_FRAC, BORDER_CORNER_OVERLAP_FRAC,
        };

        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        let mut seen: std::collections::HashSet<String> =
            std::collections::HashSet::with_capacity(border_elements.len());

        for elem in border_elements {
            seen.insert(elem.node_id.clone());
            let is_dirty = dirty_node_ids
                .map(|set| set.contains(&elem.node_id))
                .unwrap_or(true);

            let font_size = elem.border_style.font_size_pt;
            let (nx, ny) = elem.node_position;
            let (nw, nh) = elem.node_size;

            // --- Horizontal math (identical to the classic path) ---
            let approx_char_width = font_size * BORDER_APPROX_CHAR_WIDTH_FRAC;
            let char_count = ((nw / approx_char_width) + 2.0)
                .ceil()
                .max(3.0) as usize;
            let right_corner_x =
                nx - approx_char_width + (char_count - 1) as f32 * approx_char_width;
            let corner_overlap = font_size * BORDER_CORNER_OVERLAP_FRAC;
            let top_y = ny - font_size + corner_overlap;
            let bottom_y = ny + nh - corner_overlap;

            // Fast path: cached, clean, matching glyph count. Patch positions
            // in place and skip re-shaping. We require the char_count to
            // match the cached entry's top run bounds so any node-resize
            // edits (rare outside drag) still force a re-shape.
            if !is_dirty {
                if let Some(existing) = self.border_buffers.get_mut(&elem.node_id) {
                    if existing.len() == 4 {
                        // Sanity: the top run's `bounds.0` is char_count-
                        // dependent. If it has diverged (e.g. the node was
                        // resized), re-shape.
                        let expected_h_width = (char_count as f32 + 1.0) * approx_char_width;
                        if (existing[0].bounds.0 - expected_h_width).abs() < 0.5 {
                            existing[0].pos = (nx - approx_char_width, top_y);
                            existing[1].pos = (nx - approx_char_width, bottom_y);
                            existing[2].pos = (nx - approx_char_width, ny);
                            existing[3].pos = (right_corner_x, ny);
                            continue;
                        }
                    }
                }
            }

            // Slow path: shape fresh. Compute the strings, allocate 4
            // buffers, insert into the cache (replacing any previous
            // entry).
            let border_color = parse_hex_color(&elem.border_style.color)
                .unwrap_or(cosmic_text::Color::rgba(255, 255, 255, 255));
            let glyph_set = &elem.border_style.glyph_set;
            let border_attrs = Attrs::new()
                .color(border_color)
                .metrics(cosmic_text::Metrics::new(font_size, font_size));

            let h_width = (char_count as f32 + 1.0) * approx_char_width;
            let v_width = approx_char_width * 2.0;

            let row_count = (nh / font_size).round().max(1.0) as usize;

            let top_text = glyph_set.top_border(char_count);
            let bottom_text = glyph_set.bottom_border(char_count);
            let left_text: String =
                std::iter::repeat_n(format!("{}\n", glyph_set.left_char()), row_count).collect();
            let right_text: String =
                std::iter::repeat_n(format!("{}\n", glyph_set.right_char()), row_count).collect();

            let entry = vec![
                create_border_buffer(
                    &mut font_system, &top_text, &border_attrs, font_size,
                    (nx - approx_char_width, top_y),
                    (h_width, font_size * 1.5),
                ),
                create_border_buffer(
                    &mut font_system, &bottom_text, &border_attrs, font_size,
                    (nx - approx_char_width, bottom_y),
                    (h_width, font_size * 1.5),
                ),
                create_border_buffer(
                    &mut font_system, &left_text, &border_attrs, font_size,
                    (nx - approx_char_width, ny),
                    (v_width, nh),
                ),
                create_border_buffer(
                    &mut font_system, &right_text, &border_attrs, font_size,
                    (right_corner_x, ny),
                    (v_width, nh),
                ),
            ];
            self.border_buffers.insert(elem.node_id.clone(), entry);
        }

        // Evict any cached entries whose node_id is no longer in the scene
        // (fold toggle, delete, show_frame = false, etc.).
        self.border_buffers.retain(|k, _| seen.contains(k));
    }

    /// Rebuild connection buffers from flat connection elements (from RenderScene).
    ///
    /// Per-glyph viewport culling is applied here. For each connection element,
    /// the visible canvas rect (expanded by `font_size` on each side as a
    /// margin) is tested against every glyph position; glyphs outside are
    /// skipped without creating a buffer. This is Phase 4(A) of the
    /// connection-render cost work: the dominant per-frame cost in this
    /// function is cosmic-text shaping, and a long cross-link during drag
    /// has thousands of sample positions the vast majority of which are
    /// off-screen. Skipping their buffer creation avoids the shaping cost
    /// entirely. The existing downstream cull in `render()` (line ~396)
    /// was only saving the rasterization of already-shaped buffers; the
    /// shaping had already happened.
    /// Full (non-keyed) connection rebuild — wipes the keyed cache and
    /// rebuilds every element from scratch. Used on map load, undo,
    /// reparent, edge CRUD, and anywhere else the caller already knows
    /// every connection may have changed.
    pub fn rebuild_connection_buffers(&mut self, connection_elements: &[ConnectionElement]) {
        self.connection_buffers.clear();
        self.rebuild_connection_buffers_keyed(connection_elements, None);
    }

    /// Rebuild the edge grab-handle overlay buffers. Called after every
    /// scene build — the handles are bounded (≤ 5 per selected edge)
    /// and always rebuilt from scratch, so no keyed cache is used.
    /// When `handles` is empty (nothing selected or selection is a
    /// node/None) this clears the buffer list and returns.
    pub fn rebuild_edge_handle_buffers(
        &mut self,
        handles: &[baumhard::mindmap::scene_builder::EdgeHandleElement],
    ) {
        self.edge_handle_buffers.clear();
        if handles.is_empty() {
            return;
        }
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");
        for handle in handles {
            let cosmic_color = parse_hex_color(&handle.color)
                .unwrap_or(cosmic_text::Color::rgba(0, 229, 255, 255));
            let attrs = Attrs::new()
                .color(cosmic_color)
                .metrics(cosmic_text::Metrics::new(handle.font_size_pt, handle.font_size_pt));

            // Center the glyph on the handle position. `approx_char_width`
            // keeps the math consistent with the connection glyph layout.
            let half_w = handle.font_size_pt * 0.3;
            let half_h = handle.font_size_pt * 0.5;
            let pos = (handle.position.0 - half_w, handle.position.1 - half_h);
            let bounds = (handle.font_size_pt, handle.font_size_pt);

            self.edge_handle_buffers.push(create_border_buffer(
                &mut font_system,
                &handle.glyph,
                &attrs,
                handle.font_size_pt,
                pos,
                bounds,
            ));
        }
    }

    /// Rebuild the console overlay buffers. When `geometry` is
    /// `None`, the console is closed — clear the buffer list and the
    /// backdrop, and return. When `Some`, lay out a bottom-anchored
    /// glyph-rendered strip: sacred border, scrollback region,
    /// optional completion popup, and the prompt line with cursor.
    ///
    /// Everything is positioned in screen coordinates (the render
    /// pass draws `console_overlay_buffers` with `scale = 1.0`), so
    /// the console stays a fixed size regardless of canvas zoom.
    pub fn rebuild_console_overlay_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: Option<&ConsoleOverlayGeometry>,
    ) {
        use crate::application::scene_host::{OverlayDispatch, OverlayRole};

        let Some(geometry) = geometry else {
            // Closed: drop the backdrop, drop the tree, refresh
            // overlay buffers so the console disappears. The
            // structural-signature cache lives on `AppScene` and
            // is cleared inside `unregister_overlay`, so the next
            // reopen starts from a clean slate.
            self.console_backdrop = None;
            app_scene.unregister_overlay(OverlayRole::Console);
            self.rebuild_overlay_scene_buffers(app_scene);
            return;
        };

        let layout = compute_console_frame_layout(
            geometry,
            self.config.width as f32,
            self.config.height as f32,
        );
        self.console_backdrop = Some(layout.backdrop_rect());
        let signature = console_overlay_signature(&layout);

        // §B2 dispatch: if the structural signature
        // (`scrollback_rows` × `completion_rows`) hasn't changed
        // since the last build, the existing tree's slot count
        // still matches and we apply an in-place mutator that
        // overwrites every slot's variable fields. Window resize
        // is the only typical event that shifts the signature, so
        // the mutator path covers every keystroke / scrollback-
        // grow / completion-update / Tab-cycle frame.
        match app_scene.overlay_dispatch(OverlayRole::Console, signature) {
            OverlayDispatch::InPlaceMutator => {
                let mutator = {
                    let mut font_system = fonts::FONT_SYSTEM
                        .write()
                        .expect("Failed to acquire font_system lock");
                    build_console_overlay_mutator(geometry, &layout, &mut font_system)
                };
                app_scene.apply_overlay_mutator(OverlayRole::Console, &mutator);
            }
            OverlayDispatch::FullRebuild => {
                // Build the tree under the FONT_SYSTEM lock — we
                // need it for `measure_max_glyph_advance` only.
                // Tree construction itself doesn't shape; that
                // happens during the overlay-scene walk below.
                let tree = {
                    let mut font_system = fonts::FONT_SYSTEM
                        .write()
                        .expect("Failed to acquire font_system lock");
                    build_console_overlay_tree(geometry, &layout, &mut font_system)
                };
                app_scene.register_overlay(OverlayRole::Console, tree, glam::Vec2::ZERO);
                app_scene.set_overlay_signature(OverlayRole::Console, signature);
            }
        }
        self.rebuild_overlay_scene_buffers(app_scene);
    }


    /// Build the picker's overlay tree from `geometry`, register
    /// it under [`OverlayRole::ColorPicker`](crate::application::scene_host::OverlayRole),
    /// and walk the overlay sub-scene into
    /// `overlay_scene_buffers`. `None` means the picker is closed
    /// — drops the backdrop, unregisters the tree, refreshes
    /// overlay buffers so it disappears.
    ///
    /// Called by `open_color_picker`, the `Resized` handler, and
    /// the hover / chip-focus / commit / cancel paths in
    /// `app::rebuild_color_picker_overlay`.
    ///
    /// **Performance note**: every invocation re-shapes every
    /// glyph in the picker (~64 cells). The legacy split that
    /// skipped re-shaping the static hue ring on hover is gone;
    /// the planned `MutatorTree`-based hover path will mutate
    /// only changed cell colors and the indicator's position
    /// per §B1 of `lib/baumhard/CONVENTIONS.md`.
    pub fn rebuild_color_picker_overlay_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry_and_layout: Option<(
            &crate::application::color_picker::ColorPickerOverlayGeometry,
            &crate::application::color_picker::ColorPickerLayout,
        )>,
    ) {
        self.color_picker_backdrop =
            color_picker::prepare_overlay_for_rebuild(app_scene, geometry_and_layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }

    /// §B2 mutation path — apply the **layout-phase** delta to the
    /// picker overlay tree without rebuilding the arena. Pairs with
    /// [`crate::application::color_picker_overlay::build_mutator`]:
    /// every variable field on every picker GlyphArea is overwritten
    /// via an `Assign` `DeltaGlyphArea` keyed by stable channel.
    ///
    /// Use this only when something the layout depends on actually
    /// changed (viewport resize, RMB size_scale drag, drag-move
    /// repositioning the wheel). Per-frame hover/HSV/chip updates
    /// should call [`Self::apply_color_picker_overlay_dynamic_mutator`]
    /// instead — same arena, slimmer per-cell delta. Open / close
    /// still use [`Self::rebuild_color_picker_overlay_buffers`]
    /// because the arena needs to be created or torn down. Calls
    /// `rebuild_overlay_scene_buffers` afterward to refresh the
    /// shaped buffers — the cosmic-text shape pass is still per-
    /// element, which is the §B1 perf gap tracked in `ROADMAP.md`.
    pub fn apply_color_picker_overlay_mutator(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
        layout: &crate::application::color_picker::ColorPickerLayout,
    ) {
        color_picker::apply_layout_mutator(app_scene, geometry, layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }

    /// §B2 mutation path — apply the **dynamic-phase** delta to the
    /// picker overlay tree. Pairs with
    /// [`crate::application::color_picker_overlay::build_dynamic_mutator`]:
    /// only the per-frame fields (color regions, hover scale, hex
    /// text) are written; layout-phase fields stay as the previous
    /// layout-mutator wrote them.
    ///
    /// This is the per-frame hot path for hover / HSV / chip-focus
    /// updates — the picker's element set, position, and bounds are
    /// unchanged. Calls `rebuild_overlay_scene_buffers` afterward to
    /// refresh the shaped buffers.
    pub fn apply_color_picker_overlay_dynamic_mutator(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
        layout: &crate::application::color_picker::ColorPickerLayout,
    ) {
        color_picker::apply_dynamic_mutator(app_scene, geometry, layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }


    /// Keyed connection rebuild. See [`Self::rebuild_border_buffers_keyed`] for
    /// the general pattern.
    ///
    /// If `dirty_edge_keys` is `Some`, clean edges (those whose
    /// `edge_key` is not in the set AND whose cached glyph count matches
    /// the current element's) only have their glyph *positions* patched
    /// in place — no cosmic-text shaping. Dirty edges are fully re-shaped.
    /// When `dirty_edge_keys` is `None`, everything is treated as dirty.
    ///
    /// **Interaction with Phase A viewport culling**: the off-screen glyph
    /// cull is a function of the camera, not the edge geometry, so on a
    /// camera pan the set of visible glyphs for a stable edge can change.
    /// The caller is responsible for clearing `self.connection_buffers`
    /// when the camera moves (see `process_decree` / `update_surface_size`)
    /// so that the dirty-set mechanism starts from a clean slate post-pan.
    /// Per-element the cull still runs here; clean fast-path kicks in
    /// only when the resulting visible-glyph count matches the cached
    /// entry's length.
    pub fn rebuild_connection_buffers_keyed(
        &mut self,
        connection_elements: &[ConnectionElement],
        dirty_edge_keys: Option<&std::collections::HashSet<EdgeKey>>,
    ) {
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        // Compute the visible canvas-space rectangle once.
        let vp_w = self.config.width as f32;
        let vp_h = self.config.height as f32;
        let corner_tl = self.camera.screen_to_canvas(Vec2::new(0.0, 0.0));
        let corner_br = self.camera.screen_to_canvas(Vec2::new(vp_w, vp_h));
        let vp_min = corner_tl.min(corner_br);
        let vp_max = corner_tl.max(corner_br);

        let mut seen: std::collections::HashSet<EdgeKey> =
            std::collections::HashSet::with_capacity(connection_elements.len());

        for elem in connection_elements {
            seen.insert(elem.edge_key.clone());
            let is_dirty = dirty_edge_keys
                .map(|set| set.contains(&elem.edge_key))
                .unwrap_or(true);

            let font_size = elem.font_size_pt;
            let half_glyph = font_size * 0.3;
            let half_height = font_size * 0.5;
            let glyph_bounds = (font_size, font_size);

            // Collect the positions that survive the viewport cull, in
            // order: cap_start, body glyphs, cap_end. Both the fast and
            // slow paths walk this same set.
            let in_view = |x: f32, y: f32| -> bool {
                glyph_position_in_viewport(x, y, vp_min, vp_max, font_size)
            };
            let mut visible_positions: Vec<(f32, f32)> =
                Vec::with_capacity(elem.glyph_positions.len() + 2);
            if let Some((_, cap_pos)) = &elem.cap_start {
                if in_view(cap_pos.0, cap_pos.1) {
                    visible_positions.push((cap_pos.0 - half_glyph, cap_pos.1 - half_height));
                }
            }
            for &pos in &elem.glyph_positions {
                if in_view(pos.0, pos.1) {
                    visible_positions.push((pos.0 - half_glyph, pos.1 - half_height));
                }
            }
            if let Some((_, cap_pos)) = &elem.cap_end {
                if in_view(cap_pos.0, cap_pos.1) {
                    visible_positions.push((cap_pos.0 - half_glyph, cap_pos.1 - half_height));
                }
            }

            if visible_positions.is_empty() {
                // Nothing visible — keep any existing cache entry? No, we
                // need to drop it so the buffer map doesn't hold stale
                // glyphs that would re-appear if `dirty_edge_keys`
                // bypassed this edge on the next frame. Just remove it.
                self.connection_buffers.remove(&elem.edge_key);
                continue;
            }

            // Fast path: clean + cached + same glyph count → patch
            // positions in place without re-shaping.
            if !is_dirty {
                if let Some(existing) = self.connection_buffers.get_mut(&elem.edge_key) {
                    if existing.len() == visible_positions.len() {
                        for (buf, new_pos) in existing.iter_mut().zip(visible_positions.iter()) {
                            buf.pos = *new_pos;
                        }
                        continue;
                    }
                }
            }

            // Slow path: re-shape. Build attrs once per element and emit
            // a fresh `Vec` of shaped buffers matching the same order as
            // `visible_positions`.
            let conn_color = parse_hex_color(&elem.color)
                .unwrap_or(cosmic_text::Color::rgba(200, 200, 200, 255));
            let conn_attrs = Attrs::new()
                .color(conn_color)
                .metrics(cosmic_text::Metrics::new(font_size, font_size));

            let mut new_entry: Vec<MindMapTextBuffer> =
                Vec::with_capacity(visible_positions.len());

            let cap_start_visible = elem
                .cap_start
                .as_ref()
                .map(|(_, p)| in_view(p.0, p.1))
                .unwrap_or(false);
            let cap_end_visible = elem
                .cap_end
                .as_ref()
                .map(|(_, p)| in_view(p.0, p.1))
                .unwrap_or(false);

            let mut idx = 0;
            if cap_start_visible {
                let cap_text = elem.cap_start.as_ref().map(|(t, _)| t.as_str()).unwrap_or("");
                new_entry.push(create_border_buffer(
                    &mut font_system, cap_text, &conn_attrs, font_size,
                    visible_positions[idx],
                    glyph_bounds,
                ));
                idx += 1;
            }
            for &pos in &elem.glyph_positions {
                if !in_view(pos.0, pos.1) {
                    continue;
                }
                new_entry.push(create_border_buffer(
                    &mut font_system, &elem.body_glyph, &conn_attrs, font_size,
                    visible_positions[idx],
                    glyph_bounds,
                ));
                idx += 1;
            }
            if cap_end_visible {
                let cap_text = elem.cap_end.as_ref().map(|(t, _)| t.as_str()).unwrap_or("");
                new_entry.push(create_border_buffer(
                    &mut font_system, cap_text, &conn_attrs, font_size,
                    visible_positions[idx],
                    glyph_bounds,
                ));
            }

            self.connection_buffers.insert(elem.edge_key.clone(), new_entry);
        }

        // Evict any cached entries whose edge key is no longer in the
        // scene — handles edge deletion / fold toggle.
        self.connection_buffers.retain(|k, _| seen.contains(k));
    }

    /// Session 6D: rebuild the per-edge label buffers from a freshly
    /// computed scene. Labels are rendered as individual cosmic-text
    /// buffers centered on their AABB, with a hitbox recorded so the
    /// app can detect clicks for inline label editing.
    ///
    /// The inline label-edit preview (buffer text + caret) is applied
    /// upstream in `scene_builder` via `MindMapDocument::label_edit_preview`,
    /// so the renderer can treat every label element as the final
    /// text to draw — no read-time override, no side channel.
    pub fn rebuild_connection_label_buffers(
        &mut self,
        label_elements: &[baumhard::mindmap::scene_builder::ConnectionLabelElement],
    ) {
        self.connection_label_buffers.clear();
        self.connection_label_hitboxes.clear();
        if label_elements.is_empty() {
            return;
        }
        let mut font_system = fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");

        for elem in label_elements {
            let cosmic_color = parse_hex_color(&elem.color)
                .unwrap_or(cosmic_text::Color::rgba(235, 235, 235, 255));
            let attrs = Attrs::new()
                .color(cosmic_color)
                .metrics(cosmic_text::Metrics::new(elem.font_size_pt, elem.font_size_pt));

            let buffer = create_border_buffer(
                &mut font_system,
                &elem.text,
                &attrs,
                elem.font_size_pt,
                elem.position,
                elem.bounds,
            );
            self.connection_label_buffers
                .insert(elem.edge_key.clone(), buffer);

            let min = Vec2::new(elem.position.0, elem.position.1);
            let max = Vec2::new(
                elem.position.0 + elem.bounds.0,
                elem.position.1 + elem.bounds.1,
            );
            self.connection_label_hitboxes
                .insert(elem.edge_key.clone(), (min, max));
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
}
