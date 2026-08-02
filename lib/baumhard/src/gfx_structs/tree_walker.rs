// SPDX-License-Identifier: MPL-2.0

//! Walker that drives `MutatorTree::apply_to` — aligns mutators to
//! target-tree elements by their
//! [`BranchChannel`](crate::gfx_structs::tree::BranchChannel) and
//! dispatches each mutator's effect. Handles straight-line
//! alignment, `Instruction::RepeatWhile` loop expansion (with the
//! [`Predicate`](crate::gfx_structs::predicate::Predicate) language),
//! and the event-propagation side channel that `GlyphTreeEvent`s
//! ride on. The single entry point is
//! [`walk_tree_from`](crate::gfx_structs::tree_walker::walk_tree_from);
//! everything else in this module is internal scaffolding kept
//! `pub` so mutator-DSL authors
//! can compose custom terminators without forking the walker.

use crate::core::primitives::Applicable;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutator, Instruction};
use crate::gfx_structs::predicate::Predicate;
use crate::gfx_structs::tree::{BranchChannel, MutatorTree, Tree};
use crate::util::ordered_vec2::OrderedVec2;
use glam::Vec2;
use indextree::{Arena, Node, NodeId};
use log::{debug, warn};

/// Continuation called after a conditional loop ([`Instruction`])
/// exits — it resumes the normal walk from the mutator sibling
/// whose channel matches the current target. Unchecked call-site
/// inside the walker; not a stable public API but `pub` so mutator
/// authors can substitute custom terminators when extending the
/// walker.
///
/// The after-mutations attached under `mutator_id` are treated as a
/// channel-sorted stream (same logic as the normal child-alignment
/// walk): arena order is not assumed to be ascending. Every
/// after-mutation whose channel equals `t_chan` is dispatched via
/// [`walk_tree_from`]; the scan stops as soon as it passes `t_chan`.
pub const DEFAULT_TERMINATOR: fn(
    &mut Tree<GfxElement, GfxMutator>,
    &MutatorTree<GfxMutator>,
    NodeId,
    NodeId,
) = |gfx_tree: &mut Tree<GfxElement, GfxMutator>,
     mutator_tree: &MutatorTree<GfxMutator>,
     target_id: NodeId,
     mutator_id: NodeId| {
    // When a conditional loop terminates, we need to resume the normal walk
    // Both the mutator and the target will be in the exact position where
    // The predicate failed, so the target has not been mutated (yet)
    // But the mutator is one step behind
    debug!("The Terminator has received a mission.");
    let target = get_target(&mut gfx_tree.arena, target_id);
    let t_chan = target.get().channel();
    let after_mutators = collect_sorted_children(&mutator_tree.arena, mutator_id, |m| m.channel());
    for (after_id, after_chan) in after_mutators {
        if after_chan == t_chan {
            debug!("Next mutator matches the target, starting walk..");
            walk_tree_from(gfx_tree, mutator_tree, target_id, after_id);
        } else if after_chan > t_chan {
            debug!("Next mutator channel is higher than target channel, ending branch..");
            break;
        }
    }
    debug!("No more mutators, ending branch..");
};

/// Walk the entire `mutator_tree` against the `gfx_tree`, starting
/// from both roots. Convenience wrapper around [`walk_tree_from`];
/// cost is O(matching pairs) — see that function for the full
/// analysis.
pub fn walk_tree(gfx_tree: &mut Tree<GfxElement, GfxMutator>, mutator_tree: &MutatorTree<GfxMutator>) {
    walk_tree_from(gfx_tree, mutator_tree, gfx_tree.root, mutator_tree.root)
}

/// Dispatch one mutator node against one target node and recurse
/// into aligned children. `target_id` and `mutator_id` must belong
/// to the arenas that back `gfx_tree` and `mutator_tree`
/// respectively. Cost: O(sum of matching (target, mutator) pairs)
/// — every pairwise match is one apply; pruned branches are free.
pub fn walk_tree_from(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
    target_id: NodeId,
    mutator_id: NodeId,
) {
    let mutator = get_mutator(&mutator_tree.arena, mutator_id).get();

    match mutator {
        GfxMutator::Single { .. } | GfxMutator::Macro { .. } => {
            debug!("Processing Delta Node...");
            if apply_if_matching_channel(gfx_tree, target_id, mutator) {
                gfx_tree.invalidate_caches();
            }
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
                if apply_if_matching_channel(gfx_tree, target_id, mutator) {
                    gfx_tree.invalidate_caches();
                }
            }
            process_instruction_node(gfx_tree, mutator_tree, target_id, mutator_id, instruction);
            return;
        }
    }
    align_child_walks(gfx_tree, mutator_tree, target_id, mutator_id);
}

#[inline]
fn apply_if_matching_channel(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    target_id: NodeId,
    mutator: &GfxMutator,
) -> bool {
    let target = get_target(&mut gfx_tree.arena, target_id).get_mut();
    if mutator.channel() == target.channel() {
        debug!("Delta and target channel match, applying..");
        mutator.apply_to(target);
        true
    } else {
        debug!("Delta mutator channel does not match target channel.");
        false
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
            if mutator.first_child().is_none() {
                warn!("RepeatWhile instruction node has no children, skipping branch");
                return;
            }
            if target.first_child().is_none() {
                debug!("The target has no children - completing walk down this branch.");
                return;
            }
            compare_apply_repeat_while(gfx_tree, mutator_tree, target_id, mutator_id, condition)
        }
        Instruction::RotateWhile(_, _) => {
            // Reserved instruction (see `format/mutators.md` —
            // sibling rotation), unimplemented in the walker
            // today. A loaded mutator that uses RotateWhile will
            // silently no-op the rotation step. Logging at
            // `warn!` rather than panicking keeps the rest of the
            // mutation chain executing — a malformed reserved
            // instruction shouldn't abort the whole walk.
            warn!(
                "RotateWhile instruction not implemented in walker; \
                 this branch becomes a no-op (see format/mutators.md)"
            );
        }
        Instruction::SpatialDescend(point) => {
            spatial_descend(gfx_tree, mutator_tree, target_id, mutator_id, point);
        }
        Instruction::MapChildren => {
            zip_map_children(gfx_tree, mutator_tree, target_id, mutator_id);
        }
    };
}

/// Walk the children of `target_parent` and `mutator_parent` as
/// channel-sorted streams, applying [`repeat_while`] for every
/// matching (target, mutator) pair.
///
/// Mirrors [`align_child_walks`]: arena order is **not** assumed to
/// be channel-ascending, so both sibling rows are collected and
/// sorted before the merge walk. The sorted merge advances only the
/// mutator when `m_chan < t_chan` and only the target when
/// `m_chan > t_chan`, preserving broadcast semantics (one mutator
/// may apply to multiple consecutive targets sharing its channel).
///
/// Cost: O(n log n) per sibling row for the sort, where `n` is the
/// sibling count under one parent. Sibling counts are small in
/// practice (single-digit), so the sort is effectively free next to
/// the per-pair `repeat_while` recursion.
fn compare_apply_repeat_while(
    gfx_tree: &mut Tree<GfxElement, GfxMutator>,
    mutator_tree: &MutatorTree<GfxMutator>,
    target_parent_id: NodeId,
    mutator_parent_id: NodeId,
    condition: &Predicate,
) {
    let mutator_children = collect_sorted_children(&mutator_tree.arena, mutator_parent_id, |m| m.channel());
    if mutator_children.is_empty() {
        debug!("RepeatWhile mutator has no children - nothing to align.");
        return;
    }
    let target_children = collect_sorted_children(&gfx_tree.arena, target_parent_id, |t| t.channel());

    let mut t_idx = 0usize;
    for (m_id, m_chan) in mutator_children.iter().copied() {
        while t_idx < target_children.len() {
            let (t_id, t_chan) = target_children[t_idx];
            if t_chan == m_chan {
                t_idx += 1;
                repeat_while(gfx_tree, mutator_tree, t_id, m_id, condition, DEFAULT_TERMINATOR);
            } else if t_chan > m_chan {
                debug!(
                    "Target channel {} exceeds mutator channel {}, advancing to next mutator.",
                    t_chan, m_chan
                );
                break;
            } else {
                t_idx += 1;
            }
        }
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
///
/// # Channel-sorted merge walk
///
/// The walker treats the target's and mutator's children as
/// sorted streams keyed by [`channel`](GfxElement::channel) and
/// applies each mutator child to every target child sharing its
/// channel. The arena order of children is **not** assumed to be
/// channel-ascending — children are collected into local
/// `Vec`s and sorted by channel before the merge walk. This
/// removes a long-standing fragility where the walker's
/// `t_chan > m_chan` break could prematurely skip matches when
/// arena order disagreed with channel order (see
/// `console_mutator_round_trips_to_fresh_build` for a fixture
/// that exercises a non-ascending sibling row).
///
/// Cost: O(n log n) per sibling row for the sort, where `n` is
/// the sibling count under one parent. Sibling counts are small
/// in practice (single-digit), so the sort is effectively free
/// next to the per-pair `walk_tree_from` recursion.
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
    let mutator_children = collect_sorted_children(&mutator_tree.arena, mutator_id, |m| m.channel());
    if mutator_children.is_empty() {
        debug!("Mutator has no children - nothing to align.");
        return;
    }
    let target_children = collect_sorted_children(&gfx_tree.arena, target_id, |t| t.channel());

    let mut t_idx = 0usize;
    for (m_id, m_chan) in mutator_children.iter().copied() {
        while t_idx < target_children.len() {
            let (t_id, t_chan) = target_children[t_idx];
            if t_chan == m_chan {
                t_idx += 1;
                walk_tree_from(gfx_tree, mutator_tree, t_id, m_id);
            } else if t_chan > m_chan {
                debug!(
                    "Target channel {} exceeds mutator channel {}, advancing to next mutator.",
                    t_chan, m_chan
                );
                break;
            } else {
                t_idx += 1;
            }
        }
    }
}

/// Collect direct children of `parent` paired with their
/// [`channel`](GfxElement::channel) value, sorted ascending by
/// channel. Stable sort so siblings with identical channels keep
/// their arena-relative order — pairs the channel-merge walk
/// against itself deterministically.
fn collect_sorted_children<E>(
    arena: &Arena<E>,
    parent: NodeId,
    channel_of: impl Fn(&E) -> usize,
) -> Vec<(NodeId, usize)> {
    let mut out = Vec::new();
    let mut cur = arena.get(parent).and_then(|n| n.first_child());
    while let Some(id) = cur {
        if let Some(node) = arena.get(id) {
            out.push((id, channel_of(node.get())));
            cur = node.next_sibling();
        } else {
            break;
        }
    }
    out.sort_by_key(|&(_, ch)| ch);
    out
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
        let next_target = gfx_tree.arena.get(target_child_id).and_then(|n| n.next_sibling());

        // Force-apply the mutator to its paired target, then capture
        // the instruction (if the mutator is an Instruction) so the
        // subsequent recursive dispatch has no arena borrows live.
        //
        // We clone the Instruction here — not reborrow — because the
        // follow-up `process_instruction_node` call takes
        // `&mut gfx_tree` and would alias the read-only borrow we
        // hold on `mutator_tree.arena` via `m`. The clone is cheap:
        // Instruction is a small enum (no large payloads), and the
        // branch only fires on the minority of child mutators that
        // are themselves Instructions.
        let forwarded_instruction: Option<Instruction> = {
            let m = get_mutator(&mutator_tree.arena, mutator_child_id).get();
            let t = get_target(&mut gfx_tree.arena, target_child_id).get_mut();
            m.apply_to(t);
            match m {
                GfxMutator::Instruction { instruction, .. } => Some(instruction.clone()),
                _ => None,
            }
        };
        gfx_tree.invalidate_caches();
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
                align_child_walks(gfx_tree, mutator_tree, target_child_id, mutator_child_id);
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

/// As long as `condition` holds true, keep applying `mutator_id`
/// (and its descendants) to `target_id` (and its descendants). When
/// the condition fails, call `terminator` to resume the normal walk.
///
/// This is the engine behind [`Instruction::RepeatWhile`]; it is
/// exposed so mutator authors can supply a custom `terminator`
/// without forking the walker (see [`DEFAULT_TERMINATOR`] for the
/// default continuation). The `terminator` receives the same tree
/// pair and the current target/mutator ids.
pub fn repeat_while(
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
    let condition_matches = {
        let target = get_target(&mut gfx_tree.arena, target_id).get_mut();
        condition.test(&target)
    };
    if condition_matches {
        debug!(
            "Condition is met, applying mutator {} to target {}",
            mutator_id, target_id
        );
        let mutator = get_mutator(&mutator_tree.arena, mutator_id).get();
        {
            let target = get_target(&mut gfx_tree.arena, target_id).get_mut();
            mutator.apply_to(target);
        }
        gfx_tree.invalidate_caches();
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
            repeat_while(gfx_tree, mutator_tree, head_id, mutator_id, condition, terminator);
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
            {
                let target = get_target(&mut gfx_tree.arena, hit_id).get_mut();
                mutation.apply_to(target);
            }
            gfx_tree.invalidate_caches();
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
    bvh_find(arena, node_id, point, 0.0, false, best);
}

/// Unified BVH descent — single source for both [`Tree::descendant_near`]
/// (which wants slack + shape refinement so an ellipse-shaped node
/// doesn't false-hit on its corner AABB) and the mutator-builder's
/// `SpatialDescend` instruction (no slack, AABB-only).
///
/// For each child of `node_id`:
/// 1. Prune: if the child's `subtree_aabb` doesn't contain `point`
///    inflated by `slack`, skip the entire subtree.
/// 2. Test the child's own `GlyphArea` AABB (slack-inflated). On
///    hit, optionally refine via `area.shape.contains_local` so an
///    ellipse hit-tests against its actual shape rather than its
///    bounding rectangle.
/// 3. Recurse into the child's children.
///
/// Smallest-area wins on tie (so a smaller element stacked over a
/// bigger one is the hit). Uses `first_child` / `next_sibling`
/// iteration to avoid per-call `Vec` allocation (§B7).
pub(crate) fn bvh_find(
    arena: &Arena<GfxElement>,
    node_id: NodeId,
    point: Vec2,
    slack: f32,
    refine_with_shape: bool,
    best: &mut Option<(NodeId, f32)>,
) {
    let mut child_opt = arena.get(node_id).and_then(|n| n.first_child());

    while let Some(child_id) = child_opt {
        child_opt = arena.get(child_id).and_then(|n| n.next_sibling());

        let Some(node) = arena.get(child_id) else {
            continue;
        };
        let element = node.get();

        // 1. Prune: subtree AABB must contain (slack-inflated) point.
        if let Some((st_min, st_max)) = element.subtree_aabb() {
            if point.x < st_min.x - slack
                || point.x > st_max.x + slack
                || point.y < st_min.y - slack
                || point.y > st_max.y + slack
            {
                continue;
            }
        } else {
            continue; // no subtree AABB → no renderable content
        }

        // 2. Check this node's own GlyphArea AABB.
        if let Some(area) = element.glyph_area() {
            let pos = area.position.to_vec2();
            let bounds = area.render_bounds.to_vec2();
            if bounds.x > 0.0 && bounds.y > 0.0 {
                let min_x = pos.x - slack;
                let min_y = pos.y - slack;
                let max_x = pos.x + bounds.x + slack;
                let max_y = pos.y + bounds.y + slack;
                if point.x >= min_x && point.x <= max_x && point.y >= min_y && point.y <= max_y {
                    let mut hit = true;
                    if refine_with_shape {
                        // Shift into the inflated local frame so
                        // rectangle and ellipse get the same isotropic
                        // fuzzy margin the caller asked for. `slack == 0`
                        // is the exact-hit case (no-op inflation).
                        //
                        // The frame is derived from the very `min` / `max`
                        // the AABB test above used, not recomputed from
                        // `pos` and `bounds`. Float addition is not
                        // associative: for an area at x = 872.0 of width
                        // 22.4, `pos.x + bounds.x` rounds to 894.40002
                        // while `894.40002 - 872.0` rounds to 22.400024,
                        // which is *greater* than `bounds.x`. Refining
                        // against `bounds` therefore rejected points the
                        // AABB test had just accepted — a click exactly on
                        // a rectangle's right or bottom edge fell through
                        // to whatever lay beneath. Whether it bit depended
                        // on the coordinates: the same fixture's y axis
                        // rounds the other way (22.399994), so one edge
                        // could misbehave while the opposite one did not.
                        //
                        // Every hit test in the app funnels through here —
                        // `Tree::descendant_at` backs both the canvas-role
                        // routing and `document::hit_test`'s node and
                        // section picking — so this was a whole-app edge
                        // defect, not a portal one.
                        //
                        // Subtracting the same `min` from both sides makes
                        // the boundary compare exact. For a rectangle the
                        // refinement is then a provable no-op (the
                        // accepted set is exactly the closed AABB); for an
                        // ellipse the frame shifts by at most an ULP and
                        // stays inside the unchanged AABB.
                        let local = Vec2::new(point.x - min_x, point.y - min_y);
                        let inflated = Vec2::new(max_x - min_x, max_y - min_y);
                        hit = area.shape.contains_local(local, inflated);
                    }
                    if hit {
                        // Tie-break by *original* (un-slacked) area so a
                        // physically smaller element still wins.
                        let size = bounds.x * bounds.y;
                        match *best {
                            Some((_, best_size)) if best_size <= size => {}
                            _ => *best = Some((child_id, size)),
                        }
                    }
                }
            }
        }

        // 3. Recurse (the subtree-AABB test above proved at least
        //    one descendant may contain the point).
        bvh_find(arena, child_id, point, slack, refine_with_shape, best);
    }
}
