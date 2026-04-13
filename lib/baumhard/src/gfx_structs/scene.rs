//! Top-level container for multiple [`Tree`]s that together make up
//! one rendered frame.
//!
//! A [`Scene`] owns N `Tree<GfxElement, GfxMutator>` components,
//! each with its own *layer* (draw order) and screen-space *offset*.
//! The mindmap itself is one tree; borders, connections, the console
//! overlay, and the color picker are sibling trees at higher layers.
//!
//! The scene is the single hit-test and render entry point. Given a
//! screen-space point, [`Scene::component_at`] walks trees in
//! top-to-bottom draw order, asks each in turn whether it contains
//! the point, and returns the first `(SceneTreeId, NodeId)` hit.
//! Each individual tree then resolves the concrete target via
//! [`Tree::descendant_at`] — so the scene-level index stays cheap
//! (O(trees)) and the per-tree walk stays linear in the tree's own
//! node count.

use crate::core::primitives::Applicable;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::{MutatorTree, Tree};
use glam::Vec2;
use indextree::NodeId;
use slab::Slab;

/// Opaque handle referring to a tree owned by a [`Scene`].
///
/// Stable across non-removal mutations. Returned by
/// [`Scene::insert`]; used as the key for every subsequent lookup,
/// mutation, or removal.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct SceneTreeId(usize);

impl SceneTreeId {
    /// Internal accessor for tests and benches.
    pub fn raw(&self) -> usize {
        self.0
    }
}

/// One tree in a scene, plus its layering and compositing metadata.
pub struct SceneEntry {
    /// The owned tree. Mutate via [`Scene::apply_mutator`] (preferred,
    /// per §B1 "mutation, not rebuild") or borrow with [`Scene::tree_mut`]
    /// for builder phases.
    pub tree: Tree<GfxElement, GfxMutator>,
    /// Higher layers draw on top. Equal layers break ties by insertion
    /// order (older entries draw first).
    pub layer: i32,
    /// Screen-space offset added to every `GlyphArea` position before
    /// rendering and hit-testing. A scene-level offset means the tree
    /// itself can lay out relative to its own origin and be composited
    /// anywhere.
    pub offset: Vec2,
    /// Skip rendering and hit-testing when `false`. Keeping the tree
    /// allocated is cheaper than tearing it down and rebuilding on
    /// every toggle (e.g. console show/hide).
    pub visible: bool,
}

impl SceneEntry {
    fn contains(&self, point: Vec2) -> bool {
        if !self.visible {
            return false;
        }
        let Some((min, max)) = self.tree.descendants_aabb() else {
            return false;
        };
        let local = point - self.offset;
        local.x >= min.x && local.x <= max.x && local.y >= min.y && local.y <= max.y
    }
}

/// Holds every renderable tree for one frame.
///
/// Not thread-safe — the Mandala app is single-threaded (see
/// `CLAUDE.md` "Architectural shape"). The older [`slab::Slab`]-
/// plus-`Arc<Mutex<Tree>>` stub that preceded this type is gone for
/// the same reason.
pub struct Scene {
    trees: Slab<SceneEntry>,
    /// Tree ids sorted by `(layer, insertion_order)`. Rebuilt lazily
    /// whenever a layer changes; touched on every frame only via
    /// reference.
    layer_order: Vec<SceneTreeId>,
    layer_order_dirty: bool,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    /// Empty scene with no trees.
    pub fn new() -> Self {
        Scene {
            trees: Slab::new(),
            layer_order: Vec::new(),
            layer_order_dirty: false,
        }
    }

    /// Number of registered trees, regardless of visibility.
    pub fn len(&self) -> usize {
        self.trees.len()
    }

    /// Convenience for `len() == 0`.
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// Register `tree` at the given `layer` and `offset`. Returns a
    /// stable handle used for subsequent lookups and mutations.
    ///
    /// # Costs
    ///
    /// O(1) into the slab; marks the layer-order cache dirty so the
    /// next ordered iteration rebuilds it. No allocation on the hot
    /// path unless the slab grows.
    pub fn insert(
        &mut self,
        tree: Tree<GfxElement, GfxMutator>,
        layer: i32,
        offset: Vec2,
    ) -> SceneTreeId {
        let entry = SceneEntry {
            tree,
            layer,
            offset,
            visible: true,
        };
        let id = SceneTreeId(self.trees.insert(entry));
        self.layer_order.push(id);
        self.layer_order_dirty = true;
        id
    }

    /// Remove a tree from the scene, returning ownership.
    pub fn remove(&mut self, id: SceneTreeId) -> Option<SceneEntry> {
        if !self.trees.contains(id.0) {
            return None;
        }
        self.layer_order.retain(|&x| x != id);
        Some(self.trees.remove(id.0))
    }

    /// Immutable access to a tree entry.
    pub fn get(&self, id: SceneTreeId) -> Option<&SceneEntry> {
        self.trees.get(id.0)
    }

    /// Mutable access to a tree entry. Prefer [`Self::apply_mutator`]
    /// for state changes so the mutator invariant (§B1) is upheld;
    /// this escape hatch exists for builder phases (tree
    /// construction, bulk reinit).
    pub fn get_mut(&mut self, id: SceneTreeId) -> Option<&mut SceneEntry> {
        self.trees.get_mut(id.0)
    }

    /// Mutable borrow of just the tree. Convenience over
    /// [`Self::get_mut`] when the caller doesn't care about offset or
    /// layer.
    pub fn tree_mut(&mut self, id: SceneTreeId) -> Option<&mut Tree<GfxElement, GfxMutator>> {
        self.trees.get_mut(id.0).map(|e| &mut e.tree)
    }

    /// Immutable borrow of just the tree.
    pub fn tree(&self, id: SceneTreeId) -> Option<&Tree<GfxElement, GfxMutator>> {
        self.trees.get(id.0).map(|e| &e.tree)
    }

    /// Move a tree to a new layer. Cheap; re-sorts on the next
    /// ordered iteration.
    pub fn set_layer(&mut self, id: SceneTreeId, layer: i32) {
        if let Some(entry) = self.trees.get_mut(id.0) {
            if entry.layer != layer {
                entry.layer = layer;
                self.layer_order_dirty = true;
            }
        }
    }

    /// Relocate a tree's screen-space origin.
    pub fn set_offset(&mut self, id: SceneTreeId, offset: Vec2) {
        if let Some(entry) = self.trees.get_mut(id.0) {
            entry.offset = offset;
        }
    }

    /// Toggle visibility (participation in render + hit-test) without
    /// removing the tree.
    pub fn set_visible(&mut self, id: SceneTreeId, visible: bool) {
        if let Some(entry) = self.trees.get_mut(id.0) {
            entry.visible = visible;
        }
    }

    /// Apply a mutator tree to the named scene tree. The scene's
    /// blessed way of mutating state — mirrors
    /// [`MutatorTree::apply_to`] but scoped to the entry. No-op if
    /// `id` is unknown.
    pub fn apply_mutator(&mut self, id: SceneTreeId, mutator: &MutatorTree<GfxMutator>) {
        if let Some(entry) = self.trees.get_mut(id.0) {
            mutator.apply_to(&mut entry.tree);
        }
    }

    /// Iterate trees in draw order, lowest layer first. Same order
    /// the renderer should use to composite; [`Self::component_at`]
    /// walks the reverse of this.
    pub fn iter_in_layer_order(&mut self) -> impl Iterator<Item = (SceneTreeId, &SceneEntry)> {
        self.ensure_layer_order();
        self.layer_order
            .iter()
            .copied()
            .filter_map(|id| self.trees.get(id.0).map(|e| (id, e)))
    }

    /// Handles in draw order, lowest layer first. Returned as a
    /// vector so callers can hold `&mut` to the scene's trees one
    /// at a time without fighting the borrow checker.
    pub fn ids_in_layer_order(&mut self) -> Vec<SceneTreeId> {
        self.ensure_layer_order();
        self.layer_order.clone()
    }

    /// Top-down hit-test. Walks trees in reverse draw order and
    /// returns the first one whose AABB contains `point`, together
    /// with the best-matching descendant [`NodeId`] inside that tree.
    ///
    /// # Costs
    ///
    /// O(trees) for the outer scan; O(descendants) inside the
    /// matched tree via [`Tree::descendant_at`]. The scene keeps no
    /// per-node spatial index — that's the tree's responsibility if
    /// one is ever wired up (see the unimplemented
    /// [`crate::gfx_structs::util::regions::RegionIndexer`]).
    pub fn component_at(&mut self, point: Vec2) -> Option<(SceneTreeId, NodeId)> {
        self.ensure_layer_order();
        for id in self.layer_order.iter().rev().copied() {
            let Some(entry) = self.trees.get(id.0) else {
                continue;
            };
            if !entry.contains(point) {
                continue;
            }
            let local = point - entry.offset;
            if let Some(node_id) = entry.tree.descendant_at(local) {
                return Some((id, node_id));
            }
        }
        None
    }

    fn ensure_layer_order(&mut self) {
        if !self.layer_order_dirty {
            return;
        }
        // Stable sort by layer so that ties fall back to insertion
        // order (oldest first).
        self.layer_order
            .sort_by_key(|id| self.trees.get(id.0).map(|e| e.layer).unwrap_or(0));
        self.layer_order_dirty = false;
    }
}

