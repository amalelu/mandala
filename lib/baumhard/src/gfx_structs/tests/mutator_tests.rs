//! Tests for [`crate::gfx_structs::mutator`] — mutation fundamentals
//! (§T1).
//!
//! Covers `Mutation::AreaDelta`, `Mutation::AreaCommand`,
//! `Mutation::None`, `Instruction::RepeatWhile`,
//! `Instruction::RotateWhile`, and `MutatorTree::apply_to` forward-
//! apply behaviour.
//!
//! Follows the `do_*()` / `test_*()` split from §T2.2: every public
//! body is benchmarkable from `benches/test_bench.rs`.

use glam::Vec2;

use crate::core::primitives::{Applicable, ApplyOperation};
use crate::font::fonts;
use crate::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaCommand, GlyphAreaField};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutator, Instruction, Mutation};
use crate::gfx_structs::predicate::Predicate;
use crate::gfx_structs::tree::{MutatorTree, Tree};
use crate::util::geometry::almost_equal;

// ── Mutation::AreaDelta ────────────────────────────────────────────

#[test]
fn test_mutation_area_delta_applies_field() {
    do_mutation_area_delta_applies_field();
}

/// Construct a `GlyphArea` element, apply a `Mutation::AreaDelta`
/// that adds to its position, and verify the position changed.
pub fn do_mutation_area_delta_applies_field() {
    fonts::init();

    let mut element = GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "hello",
            1.0,
            10.0,
            Vec2::new(10.0, 20.0),
            Vec2::new(100.0, 50.0),
        ),
        0,
        0,
    );

    // Position starts at (10, 20).
    assert!(almost_equal(element.position().x, 10.0));
    assert!(almost_equal(element.position().y, 20.0));

    // Build an AreaDelta that adds (5, 7) to position.
    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Operation(ApplyOperation::Add),
        GlyphAreaField::position(5.0, 7.0),
    ]);
    let mutation = Mutation::area_delta(delta);
    mutation.apply_to(&mut element);

    // Position should now be (15, 27).
    assert!(
        almost_equal(element.position().x, 15.0),
        "Expected x=15.0, got {}",
        element.position().x
    );
    assert!(
        almost_equal(element.position().y, 27.0),
        "Expected y=27.0, got {}",
        element.position().y
    );
}

// ── Mutation::AreaCommand (NudgeRight) ─────────────────────────────

#[test]
fn test_mutation_area_command_nudge_right() {
    do_mutation_area_command_nudge_right();
}

/// Apply a `NudgeRight` command and verify the x position shifted
/// by the expected pixel delta.
pub fn do_mutation_area_command_nudge_right() {
    fonts::init();

    let mut element = GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "nudge",
            1.0,
            10.0,
            Vec2::new(50.0, 50.0),
            Vec2::new(100.0, 50.0),
        ),
        0,
        0,
    );

    let original_x = element.position().x;
    let original_y = element.position().y;

    let mutation = Mutation::area_command(GlyphAreaCommand::NudgeRight(12.5));
    mutation.apply_to(&mut element);

    assert!(
        almost_equal(element.position().x, original_x + 12.5),
        "Expected x={}, got {}",
        original_x + 12.5,
        element.position().x
    );
    // Y should be unchanged.
    assert!(
        almost_equal(element.position().y, original_y),
        "Y should be unchanged: expected {}, got {}",
        original_y,
        element.position().y
    );
}

// ── Mutation::None (noop) ──────────────────────────────────────────

#[test]
fn test_mutation_noop_leaves_tree_unchanged() {
    do_mutation_noop_leaves_tree_unchanged();
}

/// Apply `Mutation::None` to an element and verify nothing changed.
pub fn do_mutation_noop_leaves_tree_unchanged() {
    fonts::init();

    let mut element = GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "stable",
            2.0,
            12.0,
            Vec2::new(30.0, 40.0),
            Vec2::new(200.0, 100.0),
        ),
        0,
        0,
    );

    let pos_before = element.position();
    let text_before = element.glyph_area().unwrap().text.clone();

    let mutation = Mutation::none();
    assert!(mutation.is_none());
    assert!(!mutation.is_some());
    mutation.apply_to(&mut element);

    assert_eq!(element.position(), pos_before);
    assert_eq!(element.glyph_area().unwrap().text, text_before);
}

// ── Instruction::RepeatWhile with always_true ──────────────────────

#[test]
fn test_instruction_repeat_while_always_true() {
    do_instruction_repeat_while_always_true();
}

/// Build a target tree with a root and two children, then apply a
/// `RepeatWhile(always_true)` instruction. The instruction node's
/// child mutator carries a position delta; `repeat_while` applies
/// that child mutator to each target descendant that satisfies the
/// predicate, recursing depth-first.
pub fn do_instruction_repeat_while_always_true() {
    fonts::init();

    // -- target tree: root (Void, ch 0) -> child_a (ch 0) -> child_b (ch 0)
    let mut model: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let child_a_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str("a", 1.0, 10.0, Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0)),
        0,
        1,
    ));
    let child_b_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str("b", 1.0, 10.0, Vec2::new(100.0, 100.0), Vec2::new(50.0, 50.0)),
        0,
        2,
    ));
    child_a_id.append(child_b_id, &mut model.arena);
    model.root.append(child_a_id, &mut model.arena);

    // -- mutator tree: root (Instruction RepeatWhile always_true, ch 0)
    //                    -> child mutator (Single: position += (5, 7), ch 0)
    // The walker's `repeat_while` tests the predicate on each target
    // child, applies the child mutator to matching children, then
    // recurses into their descendants.
    let predicate = Predicate::always_true();
    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();

    let instruction_mutator = GfxMutator::Instruction {
        instruction: Instruction::RepeatWhile(predicate),
        channel: 0,
        mutation: Mutation::None,
    };
    *mutator.arena.get_mut(mutator.root).unwrap().get_mut() = instruction_mutator;

    // Child mutator: shifts position by (5, 7).
    let child_mutator = GfxMutator::new(
        Mutation::area_delta(DeltaGlyphArea::new(vec![
            GlyphAreaField::Operation(ApplyOperation::Add),
            GlyphAreaField::position(5.0, 7.0),
        ])),
        0,
    );
    let child_mut_id = mutator.arena.new_node(child_mutator);
    mutator.root.append(child_mut_id, &mut mutator.arena);

    // Apply.
    mutator.apply_to(&mut model);

    // child_a should have been mutated (position += (5, 7)).
    let child_a = model.arena.get(child_a_id).unwrap().get();
    assert!(
        almost_equal(child_a.position().x, 5.0),
        "child_a x: expected 5.0, got {}",
        child_a.position().x
    );
    assert!(
        almost_equal(child_a.position().y, 7.0),
        "child_a y: expected 7.0, got {}",
        child_a.position().y
    );

    // child_b (descendant of child_a) should also have been reached
    // by the repeat-while walk and received the same delta.
    let child_b = model.arena.get(child_b_id).unwrap().get();
    assert!(
        almost_equal(child_b.position().x, 105.0),
        "child_b x: expected 105.0, got {}",
        child_b.position().x
    );
    assert!(
        almost_equal(child_b.position().y, 107.0),
        "child_b y: expected 107.0, got {}",
        child_b.position().y
    );
}

// ── Instruction::RotateWhile ───────────────────────────────────────

#[test]
fn test_instruction_rotate_while() {
    do_instruction_rotate_while();
}

/// `RotateWhile` is currently a stub in the tree walker. Verify that
/// the variant can be constructed, stored in a `GfxMutator`, and
/// that applying the mutator directly (via `Applicable`) still
/// delivers its attached `mutation` field to the target element. The
/// rotation instruction itself is a no-op today.
pub fn do_instruction_rotate_while() {
    fonts::init();

    let mut element = GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "rotate",
            1.0,
            10.0,
            Vec2::new(20.0, 30.0),
            Vec2::new(100.0, 50.0),
        ),
        0,
        0,
    );

    // Build an Instruction::RotateWhile mutator with an attached
    // area delta. The RotateWhile instruction is a tree-walker
    // concern (currently stubbed), but the Applicable impl for
    // GfxMutator::Instruction still applies the `mutation` field.
    let mutator = GfxMutator::Instruction {
        instruction: Instruction::RotateWhile(45.0, Predicate::always_true()),
        channel: 0,
        mutation: Mutation::area_delta(DeltaGlyphArea::new(vec![
            GlyphAreaField::Operation(ApplyOperation::Add),
            GlyphAreaField::position(10.0, 10.0),
        ])),
    };

    mutator.apply_to(&mut element);

    // The direct mutation should have been applied.
    assert!(
        almost_equal(element.position().x, 30.0),
        "Expected x=30.0, got {}",
        element.position().x
    );
    assert!(
        almost_equal(element.position().y, 40.0),
        "Expected y=40.0, got {}",
        element.position().y
    );
}

// ── MutatorTree::apply_to ──────────────────────────────────────────

#[test]
fn test_mutator_tree_applies_to_target() {
    do_mutator_tree_applies_to_target();
}

/// Build a two-level target tree and a matching mutator tree, then
/// call `MutatorTree::apply_to`. Verify the root and child elements
/// received their respective mutations.
pub fn do_mutator_tree_applies_to_target() {
    fonts::init();

    // -- target tree: root (ch 0) -> child (ch 0)
    let mut model: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();

    // Replace the default void root with an area element.
    let root_element = GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "root",
            1.0,
            10.0,
            Vec2::new(0.0, 0.0),
            Vec2::new(200.0, 200.0),
        ),
        0,
        1,
    );
    *model.arena.get_mut(model.root).unwrap().get_mut() = root_element;

    let child_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str(
            "child",
            1.0,
            10.0,
            Vec2::new(50.0, 60.0),
            Vec2::new(100.0, 100.0),
        ),
        0,
        2,
    ));
    model.root.append(child_id, &mut model.arena);

    // -- mutator tree: root (NudgeRight 7.0, ch 0) -> child (position += (0, 11), ch 0)
    let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
    let root_mut = GfxMutator::new(
        Mutation::area_command(GlyphAreaCommand::NudgeRight(7.0)),
        0,
    );
    *mutator.arena.get_mut(mutator.root).unwrap().get_mut() = root_mut;

    let child_mut = GfxMutator::new(
        Mutation::area_delta(DeltaGlyphArea::new(vec![
            GlyphAreaField::Operation(ApplyOperation::Add),
            GlyphAreaField::position(0.0, 11.0),
        ])),
        0,
    );
    let child_mut_id = mutator.arena.new_node(child_mut);
    mutator.root.append(child_mut_id, &mut mutator.arena);

    // Apply.
    mutator.apply_to(&mut model);

    // Root: x should have shifted right by 7.
    let root = model.arena.get(model.root).unwrap().get();
    assert!(
        almost_equal(root.position().x, 7.0),
        "root x: expected 7.0, got {}",
        root.position().x
    );
    assert!(
        almost_equal(root.position().y, 0.0),
        "root y: expected 0.0, got {}",
        root.position().y
    );

    // Child: y should have shifted by 11.
    let child = model.arena.get(child_id).unwrap().get();
    assert!(
        almost_equal(child.position().x, 50.0),
        "child x: expected 50.0, got {}",
        child.position().x
    );
    assert!(
        almost_equal(child.position().y, 71.0),
        "child y: expected 71.0, got {}",
        child.position().y
    );
}
