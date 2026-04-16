//! Tests for [`crate::gfx_structs::scene::Scene`] and its tree hit-test
//! helpers.
//!
//! Follows the `do_*()` / `test_*()` split from §B7 — every public
//! body is benchmarkable from `benches/test_bench.rs`.

use glam::Vec2;
use indextree::NodeId;

use crate::core::primitives::Applicable;
use crate::font::fonts;
use crate::gfx_structs::area::{GlyphArea, GlyphAreaField};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutator, Mutation};
use crate::gfx_structs::scene::{Scene, SceneTreeId};
use crate::gfx_structs::tree::{MutatorTree, Tree};

/// Build a small tree with one `GlyphArea` of the given AABB,
/// returning the tree and the id of the single leaf we can hit-test
/// against.
fn tree_with_area(
    position: Vec2,
    bounds: Vec2,
    channel: usize,
) -> (Tree<GfxElement, GfxMutator>, NodeId) {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let area = GlyphArea::new_with_str("x", 1.0, 10.0, position, bounds);
    let element = GfxElement::new_area_non_indexed_with_id(area, channel, channel);
    let leaf = tree.arena.new_node(element);
    tree.root.append(leaf, &mut tree.arena);
    (tree, leaf)
}

/// A tree containing two overlapping glyph areas: an outer 100x100
/// and an inner 20x20 centred in the outer. Used to verify
/// smallest-first hit semantics.
fn tree_outer_inner() -> (Tree<GfxElement, GfxMutator>, NodeId, NodeId) {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let outer = GlyphArea::new_with_str(
        "outer",
        1.0,
        10.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 100.0),
    );
    let inner = GlyphArea::new_with_str(
        "inner",
        1.0,
        10.0,
        Vec2::new(40.0, 40.0),
        Vec2::new(20.0, 20.0),
    );
    let outer_id = tree
        .arena
        .new_node(GfxElement::new_area_non_indexed_with_id(outer, 0, 1));
    let inner_id = tree
        .arena
        .new_node(GfxElement::new_area_non_indexed_with_id(inner, 0, 2));
    tree.root.append(outer_id, &mut tree.arena);
    outer_id.append(inner_id, &mut tree.arena);
    (tree, outer_id, inner_id)
}

// =====================================================================
// descendant_at
// =====================================================================

#[test]
pub fn test_descendant_at_hits_single_area() {
    do_descendant_at_hits_single_area();
}

pub fn do_descendant_at_hits_single_area() {
    let (mut tree, leaf) = tree_with_area(Vec2::new(10.0, 10.0), Vec2::new(50.0, 50.0), 0);
    assert_eq!(tree.descendant_at(Vec2::new(20.0, 20.0)), Some(leaf));
    assert_eq!(tree.descendant_at(Vec2::new(100.0, 100.0)), None);
}

#[test]
pub fn test_descendant_at_prefers_smallest() {
    do_descendant_at_prefers_smallest();
}

pub fn do_descendant_at_prefers_smallest() {
    let (mut tree, _outer, inner) = tree_outer_inner();
    // Point inside both: inner wins.
    assert_eq!(tree.descendant_at(Vec2::new(50.0, 50.0)), Some(inner));
    // Point only in outer.
    assert!(tree.descendant_at(Vec2::new(5.0, 5.0)).is_some());
    assert_ne!(tree.descendant_at(Vec2::new(5.0, 5.0)), Some(inner));
}

#[test]
pub fn test_descendant_at_returns_none_on_empty_tree() {
    do_descendant_at_returns_none_on_empty_tree();
}

pub fn do_descendant_at_returns_none_on_empty_tree() {
    fonts::init();
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    assert_eq!(tree.descendant_at(Vec2::new(0.0, 0.0)), None);
}

#[test]
pub fn test_descendants_aabb_covers_all_areas() {
    do_descendants_aabb_covers_all_areas();
}

pub fn do_descendants_aabb_covers_all_areas() {
    let (tree, _outer, _inner) = tree_outer_inner();
    let (min, max) = tree
        .descendants_aabb()
        .expect("outer+inner tree should have a bbox");
    assert!((min.x - 0.0).abs() < 1e-3);
    assert!((min.y - 0.0).abs() < 1e-3);
    assert!((max.x - 100.0).abs() < 1e-3);
    assert!((max.y - 100.0).abs() < 1e-3);
}

// =====================================================================
// Scene: insert / remove / layer order / offset
// =====================================================================

#[test]
pub fn test_scene_insert_and_component_at() {
    do_scene_insert_and_component_at();
}

pub fn do_scene_insert_and_component_at() {
    let (tree_a, leaf_a) = tree_with_area(Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0), 0);
    let (tree_b, leaf_b) = tree_with_area(Vec2::new(100.0, 0.0), Vec2::new(50.0, 50.0), 0);

    let mut scene = Scene::new();
    let id_a = scene.insert(tree_a, 0, Vec2::ZERO);
    let id_b = scene.insert(tree_b, 0, Vec2::ZERO);

    assert_eq!(scene.len(), 2);
    assert_eq!(
        scene.component_at(Vec2::new(10.0, 10.0)),
        Some((id_a, leaf_a))
    );
    assert_eq!(
        scene.component_at(Vec2::new(110.0, 10.0)),
        Some((id_b, leaf_b))
    );
    assert_eq!(scene.component_at(Vec2::new(500.0, 500.0)), None);
}

#[test]
pub fn test_scene_layer_order_controls_hit_priority() {
    do_scene_layer_order_controls_hit_priority();
}

pub fn do_scene_layer_order_controls_hit_priority() {
    // Two trees occupying the same screen area: the higher layer
    // must win the hit.
    let (tree_bg, leaf_bg) = tree_with_area(Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0), 0);
    let (tree_fg, leaf_fg) = tree_with_area(Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0), 0);

    let mut scene = Scene::new();
    let id_bg = scene.insert(tree_bg, 0, Vec2::ZERO);
    let id_fg = scene.insert(tree_fg, 10, Vec2::ZERO);

    let hit = scene.component_at(Vec2::new(10.0, 10.0));
    assert_eq!(hit, Some((id_fg, leaf_fg)));

    // Flip layers and re-test.
    scene.set_layer(id_fg, -1);
    let hit = scene.component_at(Vec2::new(10.0, 10.0));
    assert_eq!(hit, Some((id_bg, leaf_bg)));
}

#[test]
pub fn test_scene_offset_is_applied_to_hit_test() {
    do_scene_offset_is_applied_to_hit_test();
}

pub fn do_scene_offset_is_applied_to_hit_test() {
    let (tree, leaf) = tree_with_area(Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0), 0);
    let mut scene = Scene::new();
    let id = scene.insert(tree, 0, Vec2::new(200.0, 200.0));

    // Tree-local (10,10) is screen-space (210, 210).
    assert_eq!(
        scene.component_at(Vec2::new(210.0, 210.0)),
        Some((id, leaf))
    );
    // Tree-local (10, 10) not at (10, 10) screen-space — out of bounds.
    assert_eq!(scene.component_at(Vec2::new(10.0, 10.0)), None);
}

#[test]
pub fn test_scene_invisible_trees_are_skipped() {
    do_scene_invisible_trees_are_skipped();
}

pub fn do_scene_invisible_trees_are_skipped() {
    let (tree, _leaf) = tree_with_area(Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0), 0);
    let mut scene = Scene::new();
    let id = scene.insert(tree, 0, Vec2::ZERO);

    assert!(scene.component_at(Vec2::new(10.0, 10.0)).is_some());
    scene.set_visible(id, false);
    assert!(scene.component_at(Vec2::new(10.0, 10.0)).is_none());
}

#[test]
pub fn test_scene_remove_drops_entry() {
    do_scene_remove_drops_entry();
}

pub fn do_scene_remove_drops_entry() {
    let (tree, _leaf) = tree_with_area(Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0), 0);
    let mut scene = Scene::new();
    let id = scene.insert(tree, 0, Vec2::ZERO);
    assert_eq!(scene.len(), 1);
    assert!(scene.remove(id).is_some());
    assert_eq!(scene.len(), 0);
    assert!(scene.component_at(Vec2::new(10.0, 10.0)).is_none());
}

#[test]
pub fn test_scene_ids_in_layer_order_is_stable_by_insertion() {
    do_scene_ids_in_layer_order_is_stable_by_insertion();
}

pub fn do_scene_ids_in_layer_order_is_stable_by_insertion() {
    let (t1, _) = tree_with_area(Vec2::ZERO, Vec2::new(10.0, 10.0), 0);
    let (t2, _) = tree_with_area(Vec2::ZERO, Vec2::new(10.0, 10.0), 0);
    let (t3, _) = tree_with_area(Vec2::ZERO, Vec2::new(10.0, 10.0), 0);
    let mut scene = Scene::new();
    let a = scene.insert(t1, 5, Vec2::ZERO);
    let b = scene.insert(t2, 0, Vec2::ZERO);
    let c = scene.insert(t3, 5, Vec2::ZERO);
    let order: Vec<SceneTreeId> = scene.ids_in_layer_order();
    // b has lowest layer → first. a and c share layer 5 → a before c
    // because a was inserted first.
    assert_eq!(order, vec![b, a, c]);
}

// =====================================================================
// AABB cache invalidation
// =====================================================================

#[test]
pub fn test_descendants_aabb_cache_invalidated_by_mutator() {
    do_descendants_aabb_cache_invalidated_by_mutator();
}

pub fn do_descendants_aabb_cache_invalidated_by_mutator() {
    // Channel 1 lets the leaf accept a mutator that travels through
    // the void root (channel 0) and matches on the child.
    let (mut tree, _leaf) = tree_with_area(Vec2::new(10.0, 10.0), Vec2::new(50.0, 50.0), 1);

    // Warm the bbox cache.
    let initial = tree
        .descendants_aabb()
        .expect("seeded tree has one visible area");
    assert!((initial.0.x - 10.0).abs() < 1e-3);

    // Mutator: void parent + child that overwrites the area's
    // position to (100, 100). Bounds and size unchanged.
    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let area_delta =
        crate::gfx_structs::area::DeltaGlyphArea::new(vec![
            GlyphAreaField::position(100.0, 100.0),
            GlyphAreaField::Operation(crate::core::primitives::ApplyOperation::Assign),
        ]);
    let mutator_node = mutator
        .arena
        .new_node(GfxMutator::new(Mutation::area_delta(area_delta), 1));
    mutator.root.append(mutator_node, &mut mutator.arena);

    mutator.apply_to(&mut tree);

    let after = tree
        .descendants_aabb()
        .expect("post-mutation tree still has the area");
    assert!(
        (after.0.x - 100.0).abs() < 1e-3,
        "expected position.x to track the mutator (was {}, expected 100)",
        after.0.x
    );
}

// =====================================================================
// Slack hit-test (descendant_near)
// =====================================================================

#[test]
pub fn test_descendant_near_grants_slack() {
    do_descendant_near_grants_slack();
}

pub fn do_descendant_near_grants_slack() {
    let (mut tree, leaf) = tree_with_area(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0), 0);
    // Just outside the AABB on the right.
    let pt = Vec2::new(15.0, 5.0);
    assert_eq!(tree.descendant_at(pt), None);
    assert_eq!(tree.descendant_near(pt, 6.0), Some(leaf));
    assert_eq!(tree.descendant_near(pt, 4.0), None);
}
