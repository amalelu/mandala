use crate::core::primitives::{Applicable, Flag, Flaggable};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutator, GlyphTreeEventInstance};
use crate::gfx_structs::util::regions::{RegionElementKeyPair, RegionIndexer, RegionParams};
use crate::gfx_structs::tree_walker::walk_tree_from;
use crate::util::arena_utils;
use crossbeam_channel::Sender;
use glam::Vec2;
use indextree::{Arena, Children, Descendants, Node, NodeId};
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

    /// Walks the arena looking for the smallest-AABB `GlyphArea`
    /// descendant whose rectangle contains `point`. Returns its
    /// [`NodeId`], or [`None`] if nothing matches.
    ///
    /// # Costs
    ///
    /// O(n) over the descendants of [`Self::root`]. No allocation.
    /// [`GfxElement::GlyphModel`] and [`GfxElement::Void`] nodes are
    /// skipped (they don't expose an AABB on `GlyphArea`). When
    /// multiple areas contain the point, the smallest by area wins —
    /// mirrors the "innermost first" convention used by the app-side
    /// `hit_test` helpers this method replaces.
    ///
    /// Intended for use by [`crate::gfx_structs::scene::Scene`] after
    /// it has identified which tree covers the hit; each tree then
    /// drills down to the concrete target using this method.
    pub fn descendant_at(&self, point: Vec2) -> Option<NodeId> {
        let mut best: Option<(NodeId, f32)> = None;
        for node_id in self.root.descendants(&self.arena) {
            let Some(node) = self.arena.get(node_id) else {
                continue;
            };
            let Some(area) = node.get().glyph_area() else {
                continue;
            };
            let pos = area.position.to_vec2();
            let bounds = area.render_bounds.to_vec2();
            if bounds.x <= 0.0 || bounds.y <= 0.0 {
                continue;
            }
            let max_x = pos.x + bounds.x;
            let max_y = pos.y + bounds.y;
            if point.x >= pos.x && point.x <= max_x && point.y >= pos.y && point.y <= max_y {
                let size = bounds.x * bounds.y;
                match best {
                    Some((_, best_size)) if best_size <= size => {}
                    _ => best = Some((node_id, size)),
                }
            }
        }
        best.map(|(id, _)| id)
    }

    /// Conservative AABB covering every `GlyphArea` descendant of
    /// [`Self::root`]. Returns `(top_left, bottom_right)` or `None`
    /// if the tree has no visible areas.
    ///
    /// # Costs
    ///
    /// O(n) over the descendants. No allocation. Used by
    /// [`crate::gfx_structs::scene::Scene`] to decide whether a point
    /// could possibly hit this tree before running a full
    /// [`Self::descendant_at`] walk.
    pub fn descendants_aabb(&self) -> Option<(Vec2, Vec2)> {
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

    /// Sets `flag` on the `GlyphArea` descendant closest to `point`,
    /// searching within `slack` pixels. `depth` reserved for
    /// future use when several elements overlap at the same point.
    /// Returns the flagged node's [`NodeId`], or [`None`].
    ///
    /// # Costs
    ///
    /// O(n) over the descendants of [`Self::root`]; see
    /// [`Self::descendant_at`] for the underlying walk.
    pub fn flag_near(
        &mut self,
        flag: Flag,
        point: Vec2,
        _depth: usize,
        _slack: usize,
    ) -> Option<NodeId> {
        let node_id = self.descendant_at(point)?;
        if let Some(node) = self.arena.get_mut(node_id) {
            node.get_mut().set_flag(flag);
        }
        Some(node_id)
    }
}

impl<T: Flaggable + Clone, M: Applicable<T>> Tree<T, M> {
    /// Placeholder for a future "apply a mutator to every flagged
    /// descendant" helper. No implementation yet; documented here so
    /// a call site can be trivially grepped when the feature lands.
    pub fn do_for_all_flagged(&mut self, _flag: Flag, _mutator: Tree<T, M>) {}
}

