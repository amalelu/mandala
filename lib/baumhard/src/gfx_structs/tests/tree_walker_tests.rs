use glam::Vec2;
use indextree::NodeId;
use crate::font::fonts;
use crate::gfx_structs::element::{GfxElement, GfxElementField};
use crate::gfx_structs::area::{GlyphArea, GlyphAreaCommand, GlyphAreaField};
use crate::gfx_structs::tree::{BranchChannel, MutatorTree, Tree};
use crate::gfx_structs::mutator::{GfxMutator, Mutation};
use crate::gfx_structs::mutator::Instruction::RepeatWhile;
use crate::gfx_structs::predicate::{Comparator, Predicate};
use crate::gfx_structs::tree_walker::{walk_tree, walk_tree_from};
use crate::util::ordered_vec2::OrderedVec2;

// ---------------------------------------------------------------------------
// Shared test helpers
// ---------------------------------------------------------------------------
// The original repeat_while test was built with ~4 lines of boilerplate per
// node and ~2 lines per position assertion. These helpers flatten both into a
// single line each so the test's intent is readable at a glance. They are
// also the building blocks for the focused mutation tests at the bottom of
// this file.

/// Build a `GfxElement::GlyphArea` at the given canvas position with the
/// provided channel and unique id. All other fields use sensible defaults
/// (scale 1.0, line_height 10, bounds 100x100).
fn mk_area(x: f32, y: f32, channel: usize, unique_id: usize) -> GfxElement {
    GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new(1.0, 10.0, Vec2::new(x, y), Vec2::new(100.0, 100.0)),
        channel,
        unique_id,
    )
}

/// Allocate a new `GlyphArea` element in `model`, append it to `parent`, and
/// return its `NodeId`. Collapses the 4-line arena-then-append dance used in
/// the hand-written tests into one call.
fn append_area(
    model: &mut Tree<GfxElement, GfxMutator>,
    parent: NodeId,
    x: f32,
    y: f32,
    channel: usize,
    unique_id: usize,
) -> NodeId {
    let id = model.arena.new_node(mk_area(x, y, channel, unique_id));
    parent.append(id, &mut model.arena);
    id
}

/// Assert the canvas position of the node at `id`. Replaces the pair of
/// `assert_eq!(... .position().x, ...)` / `.position().y` lines that the
/// original tests sprinkled after every mutation.
fn assert_pos(
    model: &Tree<GfxElement, GfxMutator>,
    id: NodeId,
    expected_x: f32,
    expected_y: f32,
) {
    let pos = model.arena.get(id).unwrap().get().position();
    assert_eq!(
        (pos.x, pos.y),
        (expected_x, expected_y),
        "unexpected position at node {:?}",
        id
    );
}

#[test]
pub fn test_repeat_while_skip_while() {
   repeat_while_skip_while();
}

pub fn repeat_while_skip_while() {
   // This is necessary to initialize lazy statics
   fonts::init();

   // Build a 16-node "human figure" target tree. All nodes live on channel 0;
   // only position and unique_id distinguish them. The shape is:
   //     root(50,50) → head → neck → torso → { shoulders², thighs² }
   //     shoulder → upper_arm → lower_arm (mirrored left/right)
   //     thigh   → knee     → lower_leg  (mirrored left/right)
   let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
   let mut model: Tree<GfxElement, GfxMutator> =
      Tree::new_non_indexed_with(mk_area(50.0, 50.0, 0, 0));
   let root = model.root;

   let head = append_area(&mut model, root, 100.0, 100.0, 0, 1);
   let neck = append_area(&mut model, head, 100.0, 110.0, 0, 2);
   let torso = append_area(&mut model, neck, 100.0, 120.0, 0, 3);

   // Arms (left first, matching the original test's sibling order).
   let l_shoulder = append_area(&mut model, torso, 90.0, 120.0, 0, 4);
   let r_shoulder = append_area(&mut model, torso, 110.0, 120.0, 0, 5);
   let l_upper_arm = append_area(&mut model, l_shoulder, 90.0, 130.0, 0, 6);
   let r_upper_arm = append_area(&mut model, r_shoulder, 110.0, 130.0, 0, 7);
   let l_lower_arm = append_area(&mut model, l_upper_arm, 90.0, 140.0, 0, 8);
   let r_lower_arm = append_area(&mut model, r_upper_arm, 110.0, 140.0, 0, 9);

   // Legs.
   let l_thigh = append_area(&mut model, torso, 95.0, 130.0, 0, 10);
   let r_thigh = append_area(&mut model, torso, 105.0, 130.0, 0, 11);
   let l_knee = append_area(&mut model, l_thigh, 95.0, 140.0, 0, 12);
   let r_knee = append_area(&mut model, r_thigh, 105.0, 140.0, 0, 13);
   let l_lower_leg = append_area(&mut model, l_knee, 95.0, 150.0, 0, 14);
   let r_lower_leg = append_area(&mut model, r_knee, 105.0, 150.0, 0, 15);

   // Build the mutator:
   //   instruction(RepeatWhile(pos != (105,150)))
   //     └─ void(ch 0)  ← the per-node mutator (applies nothing while walking)
   //         └─ single(NudgeDown 10)  ← the "after" mutator, fired by the
   //                                    DEFAULT_TERMINATOR on the node whose
   //                                    position equals (105, 150)
   //
   // So the walk visits every descendant applying nothing, until it reaches
   // r_lower_leg — the one node whose position matches (105, 150) and so
   // fails the predicate. The terminator then applies NudgeDown(10) to
   // r_lower_leg, moving it to (105, 160). Every other node stays put.
   let mut predicate = Predicate::new();
   predicate.fields.push((
      GfxElementField::GlyphArea(GlyphAreaField::Position(OrderedVec2::new_f32(105.0, 150.0))),
      Comparator::not_equals(),
   ));
   let instruction_node_id = mutator.arena.new_node(GfxMutator::Instruction {
      instruction: RepeatWhile(predicate),
      channel: 0,
      mutation: Mutation::None,
   });
   let void_node = mutator.arena.new_node(GfxMutator::new_void(0));
   let applicable_node = mutator.arena.new_node(GfxMutator::Single {
      mutation: Mutation::AreaCommand(Box::new(GlyphAreaCommand::NudgeDown(10.0))),
      channel: 0,
   });
   mutator.root.append(instruction_node_id, &mut mutator.arena);
   instruction_node_id.append(void_node, &mut mutator.arena);
   void_node.append(applicable_node, &mut mutator.arena);

   walk_tree(&mut model, &mutator);

   // The predicate-terminated node got the NudgeDown; everything else is
   // identical to its construction position.
   assert_pos(&model, r_lower_leg, 105.0, 160.0);
   assert_pos(&model, l_lower_leg, 95.0, 150.0);
   assert_pos(&model, l_knee, 95.0, 140.0);
   assert_pos(&model, l_thigh, 95.0, 130.0);
   assert_pos(&model, r_knee, 105.0, 140.0);
   assert_pos(&model, r_thigh, 105.0, 130.0);
   assert_pos(&model, l_lower_arm, 90.0, 140.0);
   assert_pos(&model, l_upper_arm, 90.0, 130.0);
   assert_pos(&model, l_shoulder, 90.0, 120.0);
   assert_pos(&model, r_lower_arm, 110.0, 140.0);
   assert_pos(&model, r_upper_arm, 110.0, 130.0);
   assert_pos(&model, r_shoulder, 110.0, 120.0);
   assert_pos(&model, torso, 100.0, 120.0);
   assert_pos(&model, neck, 100.0, 110.0);
   assert_pos(&model, head, 100.0, 100.0);
}

// ===========================================================================
// Focused tree-mutation regression tests
// ===========================================================================
//
// The big test above is the original bespoke happy-path coverage of the
// walker. The tests below target specific gaps a mutation-path audit turned
// up — each one guards an invariant the existing hand-written tests did not
// touch:
//
//   * `GfxMutator::Macro` had ZERO coverage prior to these tests
//   * `Mutation::None` had no "applied to a live element" guard
//   * Channel-mismatch no-op path was untested in both
//     `walk_tree_from`'s direct `apply_if_matching_channel` else branch
//     AND `align_child_walks`' skip/break sibling-scan paths
//   * Walker recursion depth was only exercised on the 16-node figure; a
//     deep chain stresses the `walk_tree_from → align_child_walks →
//     walk_tree_from` unrolling so a future refactor that inflated
//     per-frame stack usage would trip a test
//   * Wide fan-out at a single parent was only tested with 4 children
//   * `GfxElement::clone`'s hand-rolled impl had no assertion that it
//     preserves `unique_id` / `channel`
//
// Each test is self-contained and uses the `mk_area` / `append_area` /
// `assert_pos` helpers at the top of the file to keep node construction
// and assertions to one line each.

#[test]
pub fn test_macro_applies_all_mutations_in_order() {
    macro_applies_all_mutations_in_order();
}

pub fn macro_applies_all_mutations_in_order() {
    fonts::init();
    // One Macro mutator, two mutations inside, one target. Both mutations
    // must reach the target — this is the observable contract of Macro.
    let mut model: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(0.0, 0.0, 0, 0));
    let root = model.root;
    let target = append_area(&mut model, root, 100.0, 100.0, 0, 1);

    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let m = mutator.arena.new_node(GfxMutator::new_macro(
        vec![
            Mutation::AreaCommand(Box::new(GlyphAreaCommand::NudgeDown(10.0))),
            Mutation::AreaCommand(Box::new(GlyphAreaCommand::NudgeRight(5.0))),
        ],
        0,
    ));
    mutator.root.append(m, &mut mutator.arena);

    walk_tree(&mut model, &mutator);

    // Both nudges applied: (100 + 5, 100 + 10).
    assert_pos(&model, target, 105.0, 110.0);
}

#[test]
pub fn test_macro_with_empty_mutations_is_noop() {
    macro_with_empty_mutations_is_noop();
}

pub fn macro_with_empty_mutations_is_noop() {
    fonts::init();
    // A Macro wrapping an empty Vec must not corrupt the target. Guards the
    // edge case of a caller that built up a Vec<Mutation> and flushed it
    // before any mutation was pushed.
    let mut model: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(0.0, 0.0, 0, 0));
    let root = model.root;
    let target = append_area(&mut model, root, 100.0, 100.0, 0, 1);

    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let m = mutator.arena.new_node(GfxMutator::new_macro(vec![], 0));
    mutator.root.append(m, &mut mutator.arena);

    walk_tree(&mut model, &mutator);

    assert_pos(&model, target, 100.0, 100.0);
}

#[test]
pub fn test_mutation_none_is_noop() {
    mutation_none_is_noop();
}

pub fn mutation_none_is_noop() {
    fonts::init();
    // `Mutation::None` is the explicit "do nothing" mutation. Both
    // `apply_to_area` and `apply_to_model` must match it to their no-op
    // arm, otherwise tree-level mutators carrying None would panic the
    // walker at `Event(_) => panic!()`.
    let mut model: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(0.0, 0.0, 0, 0));
    let root = model.root;
    let target = append_area(&mut model, root, 42.0, 42.0, 0, 1);

    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let m = mutator.arena.new_node(GfxMutator::new(Mutation::None, 0));
    mutator.root.append(m, &mut mutator.arena);

    walk_tree(&mut model, &mutator);

    assert_pos(&model, target, 42.0, 42.0);
}

#[test]
pub fn test_single_mutator_channel_filter_in_align_child_walks() {
    single_mutator_channel_filter_in_align_child_walks();
}

pub fn single_mutator_channel_filter_in_align_child_walks() {
    fonts::init();
    // Three sibling targets on channels 0, 1, 2. A single mutator child on
    // channel 1 should match only the middle target. This exercises BOTH
    // under-tested branches of the align_child_walks inner loop:
    //   * c0 (t_chan < m_chan): "skip this target, advance"
    //   * c2 (t_chan > m_chan): "break inner loop"
    let mut model: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(0.0, 0.0, 0, 0));
    let root = model.root;
    let c0 = append_area(&mut model, root, 100.0, 100.0, 0, 1);
    let c1 = append_area(&mut model, root, 200.0, 200.0, 1, 2);
    let c2 = append_area(&mut model, root, 300.0, 300.0, 2, 3);

    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let m = mutator.arena.new_node(GfxMutator::new(
        Mutation::AreaCommand(Box::new(GlyphAreaCommand::NudgeDown(50.0))),
        1, // only c1 is on this channel
    ));
    mutator.root.append(m, &mut mutator.arena);

    walk_tree(&mut model, &mutator);

    assert_pos(&model, c0, 100.0, 100.0); // lower-channel skip path
    assert_pos(&model, c1, 200.0, 250.0); // matched → applied
    assert_pos(&model, c2, 300.0, 300.0); // higher-channel break path
}

#[test]
pub fn test_direct_walk_at_mismatched_channels_is_noop() {
    direct_walk_at_mismatched_channels_is_noop();
}

pub fn direct_walk_at_mismatched_channels_is_noop() {
    fonts::init();
    // Hits the `apply_if_matching_channel` else branch inside
    // `walk_tree_from` itself, which `align_child_walks` can never reach
    // (it only dispatches walk_tree_from for matching channel pairs).
    // We trigger it by starting walk_tree on a non-void Single root
    // mutator whose channel doesn't match the target root.
    let mut model: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(50.0, 50.0, 0, 0));
    let root = model.root;

    let mutator: MutatorTree<GfxMutator> = MutatorTree::new_with(
        GfxMutator::new(
            Mutation::AreaCommand(Box::new(GlyphAreaCommand::NudgeDown(99.0))),
            5, // mismatched with root (ch 0)
        ),
    );
    walk_tree(&mut model, &mutator);

    assert_pos(&model, root, 50.0, 50.0);
}

#[test]
pub fn test_deep_chain_walk_reaches_every_node() {
    deep_chain_walk_reaches_every_node();
}

pub fn deep_chain_walk_reaches_every_node() {
    fonts::init();
    // Recursion-depth guard. Each walk level consumes two stack frames
    // (walk_tree_from + align_child_walks), so a DEPTH-node chain drives
    // ~2*DEPTH frames. A refactor that bloated per-frame stack usage
    // would trip this at `cargo test` time rather than in a user's
    // real-world mindmap load.
    const DEPTH: usize = 300;

    let mut model: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(0.0, 0.0, 0, 0));
    let mut parent = model.root;
    let mut target_chain: Vec<NodeId> = Vec::with_capacity(DEPTH);
    for i in 0..DEPTH {
        let id = append_area(&mut model, parent, 0.0, 0.0, 0, i + 1);
        target_chain.push(id);
        parent = id;
    }

    // Mirror mutator chain: root (void) → m1 → m2 → ... → m{DEPTH},
    // each nudging its paired target down by 1.
    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let mut mparent = mutator.root;
    for _ in 0..DEPTH {
        let id = mutator.arena.new_node(GfxMutator::new(
            Mutation::AreaCommand(Box::new(GlyphAreaCommand::NudgeDown(1.0))),
            0,
        ));
        mparent.append(id, &mut mutator.arena);
        mparent = id;
    }

    walk_tree(&mut model, &mutator);

    for &id in &target_chain {
        assert_pos(&model, id, 0.0, 1.0);
    }
}

#[test]
pub fn test_wide_fan_out_applies_to_all_matching_siblings() {
    wide_fan_out_applies_to_all_matching_siblings();
}

pub fn wide_fan_out_applies_to_all_matching_siblings() {
    fonts::init();
    // A root with WIDTH sibling children, all on channel 0, mutated by a
    // single matching mutator child. The align_child_walks inner loop
    // must iterate all WIDTH siblings and apply the mutator to each —
    // this is the star-topology hot path in the mindmap data model.
    const WIDTH: usize = 200;

    let mut model: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(0.0, 0.0, 0, 0));
    let root = model.root;
    let mut kids: Vec<NodeId> = Vec::with_capacity(WIDTH);
    for i in 0..WIDTH {
        kids.push(append_area(&mut model, root, i as f32, 0.0, 0, i + 1));
    }

    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let m = mutator.arena.new_node(GfxMutator::new(
        Mutation::AreaCommand(Box::new(GlyphAreaCommand::NudgeDown(10.0))),
        0,
    ));
    mutator.root.append(m, &mut mutator.arena);

    walk_tree(&mut model, &mutator);

    for (i, &id) in kids.iter().enumerate() {
        assert_pos(&model, id, i as f32, 10.0);
    }
}

#[test]
pub fn test_applying_same_delta_twice_accumulates() {
    applying_same_delta_twice_accumulates();
}

pub fn applying_same_delta_twice_accumulates() {
    fonts::init();
    // Delta-style mutations are NOT idempotent — they compose additively.
    // Pinning this down protects against a future "dedupe identical
    // mutations" optimisation that would silently change semantics of
    // repeated nudges (which the drag path emits on purpose every frame).
    let mut model: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(0.0, 0.0, 0, 0));
    let root = model.root;
    let target = append_area(&mut model, root, 0.0, 0.0, 0, 1);

    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let m = mutator.arena.new_node(GfxMutator::new(
        Mutation::AreaCommand(Box::new(GlyphAreaCommand::NudgeDown(5.0))),
        0,
    ));
    mutator.root.append(m, &mut mutator.arena);

    walk_tree(&mut model, &mutator);
    assert_pos(&model, target, 0.0, 5.0);
    walk_tree(&mut model, &mutator);
    assert_pos(&model, target, 0.0, 10.0);
}

#[test]
pub fn test_mutation_is_deterministic_across_tree_clones() {
    mutation_is_deterministic_across_tree_clones();
}

pub fn mutation_is_deterministic_across_tree_clones() {
    fonts::init();
    // Apply the same mutator to two independent clones of the same tree.
    // Both copies must end up with byte-identical positions — the mutator
    // must be a pure function of (tree, mutator) with no hidden globals,
    // thread-locals, or iteration-order nondeterminism. Guards against a
    // regression that (for example) started walking siblings in arena
    // order instead of linked-list order.
    let mut base: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(0.0, 0.0, 0, 0));
    let base_root = base.root;
    let a = append_area(&mut base, base_root, 1.0, 2.0, 0, 1);
    let b = append_area(&mut base, a, 3.0, 4.0, 0, 2);
    let c = append_area(&mut base, b, 5.0, 6.0, 0, 3);

    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let mut mparent = mutator.root;
    for _ in 0..3 {
        let id = mutator.arena.new_node(GfxMutator::new(
            Mutation::AreaCommand(Box::new(GlyphAreaCommand::NudgeDown(7.0))),
            0,
        ));
        mparent.append(id, &mut mutator.arena);
        mparent = id;
    }

    let mut copy1 = base.clone();
    let mut copy2 = base.clone();
    walk_tree(&mut copy1, &mutator);
    walk_tree(&mut copy2, &mutator);

    for &id in &[a, b, c] {
        assert_eq!(
            copy1.arena.get(id).unwrap().get().position(),
            copy2.arena.get(id).unwrap().get().position(),
            "mutation was not deterministic across clones at {:?}",
            id,
        );
    }
}

#[test]
pub fn test_clone_preserves_unique_id_and_channel() {
    clone_preserves_unique_id_and_channel();
}

pub fn clone_preserves_unique_id_and_channel() {
    fonts::init();
    // `GfxElement` has a hand-rolled Clone impl that routes through the
    // `_with_id` constructors. A refactor that swapped it for a derived
    // Clone (or for a non-`_with_id` constructor) would silently reset
    // unique_id to 0, breaking every consumer that addresses elements by
    // id after a subtree clone (undo stack, arena_utils::clone_subtree).
    let original = mk_area(10.0, 20.0, 7, 42);
    let cloned = original.clone();

    assert_eq!(cloned.unique_id(), 42);
    assert_eq!(cloned.channel(), 7);
    assert_eq!(cloned.position(), Vec2::new(10.0, 20.0));
}
