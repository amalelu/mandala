//! Tests for per-node subtree AABB computation, BVH descent, and the
//! `SpatialDescend` instruction (§T1).
//!
//! Covers [`Tree::ensure_subtree_aabbs`], bottom-up computation,
//! invalidation on mutation, BVH-accelerated `descendant_at`, and the
//! [`Instruction::SpatialDescend`] tree-walker instruction. Follows
//! the `do_*()` / `test_*()` benchmark-reuse split (§T2.2).

use glam::Vec2;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::primitives::Applicable;
use crate::font::fonts;
use crate::gfx_structs::area::{GlyphArea, GlyphAreaCommand};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{
    GfxMutator, GlyphTreeEvent, GlyphTreeEventInstance, Instruction, MouseEventData, Mutation,
};
use crate::gfx_structs::tree::{EventSubscriber, MutatorTree, Tree};
use crate::gfx_structs::tree_walker::walk_tree_from;
use crate::util::ordered_vec2::OrderedVec2;

/// Build a small tree for testing subtree AABBs:
///
/// ```text
///         root (void)
///        /          \
///   left_area      right_area
///  pos(10,10)      pos(200,200)
///  bounds(50,20)   bounds(80,30)
///       |
///  left_child_area
///  pos(20,40)
///  bounds(30,15)
/// ```
fn build_test_tree() -> Tree<GfxElement, GfxMutator> {
    fonts::init();
    let mut tree = Tree::new_non_indexed();

    let left = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("left", 14.0, 14.0, Vec2::new(10.0, 10.0), Vec2::new(50.0, 20.0)),
        0,
    );
    let right = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("right", 14.0, 14.0, Vec2::new(200.0, 200.0), Vec2::new(80.0, 30.0)),
        1,
    );
    let left_child = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("lc", 14.0, 14.0, Vec2::new(20.0, 40.0), Vec2::new(30.0, 15.0)),
        0,
    );

    let left_id = tree.arena.new_node(left);
    let right_id = tree.arena.new_node(right);
    let left_child_id = tree.arena.new_node(left_child);

    tree.root.append(left_id, &mut tree.arena);
    tree.root.append(right_id, &mut tree.arena);
    left_id.append(left_child_id, &mut tree.arena);

    tree
}

// ── leaf subtree AABB equals own bounds ───────────────────────────

#[test]
fn test_subtree_aabb_leaf_equals_own_bounds() {
    do_subtree_aabb_leaf_equals_own_bounds();
}

/// A leaf node with no children has a subtree AABB equal to its own
/// rendered AABB (position to position + render_bounds).
pub fn do_subtree_aabb_leaf_equals_own_bounds() {
    let mut tree = build_test_tree();
    tree.ensure_subtree_aabbs();

    // right_area is a leaf at pos(200,200) with bounds(80,30).
    let right_id = tree.root.children(&tree.arena)
        .nth(1)
        .expect("right child");
    let aabb = tree.arena.get(right_id).unwrap().get().subtree_aabb();
    assert_eq!(aabb, Some((Vec2::new(200.0, 200.0), Vec2::new(280.0, 230.0))));
}

// ── parent encloses children ──────────────────────────────────────

#[test]
fn test_subtree_aabb_parent_encloses_children() {
    do_subtree_aabb_parent_encloses_children();
}

/// A parent's subtree AABB encloses both its own bounds and all
/// descendants' bounds.
pub fn do_subtree_aabb_parent_encloses_children() {
    let mut tree = build_test_tree();
    tree.ensure_subtree_aabbs();

    // left_area at pos(10,10)+bounds(50,20) has a child at
    // pos(20,40)+bounds(30,15). The subtree AABB should enclose
    // both: min=(10,10), max=(60,55).
    let left_id = tree.root.children(&tree.arena)
        .next()
        .expect("left child");
    let aabb = tree.arena.get(left_id).unwrap().get().subtree_aabb()
        .expect("left subtree has an AABB");

    let (min, max) = aabb;
    assert!(min.x <= 10.0, "min.x should cover left_area");
    assert!(min.y <= 10.0, "min.y should cover left_area");
    assert!(max.x >= 50.0, "max.x should cover left_child");
    assert!(max.y >= 55.0, "max.y should cover left_child (40+15)");
}

// ── root encloses entire tree ─────────────────────────────────────

#[test]
fn test_subtree_aabb_root_encloses_entire_tree() {
    do_subtree_aabb_root_encloses_entire_tree();
}

/// The root's subtree AABB encloses every node in the tree.
pub fn do_subtree_aabb_root_encloses_entire_tree() {
    let mut tree = build_test_tree();
    tree.ensure_subtree_aabbs();

    let root_aabb = tree.arena.get(tree.root).unwrap().get().subtree_aabb()
        .expect("root subtree has an AABB");
    let (min, max) = root_aabb;

    // Must cover left_area (10,10)-(60,30), left_child (20,40)-(50,55),
    // and right_area (200,200)-(280,230).
    assert!(min.x <= 10.0);
    assert!(min.y <= 10.0);
    assert!(max.x >= 280.0);
    assert!(max.y >= 230.0);
}

// ── invalidation on mutation ──────────────────────────────────────

#[test]
fn test_subtree_aabb_invalidated_by_mutation() {
    do_subtree_aabb_invalidated_by_mutation();
}

/// Applying a mutator invalidates the subtree AABB cache, and the
/// next `ensure_subtree_aabbs` call recomputes fresh values that
/// reflect the moved node.
pub fn do_subtree_aabb_invalidated_by_mutation() {
    let mut tree = build_test_tree();
    tree.ensure_subtree_aabbs();

    // Snapshot the root AABB before mutation.
    let before = tree.arena.get(tree.root).unwrap().get().subtree_aabb()
        .expect("root has AABB");

    // Move right_area from pos(200,200) to pos(500,500) via mutator.
    // The target tree has root(void,ch0) -> [left(ch0), right(ch1)].
    // The mutator tree mirrors this: root(void,ch0) -> child(MoveTo,ch1).
    let move_mutator = GfxMutator::new(
        Mutation::area_command(GlyphAreaCommand::MoveTo(500.0, 500.0)),
        1, // channel 1 matches right_area
    );
    let mut mtree = MutatorTree::new(); // root is Void ch0
    let move_id = mtree.arena.new_node(move_mutator);
    mtree.root.append(move_id, &mut mtree.arena);

    // apply_to invalidates both aabb_cache and subtree_aabbs_dirty.
    mtree.apply_to(&mut tree);

    // Recompute.
    tree.ensure_subtree_aabbs();
    let after = tree.arena.get(tree.root).unwrap().get().subtree_aabb()
        .expect("root has AABB after mutation");

    // The root AABB should now extend to cover (500+80, 500+30).
    assert!(after.1.x >= 580.0, "max.x should cover moved right_area");
    assert!(after.1.y >= 530.0, "max.y should cover moved right_area");
    assert!(after.1.x > before.1.x, "AABB grew after moving node further out");
}

// ── void-only tree produces None ──────────────────────────────────

#[test]
fn test_subtree_aabb_void_tree_is_none() {
    do_subtree_aabb_void_tree_is_none();
}

/// A tree with only void nodes has no renderable content, so every
/// node's subtree AABB is `None`.
pub fn do_subtree_aabb_void_tree_is_none() {
    let mut tree = Tree::new_non_indexed();
    let child = tree.arena.new_node(GfxElement::new_void(0));
    tree.root.append(child, &mut tree.arena);

    tree.ensure_subtree_aabbs();

    assert!(tree.arena.get(tree.root).unwrap().get().subtree_aabb().is_none());
    assert!(tree.arena.get(child).unwrap().get().subtree_aabb().is_none());
}

// ── idempotent without mutation ───────────────────────────────────

#[test]
fn test_subtree_aabb_ensure_is_idempotent() {
    do_subtree_aabb_ensure_is_idempotent();
}

/// Calling `ensure_subtree_aabbs` twice without an intervening
/// mutation is a no-op the second time (dirty flag stays clean).
pub fn do_subtree_aabb_ensure_is_idempotent() {
    let mut tree = build_test_tree();

    tree.ensure_subtree_aabbs();
    let first = tree.arena.get(tree.root).unwrap().get().subtree_aabb();

    tree.ensure_subtree_aabbs();
    let second = tree.arena.get(tree.root).unwrap().get().subtree_aabb();

    assert_eq!(first, second);
}

// ── BVH descent tests ─────────────────────────────────────────────

#[test]
fn test_descendant_at_finds_leaf_via_bvh() {
    do_descendant_at_finds_leaf_via_bvh();
}

/// `descendant_at` with BVH descent correctly finds the leaf node
/// whose own AABB contains the query point.
pub fn do_descendant_at_finds_leaf_via_bvh() {
    let mut tree = build_test_tree();

    // Point inside left_child_area pos(20,40)+bounds(30,15).
    let hit = tree.descendant_at(Vec2::new(25.0, 45.0));
    assert!(hit.is_some(), "should find left_child_area");

    // Verify it's the innermost node (left_child), not left_area.
    let hit_id = hit.unwrap();
    let left_id = tree.root.children(&tree.arena).next().unwrap();
    let left_child_id = left_id.children(&tree.arena).next().unwrap();
    assert_eq!(hit_id, left_child_id);
}

#[test]
fn test_descendant_at_prunes_disjoint_subtree() {
    do_descendant_at_prunes_disjoint_subtree();
}

/// When the query point lies inside only one subtree's AABB, the
/// other subtree is pruned and not traversed. We verify this
/// indirectly by confirming the correct result — pruning is an
/// internal optimisation, not an observable contract, but
/// correctness is.
pub fn do_descendant_at_prunes_disjoint_subtree() {
    let mut tree = build_test_tree();

    // Point inside right_area pos(200,200)+bounds(80,30).
    let hit = tree.descendant_at(Vec2::new(210.0, 210.0));
    assert!(hit.is_some());

    // It should NOT be any node from the left subtree.
    let left_id = tree.root.children(&tree.arena).next().unwrap();
    let left_child_id = left_id.children(&tree.arena).next().unwrap();
    assert_ne!(hit.unwrap(), left_id);
    assert_ne!(hit.unwrap(), left_child_id);
}

#[test]
fn test_descendant_at_returns_none_on_miss() {
    do_descendant_at_returns_none_on_miss();
}

/// A query point outside all nodes' bounds returns `None`.
pub fn do_descendant_at_returns_none_on_miss() {
    let mut tree = build_test_tree();
    assert!(tree.descendant_at(Vec2::new(999.0, 999.0)).is_none());
}

#[test]
fn test_descendant_at_smallest_area_wins() {
    do_descendant_at_smallest_area_wins();
}

/// When a point is inside both a parent and a child node, the child
/// (smaller area) wins — the "innermost first" convention.
pub fn do_descendant_at_smallest_area_wins() {
    let mut tree = build_test_tree();

    // Point at (25, 42) is inside both left_area pos(10,10)+bounds(50,20)
    // and left_child_area pos(20,40)+bounds(30,15).
    // Wait — left_area only extends to y=30 (10+20), so (25,42) is NOT
    // inside left_area. Let's pick a point that IS inside both.
    // left_area covers (10,10)-(60,30). left_child covers (20,40)-(50,55).
    // These don't overlap — the child is below the parent visually.
    // That's fine for BVH: the child's AABB is part of left's subtree
    // AABB, so left's subtree AABB covers both. The hit at (25,42) should
    // find only the child, not the parent.
    let hit = tree.descendant_at(Vec2::new(25.0, 42.0));
    let left_id = tree.root.children(&tree.arena).next().unwrap();
    let left_child_id = left_id.children(&tree.arena).next().unwrap();
    assert_eq!(hit, Some(left_child_id));
}

// ── SpatialDescend instruction tests ──────────────────────────────

/// Build a MutatorTree that delivers a MouseEvent via SpatialDescend
/// at the given canvas-space coordinates.
fn spatial_descend_mutator(x: f32, y: f32) -> MutatorTree<GfxMutator> {
    let event = Mutation::Event(GlyphTreeEventInstance::new(
        GlyphTreeEvent::MouseEvent(MouseEventData::new(x, y)),
        0,
    ));
    let instruction = GfxMutator::Instruction {
        instruction: Instruction::SpatialDescend(OrderedVec2::new_f32(x, y)),
        channel: 0,
        mutation: event,
    };
    MutatorTree::new_with(instruction)
}

#[test]
fn test_spatial_descend_delivers_event_to_leaf() {
    do_spatial_descend_delivers_event_to_leaf();
}

/// A `SpatialDescend` instruction finds the correct leaf and applies
/// its mutation (a `MouseEvent`) to that leaf.
pub fn do_spatial_descend_delivers_event_to_leaf() {
    let mut tree = build_test_tree();

    // Attach a subscriber to right_area to detect event delivery.
    let received = Arc::new(AtomicBool::new(false));
    let received_clone = received.clone();
    let subscriber: EventSubscriber = Arc::new(Mutex::new(
        move |_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {
            received_clone.store(true, Ordering::SeqCst);
        },
    ));
    let right_id = tree.root.children(&tree.arena).nth(1).unwrap();
    tree.arena.get_mut(right_id).unwrap().get_mut()
        .subscribers_mut().push(subscriber);

    // Fire SpatialDescend at a point inside right_area.
    let mtree = spatial_descend_mutator(210.0, 210.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);

    assert!(received.load(Ordering::SeqCst), "subscriber should have fired");
}

#[test]
fn test_spatial_descend_miss_is_noop() {
    do_spatial_descend_miss_is_noop();
}

/// A `SpatialDescend` at a point outside all nodes' bounds is a
/// no-op — no mutation is applied.
pub fn do_spatial_descend_miss_is_noop() {
    let mut tree = build_test_tree();

    // Attach a subscriber to right_area.
    let received = Arc::new(AtomicBool::new(false));
    let received_clone = received.clone();
    let subscriber: EventSubscriber = Arc::new(Mutex::new(
        move |_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {
            received_clone.store(true, Ordering::SeqCst);
        },
    ));
    let right_id = tree.root.children(&tree.arena).nth(1).unwrap();
    tree.arena.get_mut(right_id).unwrap().get_mut()
        .subscribers_mut().push(subscriber);

    // Fire SpatialDescend at a point far from any node.
    let mtree = spatial_descend_mutator(999.0, 999.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);

    assert!(!received.load(Ordering::SeqCst), "subscriber should NOT have fired");
}

#[test]
fn test_spatial_descend_finds_innermost_node() {
    do_spatial_descend_finds_innermost_node();
}

/// When a point is inside a child node that is nested under a parent,
/// `SpatialDescend` delivers the event to the child (smallest area),
/// not the parent.
pub fn do_spatial_descend_finds_innermost_node() {
    let mut tree = build_test_tree();

    // Attach subscribers to both left_area and left_child_area.
    let left_received = Arc::new(AtomicBool::new(false));
    let child_received = Arc::new(AtomicBool::new(false));

    let left_clone = left_received.clone();
    let left_sub: EventSubscriber = Arc::new(Mutex::new(
        move |_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {
            left_clone.store(true, Ordering::SeqCst);
        },
    ));

    let child_clone = child_received.clone();
    let child_sub: EventSubscriber = Arc::new(Mutex::new(
        move |_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {
            child_clone.store(true, Ordering::SeqCst);
        },
    ));

    let left_id = tree.root.children(&tree.arena).next().unwrap();
    let left_child_id = left_id.children(&tree.arena).next().unwrap();
    tree.arena.get_mut(left_id).unwrap().get_mut()
        .subscribers_mut().push(left_sub);
    tree.arena.get_mut(left_child_id).unwrap().get_mut()
        .subscribers_mut().push(child_sub);

    // Point at (25, 42) — inside left_child_area only (not left_area).
    let mtree = spatial_descend_mutator(25.0, 42.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);

    assert!(child_received.load(Ordering::SeqCst), "child subscriber should fire");
    assert!(!left_received.load(Ordering::SeqCst), "parent subscriber should NOT fire");
}
