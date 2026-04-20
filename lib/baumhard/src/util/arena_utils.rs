//! Arena-wide tree copy helpers built on `indextree`.

use indextree::{Arena, NodeId};

/// Deep-copy the subtree rooted at `source_node_id`'s children from
/// `source` into `destination`, appending every cloned node under
/// `parent_id`. The original `source_node_id` itself is **not** copied
/// — only its descendants — so callers seed the destination with a
/// matching root node first.
///
/// Costs: O(n) in the descendant count, one `T::clone()` per node, and
/// one `Arena` slot allocation per node in `destination`. Benched as
/// `arena_utils_clone`.
pub fn clone_subtree<T: Clone>(
    source: &Arena<T>,
    source_node_id: NodeId,
    destination: &mut Arena<T>,
    parent_id: NodeId,
) {
    for child_id in source_node_id.children(source) {
        let cloned_node = source[child_id].get().clone();
        let new_node_id = parent_id.append_value(cloned_node, destination);
        clone_subtree(source, child_id, destination, new_node_id);
    }
}
