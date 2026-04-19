//! Tests for [`crate::gfx_structs::mutator::Instruction::MapChildren`]
//! — the zip-by-sibling-position walker primitive that stands
//! alongside the default channel-based aligner.
//!
//! Coverage: one-to-one zip with channel-mismatch, unequal-count
//! handling on both sides, empty-children edge cases, nested
//! composition, the Instruction-own-mutation precedent, and the
//! compose-with-`Repeat` end-to-end shape used by size-aware layout
//! consumers.
//!
//! Follows the `do_*()` / `test_*()` benchmark-reuse split (§T2.2) so
//! future criterion benches can reuse the bodies.

use glam::Vec2;
use indextree::NodeId;

use crate::font::fonts;
use crate::gfx_structs::area::{GlyphArea, GlyphAreaCommand};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutator, Instruction, Mutation};
use crate::gfx_structs::tree::{MutatorTree, Tree};
use crate::gfx_structs::tree_walker::walk_tree_from;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn mk_area(x: f32, y: f32, channel: usize, unique_id: usize) -> GfxElement {
    GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new(1.0, 10.0, Vec2::new(x, y), Vec2::new(100.0, 100.0)),
        channel,
        unique_id,
    )
}

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

/// Build a target tree: root at (0, 0) on channel 0, with `n` children
/// at (100, i*10), channels from `channels`. Returns (tree, child ids
/// in declaration order) for assertion convenience.
fn build_target_with_children(
    channels: &[usize],
) -> (Tree<GfxElement, GfxMutator>, Vec<NodeId>) {
    fonts::init();
    let mut model: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(0.0, 0.0, 0, 0));
    let root = model.root;
    let mut child_ids = Vec::with_capacity(channels.len());
    for (i, ch) in channels.iter().enumerate() {
        let id = append_area(&mut model, root, 100.0, (i as f32) * 10.0, *ch, i + 1);
        child_ids.push(id);
    }
    (model, child_ids)
}

/// Build a mutator tree: root is an `Instruction(MapChildren)` with
/// the given `attached` mutation, and with N `Single` children each
/// carrying `nudges[i]` as its mutation on channel `mutator_channels[i]`.
fn build_map_children_mutator(
    attached: Mutation,
    root_channel: usize,
    mutator_channels: &[usize],
    nudges: &[f32],
) -> MutatorTree<GfxMutator> {
    assert_eq!(mutator_channels.len(), nudges.len());
    let mut mutator: MutatorTree<GfxMutator> =
        MutatorTree::new_with(GfxMutator::Instruction {
            instruction: Instruction::MapChildren,
            channel: root_channel,
            mutation: attached,
        });
    for (ch, dx) in mutator_channels.iter().zip(nudges.iter()) {
        let m = Mutation::area_command(GlyphAreaCommand::NudgeRight(*dx));
        let child = mutator.arena.new_node(GfxMutator::new(m, *ch));
        mutator.root.append(child, &mut mutator.arena);
    }
    mutator
}

fn position_x(tree: &Tree<GfxElement, GfxMutator>, id: NodeId) -> f32 {
    tree.arena.get(id).unwrap().get().position().x
}

// ---------------------------------------------------------------------------
// 1. One-to-one zip ignores channel
// ---------------------------------------------------------------------------

#[test]
fn test_one_to_one_zip_applies_each_child_by_position() {
    do_one_to_one_zip_applies_each_child_by_position();
}

pub fn do_one_to_one_zip_applies_each_child_by_position() {
    // 3 targets on channels 0/1/2; 3 mutators ALL on channel 9 (a
    // value that matches none of the targets). Channel-based
    // alignment would apply none; MapChildren must apply each by
    // sibling position.
    let (mut tree, child_ids) = build_target_with_children(&[0, 1, 2]);
    let mutator = build_map_children_mutator(
        Mutation::None,
        0,
        &[9, 9, 9],
        &[10.0, 20.0, 30.0],
    );
    let root = tree.root;
    walk_tree_from(&mut tree, &mutator, root, mutator.root);

    assert_eq!(position_x(&tree, child_ids[0]), 110.0, "child 0 got dx=10");
    assert_eq!(position_x(&tree, child_ids[1]), 120.0, "child 1 got dx=20");
    assert_eq!(position_x(&tree, child_ids[2]), 130.0, "child 2 got dx=30");
}

// ---------------------------------------------------------------------------
// 2. Unequal counts — mutator shorter
// ---------------------------------------------------------------------------

#[test]
fn test_zip_shorter_mutator_than_target() {
    do_zip_shorter_mutator_than_target();
}

pub fn do_zip_shorter_mutator_than_target() {
    let (mut tree, child_ids) = build_target_with_children(&[0, 0, 0, 0]);
    // Only 2 mutators — indices 0 and 1 should move, 2 and 3 untouched.
    let mutator = build_map_children_mutator(
        Mutation::None,
        0,
        &[0, 0],
        &[5.0, 7.0],
    );
    let root = tree.root;
    walk_tree_from(&mut tree, &mutator, root, mutator.root);

    assert_eq!(position_x(&tree, child_ids[0]), 105.0);
    assert_eq!(position_x(&tree, child_ids[1]), 107.0);
    assert_eq!(position_x(&tree, child_ids[2]), 100.0, "index 2 untouched");
    assert_eq!(position_x(&tree, child_ids[3]), 100.0, "index 3 untouched");
}

// ---------------------------------------------------------------------------
// 3. Unequal counts — target shorter
// ---------------------------------------------------------------------------

#[test]
fn test_zip_shorter_target_than_mutator() {
    do_zip_shorter_target_than_mutator();
}

pub fn do_zip_shorter_target_than_mutator() {
    let (mut tree, child_ids) = build_target_with_children(&[0, 0]);
    // 4 mutators, 2 targets — only the first two apply; the remaining
    // two are silently dropped with a debug log.
    let mutator = build_map_children_mutator(
        Mutation::None,
        0,
        &[0, 0, 0, 0],
        &[1.0, 2.0, 3.0, 4.0],
    );
    let root = tree.root;
    walk_tree_from(&mut tree, &mutator, root, mutator.root);

    assert_eq!(position_x(&tree, child_ids[0]), 101.0);
    assert_eq!(position_x(&tree, child_ids[1]), 102.0);
}

// ---------------------------------------------------------------------------
// 4–6. Empty-children edges
// ---------------------------------------------------------------------------

#[test]
fn test_zip_empty_mutator_children_is_noop() {
    do_zip_empty_mutator_children_is_noop();
}

pub fn do_zip_empty_mutator_children_is_noop() {
    let (mut tree, child_ids) = build_target_with_children(&[0, 0]);
    let mutator = build_map_children_mutator(Mutation::None, 0, &[], &[]);
    let root = tree.root;
    walk_tree_from(&mut tree, &mutator, root, mutator.root);
    assert_eq!(position_x(&tree, child_ids[0]), 100.0);
    assert_eq!(position_x(&tree, child_ids[1]), 100.0);
}

#[test]
fn test_zip_empty_target_children_is_noop() {
    do_zip_empty_target_children_is_noop();
}

pub fn do_zip_empty_target_children_is_noop() {
    // Target is a leaf — no children to zip against.
    let (mut tree, _) = build_target_with_children(&[]);
    let mutator = build_map_children_mutator(
        Mutation::None,
        0,
        &[0, 0],
        &[1.0, 2.0],
    );
    let root = tree.root;
    walk_tree_from(&mut tree, &mutator, root, mutator.root);
    // No panic; root untouched.
    assert_eq!(position_x(&tree, root), 0.0);
}

#[test]
fn test_zip_empty_both_sides_is_noop() {
    do_zip_empty_both_sides_is_noop();
}

pub fn do_zip_empty_both_sides_is_noop() {
    let (mut tree, _) = build_target_with_children(&[]);
    let mutator = build_map_children_mutator(Mutation::None, 0, &[], &[]);
    let root = tree.root;
    walk_tree_from(&mut tree, &mutator, root, mutator.root);
    assert_eq!(position_x(&tree, root), 0.0);
}

// ---------------------------------------------------------------------------
// 7. Nested recursion
// ---------------------------------------------------------------------------

#[test]
fn test_nested_map_children_descends_recursively() {
    do_nested_map_children_descends_recursively();
}

pub fn do_nested_map_children_descends_recursively() {
    fonts::init();
    // Target:
    //   root
    //   ├─ a
    //   │  ├─ a0
    //   │  └─ a1
    //   └─ b
    //      └─ b0
    let mut tree: Tree<GfxElement, GfxMutator> =
        Tree::new_non_indexed_with(mk_area(0.0, 0.0, 0, 0));
    let root = tree.root;
    let a = append_area(&mut tree, root, 0.0, 0.0, 0, 1);
    let a0 = append_area(&mut tree, a, 0.0, 0.0, 0, 2);
    let a1 = append_area(&mut tree, a, 0.0, 0.0, 0, 3);
    let b = append_area(&mut tree, root, 0.0, 0.0, 0, 4);
    let b0 = append_area(&mut tree, b, 0.0, 0.0, 0, 5);

    // Mutator: outer MapChildren with two Instruction(MapChildren)
    // children, each carrying a Single leaf mutator per grandchild.
    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::Instruction {
        instruction: Instruction::MapChildren,
        channel: 0,
        mutation: Mutation::None,
    });

    // Build a MapChildren child with two leaf Singles nudging x by 1 and 2.
    let inner_a = mutator.arena.new_node(GfxMutator::Instruction {
        instruction: Instruction::MapChildren,
        channel: 0,
        mutation: Mutation::None,
    });
    mutator.root.append(inner_a, &mut mutator.arena);
    let a0_leaf = mutator.arena.new_node(GfxMutator::new(
        Mutation::area_command(GlyphAreaCommand::NudgeRight(1.0)),
        0,
    ));
    inner_a.append(a0_leaf, &mut mutator.arena);
    let a1_leaf = mutator.arena.new_node(GfxMutator::new(
        Mutation::area_command(GlyphAreaCommand::NudgeRight(2.0)),
        0,
    ));
    inner_a.append(a1_leaf, &mut mutator.arena);

    // Build a second MapChildren child with one leaf Single nudging by 7.
    let inner_b = mutator.arena.new_node(GfxMutator::Instruction {
        instruction: Instruction::MapChildren,
        channel: 0,
        mutation: Mutation::None,
    });
    mutator.root.append(inner_b, &mut mutator.arena);
    let b0_leaf = mutator.arena.new_node(GfxMutator::new(
        Mutation::area_command(GlyphAreaCommand::NudgeRight(7.0)),
        0,
    ));
    inner_b.append(b0_leaf, &mut mutator.arena);

    walk_tree_from(&mut tree, &mutator, root, mutator.root);

    assert_eq!(position_x(&tree, a0), 1.0);
    assert_eq!(position_x(&tree, a1), 2.0);
    assert_eq!(position_x(&tree, b0), 7.0);
    // Intermediate nodes untouched (MapChildren doesn't apply to the
    // current target — only paired children).
    assert_eq!(position_x(&tree, a), 0.0);
    assert_eq!(position_x(&tree, b), 0.0);
}

// ---------------------------------------------------------------------------
// 8. Instruction's own mutation applies on matching channel
// ---------------------------------------------------------------------------

#[test]
fn test_instruction_carrying_mutation_applies_to_current_target() {
    do_instruction_carrying_mutation_applies_to_current_target();
}

pub fn do_instruction_carrying_mutation_applies_to_current_target() {
    let (mut tree, child_ids) = build_target_with_children(&[0]);
    // Instruction node on channel 0 (matches root's channel 0) with an
    // attached NudgeRight on the root + one MapChildren child that
    // moves child 0.
    let mutator = build_map_children_mutator(
        Mutation::area_command(GlyphAreaCommand::NudgeRight(50.0)),
        0,
        &[0],
        &[3.0],
    );
    let root = tree.root;
    walk_tree_from(&mut tree, &mutator, root, mutator.root);

    assert_eq!(position_x(&tree, root), 50.0, "attached mutation hit root");
    assert_eq!(position_x(&tree, child_ids[0]), 103.0, "zipped child moved too");
}

// ---------------------------------------------------------------------------
// 9. Instruction's own mutation skipped on channel mismatch
// ---------------------------------------------------------------------------

#[test]
fn test_instruction_mutation_skipped_on_channel_mismatch() {
    do_instruction_mutation_skipped_on_channel_mismatch();
}

pub fn do_instruction_mutation_skipped_on_channel_mismatch() {
    let (mut tree, child_ids) = build_target_with_children(&[0]);
    // Instruction root channel is 99 — doesn't match target root
    // (channel 0), so the attached mutation is skipped. The zip
    // continues into paired children regardless.
    let mutator = build_map_children_mutator(
        Mutation::area_command(GlyphAreaCommand::NudgeRight(50.0)),
        99,
        &[0],
        &[3.0],
    );
    let root = tree.root;
    walk_tree_from(&mut tree, &mutator, root, mutator.root);

    assert_eq!(position_x(&tree, root), 0.0, "attached mutation NOT applied");
    assert_eq!(position_x(&tree, child_ids[0]), 103.0, "zip still ran");
}

// ---------------------------------------------------------------------------
// 10. Compose with Repeat at build time
// ---------------------------------------------------------------------------

#[test]
fn test_compose_repeat_inside_map_children() {
    do_compose_repeat_inside_map_children();
}

pub fn do_compose_repeat_inside_map_children() {
    use crate::core::primitives::ApplyOperation;
    use crate::gfx_structs::area::GlyphAreaField;
    use crate::mutator_builder::{
        build, CellField, ChannelSrc, CountSrc, InstructionSpec, MutationSrc,
        MutatorNode, SectionContext,
    };

    // SectionContext that supplies per-index position values — the
    // `flower-layout` / `tree-cascade` pattern. Three children at
    // computed positions (10, 20, 30).
    struct PerIndex;
    impl SectionContext for PerIndex {
        fn count(&self, name: &str) -> usize {
            assert_eq!(name, "children");
            3
        }
        fn field(
            &self,
            section: &str,
            index: usize,
            template: &CellField,
        ) -> GlyphAreaField {
            assert_eq!(section, "children");
            match template {
                CellField::position => {
                    GlyphAreaField::position((10 * (index + 1)) as f32, 0.0)
                }
                CellField::Operation(op) => GlyphAreaField::Operation(*op),
                other => panic!("unexpected cell field in test: {:?}", other),
            }
        }
    }

    // MutatorNode: Instruction(MapChildren) wrapping a Repeat whose
    // template is a Single with an AreaDelta(position) — the exact
    // shape flower-layout would author declaratively.
    let node = MutatorNode::Instruction {
        channel: 0,
        instruction: InstructionSpec::MapChildren,
        mutation: MutationSrc::None,
        children: vec![MutatorNode::Repeat {
            section: "children".into(),
            channel_base: 0,
            count: CountSrc::Runtime("children".into()),
            skip_indices: vec![],
            template: Box::new(MutatorNode::Single {
                channel: ChannelSrc::SectionIndex,
                mutation: MutationSrc::AreaDelta(vec![
                    CellField::position,
                    CellField::Operation(ApplyOperation::Assign),
                ]),
            }),
        }],
    };

    let mutator = build(&node, &PerIndex);

    // Target: three children all on channel 0 (broadcast group). In
    // the channel-aligner world, only one mutator child would apply
    // to all; MapChildren pairs them by position.
    let (mut tree, child_ids) = build_target_with_children(&[0, 0, 0]);
    let root = tree.root;
    walk_tree_from(&mut tree, &mutator, root, mutator.root);

    assert_eq!(position_x(&tree, child_ids[0]), 10.0);
    assert_eq!(position_x(&tree, child_ids[1]), 20.0);
    assert_eq!(position_x(&tree, child_ids[2]), 30.0);
}

// ---------------------------------------------------------------------------
// 11. Channel gaps ignored
// ---------------------------------------------------------------------------

#[test]
fn test_map_children_ignores_sibling_channels_when_unequal_counts() {
    do_map_children_ignores_sibling_channels_when_unequal_counts();
}

pub fn do_map_children_ignores_sibling_channels_when_unequal_counts() {
    // Target children on channels [0, 5] — a gap that channel
    // alignment would skip over. Mutator children on [9, 9] — a
    // channel that matches neither. MapChildren pairs strictly by
    // position so both pairs apply.
    let (mut tree, child_ids) = build_target_with_children(&[0, 5]);
    let mutator = build_map_children_mutator(
        Mutation::None,
        0,
        &[9, 9],
        &[4.0, 8.0],
    );
    let root = tree.root;
    walk_tree_from(&mut tree, &mutator, root, mutator.root);

    assert_eq!(position_x(&tree, child_ids[0]), 104.0);
    assert_eq!(position_x(&tree, child_ids[1]), 108.0);
}
