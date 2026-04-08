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
}

impl<T: Flaggable + Clone, M: Applicable<T>> Tree<T, M> {
    /// # Arguments
    /// * `point` - The point from which to look for an element to flag
    /// * `depth` - The "depth" at which to look, if there's several elements in the same area
    /// * `slack` - How far around the `point` we'll look for elements
    ///
    /// # Returns
    /// * [Some]\([NodeId]) - if an element was flagged, containing the elements node ID
    /// * [None] - if no element was flagged
    ///
    /// # Example
    ///
    /// ```
    /// use std::sync::Arc;
    /// use crossbeam_channel::{unbounded, Sender};
    /// use glam::Vec2;
    /// use baumhard::gfx_structs::element::GfxElement;
    /// use baumhard::gfx_structs::tree::Tree;
    /// use baumhard::core::primitives::Flag;
    /// use baumhard::gfx_structs::mutator::GfxMutator;
    /// use baumhard::gfx_structs::util::regions::{RegionIndexer, RegionParams};
    /// let (this_sender, this_receiver) = unbounded();
    /// let mut tree: Tree<GfxElement, GfxMutator> = Tree::new(Arc::new(RegionParams::new(10, (1000,1000))), this_sender);
    /// tree.flag_near(Flag::Focused, Vec2::new(50.0, 50.0), 0, 10);
    pub fn flag_near(
        &mut self,
        flag: Flag,
        point: Vec2,
        depth: usize,
        slack: usize,
    ) -> Option<NodeId> {
        // Find all elements that intersect with the point, or the area if there's any slack
        //
        None
    }

    pub fn do_for_all_flagged(&mut self, flag: Flag, mutator: Tree<T, M>) {}
}

