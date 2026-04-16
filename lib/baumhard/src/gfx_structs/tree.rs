use crate::core::primitives::{Applicable, Flag, Flaggable};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutator, GlyphTreeEventInstance};
use crate::gfx_structs::util::regions::{RegionElementKeyPair, RegionIndexer, RegionParams};
use crate::gfx_structs::tree_walker::walk_tree_from;
use crate::util::arena_utils;
use crossbeam_channel::Sender;
use glam::Vec2;
use indextree::{Arena, Children, Descendants, Node, NodeId};
use std::cell::Cell;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

pub trait BranchChannel {
    fn channel(&self) -> usize;
}

pub type EventSubscriber =
    Arc<Mutex<dyn FnMut(&mut GfxElement, GlyphTreeEventInstance) + Send + Sync>>;

pub trait TreeEventConsumer {
    fn accept_event(&mut self, event: &GlyphTreeEventInstance);
}

pub trait TreeNode {
    fn void() -> Self;
}

#[derive(Clone, Debug)]
pub struct MutatorTree<T> {
    pub arena: Arena<T>,
    pub root: NodeId,
}

impl <T: TreeNode + Clone> MutatorTree<T> {
    pub fn new() -> Self {
        Self::new_with(T::void())
    }

    pub fn new_with(node: T) -> Self {
        let mut arena = Arena::default();
        let root = arena.new_node(node);
        MutatorTree {
            arena,
            root,
        }
    }

    pub fn get(&self, id: NodeId) -> Option<&Node<T>> {
        self.arena.get(id)
    }
}

impl Applicable<Tree<GfxElement, GfxMutator>> for MutatorTree<GfxMutator> {
    fn apply_to(&self, target: &mut Tree<GfxElement, GfxMutator>) {
        // Any mutator may touch GlyphArea position or bounds; the
        // safe default is to drop the bbox memos and let the next
        // query recompute. Cheap (one Cell write each).
        target.aabb_cache.set(None);
        target.subtree_aabbs_dirty.set(true);
        walk_tree_from(target, &self, target.root, self.root)
    }
}

#[derive(Clone, Debug)]
pub struct Tree<T: Clone, M: Applicable<T>> {
    pub arena: Arena<T>,
    phantom: PhantomData<M>,
    pub root: NodeId,
    /// Layer is used to determine the order that trees should be drawn onto the Scene
    pub layer: usize,
    /// All child positions are relative to this
    position: Vec2,
    /// Children can put mutations here as a response to some event
    pending_mutations: Vec<Arc<MutatorTree<M>>>,
    /// We want this to be Rc eventually
    region_params: Option<Arc<RegionParams>>,
    region_index: Option<Rc<RegionIndexer>>,
    /// Memoised result of [`Tree::descendants_aabb`].
    /// `None` = not yet computed (or invalidated by a mutation);
    /// `Some(None)` = computed and the tree has no visible areas;
    /// `Some(Some(_))` = computed, valid bbox.
    /// `Cell` (not `RefCell`) is fine because the inner type is
    /// `Copy` and we only ever swap whole values.
    /// Invalidated by [`MutatorTree::apply_to`] (and any
    /// `&mut self` op that touches GlyphArea positions or bounds).
    aabb_cache: Cell<Option<Option<(Vec2, Vec2)>>>,
    /// When `true`, per-node `subtree_aabb` caches in the arena are
    /// stale and must be recomputed before the next BVH query.
    /// Set to `true` on construction and after any mutation.
    subtree_aabbs_dirty: Cell<bool>,
}

impl Tree<GfxElement, GfxMutator> {
    pub(crate) fn new_with(
        element: GfxElement,
        region_params: Arc<RegionParams>,
    ) -> Self {
        let mut arena = Arena::default();
        let root = arena.new_node(element);
        Tree {
            arena,
            phantom: Default::default(),
            root,
            layer: 0,
            position: Default::default(),
            pending_mutations: vec![],
            region_params: Some(region_params),
            region_index: Some(Rc::new(RegionIndexer::default())),
            aabb_cache: Cell::new(None),
            subtree_aabbs_dirty: Cell::new(true),
        }
    }

    /// Constructs a new Tree with a root node of type void
    /// This root node will be the ancestor of all nodes in this tree
    pub fn new(
        region_params: Arc<RegionParams>,
        scene_index_sender: Sender<RegionElementKeyPair>,
    ) -> Self {
        let mut arena = Arena::default();
        let root = arena.new_node(GfxElement::void());
        Tree {
            arena,
            phantom: Default::default(),
            root,
            layer: 0,
            position: Default::default(),
            pending_mutations: vec![],
            region_params: Some(region_params),
            region_index: Some(Rc::new(RegionIndexer::default())),
            aabb_cache: Cell::new(None),
            subtree_aabbs_dirty: Cell::new(true),
        }
    }

    pub fn new_non_indexed_with(element: GfxElement) -> Self {
        let mut arena = Arena::default();
        let root = arena.new_node(element);
        Tree {
            arena,
            phantom: Default::default(),
            root,
            layer: 0,
            position: Default::default(),
            pending_mutations: vec![],
            region_params: None,
            region_index: None,
            aabb_cache: Cell::new(None),
            subtree_aabbs_dirty: Cell::new(true),
        }
    }

    /// Creates an un-indexed Tree with a default [T::void] root node
    pub fn new_non_indexed() -> Self {
        let mut arena = Arena::default();
        let root = arena.new_node(GfxElement::void());
        Tree {
            arena,
            phantom: Default::default(),
            root,
            layer: 0,
            position: Default::default(),
            pending_mutations: vec![],
            region_params: None,
            region_index: None,
            aabb_cache: Cell::new(None),
            subtree_aabbs_dirty: Cell::new(true),
        }
    }

    pub fn get(&self, id: NodeId) -> Option<&Node<GfxElement>> {
        self.arena.get(id)
    }

    /// See [NodeId::descendants]
    pub fn descendants(&self) -> Descendants<GfxElement> {
        self.root.descendants(&self.arena)
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    /// See [NodeId::children]
    pub fn children(&self) -> Children<GfxElement> {
        self.root.children(&self.arena)
    }

    /// Clones the provided [Self] into this one, ignoring the root
    pub fn import(&mut self, target: &Self) {
        self.import_arena(&target.arena, target.root);
    }

    /// Clones the provided GfxArena into this one, ignoring the root
    fn import_arena(&mut self, target: &Arena<GfxElement>, target_root: NodeId) {
        arena_utils::clone_subtree(target, target_root, &mut self.arena, self.root);
    }

    /// Find the smallest-AABB `GlyphArea` descendant whose rectangle
    /// contains `point`. Returns its [`NodeId`], or [`None`] if
    /// nothing matches.
    ///
    /// Equivalent to `descendant_near(point, 0.0)`.
    ///
    /// # Algorithm
    ///
    /// Uses a BVH (bounding-volume hierarchy) descent: each node's
    /// cached `subtree_aabb` is checked before recursing into its
    /// children. Subtrees whose aggregate AABB does not contain the
    /// point are pruned entirely.
    ///
    /// # Costs
    ///
    /// O(branching_factor × depth) when subtrees are spatially
    /// disjoint; O(n) worst case when subtree AABBs fully overlap.
    /// One `Vec` allocation on the first call after a mutation (for
    /// the subtree AABB computation pass); O(1) on subsequent calls.
    ///
    /// When multiple areas contain the point, the smallest by area
    /// wins — the "innermost first" convention.
    pub fn descendant_at(&mut self, point: Vec2) -> Option<NodeId> {
        self.descendant_near(point, 0.0)
    }

    /// Variant of [`Self::descendant_at`] that expands every
    /// `GlyphArea`'s AABB by `slack` pixels on each side before
    /// the containment test. Use for fuzzy hit-tests where the
    /// caller wants to forgive a stylus or fat-finger near-miss
    /// (e.g. edge handles, console close-buttons).
    pub fn descendant_near(&mut self, point: Vec2, slack: f32) -> Option<NodeId> {
        self.ensure_subtree_aabbs();
        let mut best: Option<(NodeId, f32)> = None;
        self.bvh_descend(self.root, point, slack, &mut best);
        best.map(|(id, _)| id)
    }

    /// Recursive BVH descent. For each child of `node_id`:
    /// 1. If the child's `subtree_aabb` does not contain `point`
    ///    (accounting for `slack`) → prune the entire subtree.
    /// 2. If the child's own `GlyphArea` AABB contains `point` →
    ///    record it as a candidate (smallest-area wins).
    /// 3. Recurse into the child's children.
    fn bvh_descend(
        &self,
        node_id: NodeId,
        point: Vec2,
        slack: f32,
        best: &mut Option<(NodeId, f32)>,
    ) {
        // Collect children into a local vec to avoid borrow conflicts
        // with the arena (we need &self.arena for both iteration and
        // node reads). Children count per node is small (typically
        // single digits), so this is cheap.
        let children: Vec<NodeId> = node_id
            .children(&self.arena)
            .collect();

        for child_id in children {
            let Some(node) = self.arena.get(child_id) else {
                continue;
            };
            let element = node.get();

            // 1. Prune: skip if subtree AABB doesn't contain point.
            if let Some((st_min, st_max)) = element.subtree_aabb() {
                if point.x < st_min.x - slack
                    || point.x > st_max.x + slack
                    || point.y < st_min.y - slack
                    || point.y > st_max.y + slack
                {
                    continue; // prune
                }
            } else {
                continue; // no subtree AABB → no renderable content
            }

            // 2. Check this node's own GlyphArea AABB.
            if let Some(area) = element.glyph_area() {
                let pos = area.position.to_vec2();
                let bounds = area.render_bounds.to_vec2();
                if bounds.x > 0.0 && bounds.y > 0.0 {
                    let min_x = pos.x - slack;
                    let min_y = pos.y - slack;
                    let max_x = pos.x + bounds.x + slack;
                    let max_y = pos.y + bounds.y + slack;
                    if point.x >= min_x
                        && point.x <= max_x
                        && point.y >= min_y
                        && point.y <= max_y
                    {
                        // Tie-break by *original* (un-slacked) area
                        // so a physically smaller element still wins.
                        let size = bounds.x * bounds.y;
                        match *best {
                            Some((_, best_size)) if best_size <= size => {}
                            _ => *best = Some((child_id, size)),
                        }
                    }
                }
            }

            // 3. Recurse into children (the subtree AABB test above
            //    already proved that at least one descendant may contain
            //    the point).
            self.bvh_descend(child_id, point, slack, best);
        }
    }

    /// Conservative AABB covering every `GlyphArea` descendant of
    /// [`Self::root`]. Returns `(top_left, bottom_right)` or `None`
    /// if the tree has no visible areas.
    ///
    /// # Costs
    ///
    /// O(n) on the first call after the tree mutates; O(1) on
    /// repeated calls thanks to a per-tree memo. The memo is
    /// cleared by [`MutatorTree::apply_to`] — any other mutating
    /// caller that bypasses the mutator pipeline must clear
    /// `aabb_cache` itself (currently only reachable from inside
    /// the crate) or the bbox will drift.
    pub fn descendants_aabb(&self) -> Option<(Vec2, Vec2)> {
        if let Some(cached) = self.aabb_cache.get() {
            return cached;
        }
        let computed = self.compute_descendants_aabb();
        self.aabb_cache.set(Some(computed));
        computed
    }

    /// Ensure every node's `subtree_aabb` cache is fresh. If the
    /// tree has been mutated since the last computation, performs a
    /// bottom-up (post-order) pass that writes each node's
    /// `subtree_aabb` as the union of its own AABB with its
    /// children's subtree AABBs.
    ///
    /// # Costs
    ///
    /// O(n) on the first call after mutation (one `Vec` allocation
    /// for post-order traversal + one write per node). O(1) on
    /// subsequent calls while the cache is clean.
    pub fn ensure_subtree_aabbs(&mut self) {
        if !self.subtree_aabbs_dirty.get() {
            return;
        }
        self.compute_subtree_aabbs();
        self.subtree_aabbs_dirty.set(false);
    }

    /// Bottom-up pass: compute and cache `subtree_aabb` for every
    /// node in the arena. Post-order guarantees that children are
    /// processed before their parents, so each parent can read its
    /// children's already-computed subtree AABBs.
    fn compute_subtree_aabbs(&mut self) {
        // Collect descendants in pre-order, then reverse for post-order.
        let post_order: Vec<NodeId> = {
            let mut ids: Vec<NodeId> = self.root.descendants(&self.arena).collect();
            ids.reverse();
            ids
        };

        for node_id in post_order {
            // Start with this node's own AABB (if it has one).
            let own_aabb = self.node_own_aabb(node_id);
            let mut combined = own_aabb;

            // Merge children's subtree AABBs (already computed due
            // to post-order).
            let mut child_id = self.arena.get(node_id)
                .and_then(|n| n.first_child());
            while let Some(cid) = child_id {
                if let Some(child_aabb) = self.arena.get(cid)
                    .and_then(|n| n.get().subtree_aabb())
                {
                    combined = Some(union_aabb(combined, child_aabb));
                }
                child_id = self.arena.get(cid).and_then(|n| n.next_sibling());
            }

            if let Some(node) = self.arena.get_mut(node_id) {
                node.get_mut().set_subtree_aabb(combined);
            }
        }
    }

    /// Return the AABB of a single node's own renderable content
    /// (position + render_bounds for `GlyphArea`, `None` otherwise).
    /// Does not include descendants.
    fn node_own_aabb(&self, node_id: NodeId) -> Option<(Vec2, Vec2)> {
        let node = self.arena.get(node_id)?;
        let area = node.get().glyph_area()?;
        let bounds = area.render_bounds.to_vec2();
        if bounds.x <= 0.0 || bounds.y <= 0.0 {
            return None;
        }
        let pos = area.position.to_vec2();
        Some((pos, pos + bounds))
    }

    fn compute_descendants_aabb(&self) -> Option<(Vec2, Vec2)> {
        let mut min = Vec2::new(f32::INFINITY, f32::INFINITY);
        let mut max = Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        let mut any = false;
        for node_id in self.root.descendants(&self.arena) {
            let Some(node) = self.arena.get(node_id) else {
                continue;
            };
            let Some(area) = node.get().glyph_area() else {
                continue;
            };
            let bounds = area.render_bounds.to_vec2();
            if bounds.x <= 0.0 || bounds.y <= 0.0 {
                continue;
            }
            let pos = area.position.to_vec2();
            any = true;
            if pos.x < min.x {
                min.x = pos.x;
            }
            if pos.y < min.y {
                min.y = pos.y;
            }
            let mx = pos.x + bounds.x;
            let my = pos.y + bounds.y;
            if mx > max.x {
                max.x = mx;
            }
            if my > max.y {
                max.y = my;
            }
        }
        if any {
            Some((min, max))
        } else {
            None
        }
    }
}

/// Return the smallest AABB enclosing both `base` (if `Some`) and
/// `other`. When `base` is `None`, returns `other` unchanged.
///
/// O(1), no allocation. Used by [`Tree::compute_subtree_aabbs`] to
/// merge a parent's own AABB with its children's subtree AABBs.
fn union_aabb(base: Option<(Vec2, Vec2)>, other: (Vec2, Vec2)) -> (Vec2, Vec2) {
    match base {
        None => other,
        Some((min_a, max_a)) => {
            let (min_b, max_b) = other;
            (
                Vec2::new(min_a.x.min(min_b.x), min_a.y.min(min_b.y)),
                Vec2::new(max_a.x.max(max_b.x), max_a.y.max(max_b.y)),
            )
        }
    }
}
