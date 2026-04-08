use indextree::{Arena, NodeId};

pub fn clone_subtree<T: Clone>(
    source: &Arena<T>,
    source_node_id: NodeId,
    destination: &mut Arena<T>,
    parent_id: NodeId,
) {
    for child_id in source_node_id.children(source) {
        // Clone the node from source and add it to destination
        let cloned_node = source[child_id].get().clone();
        let new_node_id = parent_id.append_value(cloned_node, destination);

        // Recursively clone children
        clone_subtree(source, child_id, destination, new_node_id);
    }
}
