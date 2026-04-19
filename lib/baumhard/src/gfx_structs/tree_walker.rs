use glam::Vec2;
use indextree::{Arena, Node, NodeId};
use log::{debug, warn};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::tree::{BranchChannel, MutatorTree, Tree};
use crate::gfx_structs::mutator::{GfxMutator, Instruction};
use crate::gfx_structs::predicate::Predicate;
use crate::core::primitives::Applicable;
use crate::util::ordered_vec2::OrderedVec2;

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
        Instruction::SpatialDescend(point) => {
            spatial_descend(gfx_tree, mutator_tree, target_id, mutator_id, point);
        }
        Instruction::MapChildren => {
            zip_map_children(gfx_tree, mutator_tree, target_id, mutator_id);
        }
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
///
/// See also [`zip_map_children`] — the opt-out alternative that pairs
/// children by sibling position (zip) instead of by channel, for
/// mutations that need per-index targeting.
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

/// Zip the mutator's direct children against the target's direct
/// children by sibling position — the alternative to
/// [`align_child_walks`] for consumers that need per-index
/// targeting independent of channel semantics (e.g. size-aware
/// layouts where every target child sits on the same broadcast
/// channel).
///
/// For each pair up to
/// `min(mutator_children_len, target_children_len)`, the mutator's
/// own effect is **force-applied** to its paired target — bypassing
/// the channel-match check that [`apply_if_matching_channel`]
/// normally enforces. That bypass is the whole point: channels on
/// the paired children are broadcast tags, and MapChildren's job is
/// to ignore them at the pairing site.
///
/// After the force-apply, the mutator's subtree descends against the
/// target's subtree through the standard path:
/// - Single/Macro/Void → [`align_child_walks`] (channel-aware).
/// - Instruction → [`process_instruction_node`], which re-dispatches
///   into whichever instruction-body the child carries (including a
///   nested MapChildren, which will zip one level deeper).
///
/// Excess children on either side are silently dropped with a single
/// `debug!` line at termination. No allocation inside the loop
/// (§B7). Graceful no-op on empty children on either side.
///
/// The *outer* instruction's own attached mutation is already
/// applied to the current target by [`walk_tree_from`] before this
/// function is called (same precedent as
/// [`compare_apply_repeat_while`]) — this function handles only the
/// descent into paired children.
#[inline]
fn zip_map_children(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
    target_id: NodeId,
    mutator_id: NodeId,
) {
    let mut option_mutator_child = get_mutator(&mutator_tree.arena, mutator_id).first_child();
    let mut option_target_child = get_target(&mut gfx_tree.arena, target_id).first_child();

    let mut paired: usize = 0;
    loop {
        let (mutator_child_id, target_child_id) = match (option_mutator_child, option_target_child) {
            (Some(m), Some(t)) => (m, t),
            _ => break,
        };
        // Look up the next-siblings *before* the force-apply that
        // takes `&mut gfx_tree.arena`, so the read-only borrows end
        // cleanly.
        let next_mutator = mutator_tree
            .arena
            .get(mutator_child_id)
            .and_then(|n| n.next_sibling());
        let next_target = gfx_tree
            .arena
            .get(target_child_id)
            .and_then(|n| n.next_sibling());

        // Force-apply the mutator to its paired target, then capture
        // the instruction (if the mutator is an Instruction) so the
        // subsequent recursive dispatch has no arena borrows live.
        let forwarded_instruction: Option<Instruction> = {
            let m = get_mutator(&mutator_tree.arena, mutator_child_id).get();
            let t = get_target(&mut gfx_tree.arena, target_child_id).get_mut();
            m.apply_to(t);
            match m {
                GfxMutator::Instruction { instruction, .. } => Some(instruction.clone()),
                _ => None,
            }
        };
        match forwarded_instruction {
            Some(instruction) => {
                // Nested instruction: dispatch at the paired target.
                // Matches `walk_tree_from`'s post-apply path for
                // Instruction (`process_instruction_node` + early
                // return — no align_child_walks).
                process_instruction_node(
                    gfx_tree,
                    mutator_tree,
                    target_child_id,
                    mutator_child_id,
                    &instruction,
                );
            }
            None => {
                // Single / Macro / Void: descend via channel-based
                // align at the next level down. A user who wants the
                // deeper level to also zip nests MapChildren inside.
                align_child_walks(
                    gfx_tree,
                    mutator_tree,
                    target_child_id,
                    mutator_child_id,
                );
            }
        }

        paired += 1;
        option_mutator_child = next_mutator;
        option_target_child = next_target;
    }

    // Count any leftover children on either side — useful when a
    // runtime-expanded mutator (via Repeat) disagrees with the
    // actual target fan-out and the author wants to see it in logs.
    // Only one of the two loops runs because we broke out as soon
    // as either side ran dry.
    let mut excess_mutator: usize = 0;
    while let Some(m) = option_mutator_child {
        excess_mutator += 1;
        option_mutator_child = mutator_tree.arena.get(m).and_then(|n| n.next_sibling());
    }
    let mut excess_target: usize = 0;
    while let Some(t) = option_target_child {
        excess_target += 1;
        option_target_child = gfx_tree.arena.get(t).and_then(|n| n.next_sibling());
    }
    if excess_mutator > 0 || excess_target > 0 {
        debug!(
            "MapChildren zip paired {} children; {} excess mutators, {} excess targets ignored",
            paired, excess_mutator, excess_target
        );
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

// ── SpatialDescend ────────────────────────────────────────────────

/// BVH-accelerated spatial descent: find the deepest, smallest-area
/// `GlyphArea` node whose AABB contains `point`, then apply the
/// instruction's attached mutation to it.
///
/// Mirrors [`Tree::descendant_at`] but operates inside the mutator
/// pipeline — instead of returning a `NodeId`, it delivers the
/// mutation to the hit node.
///
/// # Algorithm
///
/// 1. Ensure subtree AABBs are fresh.
/// 2. Recursively descend from `target_id`: for each child, prune
///    if its `subtree_aabb` does not contain the point.
/// 3. Among all candidate nodes whose own AABB contains the point,
///    pick the smallest by area (innermost-first convention).
/// 4. Apply the instruction's attached mutation to that node.
///
/// If no node contains the point, the instruction is a no-op.
fn spatial_descend(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
    target_id: NodeId,
    mutator_id: NodeId,
    point: &OrderedVec2,
) {
    let point_vec = point.to_vec2();

    // Ensure subtree AABBs are fresh before descent.
    gfx_tree.ensure_subtree_aabbs();

    // BVH descent to find the hit node.
    let mut best: Option<(NodeId, f32)> = None;
    spatial_descend_recurse(&gfx_tree.arena, target_id, point_vec, &mut best);

    // Apply the instruction's mutation to the hit node.
    let Some((hit_id, _)) = best else {
        debug!("SpatialDescend: no node contains the point, no-op.");
        return;
    };
    debug!("SpatialDescend: hit node {:?}, applying mutation.", hit_id);

    // The instruction node's mutation (if any) is applied to the hit
    // target, regardless of channel — the spatial match overrides
    // channel alignment for event delivery.
    let mutator = get_mutator(&mutator_tree.arena, mutator_id).get();
    if let GfxMutator::Instruction { mutation, .. } = mutator {
        if mutation.is_some() {
            let target = get_target(&mut gfx_tree.arena, hit_id).get_mut();
            mutation.apply_to(target);
        }
    }
}

/// Recursive BVH descent helper for [`spatial_descend`]. Read-only
/// arena traversal that collects the best (smallest-area) hit.
///
/// Uses `first_child` / `next_sibling` iteration to avoid
/// allocating a `Vec` on every recursive call (§B7).
fn spatial_descend_recurse(
    arena: &Arena<GfxElement>,
    node_id: NodeId,
    point: Vec2,
    best: &mut Option<(NodeId, f32)>,
) {
    let mut child_opt = arena.get(node_id).and_then(|n| n.first_child());

    while let Some(child_id) = child_opt {
        child_opt = arena.get(child_id).and_then(|n| n.next_sibling());

        let Some(node) = arena.get(child_id) else {
            continue;
        };
        let element = node.get();

        // Prune: skip if subtree AABB doesn't contain point.
        if let Some((st_min, st_max)) = element.subtree_aabb() {
            if point.x < st_min.x || point.x > st_max.x
                || point.y < st_min.y || point.y > st_max.y
            {
                continue;
            }
        } else {
            continue;
        }

        // Check this node's own GlyphArea AABB.
        if let Some(area) = element.glyph_area() {
            let pos = area.position.to_vec2();
            let bounds = area.render_bounds.to_vec2();
            if bounds.x > 0.0 && bounds.y > 0.0
                && point.x >= pos.x && point.x <= pos.x + bounds.x
                && point.y >= pos.y && point.y <= pos.y + bounds.y
            {
                let size = bounds.x * bounds.y;
                match *best {
                    Some((_, best_size)) if best_size <= size => {}
                    _ => *best = Some((child_id, size)),
                }
            }
        }

        // Recurse deeper.
        spatial_descend_recurse(arena, child_id, point, best);
    }
}
