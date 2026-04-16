//! Tests for per-node subtree AABB computation and invalidation (§T1).
//!
//! Covers [`Tree::ensure_subtree_aabbs`], bottom-up computation,
//! invalidation on mutation, and edge cases (void trees, negative
//! positions, zero-bounds areas, deep/wide trees). Follows the
//! `do_*()` / `test_*()` benchmark-reuse split (§T2.2).
//!
//! BVH descent tests live in `bvh_descent_tests.rs`.
//! SpatialDescend tests live in `spatial_descend_tests.rs`.

use glam::Vec2;

use crate::core::primitives::Applicable;
use crate::font::fonts;
use crate::gfx_structs::area::{GlyphArea, GlyphAreaCommand};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutator, Mutation};
use crate::gfx_structs::tree::{MutatorTree, Tree};

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
pub fn build_test_tree() -> Tree<GfxElement, GfxMutator> {
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

/// Deep chain: root → 4 void nodes → 1 area leaf (5 levels).
pub fn build_deep_chain() -> Tree<GfxElement, GfxMutator> {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let mut parent = tree.root;
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

/// Wide tree: root has 20 GlyphArea children at different x positions.
pub fn build_wide_tree() -> Tree<GfxElement, GfxMutator> {
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

// ── leaf subtree AABB equals own bounds ───────────────────────────

#[test]
fn test_subtree_aabb_leaf_equals_own_bounds() {
    do_subtree_aabb_leaf_equals_own_bounds();
}

pub fn do_subtree_aabb_leaf_equals_own_bounds() {
    let mut tree = build_test_tree();
    tree.ensure_subtree_aabbs();
    let right_id = tree.root.children(&tree.arena).nth(1).expect("right child");
    let aabb = tree.arena.get(right_id).unwrap().get().subtree_aabb();
    assert_eq!(aabb, Some((Vec2::new(200.0, 200.0), Vec2::new(280.0, 230.0))));
}

// ── parent encloses children ──────────────────────────────────────

#[test]
fn test_subtree_aabb_parent_encloses_children() {
    do_subtree_aabb_parent_encloses_children();
}

pub fn do_subtree_aabb_parent_encloses_children() {
    let mut tree = build_test_tree();
    tree.ensure_subtree_aabbs();
    let left_id = tree.root.children(&tree.arena).next().expect("left child");
    let (min, max) = tree.arena.get(left_id).unwrap().get().subtree_aabb()
        .expect("left subtree has an AABB");
    assert!(min.x <= 10.0);
    assert!(min.y <= 10.0);
    assert!(max.x >= 50.0);
    assert!(max.y >= 55.0);
}

// ── root encloses entire tree ─────────────────────────────────────

#[test]
fn test_subtree_aabb_root_encloses_entire_tree() {
    do_subtree_aabb_root_encloses_entire_tree();
}

pub fn do_subtree_aabb_root_encloses_entire_tree() {
    let mut tree = build_test_tree();
    tree.ensure_subtree_aabbs();
    let (min, max) = tree.arena.get(tree.root).unwrap().get().subtree_aabb()
        .expect("root subtree has an AABB");
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

pub fn do_subtree_aabb_invalidated_by_mutation() {
    let mut tree = build_test_tree();
    tree.ensure_subtree_aabbs();
    let before = tree.arena.get(tree.root).unwrap().get().subtree_aabb()
        .expect("root has AABB");

    let move_mutator = GfxMutator::new(
        Mutation::area_command(GlyphAreaCommand::MoveTo(500.0, 500.0)),
        1,
    );
    let mut mtree = MutatorTree::new();
    let move_id = mtree.arena.new_node(move_mutator);
    mtree.root.append(move_id, &mut mtree.arena);
    mtree.apply_to(&mut tree);

    tree.ensure_subtree_aabbs();
    let after = tree.arena.get(tree.root).unwrap().get().subtree_aabb()
        .expect("root has AABB after mutation");
    assert!(after.1.x >= 580.0);
    assert!(after.1.y >= 530.0);
    assert!(after.1.x > before.1.x);
}

// ── void-only tree produces None ──────────────────────────────────

#[test]
fn test_subtree_aabb_void_tree_is_none() {
    do_subtree_aabb_void_tree_is_none();
}

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

pub fn do_subtree_aabb_ensure_is_idempotent() {
    let mut tree = build_test_tree();
    tree.ensure_subtree_aabbs();
    let first = tree.arena.get(tree.root).unwrap().get().subtree_aabb();
    tree.ensure_subtree_aabbs();
    let second = tree.arena.get(tree.root).unwrap().get().subtree_aabb();
    assert_eq!(first, second);
}

// ── deep chain propagation ────────────────────────────────────────

#[test]
fn test_subtree_aabb_deep_chain_propagates_to_root() {
    do_subtree_aabb_deep_chain_propagates_to_root();
}

pub fn do_subtree_aabb_deep_chain_propagates_to_root() {
    let mut tree = build_deep_chain();
    tree.ensure_subtree_aabbs();
    let leaf_aabb = (Vec2::new(100.0, 100.0), Vec2::new(120.0, 110.0));
    let mut node = tree.root;
    loop {
        let aabb = tree.arena.get(node).unwrap().get().subtree_aabb();
        if tree.arena.get(node).unwrap().first_child().is_some() {
            assert_eq!(aabb, Some(leaf_aabb));
            node = tree.arena.get(node).unwrap().first_child().unwrap();
        } else {
            assert_eq!(aabb, Some(leaf_aabb));
            break;
        }
    }
}

// ── wide tree root covers all ─────────────────────────────────────

#[test]
fn test_subtree_aabb_wide_tree_root_covers_all() {
    do_subtree_aabb_wide_tree_root_covers_all();
}

pub fn do_subtree_aabb_wide_tree_root_covers_all() {
    let mut tree = build_wide_tree();
    tree.ensure_subtree_aabbs();
    let root_aabb = tree.arena.get(tree.root).unwrap().get().subtree_aabb()
        .expect("root should have AABB");
    assert!(root_aabb.0.x <= 0.0);
    assert!(root_aabb.1.x >= 980.0);
    assert!(root_aabb.1.y >= 20.0);
}

// ── single area node ──────────────────────────────────────────────

#[test]
fn test_subtree_aabb_single_area_node() {
    do_subtree_aabb_single_area_node();
}

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
    assert_eq!(
        tree.arena.get(tree.root).unwrap().get().subtree_aabb().unwrap(),
        (Vec2::new(50.0, 60.0), Vec2::new(90.0, 90.0))
    );
}

// ── zero bounds area ignored ──────────────────────────────────────

#[test]
fn test_subtree_aabb_zero_bounds_area_ignored() {
    do_subtree_aabb_zero_bounds_area_ignored();
}

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

// ── negative position ─────────────────────────────────────────────

#[test]
fn test_subtree_aabb_negative_position() {
    do_subtree_aabb_negative_position();
}

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
    assert_eq!(
        tree.arena.get(tree.root).unwrap().get().subtree_aabb().unwrap(),
        (Vec2::new(-50.0, -30.0), Vec2::new(50.0, 30.0))
    );
}
