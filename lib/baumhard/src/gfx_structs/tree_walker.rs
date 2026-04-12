use indextree::{Arena, Node, NodeId};
use log::{debug, warn};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::tree::{BranchChannel, MutatorTree, Tree};
use crate::gfx_structs::mutator::{GfxMutator, Instruction};
use crate::gfx_structs::predicate::Predicate;
use crate::core::primitives::Applicable;

/// The term 'terminator' here refers conceptually to a function that should run
/// after a conditional loop is performed on the target tree (using an [Instruction])
pub const DEFAULT_TERMINATOR: fn(&mut Tree<GfxElement, GfxMutator>, &MutatorTree<GfxMutator>, NodeId, NodeId) =
    |gfx_tree: &mut Tree<GfxElement, GfxMutator>,
     mutator_tree: &MutatorTree<GfxMutator>,
     target_id: NodeId,
     mutator_id: NodeId| {
        // When a conditional loop terminates, we need to resume the normal walk
        // Both the mutator and the target will be in the exact position where
        // The predicate failed, so the target has not been mutated (yet)
        // But the mutator is one step behind
        debug!("The Terminator has received a mission.");
        let mutator = get_mutator(&mutator_tree.arena, mutator_id);
        let target = get_target(&mut gfx_tree.arena, target_id);
        let t_chan = target.get().channel();
        let mut option_next_mutator_id = mutator.first_child();
        loop {
            if option_next_mutator_id.is_some() {
                let next_mutator_id = option_next_mutator_id.unwrap();
                let next_mutator = get_mutator(&mutator_tree.arena, next_mutator_id);
                if next_mutator.get().channel() == t_chan {
                    debug!("Next mutator matches the target, starting walk..");
                    walk_tree_from(gfx_tree, mutator_tree, target_id, next_mutator_id);
                } else if next_mutator.get().channel() > t_chan {
                    debug!("Next mutator channel is higher than target channel, ending branch..");
                    break;
                }
                debug!("Trying next mutator sibling...");
                option_next_mutator_id = next_mutator.next_sibling();
            } else {
                debug!("No more mutators, ending branch..");
                break;
            }
        }
    };

pub fn walk_tree(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
) {
    walk_tree_from(gfx_tree, mutator_tree, gfx_tree.root, mutator_tree.root)
}

/// Recursively descend the trees, applying the mutator-tree to the target-tree
pub fn walk_tree_from(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
    target_id: NodeId,
    mutator_id: NodeId,
) {
    let mutator = get_mutator(&mutator_tree.arena, mutator_id).get();
    let target = get_target(&mut gfx_tree.arena, target_id).get_mut();

    match mutator {
        GfxMutator::Single { .. } | GfxMutator::Macro { .. } => {
            debug!("Processing Delta Node...");
            apply_if_matching_channel(mutator, target);
        }
        GfxMutator::Void { .. } => {
            debug!("Void mutator node, skipping")
        }
        GfxMutator::Instruction {
            instruction,
            mutation: section,
            ..
        } => {
            debug!("Processing Instruction node...");
            if section.is_some() {
                debug!("This instruction node has a Delta..");
                apply_if_matching_channel(mutator, target);
            }
            process_instruction_node(gfx_tree, mutator_tree, target_id, mutator_id, instruction);
            return;
        }
    }
    align_child_walks(gfx_tree, mutator_tree, target_id, mutator_id);
}

#[inline]
fn apply_if_matching_channel(mutator: &GfxMutator, target: &mut GfxElement) {
    if mutator.channel() == target.channel() {
        debug!("Delta and target channel match, applying..");
        mutator.apply_to(target);
    } else {
        debug!("Delta mutator channel does not match target channel.")
    }
}

#[inline]
fn process_instruction_node(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
    target_id: NodeId,
    mutator_id: NodeId,
    instruction: &Instruction,
) {
    match instruction {
        Instruction::RepeatWhile(condition) => {
            let mutator = get_mutator(&mutator_tree.arena, mutator_id);
            let target = get_target(&mut gfx_tree.arena, target_id);
            // Interactive path: a malformed RepeatWhile mutator (no
            // children to repeat) should degrade the walk, not abort
            // mutation application. The caller treats a no-op as
            // success.
            let Some(current_mutator_child_id) = mutator.first_child() else {
                warn!(
                    "RepeatWhile instruction node has no children, skipping branch"
                );
                return;
            };
            let Some(current_target_child_id) = target.first_child() else {
                debug!("The target has no children - completing walk down this branch.");
                return;
            };
            compare_apply_repeat_while(
                gfx_tree,
                mutator_tree,
                current_target_child_id,
                current_mutator_child_id,
                condition,
            )
        }
        Instruction::RotateWhile(_, _) => {}
    };
}

/// Assumes that the order of siblings is according to their channels, ascending.
/// Starting with the target, compare mutator and target channel and apply repeat_while
/// if they match. If mutator channel is greater or equal than target channel, then next
/// target sibling will also be checked
fn compare_apply_repeat_while(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
    target_id: NodeId,
    mutator_id: NodeId,
    condition: &Predicate,
) {
    let mutator_node = get_mutator(&mutator_tree.arena, mutator_id);
    let target_node = get_target(&mut gfx_tree.arena, target_id);
    let mutator = mutator_node.get();
    let maybe_next_target = target_node.next_sibling();
    let target = target_node.get_mut();

    let m_chan = mutator.channel();
    let t_chan = target.channel();
    let next_mutator = mutator_node.next_sibling();

    if m_chan == t_chan {
        debug!("Mutator and target channels matches - applying RepeatWhile.");
        repeat_while(
            gfx_tree,
            mutator_tree,
            target_id,
            mutator_id,
            condition,
            DEFAULT_TERMINATOR,
        );
    }

    // This is in case there are more target siblings with same channel
    if m_chan >= t_chan {
        if maybe_next_target.is_some() {
            return compare_apply_repeat_while(
                gfx_tree,
                mutator_tree,
                maybe_next_target.unwrap(),
                mutator_id,
                condition,
            );
        }
    }

    if next_mutator.is_some() && maybe_next_target.is_some() {
        debug!("Changing to next mutator-sibling");
        compare_apply_repeat_while(
            gfx_tree,
            mutator_tree,
            maybe_next_target.unwrap(),
            next_mutator.unwrap(),
            condition,
        )
    }
}

/// Look up a mutator-tree node by id.
///
/// **Precondition:** `id` must come from `mutator_tree.arena` (every
/// caller in this file walks the same arena it was handed). Violating
/// the precondition means the mutator tree is corrupted, which is a
/// bug at the call site, not user-recoverable input.
///
/// The remaining `expect` is *not* an interactive-path violation under
/// CODE_CONVENTIONS.md §4: it asserts a tight internal invariant that,
/// if broken, means the walker is operating on inconsistent state and
/// continuing would silently corrupt the user's mindmap. We prefer a
/// clean panic over a corrupt save.
#[inline]
fn get_mutator(arena: &Arena<GfxMutator>, id: NodeId) -> &Node<GfxMutator> {
    arena
        .get(id)
        .expect("walker invariant: mutator NodeId must belong to mutator_tree.arena")
}

/// Look up a target-tree node by id, see [`get_mutator`] for the
/// invariant rationale. Same precondition: `id` must originate from
/// `gfx_tree.arena`.
#[inline]
fn get_target(arena: &mut Arena<GfxElement>, id: NodeId) -> &mut Node<GfxElement> {
    arena
        .get_mut(id)
        .expect("walker invariant: target NodeId must belong to gfx_tree.arena")
}

/// Take the children of the mutator, and the target, and start a walk for each matching channel pairs
/// If one mutator matches many targets, then mutate all targets with that mutator
/// If one target matches many mutators, then mutate that target with all the mutators
#[inline]
fn align_child_walks(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
    target_id: NodeId,
    mutator_id: NodeId,
) {
    debug!(
        "Aligning children of target node {} and mutator node {}.",
        target_id, mutator_id
    );
    let mut option_mutator_child_id = get_mutator(&mutator_tree.arena, mutator_id).first_child();
    if option_mutator_child_id.is_none() {
        debug!("Mutator has no children - nothing to align.");
        return;
    }
    let mut option_target_child_id = get_target(&mut gfx_tree.arena, target_id).first_child();
    loop {
        if option_mutator_child_id.is_some() {
            let mutator_child_id = option_mutator_child_id.unwrap();
            let mutator_child = get_mutator(&mutator_tree.arena, mutator_child_id);
            option_mutator_child_id = mutator_child.next_sibling();
            debug!("Mutator is present, seeking matching targets..");
            loop {
                if option_target_child_id.is_some() {
                    let target_child_id = option_target_child_id.unwrap();
                    let target_child = get_target(&mut gfx_tree.arena, target_child_id);
                    let m_chan = mutator_child.get().channel();
                    let t_chan = target_child.get().channel();
                    if t_chan == m_chan {
                        option_target_child_id = target_child.next_sibling();
                        walk_tree_from(gfx_tree, mutator_tree, target_child_id, mutator_child_id);
                        debug!("Applied mutation-walk on child node, checking next sibling...");
                    } else if t_chan > m_chan {
                        debug!("Target channel is higher than mutator channel, breaking out of mutator loop.");
                        break;
                    } else {
                        option_target_child_id = target_child.next_sibling();
                    }
                } else {
                    debug!("Reached end of siblings, breaking inner mutation loop.");
                    break;
                }
            }
        } else {
            debug!("Reached end of mutator siblings, breaking outer mutation loop.");
            break;
        }
    }
}

/// As long as the condition holds true, keep applying it recursively
fn repeat_while(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
    target_id: NodeId,
    mutator_id: NodeId,
    condition: &Predicate,
    terminator: fn(
        gfx_arena: &mut Tree<GfxElement, GfxMutator>,
        mutator_arena: &MutatorTree<GfxMutator>,
        target_id: NodeId,
        mutator_id: NodeId,
    ),
) {
    let target = get_target(&mut gfx_tree.arena, target_id).get_mut();
    if condition.test(&target) {
        debug!(
            "Condition is met, applying mutator {} to target {}",
            mutator_id, target_id
        );
        let mutator = get_mutator(&mutator_tree.arena, mutator_id).get();
        mutator.apply_to(target);
        apply_repeat_while_to_children(
            gfx_tree,
            mutator_tree,
            target_id,
            mutator_id,
            condition,
            terminator,
        );
    } else {
        terminator(gfx_tree, mutator_tree, target_id, mutator_id);
    }
}

#[inline]
fn apply_repeat_while_to_children(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
    target_id: NodeId,
    mutator_id: NodeId,
    condition: &Predicate,
    terminator: fn(
        gfx_tree: &mut Tree<GfxElement, GfxMutator>,
        mutator_tree: &MutatorTree<GfxMutator>,
        target_id: NodeId,
        mutator_id: NodeId,
    ),
) {
    let parent_node = get_target(&mut gfx_tree.arena, target_id);
    let mut head = parent_node.first_child();
    loop {
        if head.is_some() {
            debug!("Found child, recursing down sub-tree");
            let head_id = head.unwrap();
            let current = get_target(&mut gfx_tree.arena, head_id);
            head = current.next_sibling();
            repeat_while(
                gfx_tree,
                mutator_tree,
                head_id,
                mutator_id,
                condition,
                terminator,
            );
        } else {
            break;
        }
    }
}
