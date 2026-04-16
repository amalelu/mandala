//! Tests for BVH-accelerated `descendant_at` / `descendant_near` (§T1).
//!
//! Covers hit/miss, pruning, overlapping siblings, boundary points,
//! negative coordinates, deep/wide trees, GlyphModel skipping, and
//! the subtree-AABB-but-not-own-area case. Follows the `do_*()` /
//! `test_*()` benchmark-reuse split (§T2.2).
//!
//! Shared fixtures live in `subtree_aabb_tests` (build_test_tree,
//! build_deep_chain, build_wide_tree).

use glam::Vec2;

use crate::font::fonts;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::gfx_structs::tests::subtree_aabb_tests::{
    build_test_tree, build_deep_chain, build_wide_tree,
};

// ── basic hit/miss ────────────────────────────────────────────────

#[test]
fn test_descendant_at_finds_leaf_via_bvh() {
    do_descendant_at_finds_leaf_via_bvh();
}

pub fn do_descendant_at_finds_leaf_via_bvh() {
    let mut tree = build_test_tree();
    let hit = tree.descendant_at(Vec2::new(25.0, 45.0));
    assert!(hit.is_some());
    let left_id = tree.root.children(&tree.arena).next().unwrap();
    let left_child_id = left_id.children(&tree.arena).next().unwrap();
    assert_eq!(hit.unwrap(), left_child_id);
}

#[test]
fn test_descendant_at_prunes_disjoint_subtree() {
    do_descendant_at_prunes_disjoint_subtree();
}

pub fn do_descendant_at_prunes_disjoint_subtree() {
    let mut tree = build_test_tree();
    let hit = tree.descendant_at(Vec2::new(210.0, 210.0));
    assert!(hit.is_some());
    let left_id = tree.root.children(&tree.arena).next().unwrap();
    let left_child_id = left_id.children(&tree.arena).next().unwrap();
    assert_ne!(hit.unwrap(), left_id);
    assert_ne!(hit.unwrap(), left_child_id);
}

#[test]
fn test_descendant_at_returns_none_on_miss() {
    do_descendant_at_returns_none_on_miss();
}

pub fn do_descendant_at_returns_none_on_miss() {
    let mut tree = build_test_tree();
    assert!(tree.descendant_at(Vec2::new(999.0, 999.0)).is_none());
}

#[test]
fn test_descendant_at_smallest_area_wins() {
    do_descendant_at_smallest_area_wins();
}

pub fn do_descendant_at_smallest_area_wins() {
    let mut tree = build_test_tree();
    let hit = tree.descendant_at(Vec2::new(25.0, 42.0));
    let left_id = tree.root.children(&tree.arena).next().unwrap();
    let left_child_id = left_id.children(&tree.arena).next().unwrap();
    assert_eq!(hit, Some(left_child_id));
}

// ── deep / wide trees ─────────────────────────────────────────────

#[test]
fn test_descendant_at_deep_chain_finds_leaf() {
    do_descendant_at_deep_chain_finds_leaf();
}

pub fn do_descendant_at_deep_chain_finds_leaf() {
    let mut tree = build_deep_chain();
    assert!(tree.descendant_at(Vec2::new(105.0, 105.0)).is_some());
}

#[test]
fn test_descendant_at_deep_chain_miss() {
    do_descendant_at_deep_chain_miss();
}

pub fn do_descendant_at_deep_chain_miss() {
    let mut tree = build_deep_chain();
    assert!(tree.descendant_at(Vec2::new(0.0, 0.0)).is_none());
}

#[test]
fn test_descendant_at_wide_tree_finds_correct_child() {
    do_descendant_at_wide_tree_finds_correct_child();
}

pub fn do_descendant_at_wide_tree_finds_correct_child() {
    let mut tree = build_wide_tree();
    let hit = tree.descendant_at(Vec2::new(510.0, 10.0));
    assert!(hit.is_some());
    let children: Vec<_> = tree.root.children(&tree.arena).collect();
    assert_eq!(hit.unwrap(), children[10]);
}

#[test]
fn test_descendant_at_wide_tree_gap_is_miss() {
    do_descendant_at_wide_tree_gap_is_miss();
}

pub fn do_descendant_at_wide_tree_gap_is_miss() {
    let mut tree = build_wide_tree();
    assert!(tree.descendant_at(Vec2::new(290.0, 10.0)).is_none());
}

// ── overlapping siblings ──────────────────────────────────────────

#[test]
fn test_descendant_at_overlapping_siblings() {
    do_descendant_at_overlapping_siblings();
}

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

    assert_eq!(tree.descendant_at(Vec2::new(55.0, 55.0)), Some(small_id));
    assert_eq!(tree.descendant_at(Vec2::new(5.0, 5.0)), Some(big_id));
}

// ── negative coordinates ──────────────────────────────────────────

#[test]
fn test_descendant_near_negative_coords() {
    do_descendant_near_negative_coords();
}

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

// ── GlyphModel skipped ───────────────────────────────────────────

#[test]
fn test_bvh_descend_skips_glyph_model_nodes() {
    do_bvh_descend_skips_glyph_model_nodes();
}

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

    assert_eq!(tree.descendant_at(Vec2::new(110.0, 110.0)), Some(area_id));
    assert!(tree.descendant_at(Vec2::new(0.0, 0.0)).is_none());
}

// ── exact boundary points ─────────────────────────────────────────

#[test]
fn test_bvh_descend_point_on_exact_boundary() {
    do_bvh_descend_point_on_exact_boundary();
}

/// Containment uses `>=` / `<=` — boundaries are inclusive.
pub fn do_bvh_descend_point_on_exact_boundary() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("b", 14.0, 14.0, Vec2::new(10.0, 20.0), Vec2::new(30.0, 40.0)),
        0,
    );
    let id = tree.arena.new_node(area);
    tree.root.append(id, &mut tree.arena);

    assert_eq!(tree.descendant_at(Vec2::new(10.0, 20.0)), Some(id));
    assert_eq!(tree.descendant_at(Vec2::new(40.0, 60.0)), Some(id));
    assert!(tree.descendant_at(Vec2::new(40.01, 60.0)).is_none());
    assert!(tree.descendant_at(Vec2::new(40.0, 60.01)).is_none());
    assert!(tree.descendant_at(Vec2::new(9.99, 20.0)).is_none());
}

// ── point in subtree AABB but outside own area ────────────────────

#[test]
fn test_bvh_point_in_subtree_aabb_but_outside_own_area() {
    do_bvh_point_in_subtree_aabb_but_outside_own_area();
}

/// The core BVH case: child extends beyond parent's own area.
pub fn do_bvh_point_in_subtree_aabb_but_outside_own_area() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let parent = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("p", 14.0, 14.0, Vec2::new(10.0, 10.0), Vec2::new(50.0, 20.0)),
        0,
    );
    let child = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("c", 14.0, 14.0, Vec2::new(70.0, 50.0), Vec2::new(30.0, 20.0)),
        0,
    );
    let parent_id = tree.arena.new_node(parent);
    let child_id = tree.arena.new_node(child);
    tree.root.append(parent_id, &mut tree.arena);
    parent_id.append(child_id, &mut tree.arena);

    assert_eq!(tree.descendant_at(Vec2::new(80.0, 55.0)), Some(child_id));
    assert_eq!(tree.descendant_at(Vec2::new(20.0, 15.0)), Some(parent_id));
}

// ── descendant_near with slack ────────────────────────────────────

#[test]
fn test_descendant_near_slack_expands_hit_region() {
    do_descendant_near_slack_expands_hit_region();
}

/// `descendant_near` with `slack > 0` finds nodes when the point is
/// just outside the AABB but within the slack margin.
pub fn do_descendant_near_slack_expands_hit_region() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("s", 14.0, 14.0, Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0)),
        0,
    );
    let id = tree.arena.new_node(area);
    tree.root.append(id, &mut tree.arena);

    // Point just outside on the right: x=31, AABB max_x=30.
    assert!(tree.descendant_at(Vec2::new(31.0, 20.0)).is_none());
    assert_eq!(tree.descendant_near(Vec2::new(31.0, 20.0), 2.0), Some(id));
    // Slack too small.
    assert!(tree.descendant_near(Vec2::new(31.0, 20.0), 0.5).is_none());

    // Point just outside on the top: y=9, AABB min_y=10.
    assert!(tree.descendant_at(Vec2::new(20.0, 9.0)).is_none());
    assert_eq!(tree.descendant_near(Vec2::new(20.0, 9.0), 2.0), Some(id));
}

#[test]
fn test_descendant_near_slack_smallest_area_still_wins() {
    do_descendant_near_slack_smallest_area_still_wins();
}

/// When slack causes two nodes' expanded AABBs to both contain the
/// point, the smaller-area node still wins (tie-break by original
/// area, not slacked area).
pub fn do_descendant_near_slack_smallest_area_still_wins() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let big = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("big", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0)),
        0,
    );
    let small = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("sm", 14.0, 14.0, Vec2::new(50.0, 50.0), Vec2::new(10.0, 10.0)),
        1,
    );
    let big_id = tree.arena.new_node(big);
    let small_id = tree.arena.new_node(small);
    tree.root.append(big_id, &mut tree.arena);
    tree.root.append(small_id, &mut tree.arena);

    // Point at (62, 55) is outside small's AABB (max_x=60) but within
    // slack=5. Both big and small+slack contain the point; small wins.
    let hit = tree.descendant_near(Vec2::new(62.0, 55.0), 5.0);
    assert_eq!(hit, Some(small_id));
}
