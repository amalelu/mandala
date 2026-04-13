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

/// The fixed overlay components that the app's `AppScene` can host.
///
/// Extend this enum when a new overlay lands. Mindmap itself is not
/// a variant — by design, the mindmap tree joins `AppScene` as the
/// lowest layer only once all overlays are tree-shaped.
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

/// Conventional layer integers used when inserting overlay trees.
/// Higher layers draw (and hit-test) on top.
pub mod layers {
    /// Mindmap body — every overlay draws above this once the
    /// migration reaches that step.
    pub const MINDMAP: i32 = 0;
    /// Color picker modal. Above mindmap, below console (so typing
    /// in the console while a picker is open still works if the
    /// two are ever both visible, although today they're mutually
    /// exclusive).
    pub const COLOR_PICKER: i32 = 100;
    /// Console overlay.
    pub const CONSOLE: i32 = 200;
}

/// App-facing scene owning every tree-rendered component.
pub struct AppScene {
    scene: Scene,
    console: Option<SceneTreeId>,
    color_picker: Option<SceneTreeId>,
}

impl Default for AppScene {
    fn default() -> Self {
        Self::new()
    }
}

impl AppScene {
    /// Empty scene with no overlay trees registered.
    pub fn new() -> Self {
        AppScene {
            scene: Scene::new(),
            console: None,
            color_picker: None,
        }
    }

    /// Raw access to the underlying baumhard scene. Preferred for
    /// rendering passes that iterate every tree; use the role-based
    /// accessors below for mutation.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Mutable raw access. Use sparingly — most state changes should
    /// flow through [`Self::apply_mutator`] so the mutator invariant
    /// (§B1 of baumhard conventions) holds.
    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }

    /// Register (or replace) the tree backing a named overlay role.
    /// Returns the new handle.
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
        if let Some(old) = self.role_slot_mut(role).take() {
            self.scene.remove(old);
        }
        let layer = match role {
            OverlayRole::Console => layers::CONSOLE,
            OverlayRole::ColorPicker => layers::COLOR_PICKER,
        };
        let id = self.scene.insert(tree, layer, offset);
        *self.role_slot_mut(role) = Some(id);
        id
    }

    /// Remove an overlay's tree, returning ownership if it was
    /// registered. No-op for unknown roles.
    pub fn unregister_overlay(&mut self, role: OverlayRole) {
        if let Some(id) = self.role_slot_mut(role).take() {
            self.scene.remove(id);
        }
    }

    /// Handle currently assigned to the named role, if any.
    pub fn overlay_id(&self, role: OverlayRole) -> Option<SceneTreeId> {
        match role {
            OverlayRole::Console => self.console,
            OverlayRole::ColorPicker => self.color_picker,
        }
    }

    /// Mutable borrow of a specific overlay's tree. Use during the
    /// builder phase; switch to [`Self::apply_mutator`] for
    /// steady-state updates (keystroke, hover preview).
    pub fn overlay_tree_mut(
        &mut self,
        role: OverlayRole,
    ) -> Option<&mut Tree<GfxElement, GfxMutator>> {
        let id = self.overlay_id(role)?;
        self.scene.tree_mut(id)
    }

    /// Apply a mutator to the tree registered for `role`. No-op if
    /// nothing is registered — callers typically check
    /// [`Self::overlay_id`] first when they need to distinguish.
    pub fn apply_mutator(&mut self, role: OverlayRole, mutator: &MutatorTree<GfxMutator>) {
        if let Some(id) = self.overlay_id(role) {
            self.scene.apply_mutator(id, mutator);
        }
    }

    /// Toggle visibility of a role's tree without removing it.
    pub fn set_overlay_visible(&mut self, role: OverlayRole, visible: bool) {
        if let Some(id) = self.overlay_id(role) {
            self.scene.set_visible(id, visible);
        }
    }

    /// Move a role's tree to a new screen-space offset.
    pub fn set_overlay_offset(&mut self, role: OverlayRole, offset: Vec2) {
        if let Some(id) = self.overlay_id(role) {
            self.scene.set_offset(id, offset);
        }
    }

    /// Hit-test the scene for overlays. Returns `(role, node)` if a
    /// registered overlay covers `screen_pt`. The mindmap is
    /// explicitly not considered — until the migration finishes, the
    /// mindmap tree keeps its own `document::hit_test` path.
    pub fn overlay_at(&mut self, screen_pt: Vec2) -> Option<(OverlayRole, NodeId)> {
        let hit = self.scene.component_at(screen_pt)?;
        let role = self.role_for_id(hit.0)?;
        Some((role, hit.1))
    }

    fn role_slot_mut(&mut self, role: OverlayRole) -> &mut Option<SceneTreeId> {
        match role {
            OverlayRole::Console => &mut self.console,
            OverlayRole::ColorPicker => &mut self.color_picker,
        }
    }

    fn role_for_id(&self, id: SceneTreeId) -> Option<OverlayRole> {
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
        assert_eq!(app.scene().len(), 0);
    }

    #[test]
    fn re_registering_replaces_the_previous_tree() {
        // Re-registering must drop the previous entry so the slab
        // doesn't leak — slab recycles keys so id equality here is
        // incidental, what matters is that `len() == 1` and the
        // role points at the newer insertion.
        let mut app = AppScene::new();
        let (t1, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        let (t2, _) = overlay_tree(Vec2::ZERO, Vec2::new(10.0, 10.0));
        app.register_overlay(OverlayRole::Console, t1, Vec2::ZERO);
        let id2 = app.register_overlay(OverlayRole::Console, t2, Vec2::ZERO);
        assert_eq!(app.scene().len(), 1);
        assert_eq!(app.overlay_id(OverlayRole::Console), Some(id2));
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
