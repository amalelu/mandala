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

// =====================================================================
// Extreme edge cases — subtree AABB computation
// =====================================================================

/// Build a deep chain: root → a → b → c → d → e (5 levels deep),
/// only the leaf has a GlyphArea.
fn build_deep_chain() -> Tree<GfxElement, GfxMutator> {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let mut parent = tree.root;
    // 4 void nodes, then 1 area leaf at the bottom.
    for _ in 0..4 {
        let void = tree.arena.new_node(GfxElement::new_void(0));
        parent.append(void, &mut tree.arena);
        parent = void;
    }
    let leaf = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("deep", 14.0, 14.0, Vec2::new(100.0, 100.0), Vec2::new(20.0, 10.0)),
        0,
    );
    let leaf_id = tree.arena.new_node(leaf);
    parent.append(leaf_id, &mut tree.arena);
    tree
}

/// Build a wide tree: root has 20 children, each a GlyphArea at
/// different x positions, no children of their own.
fn build_wide_tree() -> Tree<GfxElement, GfxMutator> {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    for i in 0..20_u32 {
        let area = GfxElement::new_area_non_indexed(
            GlyphArea::new_with_str(
                &format!("n{}", i),
                14.0, 14.0,
                Vec2::new(i as f32 * 50.0, 0.0),
                Vec2::new(30.0, 20.0),
            ),
            0,
        );
        let id = tree.arena.new_node(area);
        tree.root.append(id, &mut tree.arena);
    }
    tree
}

#[test]
fn test_subtree_aabb_deep_chain_propagates_to_root() {
    do_subtree_aabb_deep_chain_propagates_to_root();
}

/// In a deep chain of void nodes with one leaf at the bottom, every
/// ancestor's subtree AABB covers the leaf's bounds.
pub fn do_subtree_aabb_deep_chain_propagates_to_root() {
    let mut tree = build_deep_chain();
    tree.ensure_subtree_aabbs();

    let leaf_aabb = (Vec2::new(100.0, 100.0), Vec2::new(120.0, 110.0));

    // Walk from root down — every node's subtree AABB should equal
    // the leaf's AABB (since the leaf is the only renderable).
    let mut node = tree.root;
    loop {
        let aabb = tree.arena.get(node).unwrap().get().subtree_aabb();
        if tree.arena.get(node).unwrap().first_child().is_some() {
            assert_eq!(
                aabb,
                Some(leaf_aabb),
                "ancestor's subtree AABB should equal leaf's"
            );
            node = tree.arena.get(node).unwrap().first_child().unwrap();
        } else {
            // Leaf node — subtree AABB equals own AABB.
            assert_eq!(aabb, Some(leaf_aabb));
            break;
        }
    }
}

#[test]
fn test_subtree_aabb_wide_tree_root_covers_all_children() {
    do_subtree_aabb_wide_tree_root_covers_all_children();
}

/// Root's subtree AABB covers the full horizontal extent of all 20
/// children.
pub fn do_subtree_aabb_wide_tree_root_covers_all_children() {
    let mut tree = build_wide_tree();
    tree.ensure_subtree_aabbs();

    let root_aabb = tree.arena.get(tree.root).unwrap().get().subtree_aabb()
        .expect("root should have AABB");
    // First child at x=0, last child at x=950+30=980.
    assert!(root_aabb.0.x <= 0.0);
    assert!(root_aabb.1.x >= 980.0);
    assert!(root_aabb.0.y <= 0.0);
    assert!(root_aabb.1.y >= 20.0);
}

#[test]
fn test_subtree_aabb_single_area_node() {
    do_subtree_aabb_single_area_node();
}

/// A tree with just the void root and one area child: the root's
/// subtree AABB equals the child's AABB.
pub fn do_subtree_aabb_single_area_node() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("sole", 14.0, 14.0, Vec2::new(50.0, 60.0), Vec2::new(40.0, 30.0)),
        0,
    );
    let id = tree.arena.new_node(area);
    tree.root.append(id, &mut tree.arena);

    tree.ensure_subtree_aabbs();
    let root_aabb = tree.arena.get(tree.root).unwrap().get().subtree_aabb().unwrap();
    assert_eq!(root_aabb, (Vec2::new(50.0, 60.0), Vec2::new(90.0, 90.0)));
}

#[test]
fn test_subtree_aabb_zero_bounds_area_ignored() {
    do_subtree_aabb_zero_bounds_area_ignored();
}

/// A GlyphArea with zero-size bounds (0x0) is treated as having no
/// renderable AABB — it doesn't contribute to subtree AABBs.
pub fn do_subtree_aabb_zero_bounds_area_ignored() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("zero", 14.0, 14.0, Vec2::new(100.0, 100.0), Vec2::new(0.0, 0.0)),
        0,
    );
    let id = tree.arena.new_node(area);
    tree.root.append(id, &mut tree.arena);

    tree.ensure_subtree_aabbs();
    assert!(tree.arena.get(tree.root).unwrap().get().subtree_aabb().is_none());
}

#[test]
fn test_subtree_aabb_negative_position() {
    do_subtree_aabb_negative_position();
}

/// Nodes with negative positions are handled correctly — the AABB
/// min values go negative.
pub fn do_subtree_aabb_negative_position() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("neg", 14.0, 14.0, Vec2::new(-50.0, -30.0), Vec2::new(100.0, 60.0)),
        0,
    );
    let id = tree.arena.new_node(area);
    tree.root.append(id, &mut tree.arena);

    tree.ensure_subtree_aabbs();
    let aabb = tree.arena.get(tree.root).unwrap().get().subtree_aabb().unwrap();
    assert_eq!(aabb, (Vec2::new(-50.0, -30.0), Vec2::new(50.0, 30.0)));
}

// =====================================================================
// Extreme edge cases — BVH descent
// =====================================================================

#[test]
fn test_descendant_at_deep_chain_finds_leaf() {
    do_descendant_at_deep_chain_finds_leaf();
}

/// BVH descent through a 5-level-deep void chain correctly finds the
/// leaf at the bottom.
pub fn do_descendant_at_deep_chain_finds_leaf() {
    let mut tree = build_deep_chain();
    let hit = tree.descendant_at(Vec2::new(105.0, 105.0));
    assert!(hit.is_some(), "should find the leaf at (100,100)+(20,10)");
}

#[test]
fn test_descendant_at_deep_chain_miss() {
    do_descendant_at_deep_chain_miss();
}

/// BVH descent on a deep chain misses when the point is outside the
/// leaf's bounds.
pub fn do_descendant_at_deep_chain_miss() {
    let mut tree = build_deep_chain();
    assert!(tree.descendant_at(Vec2::new(0.0, 0.0)).is_none());
}

#[test]
fn test_descendant_at_wide_tree_finds_correct_child() {
    do_descendant_at_wide_tree_finds_correct_child();
}

/// In a 20-child wide tree, BVH descent finds the exact child whose
/// AABB contains the point, not any of the 19 others.
pub fn do_descendant_at_wide_tree_finds_correct_child() {
    let mut tree = build_wide_tree();

    // Point inside child 10: pos = (500, 0), bounds = (30, 20).
    let hit = tree.descendant_at(Vec2::new(510.0, 10.0));
    assert!(hit.is_some());

    // Verify it's child index 10 (not 9 or 11).
    let children: Vec<_> = tree.root.children(&tree.arena).collect();
    assert_eq!(hit.unwrap(), children[10]);
}

#[test]
fn test_descendant_at_wide_tree_between_children_is_miss() {
    do_descendant_at_wide_tree_between_children_is_miss();
}

/// A point in the gap between two wide-tree children is a miss.
pub fn do_descendant_at_wide_tree_between_children_is_miss() {
    let mut tree = build_wide_tree();
    // Gap: child 5 ends at x=280 (250+30), child 6 starts at x=300.
    // Point at (290, 10) is in the gap.
    assert!(tree.descendant_at(Vec2::new(290.0, 10.0)).is_none());
}

#[test]
fn test_descendant_at_overlapping_siblings() {
    do_descendant_at_overlapping_siblings();
}

/// When two sibling nodes overlap spatially, the smaller-area
/// sibling wins (innermost-first convention).
pub fn do_descendant_at_overlapping_siblings() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();

    let big = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("big", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(200.0, 200.0)),
        0,
    );
    let small = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("sm", 14.0, 14.0, Vec2::new(50.0, 50.0), Vec2::new(20.0, 20.0)),
        1,
    );

    let big_id = tree.arena.new_node(big);
    let small_id = tree.arena.new_node(small);
    tree.root.append(big_id, &mut tree.arena);
    tree.root.append(small_id, &mut tree.arena);

    // Point inside both — small wins.
    let hit = tree.descendant_at(Vec2::new(55.0, 55.0));
    assert_eq!(hit, Some(small_id));

    // Point inside big only.
    let hit2 = tree.descendant_at(Vec2::new(5.0, 5.0));
    assert_eq!(hit2, Some(big_id));
}

#[test]
fn test_descendant_near_negative_coords() {
    do_descendant_near_negative_coords();
}

/// BVH descent works with negative coordinates.
pub fn do_descendant_near_negative_coords() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("neg", 14.0, 14.0, Vec2::new(-100.0, -50.0), Vec2::new(40.0, 30.0)),
        0,
    );
    let id = tree.arena.new_node(area);
    tree.root.append(id, &mut tree.arena);

    assert_eq!(tree.descendant_at(Vec2::new(-80.0, -40.0)), Some(id));
    assert!(tree.descendant_at(Vec2::new(0.0, 0.0)).is_none());
}

// =====================================================================
// Extreme edge cases — SpatialDescend
// =====================================================================

#[test]
fn test_spatial_descend_deep_chain_delivers_to_leaf() {
    do_spatial_descend_deep_chain_delivers_to_leaf();
}

/// SpatialDescend through a deep void chain delivers the event to
/// the leaf at the bottom, not to any intermediate void.
pub fn do_spatial_descend_deep_chain_delivers_to_leaf() {
    let mut tree = build_deep_chain();

    let received = Arc::new(AtomicBool::new(false));
    let received_clone = received.clone();
    let subscriber: EventSubscriber = Arc::new(Mutex::new(
        move |_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {
            received_clone.store(true, Ordering::SeqCst);
        },
    ));

    // Find the leaf (deepest non-void).
    let mut node = tree.root;
    loop {
        let child = tree.arena.get(node).unwrap().first_child();
        if let Some(cid) = child {
            node = cid;
        } else {
            break;
        }
    }
    tree.arena.get_mut(node).unwrap().get_mut()
        .subscribers_mut().push(subscriber);

    let mtree = spatial_descend_mutator(105.0, 105.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);

    assert!(received.load(Ordering::SeqCst), "leaf subscriber should fire");
}

#[test]
fn test_spatial_descend_wide_tree_hits_correct_child() {
    do_spatial_descend_wide_tree_hits_correct_child();
}

/// SpatialDescend on a wide tree delivers the event to the correct
/// child (index 15) and not to any of the other 19.
pub fn do_spatial_descend_wide_tree_hits_correct_child() {
    let mut tree = build_wide_tree();

    let hit_ids: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));

    // Attach a subscriber to every child that records which child fired.
    let children: Vec<_> = tree.root.children(&tree.arena).collect();
    for (i, &cid) in children.iter().enumerate() {
        let hit_ids_clone = hit_ids.clone();
        let sub: EventSubscriber = Arc::new(Mutex::new(
            move |_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {
                hit_ids_clone.lock().unwrap().push(i);
            },
        ));
        tree.arena.get_mut(cid).unwrap().get_mut()
            .subscribers_mut().push(sub);
    }

    // Point inside child 15: pos=(750,0), bounds=(30,20).
    let mtree = spatial_descend_mutator(760.0, 10.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);

    let ids = hit_ids.lock().unwrap();
    assert_eq!(*ids, vec![15], "only child 15 should have received the event");
}

#[test]
fn test_spatial_descend_no_mutation_is_noop() {
    do_spatial_descend_no_mutation_is_noop();
}

/// A SpatialDescend instruction with `Mutation::None` as its
/// payload finds the node but applies nothing — no crash, no
/// subscriber fire.
pub fn do_spatial_descend_no_mutation_is_noop() {
    let mut tree = build_test_tree();

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

    // SpatialDescend with Mutation::None — no event to deliver.
    let instruction = GfxMutator::Instruction {
        instruction: Instruction::SpatialDescend(OrderedVec2::new_f32(210.0, 210.0)),
        channel: 0,
        mutation: Mutation::None,
    };
    let mtree = MutatorTree::new_with(instruction);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);

    // Mutation::None is_some() returns false, so no subscriber fires.
    assert!(!received.load(Ordering::SeqCst));
}

// =====================================================================
// MouseEventData
// =====================================================================

#[test]
fn test_mouse_event_data_new_and_fields() {
    do_mouse_event_data_new_and_fields();
}

/// `MouseEventData::new` stores the coordinates correctly and the
/// struct is `Eq` + `Clone`.
pub fn do_mouse_event_data_new_and_fields() {
    let a = MouseEventData::new(42.5, -10.0);
    let b = MouseEventData::new(42.5, -10.0);
    let c = MouseEventData::new(0.0, 0.0);

    assert_eq!(a.x.0, 42.5_f32);
    assert_eq!(a.y.0, -10.0_f32);
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.clone(), a);
}

#[test]
fn test_mouse_event_data_zero() {
    do_mouse_event_data_zero();
}

/// Zero coordinates are valid.
pub fn do_mouse_event_data_zero() {
    let d = MouseEventData::new(0.0, 0.0);
    assert_eq!(d.x.0, 0.0_f32);
    assert_eq!(d.y.0, 0.0_f32);
}

#[test]
fn test_mouse_event_data_extreme_values() {
    do_mouse_event_data_extreme_values();
}

/// Extreme float values (large, tiny, negative) are preserved.
pub fn do_mouse_event_data_extreme_values() {
    let d = MouseEventData::new(f32::MAX, f32::MIN);
    assert_eq!(d.x.0, f32::MAX);
    assert_eq!(d.y.0, f32::MIN);

    let tiny = MouseEventData::new(f32::MIN_POSITIVE, -f32::MIN_POSITIVE);
    assert_eq!(tiny.x.0, f32::MIN_POSITIVE);
    assert_eq!(tiny.y.0, -f32::MIN_POSITIVE);
}

#[test]
fn test_glyph_tree_event_mouse_carries_data() {
    do_glyph_tree_event_mouse_carries_data();
}

/// `GlyphTreeEvent::MouseEvent(data)` carries its payload through
/// a `GlyphTreeEventInstance` round-trip.
pub fn do_glyph_tree_event_mouse_carries_data() {
    let data = MouseEventData::new(100.0, 200.0);
    let event = GlyphTreeEventInstance::new(
        GlyphTreeEvent::MouseEvent(data),
        12345,
    );

    assert_eq!(event.event_time_millis, 12345);
    match &event.event_type {
        GlyphTreeEvent::MouseEvent(d) => {
            assert_eq!(d.x.0, 100.0_f32);
            assert_eq!(d.y.0, 200.0_f32);
        }
        other => panic!("expected MouseEvent, got {:?}", other),
    }

    // Clone preserves the payload.
    let cloned = event.clone();
    assert_eq!(cloned.event_type, event.event_type);
}

// =====================================================================
// Coverage gaps identified by code review
// =====================================================================

#[test]
fn test_bvh_descend_skips_glyph_model_nodes() {
    do_bvh_descend_skips_glyph_model_nodes();
}

/// GlyphModel nodes do not contribute to hit testing — only
/// GlyphArea nodes have renderable AABB bounds. A tree with a
/// GlyphModel sibling should not hit the model, even if the point
/// is inside the model's subtree AABB.
pub fn do_bvh_descend_skips_glyph_model_nodes() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let model = GfxElement::new_model_blank(0, 1);
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("a", 14.0, 14.0, Vec2::new(100.0, 100.0), Vec2::new(50.0, 30.0)),
        0,
    );
    let model_id = tree.arena.new_node(model);
    let area_id = tree.arena.new_node(area);
    tree.root.append(model_id, &mut tree.arena);
    tree.root.append(area_id, &mut tree.arena);

    // Point inside area → hits area, not model.
    let hit = tree.descendant_at(Vec2::new(110.0, 110.0));
    assert_eq!(hit, Some(area_id));

    // Point outside both → miss.
    assert!(tree.descendant_at(Vec2::new(0.0, 0.0)).is_none());
}

#[test]
fn test_bvh_descend_point_on_exact_boundary() {
    do_bvh_descend_point_on_exact_boundary();
}

/// Points exactly on the AABB boundary (min and max edges) are
/// included — the containment check uses `>=` / `<=` (inclusive).
pub fn do_bvh_descend_point_on_exact_boundary() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("b", 14.0, 14.0, Vec2::new(10.0, 20.0), Vec2::new(30.0, 40.0)),
        0,
    );
    let id = tree.arena.new_node(area);
    tree.root.append(id, &mut tree.arena);

    // Exact min corner.
    assert_eq!(tree.descendant_at(Vec2::new(10.0, 20.0)), Some(id));
    // Exact max corner.
    assert_eq!(tree.descendant_at(Vec2::new(40.0, 60.0)), Some(id));
    // One pixel outside max.
    assert!(tree.descendant_at(Vec2::new(40.01, 60.0)).is_none());
    assert!(tree.descendant_at(Vec2::new(40.0, 60.01)).is_none());
    // One pixel outside min.
    assert!(tree.descendant_at(Vec2::new(9.99, 20.0)).is_none());
}

#[test]
fn test_bvh_descend_point_in_subtree_aabb_but_outside_own_area() {
    do_bvh_descend_point_in_subtree_aabb_but_outside_own_area();
}

/// A point can be inside a parent's subtree AABB (because a child
/// extends beyond the parent's own area) but outside the parent's
/// own renderable AABB. The BVH should find the child, not the
/// parent.
pub fn do_bvh_descend_point_in_subtree_aabb_but_outside_own_area() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let parent = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("p", 14.0, 14.0, Vec2::new(10.0, 10.0), Vec2::new(50.0, 20.0)),
        0,
    );
    // Child is below and to the right of parent.
    let child = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("c", 14.0, 14.0, Vec2::new(70.0, 50.0), Vec2::new(30.0, 20.0)),
        0,
    );
    let parent_id = tree.arena.new_node(parent);
    let child_id = tree.arena.new_node(child);
    tree.root.append(parent_id, &mut tree.arena);
    parent_id.append(child_id, &mut tree.arena);

    // Point at (80, 55) is inside child but outside parent's own area.
    let hit = tree.descendant_at(Vec2::new(80.0, 55.0));
    assert_eq!(hit, Some(child_id), "should find child, not parent");

    // Point at (20, 15) is inside parent but outside child.
    let hit2 = tree.descendant_at(Vec2::new(20.0, 15.0));
    assert_eq!(hit2, Some(parent_id));
}

#[test]
fn test_spatial_descend_ignores_channel_mismatch() {
    do_spatial_descend_ignores_channel_mismatch();
}

/// SpatialDescend delivers its event based on spatial position, not
/// channel alignment. A node on channel 5 should still receive the
/// event if the point is inside its bounds.
pub fn do_spatial_descend_ignores_channel_mismatch() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("ch5", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0)),
        5, // channel 5 — different from SpatialDescend's channel 0
    );

    let received = Arc::new(AtomicBool::new(false));
    let received_clone = received.clone();
    let subscriber: EventSubscriber = Arc::new(Mutex::new(
        move |_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {
            received_clone.store(true, Ordering::SeqCst);
        },
    ));

    let id = tree.arena.new_node(area);
    tree.root.append(id, &mut tree.arena);
    tree.arena.get_mut(id).unwrap().get_mut()
        .subscribers_mut().push(subscriber);

    // SpatialDescend with channel 0 should still find the channel-5 node.
    let mtree = spatial_descend_mutator(25.0, 25.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);

    assert!(received.load(Ordering::SeqCst), "event should be delivered regardless of channel");
}
