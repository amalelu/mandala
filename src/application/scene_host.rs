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
//! are proven working in the new shape (see ROADMAP).

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
/// event loop until Session 5 moves it in.
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
        }
    }

    // --- Canvas-space sub-scene -------------------------------

    /// Read-only view of the canvas-space sub-scene. Used by the
    /// renderer's main (camera-transformed) pass.
    pub fn canvas_scene(&self) -> &Scene {
        &self.canvas
    }

    /// Mutable access to the canvas-space sub-scene — for the
    /// renderer's buffer walker which needs to sort ids by layer.
    pub fn canvas_scene_mut(&mut self) -> &mut Scene {
        &mut self.canvas
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

    /// Remove a canvas role's tree, if registered.
    pub fn unregister_canvas(&mut self, role: CanvasRole) {
        if let Some(id) = self.canvas_role_slot_mut(role).take() {
            self.canvas.remove(id);
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

    /// Mutable borrow of a canvas role's tree.
    pub fn canvas_tree_mut(
        &mut self,
        role: CanvasRole,
    ) -> Option<&mut Tree<GfxElement, GfxMutator>> {
        let id = self.canvas_id(role)?;
        self.canvas.tree_mut(id)
    }

    /// Apply a mutator to a canvas role's tree.
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

    /// Mutable access to the overlay sub-scene.
    pub fn overlay_scene_mut(&mut self) -> &mut Scene {
        &mut self.overlay
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

    /// Remove an overlay's tree if registered.
    pub fn unregister_overlay(&mut self, role: OverlayRole) {
        if let Some(id) = self.overlay_role_slot_mut(role).take() {
            self.overlay.remove(id);
        }
    }

    /// Handle currently assigned to the named overlay role.
    pub fn overlay_id(&self, role: OverlayRole) -> Option<SceneTreeId> {
        match role {
            OverlayRole::Console => self.console,
            OverlayRole::ColorPicker => self.color_picker,
        }
    }

    /// Mutable borrow of a specific overlay's tree.
    pub fn overlay_tree_mut(
        &mut self,
        role: OverlayRole,
    ) -> Option<&mut Tree<GfxElement, GfxMutator>> {
        let id = self.overlay_id(role)?;
        self.overlay.tree_mut(id)
    }

    /// Apply a mutator to the tree registered for an overlay role.
    pub fn apply_overlay_mutator(
        &mut self,
        role: OverlayRole,
        mutator: &MutatorTree<GfxMutator>,
    ) {
        if let Some(id) = self.overlay_id(role) {
            self.overlay.apply_mutator(id, mutator);
        }
    }

    /// Toggle visibility of an overlay role's tree without removing.
    pub fn set_overlay_visible(&mut self, role: OverlayRole, visible: bool) {
        if let Some(id) = self.overlay_id(role) {
            self.overlay.set_visible(id, visible);
        }
    }

    /// Move an overlay role's tree to a new screen-space offset.
    pub fn set_overlay_offset(&mut self, role: OverlayRole, offset: Vec2) {
        if let Some(id) = self.overlay_id(role) {
            self.overlay.set_offset(id, offset);
        }
    }

    /// Hit-test the overlay sub-scene. Canvas-space roles are
    /// intentionally not checked — overlay hits should always win
    /// over canvas hits, and canvas hits route through the
    /// mindmap's own `document::hit_test` path until Session 5.
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
}
