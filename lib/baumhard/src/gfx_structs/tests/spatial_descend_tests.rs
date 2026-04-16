//! Tests for [`Instruction::SpatialDescend`] and [`MouseEventData`]
//! (§T1).
//!
//! Covers event delivery, miss/no-op, innermost-node selection,
//! deep/wide trees, channel-override behaviour, Mutation::None
//! payload, and MouseEventData construction/equality. Follows the
//! `do_*()` / `test_*()` benchmark-reuse split (§T2.2).
//!
//! Shared fixtures live in `subtree_aabb_tests`.

use glam::Vec2;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::font::fonts;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{
    GfxMutator, GlyphTreeEvent, GlyphTreeEventInstance, Instruction,
    MouseEventData, Mutation,
};
use crate::gfx_structs::tree::{EventSubscriber, MutatorTree, Tree};
use crate::gfx_structs::tree_walker::walk_tree_from;
use crate::util::ordered_vec2::OrderedVec2;

use crate::gfx_structs::tests::subtree_aabb_tests::{
    build_test_tree, build_deep_chain, build_wide_tree,
};

/// Build a MutatorTree that delivers a MouseEvent via SpatialDescend.
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

// ── event delivery ────────────────────────────────────────────────

#[test]
fn test_spatial_descend_delivers_event_to_leaf() {
    do_spatial_descend_delivers_event_to_leaf();
}

pub fn do_spatial_descend_delivers_event_to_leaf() {
    let mut tree = build_test_tree();
    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Arc::new(Mutex::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| { rc.store(true, Ordering::SeqCst); },
    ));
    let right_id = tree.root.children(&tree.arena).nth(1).unwrap();
    tree.arena.get_mut(right_id).unwrap().get_mut()
        .subscribers_mut().push(sub);

    let mtree = spatial_descend_mutator(210.0, 210.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);
    assert!(received.load(Ordering::SeqCst));
}

// ── miss is no-op ─────────────────────────────────────────────────

#[test]
fn test_spatial_descend_miss_is_noop() {
    do_spatial_descend_miss_is_noop();
}

pub fn do_spatial_descend_miss_is_noop() {
    let mut tree = build_test_tree();
    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Arc::new(Mutex::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| { rc.store(true, Ordering::SeqCst); },
    ));
    let right_id = tree.root.children(&tree.arena).nth(1).unwrap();
    tree.arena.get_mut(right_id).unwrap().get_mut()
        .subscribers_mut().push(sub);

    let mtree = spatial_descend_mutator(999.0, 999.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);
    assert!(!received.load(Ordering::SeqCst));
}

// ── innermost node wins ───────────────────────────────────────────

#[test]
fn test_spatial_descend_finds_innermost_node() {
    do_spatial_descend_finds_innermost_node();
}

pub fn do_spatial_descend_finds_innermost_node() {
    let mut tree = build_test_tree();
    let left_received = Arc::new(AtomicBool::new(false));
    let child_received = Arc::new(AtomicBool::new(false));

    let lr = left_received.clone();
    let left_sub: EventSubscriber = Arc::new(Mutex::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| { lr.store(true, Ordering::SeqCst); },
    ));
    let cr = child_received.clone();
    let child_sub: EventSubscriber = Arc::new(Mutex::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| { cr.store(true, Ordering::SeqCst); },
    ));

    let left_id = tree.root.children(&tree.arena).next().unwrap();
    let left_child_id = left_id.children(&tree.arena).next().unwrap();
    tree.arena.get_mut(left_id).unwrap().get_mut()
        .subscribers_mut().push(left_sub);
    tree.arena.get_mut(left_child_id).unwrap().get_mut()
        .subscribers_mut().push(child_sub);

    let mtree = spatial_descend_mutator(25.0, 42.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);

    assert!(child_received.load(Ordering::SeqCst));
    assert!(!left_received.load(Ordering::SeqCst));
}

// ── deep chain ────────────────────────────────────────────────────

#[test]
fn test_spatial_descend_deep_chain_delivers_to_leaf() {
    do_spatial_descend_deep_chain_delivers_to_leaf();
}

pub fn do_spatial_descend_deep_chain_delivers_to_leaf() {
    let mut tree = build_deep_chain();
    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Arc::new(Mutex::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| { rc.store(true, Ordering::SeqCst); },
    ));

    // Walk to deepest node.
    let mut node = tree.root;
    loop {
        match tree.arena.get(node).unwrap().first_child() {
            Some(cid) => node = cid,
            None => break,
        }
    }
    tree.arena.get_mut(node).unwrap().get_mut()
        .subscribers_mut().push(sub);

    let mtree = spatial_descend_mutator(105.0, 105.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);
    assert!(received.load(Ordering::SeqCst));
}

// ── wide tree hits correct child ──────────────────────────────────

#[test]
fn test_spatial_descend_wide_tree_hits_correct_child() {
    do_spatial_descend_wide_tree_hits_correct_child();
}

pub fn do_spatial_descend_wide_tree_hits_correct_child() {
    let mut tree = build_wide_tree();
    let hit_ids: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));

    let children: Vec<_> = tree.root.children(&tree.arena).collect();
    for (i, &cid) in children.iter().enumerate() {
        let ids = hit_ids.clone();
        let sub: EventSubscriber = Arc::new(Mutex::new(
            move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
                ids.lock().unwrap().push(i);
            },
        ));
        tree.arena.get_mut(cid).unwrap().get_mut()
            .subscribers_mut().push(sub);
    }

    let mtree = spatial_descend_mutator(760.0, 10.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);
    assert_eq!(*hit_ids.lock().unwrap(), vec![15]);
}

// ── Mutation::None payload ────────────────────────────────────────

#[test]
fn test_spatial_descend_no_mutation_is_noop() {
    do_spatial_descend_no_mutation_is_noop();
}

pub fn do_spatial_descend_no_mutation_is_noop() {
    let mut tree = build_test_tree();
    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Arc::new(Mutex::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| { rc.store(true, Ordering::SeqCst); },
    ));
    let right_id = tree.root.children(&tree.arena).nth(1).unwrap();
    tree.arena.get_mut(right_id).unwrap().get_mut()
        .subscribers_mut().push(sub);

    let instruction = GfxMutator::Instruction {
        instruction: Instruction::SpatialDescend(OrderedVec2::new_f32(210.0, 210.0)),
        channel: 0,
        mutation: Mutation::None,
    };
    let mtree = MutatorTree::new_with(instruction);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);
    assert!(!received.load(Ordering::SeqCst));
}

// ── channel override ──────────────────────────────────────────────

#[test]
fn test_spatial_descend_ignores_channel_mismatch() {
    do_spatial_descend_ignores_channel_mismatch();
}

/// SpatialDescend delivers based on position, not channel.
pub fn do_spatial_descend_ignores_channel_mismatch() {
    fonts::init();
    let mut tree = Tree::new_non_indexed();
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("ch5", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0)),
        5, // channel 5
    );

    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Arc::new(Mutex::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| { rc.store(true, Ordering::SeqCst); },
    ));

    let id = tree.arena.new_node(area);
    tree.root.append(id, &mut tree.arena);
    tree.arena.get_mut(id).unwrap().get_mut()
        .subscribers_mut().push(sub);

    // SpatialDescend channel 0 should still find channel-5 node.
    let mtree = spatial_descend_mutator(25.0, 25.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root);
    assert!(received.load(Ordering::SeqCst));
}

// =====================================================================
// MouseEventData
// =====================================================================

#[test]
fn test_mouse_event_data_new_and_fields() {
    do_mouse_event_data_new_and_fields();
}

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

pub fn do_mouse_event_data_zero() {
    let d = MouseEventData::new(0.0, 0.0);
    assert_eq!(d.x.0, 0.0_f32);
    assert_eq!(d.y.0, 0.0_f32);
}

#[test]
fn test_mouse_event_data_extreme_values() {
    do_mouse_event_data_extreme_values();
}

pub fn do_mouse_event_data_extreme_values() {
    let d = MouseEventData::new(f32::MAX, f32::MIN);
    assert_eq!(d.x.0, f32::MAX);
    assert_eq!(d.y.0, f32::MIN);
    let tiny = MouseEventData::new(f32::MIN_POSITIVE, -f32::MIN_POSITIVE);
    assert_eq!(tiny.x.0, f32::MIN_POSITIVE);
}

#[test]
fn test_glyph_tree_event_mouse_carries_data() {
    do_glyph_tree_event_mouse_carries_data();
}

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
    assert_eq!(event.clone().event_type, event.event_type);
}
