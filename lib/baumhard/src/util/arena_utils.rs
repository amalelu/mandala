// SPDX-License-Identifier: MPL-2.0

//! Arena-wide tree copy helpers built on `indextree`.

use indextree::{Arena, NodeId};

/// Deep-copy the subtree rooted at `source_node_id`'s children from
/// `source` into `destination`, appending every cloned node under
/// `parent_id`. The original `source_node_id` itself is **not** copied
/// — only its descendants — so callers seed the destination with a
/// matching root node first.
///
/// Iterative rather than recursive: a scene tree's depth is set by
/// the `.mindmap.json` it was built from, and a long `parent_id`
/// chain is legal and acyclic, so the loader accepts it. A recursive
/// copy inherits that depth and exhausts the stack — `SIGABRT`, not
/// a panic, so nothing catches or logs it. Nothing outside the tests
/// reaches this today, but it is `pub`, and the undo stack is the
/// named consumer, so the depth is the caller's to hand in.
///
/// The frontier holds `(source, destination parent)` pairs for nodes
/// not yet cloned, and children are pushed reversed so `pop()` yields
/// them left-to-right. That reproduces the recursion's depth-first
/// pre-order exactly, which matters because it fixes the order slots
/// are allocated in `destination` — and therefore the `NodeId`s the
/// caller sees.
///
/// Costs: O(n) in the descendant count, one `T::clone()` per node,
/// one `Arena` slot allocation per node in `destination`, plus one
/// heap vector holding the frontier — the sum of the unprocessed
/// sibling rows along the current path — **one element for a
/// linear chain**, since each node's only child replaces it, O(n)
/// for a shallow wide tree, and O(depth x branching) in general. (Not a branching width: this
/// test module's own seven-node fixture reaches four entries
/// against a branching width of three.) Benched as
/// `arena_utils_clone`.
pub fn clone_subtree<T: Clone>(
    source: &Arena<T>,
    source_node_id: NodeId,
    destination: &mut Arena<T>,
    parent_id: NodeId,
) {
    // Seed with the root's children rather than the root itself: the
    // contract copies descendants only, and the caller has already
    // supplied the matching destination root.
    let mut frontier: Vec<(NodeId, NodeId)> = Vec::new();
    let mark = frontier.len();
    for child_id in source_node_id.children(source) {
        frontier.push((child_id, parent_id));
    }
    frontier[mark..].reverse();

    while let Some((src_id, dst_parent)) = frontier.pop() {
        let cloned_node = source[src_id].get().clone();
        let new_node_id = dst_parent.append_value(cloned_node, destination);
        let mark = frontier.len();
        for child_id in src_id.children(source) {
            frontier.push((child_id, new_node_id));
        }
        frontier[mark..].reverse();
    }
}
