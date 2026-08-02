// SPDX-License-Identifier: MPL-2.0

//! App-layer scene host bridging the application and
//! [`baumhard::gfx_structs::scene::Scene`].
//!
//! `AppScene` owns Baumhard `Scene` sub-scenes plus role-keyed slots
//! for the canvas trees (borders, connections, portals, edge handles,
//! connection labels) and the screen-space overlays (console, color
//! picker). Each role's tree is registered/unregistered or updated in
//! place via §B2 mutator dispatch driven by a structural signature.
//!
//! It is also the app's hit-test entry point for tree-rendered
//! content. `overlay_at` resolves screen-space overlay clicks;
//! `portal_at` / `edge_label_at` resolve canvas-space clicks on the
//! interactive canvas roles by running Baumhard's BVH over the very
//! tree that produced the pixels, then naming the hit through the
//! role's identity index. There is one spatial index for canvas
//! content — the per-tree BVH — and no side table of rectangles.

use std::collections::HashMap;

use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::scene::{Scene, SceneTreeId};
use baumhard::gfx_structs::tree::{MutatorTree, Tree};
use baumhard::mindmap::scene_cache::EdgeKey;
use baumhard::mindmap::tree_builder::{ConnectionLabelHitIndex, PortalHit, PortalHitIndex};
use glam::Vec2;
use indextree::NodeId;

/// The fixed **screen-space** overlay components that the app's
/// `AppScene` can host. Overlay trees are drawn without the camera
/// transform at `scale = 1.0` (see the renderer's palette pass).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum OverlayRole {
    /// Command-line overlay. Registered when the console opens,
    /// removed on close. See `src/application/console/`.
    Console,
    /// Inline color wheel. Registered when the picker opens,
    /// removed on commit or cancel. See
    /// `src/application/color_picker.rs`.
    ColorPicker,
}

/// The fixed **canvas-space** components that the app's `AppScene`
/// can host. Canvas-space trees are drawn with the camera transform
/// so they pan and zoom with the mindmap. The mindmap itself is
/// deliberately not a variant — its tree still lives next to the
/// event loop and will migrate in a later pass.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum CanvasRole {
    /// The box-drawing glyphs framing each bordered node. Rebuilt
    /// from the MindMap model whenever layout changes.
    Borders,
    /// Bezier-path glyph strings connecting edge endpoints,
    /// including caps.
    Connections,
    /// Portal marker glyphs drawn above endpoints.
    Portals,
    /// Grab-handles on a selected edge (≤ 5 per selection).
    EdgeHandles,
    /// Labels attached to labeled edges.
    ConnectionLabels,
    /// Resize handles on the selected `Some`-sized section
    /// (8 per selection, or 0 for `None`-sized fill-parent
    /// sections / non-section selection).
    SectionResizeHandles,
    /// Resize handles on the selected node (8 per Single-node
    /// selection with finite + positive size, 0 otherwise).
    NodeResizeHandles,
    /// Per-section thin frames rendered on the active NodeEdit
    /// node so the user sees which section subdivisions can be
    /// picked. Empty in Default mode and on single-section nodes.
    SectionFrames,
}

/// Two arms of the §B2 canvas-tree dispatch: apply a mutator to the
/// existing arena, or rebuild wholesale and re-register. Returned
/// from [`AppScene::canvas_dispatch`] so the caller can route
/// side-effects (hit-index re-stamping, renderer state) through
/// the matching arm without re-implementing the check.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum CanvasDispatch {
    /// Signature matched the registered tree — apply the mutator.
    InPlaceMutator,
    /// Signature mismatch (or nothing registered) — rebuild + re-register.
    FullRebuild,
}

/// Two arms of the §B2 overlay-tree dispatch — mirrors
/// [`CanvasDispatch`] for the screen-space sub-scene. Returned by
/// [`AppScene::overlay_dispatch`] so the console / color-picker
/// rebuild paths can choose between the mutator and full-rebuild
/// arms using the same idiom as the canvas-side dispatchers.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum OverlayDispatch {
    /// Signature matched the registered tree — apply the mutator.
    InPlaceMutator,
    /// Signature mismatch (or nothing registered) — rebuild + re-register.
    FullRebuild,
}

/// Hash an arbitrary structural identity into the 64-bit signature
/// [`AppScene::canvas_signature`] tracks. Shared by every canvas-role
/// dispatcher — one DefaultHasher incantation instead of four.
pub fn hash_canvas_signature<T: std::hash::Hash>(identity: &T) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    let mut h = DefaultHasher::new();
    identity.hash(&mut h);
    h.finish()
}

/// Conventional layer integers used when inserting trees into the
/// appropriate `AppScene` sub-scene. Higher layers draw (and
/// hit-test) on top inside their own sub-scene; cross-sub-scene
/// ordering is fixed by the render pass (canvas-space first, then
/// screen-space overlays on top).
pub mod layers {
    // --- Canvas-space layers ----------------------------------

    /// Mindmap body — every canvas-space component draws above
    /// this once the migration reaches that step.
    pub const MINDMAP: i32 = 0;
    /// Borders sit just above the mindmap text.
    pub const BORDERS: i32 = 10;
    /// Section frames sit just above the node borders so the
    /// per-section subdivisions are visible on top of the node
    /// chrome but below connections (which already draw above
    /// borders to avoid label occlusion).
    pub const SECTION_FRAMES: i32 = 15;
    /// Connections over the borders so a labeled edge on a framed
    /// node isn't occluded by the frame glyphs.
    pub const CONNECTIONS: i32 = 20;
    /// Portal markers.
    pub const PORTALS: i32 = 30;
    /// Connection labels.
    pub const CONNECTION_LABELS: i32 = 40;
    /// Edge handles draw on top of everything canvas-space so the
    /// user can always grab them.
    pub const EDGE_HANDLES: i32 = 50;
    /// Section resize handles — same "always on top" rationale as
    /// edge handles. One layer above so they win on the rare
    /// pixel-overlap with an edge handle.
    pub const SECTION_RESIZE_HANDLES: i32 = 51;
    /// Node resize handles — one layer above section resize
    /// handles so they win on a layered selection cycle (a Single
    /// node selection precludes a Section selection by enum
    /// construction, so they don't actually coexist; the layer
    /// ordering is precautionary).
    pub const NODE_RESIZE_HANDLES: i32 = 52;

    // --- Screen-space (overlay) layers ------------------------

    /// Color picker modal. Above the canvas scene, below console
    /// (so typing in the console while a picker is open still
    /// works if the two are ever both visible, although today
    /// they're mutually exclusive).
    pub const COLOR_PICKER: i32 = 100;
    /// Console overlay.
    pub const CONSOLE: i32 = 200;
}

/// App-facing scene owning every tree-rendered component, split by
/// coordinate space: canvas-space (camera-transformed) and
/// overlay (screen-space, scale = 1.0). The renderer uses different
/// pipelines for each.
pub struct AppScene {
    canvas: Scene,
    overlay: Scene,
    console: Option<SceneTreeId>,
    color_picker: Option<SceneTreeId>,
    borders: Option<SceneTreeId>,
    connections: Option<SceneTreeId>,
    portals: Option<SceneTreeId>,
    edge_handles: Option<SceneTreeId>,
    connection_labels: Option<SceneTreeId>,
    section_resize_handles: Option<SceneTreeId>,
    node_resize_handles: Option<SceneTreeId>,
    section_frames: Option<SceneTreeId>,
    /// Opaque per-role hash describing the **structural shape** of
    /// the registered canvas tree (not its variable-field state). A
    /// caller that wants to dispatch between full-rebuild and §B2
    /// in-place mutator paths records the structural hash alongside
    /// the registration; the next call hashes the new state and uses
    /// equality to decide. Cleared on `unregister_canvas`. The
    /// definition of "structure" is the caller's — for portals, it's
    /// the visible-pair identity sequence; for edge handles, it's the
    /// selected-edge identity; etc.
    canvas_signatures: HashMap<CanvasRole, u64>,
    /// Same as `canvas_signatures`, for the overlay sub-scene. The
    /// console overlay uses `(scrollback_rows, completion_rows)` as
    /// its structural signature; color-picker uses none yet (its
    /// cell count is a compile-time constant). Cleared on
    /// `unregister_overlay`.
    overlay_signatures: HashMap<OverlayRole, u64>,
    /// Channel-path → `(EdgeKey, endpoint)` names for the
    /// [`CanvasRole::Portals`] tree. Re-stamped by the portal
    /// dispatcher on both the rebuild and in-place arms, so it
    /// always describes the tree currently registered. Carries no
    /// geometry — [`Self::portal_at`] gets that from the tree's own
    /// BVH.
    portal_hit_index: PortalHitIndex,
    /// Channel → `EdgeKey` names for the
    /// [`CanvasRole::ConnectionLabels`] tree. Same contract as
    /// [`Self::portal_hit_index`].
    connection_label_hit_index: ConnectionLabelHitIndex,
}

impl Default for AppScene {
    fn default() -> Self {
        Self::new()
    }
}

impl AppScene {
    /// Empty scene with no trees registered.
    pub fn new() -> Self {
        AppScene {
            canvas: Scene::new(),
            overlay: Scene::new(),
            console: None,
            color_picker: None,
            borders: None,
            connections: None,
            portals: None,
            edge_handles: None,
            connection_labels: None,
            section_resize_handles: None,
            node_resize_handles: None,
            section_frames: None,
            canvas_signatures: HashMap::new(),
            overlay_signatures: HashMap::new(),
            portal_hit_index: PortalHitIndex::default(),
            connection_label_hit_index: ConnectionLabelHitIndex::default(),
        }
    }

    // --- Canvas-space sub-scene -------------------------------

    /// Read-only view of the canvas-space sub-scene. Used by the
    /// renderer's main (camera-transformed) pass.
    pub fn canvas_scene(&self) -> &Scene {
        &self.canvas
    }

    /// Layer-ordered handles into the canvas sub-scene. Returned
    /// as a vector so the renderer can iterate without holding a
    /// `&mut Scene` (which would let a caller bypass `AppScene`
    /// role tracking by removing trees directly).
    pub fn canvas_ids_in_layer_order(&mut self) -> Vec<SceneTreeId> {
        self.canvas.ids_in_layer_order()
    }

    /// Register (or replace) a canvas-space role tree.
    pub fn register_canvas(
        &mut self,
        role: CanvasRole,
        tree: Tree<GfxElement, GfxMutator>,
        offset: Vec2,
    ) -> SceneTreeId {
        if let Some(old) = self.canvas_role_slot_mut(role).take() {
            self.canvas.remove(old);
        }
        let layer = match role {
            CanvasRole::Borders => layers::BORDERS,
            CanvasRole::Connections => layers::CONNECTIONS,
            CanvasRole::Portals => layers::PORTALS,
            CanvasRole::EdgeHandles => layers::EDGE_HANDLES,
            CanvasRole::ConnectionLabels => layers::CONNECTION_LABELS,
            CanvasRole::SectionResizeHandles => layers::SECTION_RESIZE_HANDLES,
            CanvasRole::NodeResizeHandles => layers::NODE_RESIZE_HANDLES,
            CanvasRole::SectionFrames => layers::SECTION_FRAMES,
        };
        let id = self.canvas.insert(tree, layer, offset);
        *self.canvas_role_slot_mut(role) = Some(id);
        id
    }

    /// Remove a canvas role's tree, if registered. Also clears any
    /// recorded structural signature — a future re-register starts
    /// from a clean slate, forcing a full rebuild before the in-place
    /// mutator path can run again — and the role's hit index, so no
    /// per-role state outlives the tree it describes.
    pub fn unregister_canvas(&mut self, role: CanvasRole) {
        if let Some(id) = self.canvas_role_slot_mut(role).take() {
            self.canvas.remove(id);
        }
        self.canvas_signatures.remove(&role);
        match role {
            CanvasRole::Portals => self.portal_hit_index = PortalHitIndex::default(),
            CanvasRole::ConnectionLabels => {
                self.connection_label_hit_index = ConnectionLabelHitIndex::default()
            }
            _ => {}
        }
    }

    /// Record the structural signature of the tree currently
    /// registered for `role`. Pair with [`Self::canvas_signature`] to
    /// dispatch between full-rebuild and in-place mutator paths.
    /// Hash space is the caller's; pass the same hash function on
    /// both sides.
    pub fn set_canvas_signature(&mut self, role: CanvasRole, signature: u64) {
        self.canvas_signatures.insert(role, signature);
    }

    /// Last recorded structural signature for a canvas role, if any.
    /// `None` when no tree is registered or the signature was never
    /// set — the caller treats that as "force full rebuild".
    pub fn canvas_signature(&self, role: CanvasRole) -> Option<u64> {
        self.canvas_signatures.get(&role).copied()
    }

    /// Decide which §B2 arm to take for a canvas role at the given
    /// structural signature: [`CanvasDispatch::InPlaceMutator`] if a
    /// tree is registered and its signature matches, else
    /// [`CanvasDispatch::FullRebuild`]. The caller then runs the
    /// matching build path and calls either `apply_canvas_mutator`
    /// or `register_canvas` + `set_canvas_signature`.
    ///
    /// Exists so every role's dispatcher has the same "if registered
    /// and signature matches" single source of truth, not four
    /// inlined copies of the check.
    pub fn canvas_dispatch(&self, role: CanvasRole, signature: u64) -> CanvasDispatch {
        if self.canvas_id(role).is_some() && self.canvas_signature(role) == Some(signature) {
            CanvasDispatch::InPlaceMutator
        } else {
            CanvasDispatch::FullRebuild
        }
    }

    /// Handle for a canvas role, if registered.
    pub fn canvas_id(&self, role: CanvasRole) -> Option<SceneTreeId> {
        match role {
            CanvasRole::Borders => self.borders,
            CanvasRole::Connections => self.connections,
            CanvasRole::Portals => self.portals,
            CanvasRole::EdgeHandles => self.edge_handles,
            CanvasRole::ConnectionLabels => self.connection_labels,
            CanvasRole::SectionResizeHandles => self.section_resize_handles,
            CanvasRole::NodeResizeHandles => self.node_resize_handles,
            CanvasRole::SectionFrames => self.section_frames,
        }
    }

    /// Apply a mutator to a canvas role's tree. Path of choice
    /// for incremental updates per §B1 of the baumhard
    /// conventions — the upcoming connection / border drag
    /// shaping cache will land through this.
    pub fn apply_canvas_mutator(&mut self, role: CanvasRole, mutator: &MutatorTree<GfxMutator>) {
        if let Some(id) = self.canvas_id(role) {
            self.canvas.apply_mutator(id, mutator);
        }
    }

    // --- Screen-space sub-scene -------------------------------

    /// Read-only view of the overlay sub-scene. Used by the
    /// renderer's palette (screen-space) pass.
    pub fn overlay_scene(&self) -> &Scene {
        &self.overlay
    }

    /// Layer-ordered handles into the overlay sub-scene. Same
    /// rationale as [`Self::canvas_ids_in_layer_order`].
    pub fn overlay_ids_in_layer_order(&mut self) -> Vec<SceneTreeId> {
        self.overlay.ids_in_layer_order()
    }

    /// Register (or replace) the tree backing a named overlay role.
    ///
    /// If a tree was already registered for the role it is removed
    /// first — overlay open-close cycles shouldn't leak slab
    /// entries.
    pub fn register_overlay(
        &mut self,
        role: OverlayRole,
        tree: Tree<GfxElement, GfxMutator>,
        offset: Vec2,
    ) -> SceneTreeId {
        if let Some(old) = self.overlay_role_slot_mut(role).take() {
            self.overlay.remove(old);
        }
        let layer = match role {
            OverlayRole::Console => layers::CONSOLE,
            OverlayRole::ColorPicker => layers::COLOR_PICKER,
        };
        let id = self.overlay.insert(tree, layer, offset);
        *self.overlay_role_slot_mut(role) = Some(id);
        id
    }

    /// Remove an overlay's tree if registered. Also clears any
    /// recorded structural signature — a future re-register starts
    /// from a clean slate, forcing a full rebuild before the
    /// in-place mutator path can run again. Mirrors
    /// [`Self::unregister_canvas`].
    pub fn unregister_overlay(&mut self, role: OverlayRole) {
        if let Some(id) = self.overlay_role_slot_mut(role).take() {
            self.overlay.remove(id);
        }
        self.overlay_signatures.remove(&role);
    }

    /// Record the structural signature of the tree currently
    /// registered for `role`. Pair with
    /// [`Self::overlay_signature`] to dispatch between
    /// full-rebuild and in-place mutator paths. Mirrors
    /// [`Self::set_canvas_signature`].
    pub fn set_overlay_signature(&mut self, role: OverlayRole, signature: u64) {
        self.overlay_signatures.insert(role, signature);
    }

    /// Last recorded structural signature for an overlay role, if
    /// any. `None` when no tree is registered or the signature was
    /// never set — the caller treats that as "force full rebuild".
    /// Mirrors [`Self::canvas_signature`].
    pub fn overlay_signature(&self, role: OverlayRole) -> Option<u64> {
        self.overlay_signatures.get(&role).copied()
    }

    /// Decide which §B2 arm to take for an overlay role at the
    /// given structural signature. Mirrors
    /// [`Self::canvas_dispatch`] for the screen-space sub-scene.
    pub fn overlay_dispatch(&self, role: OverlayRole, signature: u64) -> OverlayDispatch {
        if self.overlay_id(role).is_some() && self.overlay_signature(role) == Some(signature) {
            OverlayDispatch::InPlaceMutator
        } else {
            OverlayDispatch::FullRebuild
        }
    }

    /// Handle currently assigned to the named overlay role.
    pub fn overlay_id(&self, role: OverlayRole) -> Option<SceneTreeId> {
        match role {
            OverlayRole::Console => self.console,
            OverlayRole::ColorPicker => self.color_picker,
        }
    }

    /// Apply a mutator to the tree registered for an overlay
    /// role. Path of choice for hover updates per §B1; the
    /// upcoming color-picker mutator-only hover path lands
    /// through this.
    pub fn apply_overlay_mutator(&mut self, role: OverlayRole, mutator: &MutatorTree<GfxMutator>) {
        if let Some(id) = self.overlay_id(role) {
            self.overlay.apply_mutator(id, mutator);
        }
    }

    /// Record the identity names for the currently registered
    /// [`CanvasRole::Portals`] tree. Called by the portal
    /// dispatcher on both §B2 arms — the index is cheap to rebuild,
    /// and stamping it unconditionally in the same call that
    /// registers or mutates the tree is what keeps channel order
    /// and identity order in step.
    ///
    /// [`Self::unregister_canvas`] clears it again, so the index
    /// never outlives the tree it names.
    pub fn set_portal_hit_index(&mut self, index: PortalHitIndex) {
        self.portal_hit_index = index;
    }

    /// Record the identity names for the currently registered
    /// [`CanvasRole::ConnectionLabels`] tree. Mirrors
    /// [`Self::set_portal_hit_index`].
    pub fn set_connection_label_hit_index(&mut self, index: ConnectionLabelHitIndex) {
        self.connection_label_hit_index = index;
    }

    /// Hit-test the portal role at a canvas-space point and name
    /// the sub-part that was hit.
    ///
    /// Resolution runs through the registered tree's BVH
    /// ([`Scene::component_in`]), so overlapping markers resolve
    /// by the tree's own rule — **smallest area wins**, ties
    /// broken toward the earlier sibling — rather than by the
    /// iteration order of a side map. Two icons stacked on each
    /// other therefore route the click to the visually topmost
    /// one, deterministically and independently of node ids.
    ///
    /// Returns `None` when no portal tree is registered or the
    /// point misses every marker. A text-less endpoint's reserved
    /// text slot lays out at zero extent and is invisible here,
    /// so a click beside a bare icon falls through to whatever is
    /// underneath instead of selecting empty portal text.
    ///
    /// # Costs
    ///
    /// One memoized tree-AABB reject, then on a hit one BVH
    /// descent plus an O(1) index lookup.
    pub fn portal_at(&mut self, canvas_pt: Vec2) -> Option<PortalHit> {
        let id = self.portals?;
        let node_id = self.canvas.component_in(id, canvas_pt)?;
        let tree = self.canvas.tree(id)?;
        self.portal_hit_index.resolve(tree, node_id)
    }

    /// Hit-test the connection-label role at a canvas-space point
    /// and name the owning edge. Sibling of [`Self::portal_at`]
    /// with the same resolution rule: labels on crossing edges
    /// that overlap resolve to the smaller label, not to whichever
    /// edge happened to hash first.
    ///
    /// # Costs
    ///
    /// One memoized tree-AABB reject, then on a hit one BVH
    /// descent plus an O(1) index lookup.
    pub fn edge_label_at(&mut self, canvas_pt: Vec2) -> Option<EdgeKey> {
        let id = self.connection_labels?;
        let node_id = self.canvas.component_in(id, canvas_pt)?;
        let tree = self.canvas.tree(id)?;
        self.connection_label_hit_index.resolve(tree, node_id)
    }

    /// Hit-test the overlay sub-scene. Canvas-space roles have
    /// their own entry points ([`Self::portal_at`],
    /// [`Self::edge_label_at`]) rather than being folded in here:
    /// overlay hits always win over canvas hits, and the canvas
    /// side is a priority ladder (node → portal → label → edge)
    /// that the app owns, not a flat top-most query. Node hits
    /// still go through `document::hit_test` against the mindmap
    /// tree, which has not migrated into `AppScene`.
    pub fn overlay_at(&mut self, screen_pt: Vec2) -> Option<(OverlayRole, NodeId)> {
        let hit = self.overlay.component_at(screen_pt)?;
        let role = self.overlay_role_for_id(hit.0)?;
        Some((role, hit.1))
    }

    fn overlay_role_slot_mut(&mut self, role: OverlayRole) -> &mut Option<SceneTreeId> {
        match role {
            OverlayRole::Console => &mut self.console,
            OverlayRole::ColorPicker => &mut self.color_picker,
        }
    }

    fn canvas_role_slot_mut(&mut self, role: CanvasRole) -> &mut Option<SceneTreeId> {
        match role {
            CanvasRole::Borders => &mut self.borders,
            CanvasRole::Connections => &mut self.connections,
            CanvasRole::Portals => &mut self.portals,
            CanvasRole::EdgeHandles => &mut self.edge_handles,
            CanvasRole::ConnectionLabels => &mut self.connection_labels,
            CanvasRole::SectionResizeHandles => &mut self.section_resize_handles,
            CanvasRole::NodeResizeHandles => &mut self.node_resize_handles,
            CanvasRole::SectionFrames => &mut self.section_frames,
        }
    }

    fn overlay_role_for_id(&self, id: SceneTreeId) -> Option<OverlayRole> {
        if Some(id) == self.console {
            Some(OverlayRole::Console)
        } else if Some(id) == self.color_picker {
            Some(OverlayRole::ColorPicker)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::font::fonts;
    use baumhard::gfx_structs::area::GlyphArea;
    use baumhard::mindmap::tree_builder::PortalPart;

    // ── canvas-role hit-testing ───────────────────────────────────

    /// Two portal-mode edges leaving `hub`, one carrying endpoint
    /// text. Written as JSON and pushed through the real loader so
    /// the fixture exercises the same parse path the app does —
    /// and because portal-mode edges are absent from every map in
    /// `maps/`, there is no on-disk fixture to borrow.
    const PORTAL_FIXTURE_JSON: &str = r##"
        {
          "canvas": {
            "background_color": "#141414",
            "theme_variables": {}
          },
          "edges": [
            {
              "anchor_from": "auto",
              "anchor_to": "auto",
              "color": "#ff0000",
              "control_points": [],
              "display_mode": "portal",
              "from_id": "hub",
              "glyph_connection": {
                "body": "\u25c8",
                "font_size_pt": 16.0
              },
              "label": null,
              "line_style": "solid",
              "portal_from": {
                "text": "label"
              },
              "to_id": "far",
              "type": "cross_link",
              "visible": true,
              "width": 3
            },
            {
              "anchor_from": "auto",
              "anchor_to": "auto",
              "color": "#00ff00",
              "control_points": [],
              "display_mode": "portal",
              "from_id": "hub",
              "glyph_connection": {
                "body": "\u25c8",
                "font_size_pt": 16.0
              },
              "label": null,
              "line_style": "solid",
              "to_id": "near",
              "type": "cross_link",
              "visible": true,
              "width": 3
            }
          ],
          "name": "portal_hit_fixture",
          "nodes": {
            "far": {
              "channel": 0,
              "folded": false,
              "id": "far",
              "layout": {
                "direction": "auto",
                "spacing": 50.0,
                "type": "map"
              },
              "notes": "",
              "parent_id": null,
              "position": {
                "x": 600.0,
                "y": 0.0
              },
              "sections": [
                {
                  "text": "far",
                  "text_runs": []
                }
              ],
              "size": {
                "height": 40.0,
                "width": 80.0
              },
              "style": {
                "background_color": "#141414",
                "corner_radius_percent": 0.0,
                "frame_color": "#888888",
                "frame_thickness": 1.0,
                "shape": "rectangle",
                "show_frame": true,
                "show_shadow": false,
                "text_color": "#eeeeee"
              }
            },
            "hub": {
              "channel": 0,
              "folded": false,
              "id": "hub",
              "layout": {
                "direction": "auto",
                "spacing": 50.0,
                "type": "map"
              },
              "notes": "",
              "parent_id": null,
              "position": {
                "x": 0.0,
                "y": 0.0
              },
              "sections": [
                {
                  "text": "hub",
                  "text_runs": []
                }
              ],
              "size": {
                "height": 40.0,
                "width": 80.0
              },
              "style": {
                "background_color": "#141414",
                "corner_radius_percent": 0.0,
                "frame_color": "#888888",
                "frame_thickness": 1.0,
                "shape": "rectangle",
                "show_frame": true,
                "show_shadow": false,
                "text_color": "#eeeeee"
              }
            },
            "near": {
              "channel": 0,
              "folded": false,
              "id": "near",
              "layout": {
                "direction": "auto",
                "spacing": 50.0,
                "type": "map"
              },
              "notes": "",
              "parent_id": null,
              "position": {
                "x": 0.0,
                "y": 400.0
              },
              "sections": [
                {
                  "text": "near",
                  "text_runs": []
                }
              ],
              "size": {
                "height": 40.0,
                "width": 80.0
              },
              "style": {
                "background_color": "#141414",
                "corner_radius_percent": 0.0,
                "frame_color": "#888888",
                "frame_thickness": 1.0,
                "shape": "rectangle",
                "show_frame": true,
                "show_shadow": false,
                "text_color": "#eeeeee"
              }
            }
          },
          "version": "1.0"
        }
    "##;

    /// Two portal-mode edges from `hub`, one carrying text, laid
    /// out by the real tree builder. Returns the map so a test can
    /// re-derive the pair data it needs.
    fn portal_fixture_map() -> baumhard::mindmap::model::MindMap {
        baumhard::mindmap::loader::load_from_str(PORTAL_FIXTURE_JSON).expect("portal fixture loads")
    }

    /// Register the portal role from `map` exactly the way
    /// `CanvasFrame::update_portal_tree` does — tree and hit index
    /// stamped together.
    fn register_portals(app: &mut AppScene, map: &baumhard::mindmap::model::MindMap) {
        fonts::init();
        let hidden = map.fold_hidden_set();
        let pairs = baumhard::mindmap::tree_builder::portal_pair_data(
            map,
            &HashMap::new(),
            None,
            None,
            None,
            None,
            1.0,
            &hidden,
        );
        let built = baumhard::mindmap::tree_builder::build_portal_tree_from_pairs(&pairs);
        app.set_portal_hit_index(built.hit_index);
        app.register_canvas(CanvasRole::Portals, built.tree, Vec2::ZERO);
    }

    /// The `(position, extent)` of every clickable leaf in the
    /// registered portal tree, paired with the identity the index
    /// gives it — the ground truth a probe is aimed at.
    fn portal_leaf_probes(app: &AppScene) -> Vec<(Vec2, PortalHit)> {
        let id = app.canvas_id(CanvasRole::Portals).expect("portals registered");
        let tree = app.canvas_scene().tree(id).expect("portal tree");
        let mut out = Vec::new();
        for pair in tree.root.children(&tree.arena) {
            for endpoint in pair.children(&tree.arena) {
                for leaf in endpoint.children(&tree.arena) {
                    let area = match tree.arena.get(leaf).and_then(|n| n.get().glyph_area()) {
                        Some(a) => a,
                        None => continue,
                    };
                    let pos = Vec2::new(area.position.x.0, area.position.y.0);
                    let extent = Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0);
                    if extent.x <= 0.0 || extent.y <= 0.0 {
                        continue;
                    }
                    if let Some(hit) = app.portal_hit_index.resolve(tree, leaf) {
                        out.push((pos + extent * 0.5, hit));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn test_portal_at_names_every_clickable_leaf_and_misses_elsewhere() {
        let map = portal_fixture_map();
        let mut app = AppScene::new();
        register_portals(&mut app, &map);

        let probes = portal_leaf_probes(&app);
        // Two pairs × two endpoints = four icons, plus one text
        // label on the `hub` side of the first edge. The other
        // three text slots are zero-extent and unclickable.
        assert_eq!(probes.len(), 5, "unexpected clickable-leaf count: {probes:?}");
        assert_eq!(
            probes.iter().filter(|(_, h)| h.part == PortalPart::Text).count(),
            1
        );
        for (point, expected) in &probes {
            assert_eq!(app.portal_at(*point).as_ref(), Some(expected));
        }
        assert_eq!(app.portal_at(Vec2::new(-5000.0, -5000.0)), None);
    }

    #[test]
    fn test_portal_at_is_none_when_the_role_is_not_registered() {
        // Two independent guards, both asserted here because either
        // alone would let a click resolve against state that no
        // longer describes anything on screen.
        let map = portal_fixture_map();
        let mut app = AppScene::new();
        assert_eq!(app.portal_at(Vec2::ZERO), None);

        // 1. An index with no tree behind it answers nothing —
        //    `portal_at` gates on the role slot, not on the index.
        register_portals(&mut app, &map);
        let probe = portal_leaf_probes(&app)[0].0;
        let orphan_index = app.portal_hit_index.clone();
        assert!(app.portal_at(probe).is_some());

        let mut bare = AppScene::new();
        bare.set_portal_hit_index(orphan_index);
        assert_eq!(
            bare.portal_at(probe),
            None,
            "an index with no registered tree must never produce a hit"
        );

        // 2. Unregistering drops the index too, so no per-role state
        //    outlives the tree it names.
        app.unregister_canvas(CanvasRole::Portals);
        assert_eq!(app.portal_at(probe), None);
        assert!(
            app.portal_hit_index.is_empty(),
            "unregister_canvas must clear the role's hit index"
        );
    }

    #[test]
    fn test_edge_label_at_names_the_owning_edge() {
        use baumhard::mindmap::tree_builder::{build_connection_label_tree, build_label_elements};

        fonts::init();
        let map = baumhard::mindmap::loader::load_from_file(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/maps/testament.mindmap.json"
        )))
        .expect("testament map loads");
        let hidden = map.fold_hidden_set();
        let elements = build_label_elements(&map, &HashMap::new(), None, None, None, 1.0, &hidden);
        assert!(!elements.is_empty(), "fixture should carry labels");
        let built = build_connection_label_tree(&elements);

        let mut app = AppScene::new();
        app.set_connection_label_hit_index(built.hit_index);
        app.register_canvas(CanvasRole::ConnectionLabels, built.tree, Vec2::ZERO);

        // A label center could in principle sit inside a smaller
        // overlapping label, in which case the smaller one is the
        // correct answer. Count how many label rects cover each
        // center so the assertion can be exact where the answer is
        // unambiguous, and still meaningful where it isn't.
        let rects: Vec<(Vec2, Vec2, &baumhard::mindmap::scene_cache::EdgeKey)> = elements
            .iter()
            .map(|e| {
                let min = Vec2::new(e.position.0, e.position.1);
                (min, min + Vec2::new(e.bounds.0, e.bounds.1), &e.edge_key)
            })
            .collect();
        let mut unambiguous = 0usize;
        for elem in &elements {
            let center = Vec2::new(
                elem.position.0 + elem.bounds.0 * 0.5,
                elem.position.1 + elem.bounds.1 * 0.5,
            );
            let covering: Vec<_> = rects
                .iter()
                .filter(|(min, max, _)| {
                    center.x >= min.x && center.x <= max.x && center.y >= min.y && center.y <= max.y
                })
                .collect();
            let hit = app.edge_label_at(center);
            if covering.len() == 1 {
                unambiguous += 1;
                assert_eq!(
                    hit.as_ref(),
                    Some(elem.edge_key.clone()).as_ref(),
                    "a label's own center must route to that label"
                );
            } else {
                // Overlap: whichever label answers must at least be
                // one of the labels actually covering the point.
                let hit = hit.expect("label center must hit some label");
                assert!(
                    covering.iter().any(|(_, _, k)| **k == hit),
                    "overlap answer {hit:?} is not one of the covering labels"
                );
            }
        }
        assert!(
            unambiguous > 0,
            "fixture should contain labels that do not overlap anything"
        );
        assert_eq!(app.edge_label_at(Vec2::new(-1.0e6, -1.0e6)), None);
    }

    #[test]
    fn test_canvas_role_hit_tests_honor_the_registered_offset() {
        // `register_canvas` takes an offset; a role registered away
        // from the origin must be hit-tested in the same space the
        // caller passes (canvas coordinates), not tree-local ones.
        let map = portal_fixture_map();
        let mut app = AppScene::new();
        register_portals(&mut app, &map);
        let probe = portal_leaf_probes(&app)[0].0;
        let expected = app.portal_at(probe).expect("baseline hit");

        let id = app.canvas_id(CanvasRole::Portals).unwrap();
        let shift = Vec2::new(1000.0, -250.0);
        app.canvas.set_offset(id, shift);
        assert_eq!(app.portal_at(probe), None, "old location must stop hitting");
        assert_eq!(app.portal_at(probe + shift).as_ref(), Some(&expected));
    }

    fn overlay_tree(position: Vec2, bounds: Vec2) -> (Tree<GfxElement, GfxMutator>, NodeId) {
        fonts::init();
        let mut tree = Tree::new_non_indexed();
        let area = GlyphArea::new_with_str("overlay", 1.0, 10.0, position, bounds);
        let element = GfxElement::new_area_non_indexed_with_id(area, 0, 0);
        let leaf = tree.arena.new_node(element);
        tree.root.append(leaf, &mut tree.arena);
        (tree, leaf)
    }

    #[test]
    fn register_and_unregister_roundtrips_cleanly() {
        let mut app = AppScene::new();
        let (tree, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        let id = app.register_overlay(OverlayRole::Console, tree, Vec2::ZERO);
        assert_eq!(app.overlay_id(OverlayRole::Console), Some(id));

        app.unregister_overlay(OverlayRole::Console);
        assert_eq!(app.overlay_id(OverlayRole::Console), None);
        assert_eq!(app.overlay_scene().len(), 0);
    }

    #[test]
    fn re_registering_replaces_the_previous_tree() {
        let mut app = AppScene::new();
        let (t1, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        let (t2, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        app.register_overlay(OverlayRole::Console, t1, Vec2::ZERO);
        let id2 = app.register_overlay(OverlayRole::Console, t2, Vec2::ZERO);
        assert_eq!(app.overlay_scene().len(), 1);
        assert_eq!(app.overlay_id(OverlayRole::Console), Some(id2));
    }

    #[test]
    fn canvas_and_overlay_roles_live_in_separate_sub_scenes() {
        let mut app = AppScene::new();
        let (border_tree, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        let (console_tree, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));

        app.register_canvas(CanvasRole::Borders, border_tree, Vec2::ZERO);
        app.register_overlay(OverlayRole::Console, console_tree, Vec2::ZERO);

        assert_eq!(app.canvas_scene().len(), 1);
        assert_eq!(app.overlay_scene().len(), 1);
        assert!(app.canvas_id(CanvasRole::Borders).is_some());
        assert!(app.overlay_id(OverlayRole::Console).is_some());

        // Hit-test on overlay ignores the canvas border tree.
        let hit = app.overlay_at(Vec2::new(5.0, 5.0));
        assert_eq!(hit.map(|(r, _)| r), Some(OverlayRole::Console));
    }

    #[test]
    fn overlay_at_returns_role_for_hits() {
        let mut app = AppScene::new();
        let (console, leaf) = overlay_tree(Vec2::ZERO, Vec2::new(50.0, 50.0));
        app.register_overlay(OverlayRole::Console, console, Vec2::new(100.0, 100.0));

        let hit = app.overlay_at(Vec2::new(110.0, 110.0));
        assert_eq!(hit, Some((OverlayRole::Console, leaf)));

        let miss = app.overlay_at(Vec2::new(10.0, 10.0));
        assert_eq!(miss, None);
    }

    #[test]
    fn console_draws_above_color_picker_by_default_layer() {
        // Both cover (0,0)-(50,50). Console should win because its
        // layer is higher in `layers::*`.
        let mut app = AppScene::new();
        let (picker_tree, _) = overlay_tree(Vec2::ZERO, Vec2::new(50.0, 50.0));
        let (console_tree, console_leaf) = overlay_tree(Vec2::ZERO, Vec2::new(50.0, 50.0));
        app.register_overlay(OverlayRole::ColorPicker, picker_tree, Vec2::ZERO);
        app.register_overlay(OverlayRole::Console, console_tree, Vec2::ZERO);

        let hit = app.overlay_at(Vec2::new(10.0, 10.0));
        assert_eq!(hit, Some((OverlayRole::Console, console_leaf)));
    }

    /// `canvas_signature` is `None` for an unregistered role and
    /// any role whose signature was never set. Pins the contract
    /// that `update_portal_tree` (and its sibling canvas-role
    /// dispatchers) rely on to dispatch into a full rebuild on
    /// the first frame after open.
    #[test]
    fn canvas_signature_is_none_when_unset_or_unregistered() {
        let app = AppScene::new();
        assert_eq!(app.canvas_signature(CanvasRole::Portals), None);
        assert_eq!(app.canvas_signature(CanvasRole::Borders), None);
    }

    /// `set_canvas_signature` records a value the next
    /// `canvas_signature` returns; `unregister_canvas` drops it so
    /// a subsequent reopen forces a full rebuild before the
    /// in-place mutator path can run again.
    #[test]
    fn canvas_signature_round_trips_and_clears_on_unregister() {
        let mut app = AppScene::new();
        let (tree, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        app.register_canvas(CanvasRole::Portals, tree, Vec2::ZERO);
        app.set_canvas_signature(CanvasRole::Portals, 0xDEADBEEF);
        assert_eq!(app.canvas_signature(CanvasRole::Portals), Some(0xDEADBEEF));

        app.unregister_canvas(CanvasRole::Portals);
        assert_eq!(app.canvas_signature(CanvasRole::Portals), None);
    }

    #[test]
    fn canvas_dispatch_full_rebuild_when_no_tree() {
        let app = AppScene::new();
        assert_eq!(
            app.canvas_dispatch(CanvasRole::Borders, 42),
            CanvasDispatch::FullRebuild,
        );
    }

    #[test]
    fn canvas_dispatch_full_rebuild_on_signature_mismatch() {
        let mut app = AppScene::new();
        let (tree, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        app.register_canvas(CanvasRole::Borders, tree, Vec2::ZERO);
        app.set_canvas_signature(CanvasRole::Borders, 100);
        assert_eq!(
            app.canvas_dispatch(CanvasRole::Borders, 200),
            CanvasDispatch::FullRebuild,
        );
    }

    #[test]
    fn canvas_dispatch_in_place_on_signature_match() {
        let mut app = AppScene::new();
        let (tree, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        app.register_canvas(CanvasRole::Borders, tree, Vec2::ZERO);
        app.set_canvas_signature(CanvasRole::Borders, 42);
        assert_eq!(
            app.canvas_dispatch(CanvasRole::Borders, 42),
            CanvasDispatch::InPlaceMutator,
        );
    }

    #[test]
    fn overlay_dispatch_full_rebuild_when_no_tree() {
        let app = AppScene::new();
        assert_eq!(
            app.overlay_dispatch(OverlayRole::Console, 0),
            OverlayDispatch::FullRebuild,
        );
    }

    #[test]
    fn overlay_dispatch_in_place_on_match() {
        let mut app = AppScene::new();
        let (tree, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        app.register_overlay(OverlayRole::ColorPicker, tree, Vec2::ZERO);
        app.set_overlay_signature(OverlayRole::ColorPicker, 99);
        assert_eq!(
            app.overlay_dispatch(OverlayRole::ColorPicker, 99),
            OverlayDispatch::InPlaceMutator,
        );
    }

    #[test]
    fn overlay_signature_cleared_on_unregister() {
        let mut app = AppScene::new();
        let (tree, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        app.register_overlay(OverlayRole::Console, tree, Vec2::ZERO);
        app.set_overlay_signature(OverlayRole::Console, 55);
        assert_eq!(app.overlay_signature(OverlayRole::Console), Some(55));

        app.unregister_overlay(OverlayRole::Console);
        assert_eq!(app.overlay_signature(OverlayRole::Console), None);
    }

    #[test]
    fn hash_canvas_signature_deterministic() {
        let key = ("node_a", "node_b", "default");
        let h1 = hash_canvas_signature(&key);
        let h2 = hash_canvas_signature(&key);
        assert_eq!(h1, h2);

        // Different input produces a different hash (with overwhelming probability)
        let h3 = hash_canvas_signature(&("node_a", "node_b", "cross_link"));
        assert_ne!(h1, h3);
    }

    #[test]
    fn canvas_ids_in_layer_order_returns_all_registered() {
        let mut app = AppScene::new();
        let (t1, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        let (t2, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        app.register_canvas(CanvasRole::Borders, t1, Vec2::ZERO);
        app.register_canvas(CanvasRole::Connections, t2, Vec2::ZERO);
        let ids = app.canvas_ids_in_layer_order();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn default_scene_has_no_registrations() {
        let app = AppScene::default();
        assert_eq!(app.canvas_scene().len(), 0);
        assert_eq!(app.overlay_scene().len(), 0);
        assert!(app.canvas_id(CanvasRole::Borders).is_none());
        assert!(app.overlay_id(OverlayRole::Console).is_none());
    }
}
