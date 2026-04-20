//! App-layer scene host — bridges the application and
//! [`baumhard::gfx_structs::scene::Scene`].
//!
//! `AppScene` owns a Baumhard [`Scene`] plus a small amount of
//! app-specific bookkeeping: named slots for the overlay components
//! (console and color picker) that are migrating to tree-based
//! rendering, plus the insertion-order layer integers the renderer
//! uses to composite them.
//!
//! # Why a separate type
//!
//! `baumhard::Scene` itself is a generic container — it knows trees,
//! layers, and offsets, and nothing else. Mandala's app has fixed
//! roles ("the console", "the color picker") and needs to look
//! those up by identity, not by slab key. `AppScene` encodes that
//! identity as an [`OverlayRole`] enum.
//!
//! The mindmap tree itself is **not** registered here yet — it
//! still lives next to the event loop (see `app.rs: mindmap_tree`)
//! and is consumed directly by the renderer. That's the last
//! migration step and the cleanest to defer until the overlays
//! are proven working in the new shape.

use std::collections::HashMap;

use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::scene::{Scene, SceneTreeId};
use baumhard::gfx_structs::tree::{MutatorTree, Tree};
use glam::Vec2;
use indextree::NodeId;

/// The fixed **screen-space** overlay components that the app's
/// `AppScene` can host. Overlay trees are drawn without the camera
/// transform at `scale = 1.0` (see the renderer's palette pass).
///
/// Extend this enum when a new screen-space overlay lands.
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
}

/// Two arms of the §B2 canvas-tree dispatch: apply a mutator to the
/// existing arena, or rebuild wholesale and re-register. Returned
/// from [`AppScene::canvas_dispatch`] so the caller can route
/// side-effects (hitbox updates, renderer state) through the
/// matching arm without re-implementing the check.
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

    // --- Screen-space (overlay) layers ------------------------

    /// Color picker modal. Above the canvas scene, below console
    /// (so typing in the console while a picker is open still
    /// works if the two are ever both visible, although today
    /// they're mutually exclusive).
    pub const COLOR_PICKER: i32 = 100;
    /// Console overlay.
    pub const CONSOLE: i32 = 200;
}

/// App-facing scene owning every tree-rendered component.
///
/// Internally splits into two sub-scenes by coordinate space:
///
/// - `canvas` — camera-transformed trees (mindmap, borders,
///   connections, portals). Draw order inside this scene is
///   `CanvasRole`-layered; draw order *across* all components is:
///   canvas first, then overlay on top.
/// - `overlay` — screen-space trees (console, color picker). At
///   `scale = 1.0`, no camera transform.
///
/// The split exists because the renderer uses different pipelines
/// for the two spaces; lumping them into one sub-scene would mean
/// tagging every tree with its coordinate space and branching on
/// that at every render step. Two scenes make the invariant
/// structural.
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
            canvas_signatures: HashMap::new(),
            overlay_signatures: HashMap::new(),
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
        };
        let id = self.canvas.insert(tree, layer, offset);
        *self.canvas_role_slot_mut(role) = Some(id);
        id
    }

    /// Remove a canvas role's tree, if registered. Also clears any
    /// recorded structural signature — a future re-register starts
    /// from a clean slate, forcing a full rebuild before the in-place
    /// mutator path can run again.
    pub fn unregister_canvas(&mut self, role: CanvasRole) {
        if let Some(id) = self.canvas_role_slot_mut(role).take() {
            self.canvas.remove(id);
        }
        self.canvas_signatures.remove(&role);
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
        }
    }

    /// Apply a mutator to a canvas role's tree. Path of choice
    /// for incremental updates per §B1 of the baumhard
    /// conventions — the upcoming connection / border drag
    /// shaping cache will land through this.
    pub fn apply_canvas_mutator(
        &mut self,
        role: CanvasRole,
        mutator: &MutatorTree<GfxMutator>,
    ) {
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
    pub fn apply_overlay_mutator(
        &mut self,
        role: OverlayRole,
        mutator: &MutatorTree<GfxMutator>,
    ) {
        if let Some(id) = self.overlay_id(role) {
            self.overlay.apply_mutator(id, mutator);
        }
    }

    /// Hit-test the overlay sub-scene. Canvas-space roles are
    /// intentionally not checked — overlay hits should always
    /// win over canvas hits, and the canvas hit-test still goes
    /// through `document::hit_test` (unifying those two paths is
    /// deferred work).
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
