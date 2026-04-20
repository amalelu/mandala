//! Constructor helpers that produce a
//! [`MutatorNode`](crate::mutator_builder::MutatorNode) equivalent to
//! each legacy `TargetScope` variant. A
//! [`CustomMutation`](crate::mindmap::custom_mutation::CustomMutation)
//! whose payload used to be a flat `Vec<Mutation>` + `target_scope`
//! now carries a `mutator: MutatorNode` — these helpers bake a flat
//! `Vec<Mutation>` into the AST shape that matches the scope the
//! author intended.
//!
//! The shapes mirror the ones the (now-deleted)
//! `build_mutator_tree_for_scope` function emitted. Keeping the
//! constructors as named helpers lets the backward-compat deserializer,
//! tests, and human authors reach for them without open-coding the
//! AST shape each time.
//!
//! Not every legacy scope is expressible as a single MutatorTree —
//! `Parent` and `Siblings` require explicit iteration over the
//! application's structural index. The helpers for those produce a
//! `SelfOnly`-shaped MutatorNode; the application layer is
//! responsible for iterating the right set of targets and anchoring
//! the mutator at each of them.

use crate::gfx_structs::mutator::Mutation;
use crate::gfx_structs::predicate::Predicate;
use crate::mutator_builder::{
    InstructionSpec, MutationListSrc, MutationSrc, MutatorNode,
};

/// Build a MutatorNode that applies `mutations` only to the
/// anchor node.
///
/// Shape: a single `Macro` node on channel 0 with no children.
pub fn self_only(mutations: Vec<Mutation>) -> MutatorNode {
    MutatorNode::Macro {
        channel: 0,
        mutations: MutationListSrc::Literal(mutations),
        children: vec![],
    }
}

/// Build a MutatorNode that applies `mutations` to every descendant
/// of the anchor, but not the anchor itself.
///
/// Shape: an `Instruction(RepeatWhile(always_true))` root wrapping one
/// `Macro` child carrying the mutations.
pub fn descendants(mutations: Vec<Mutation>) -> MutatorNode {
    MutatorNode::Instruction {
        channel: 0,
        instruction: InstructionSpec::RepeatWhile(Predicate::always_true()),
        mutation: MutationSrc::None,
        children: vec![MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(mutations),
            children: vec![],
        }],
    }
}

/// Build a MutatorNode that applies `mutations` to the anchor AND all
/// its descendants.
///
/// Shape: a `Macro` root carrying the mutations on channel 0 (applies
/// to self), with one `Instruction(RepeatWhile(always_true))` child
/// whose own `Macro` child carries the same mutations (applies to
/// each descendant via the walker's repeat-while semantics). Mirrors
/// the topology the (now-deleted) `build_mutator_tree_for_scope`
/// function emitted.
///
/// The `mutations` list is cloned once into the root Macro and the
/// ownership-moved original passes to the nested Macro. The two
/// copies are independent payloads on the wire (round-tripping
/// through serde); at apply time they trigger the same `Mutation`
/// on the anchor and on every descendant via separate walker steps.
pub fn self_and_descendants(mutations: Vec<Mutation>) -> MutatorNode {
    MutatorNode::Macro {
        channel: 0,
        // First copy: the root Macro applies this list to the anchor.
        mutations: MutationListSrc::Literal(mutations.clone()),
        children: vec![MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhile(Predicate::always_true()),
            mutation: MutationSrc::None,
            children: vec![MutatorNode::Macro {
                channel: 0,
                // Second copy (moved): the RepeatWhile body applies
                // this list to every descendant.
                mutations: MutationListSrc::Literal(mutations),
                children: vec![],
            }],
        }],
    }
}

/// Build a MutatorNode that applies `mutations` to a single target
/// element (its channel alignment only). The application layer is
/// responsible for iterating the right set of targets (children,
/// parent, siblings) and anchoring this mutator at each in turn.
///
/// Same shape as [`self_only`]. Kept as a distinct helper so author
/// intent is explicit in code.
pub fn at_anchor(mutations: Vec<Mutation>) -> MutatorNode {
    self_only(mutations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx_structs::area::GlyphAreaCommand;
    use crate::gfx_structs::mutator::{Instruction, MutatorType};
    use crate::mutator_builder::{build, SectionContext};

    struct NoCtx;
    impl SectionContext for NoCtx {}

    fn nudge(n: f32) -> Mutation {
        Mutation::area_command(GlyphAreaCommand::NudgeRight(n))
    }

    #[test]
    fn self_only_produces_macro_root_carrying_literal_list() {
        let node = self_only(vec![nudge(10.0)]);
        match &node {
            MutatorNode::Macro {
                channel, mutations, ..
            } => {
                assert_eq!(*channel, 0);
                match mutations {
                    MutationListSrc::Literal(l) => assert_eq!(l.len(), 1),
                    _ => panic!("expected Literal"),
                }
            }
            _ => panic!("expected Macro root"),
        }
    }

    #[test]
    fn self_only_builds_to_macro_mutator() {
        let node = self_only(vec![nudge(10.0)]);
        let mt = build(&node, &NoCtx);
        let root = mt.arena.get(mt.root).unwrap().get();
        assert!(matches!(root.get_type(), MutatorType::Macro));
    }

    #[test]
    fn descendants_wraps_macro_in_repeat_while_always_true() {
        let node = descendants(vec![nudge(1.0)]);
        match node {
            MutatorNode::Instruction {
                instruction: InstructionSpec::RepeatWhile(_),
                children,
                ..
            } => {
                assert_eq!(children.len(), 1);
                assert!(matches!(children[0], MutatorNode::Macro { .. }));
            }
            _ => panic!("expected Instruction wrapping Macro"),
        }
    }

    #[test]
    fn descendants_builds_to_instruction_with_always_true_predicate() {
        let node = descendants(vec![nudge(1.0)]);
        let mt = build(&node, &NoCtx);
        let root = mt.arena.get(mt.root).unwrap().get();
        assert!(matches!(
            root,
            crate::gfx_structs::mutator::GfxMutator::Instruction {
                instruction: Instruction::RepeatWhile(_),
                ..
            }
        ));
    }

    #[test]
    fn self_and_descendants_has_macro_root_with_instruction_child() {
        let node = self_and_descendants(vec![nudge(1.0)]);
        match node {
            MutatorNode::Macro {
                channel,
                mutations,
                children,
            } => {
                assert_eq!(channel, 0);
                assert!(matches!(mutations, MutationListSrc::Literal(_)));
                assert_eq!(children.len(), 1);
                assert!(matches!(children[0], MutatorNode::Instruction { .. }));
            }
            _ => panic!("expected Macro root carrying mutations + Instruction child"),
        }
    }
}
