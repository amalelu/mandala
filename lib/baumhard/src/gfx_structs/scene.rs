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
    /// Crate-internal accessor for tests and benches.
    #[allow(dead_code)]
    pub(crate) fn raw(&self) -> usize {
        self.0
    }
}

/// One tree in a scene, plus its layering and compositing metadata.
///
/// All fields are private so the parent [`Scene`]'s invariants
/// (layer-order cache freshness, role-slot consistency in
/// downstream wrappers) can't be silently violated by direct field
/// assignment. Read via the accessors; mutate via [`Scene`]'s
/// `set_layer` / `set_offset` / `set_visible` / `apply_mutator`.
pub struct SceneEntry {
    tree: Tree<GfxElement, GfxMutator>,
    layer: i32,
    offset: Vec2,
    visible: bool,
}

impl SceneEntry {
    /// Immutable borrow of the registered tree. Used by the
    /// renderer's per-tree buffer walker.
    pub fn tree(&self) -> &Tree<GfxElement, GfxMutator> {
        &self.tree
    }

    /// Higher layers draw on top. Equal layers break ties by
    /// insertion order (older entries draw first).
    pub fn layer(&self) -> i32 {
        self.layer
    }

    /// Coordinate-space offset added to every `GlyphArea` position
    /// before rendering and hit-testing. The space (canvas vs
    /// screen) is a property of the consuming render pass, not of
    /// the scene itself.
    pub fn offset(&self) -> Vec2 {
        self.offset
    }

    /// Whether this entry participates in render + hit-test.
    pub fn visible(&self) -> bool {
        self.visible
    }

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

    /// Number of registered trees, regardless of visibility. O(1).
    pub fn len(&self) -> usize {
        self.trees.len()
    }

    /// Convenience for `len() == 0`. O(1).
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

    /// Immutable access to a tree entry. Use the accessors on
    /// [`SceneEntry`] (`tree()`, `layer()`, `offset()`, `visible()`)
    /// to inspect.
    pub fn get(&self, id: SceneTreeId) -> Option<&SceneEntry> {
        self.trees.get(id.0)
    }

    /// Mutable borrow of just the tree. Builder-phase escape hatch.
    /// Mutations via the returned reference that touch GlyphArea
    /// position or bounds **must** call
    /// [`Tree::invalidate_caches`] afterwards, or the scene's
    /// hit-test memos will drift. Prefer [`Self::apply_mutator`]
    /// which handles invalidation automatically.
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

    /// Handles in draw order, lowest layer first. Returned as a
    /// vector so callers can hold `&mut` to the scene's trees one
    /// at a time without fighting the borrow checker. Same order
    /// the renderer should use to composite; [`Self::component_at`]
    /// walks the reverse of this.
    pub fn ids_in_layer_order(&mut self) -> Vec<SceneTreeId> {
        self.ensure_layer_order();
        self.layer_order.clone()
    }

    /// Top-down hit-test. Walks trees in reverse draw order and
    /// returns the first one whose AABB contains `point`, together
    /// with the best-matching descendant [`NodeId`] inside that
    /// tree.
    ///
    /// # Costs
    ///
    /// O(trees) outer scan + a memoised AABB check per tree
    /// (O(1) when warm, O(descendants) once after each mutator).
    /// On a hit, one additional O(descendants) walk inside the
    /// matched tree via [`Tree::descendant_at`]. Misses cost only
    /// the bbox check — they don't trigger the inner walk.
    ///
    /// # Precondition
    ///
    /// All pending mutations must be applied before calling this
    /// method. The BVH caches are invalidated by
    /// [`MutatorTree::apply_to`](crate::gfx_structs::tree::MutatorTree)
    /// and lazily recomputed on the first query after mutation.
    /// Calling `component_at` between a mutation and its
    /// application may return stale results.
    pub fn component_at(&mut self, point: Vec2) -> Option<(SceneTreeId, NodeId)> {
        self.ensure_layer_order();
        // Two-pass: first collect candidates via cheap AABB reject
        // (shared borrow), then drill into the first hit via BVH
        // descent (mutable borrow). This avoids holding &mut and &
        // on self.trees simultaneously.
        let candidates: Vec<(SceneTreeId, Vec2)> = self
            .layer_order
            .iter()
            .rev()
            .copied()
            .filter_map(|id| {
                let entry = self.trees.get(id.0)?;
                if !entry.contains(point) {
                    return None;
                }
                Some((id, point - entry.offset))
            })
            .collect();

        for (id, local) in candidates {
            let Some(entry) = self.trees.get_mut(id.0) else {
                continue;
            };
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

