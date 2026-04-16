//! Dewey-decimal ID assignment: walks the v1 tree (parent_id + index)
//! and assigns structural IDs like "0", "0.0", "0.1", "1", "1.0.2".
//! Returns an old→new ID mapping that callers use to rewrite references.

use serde_json::Value;
use std::collections::HashMap;

/// Walk the node tree and return a mapping from old IDs to new
/// Dewey-decimal IDs. Roots are numbered 0, 1, 2, ... in index order.
/// Children are numbered by their sibling position under each parent.
pub fn assign_dewey_ids(nodes: &serde_json::Map<String, Value>) -> HashMap<String, String> {
    let mut id_map: HashMap<String, String> = HashMap::new();

    // Collect children grouped by parent, sorted by index.
    let roots = sorted_children_of(nodes, None);
    for (i, root_id) in roots.iter().enumerate() {
        let new_id = i.to_string();
        id_map.insert(root_id.clone(), new_id.clone());
        assign_children(nodes, root_id, &new_id, &mut id_map);
    }

    id_map
}

fn assign_children(
    nodes: &serde_json::Map<String, Value>,
    parent_old_id: &str,
    parent_new_id: &str,
    id_map: &mut HashMap<String, String>,
) {
    let children = sorted_children_of(nodes, Some(parent_old_id));
    for (i, child_old_id) in children.iter().enumerate() {
        let child_new_id = format!("{}.{}", parent_new_id, i);
        id_map.insert(child_old_id.clone(), child_new_id.clone());
        assign_children(nodes, child_old_id, &child_new_id, id_map);
    }
}

/// Return old IDs of nodes whose parent_id matches `parent`, sorted by
/// ascending index.
fn sorted_children_of(
    nodes: &serde_json::Map<String, Value>,
    parent: Option<&str>,
) -> Vec<String> {
    let mut children: Vec<(&str, i64)> = nodes
        .iter()
        .filter(|(_, node)| {
            let pid = node.get("parent_id").and_then(|v| v.as_str());
            pid == parent
        })
        .map(|(id, node)| {
            let index = node.get("index").and_then(|v| v.as_i64()).unwrap_or(0);
            (id.as_str(), index)
        })
        .collect();
    children.sort_by_key(|(_, idx)| *idx);
    children.into_iter().map(|(id, _)| id.to_string()).collect()
}

/// Rewrite all node IDs, parent_id references, edge from_id/to_id, and
/// portal endpoint_a/endpoint_b using the old→new mapping.
pub fn rewrite_ids(root: &mut Value, id_map: &HashMap<String, String>) {
    rewrite_nodes(root, id_map);
    rewrite_edges(root, id_map);
    rewrite_portals(root, id_map);
}

fn rewrite_nodes(root: &mut Value, id_map: &HashMap<String, String>) {
    let nodes_obj = match root.get("nodes").and_then(|v| v.as_object()) {
        Some(obj) => obj.clone(),
        None => return,
    };

    let mut new_nodes = serde_json::Map::new();
    for (old_id, mut node) in nodes_obj {
        let new_id = id_map.get(&old_id).cloned().unwrap_or(old_id);

        // Update the id field inside the node
        if let Some(obj) = node.as_object_mut() {
            obj.insert("id".to_string(), Value::String(new_id.clone()));
        }

        // Update parent_id
        if let Some(obj) = node.as_object_mut() {
            if let Some(pid_val) = obj.get("parent_id") {
                if let Some(old_pid) = pid_val.as_str() {
                    if let Some(new_pid) = id_map.get(old_pid) {
                        obj.insert(
                            "parent_id".to_string(),
                            Value::String(new_pid.clone()),
                        );
                    }
                }
            }
        }

        new_nodes.insert(new_id, node);
    }

    if let Some(obj) = root.as_object_mut() {
        obj.insert("nodes".to_string(), Value::Object(new_nodes));
    }
}

fn rewrite_edges(root: &mut Value, id_map: &HashMap<String, String>) {
    let edges = match root.get_mut("edges").and_then(|v| v.as_array_mut()) {
        Some(arr) => arr,
        None => return,
    };
    for edge in edges.iter_mut() {
        rewrite_field(edge, "from_id", id_map);
        rewrite_field(edge, "to_id", id_map);
    }
}

fn rewrite_portals(root: &mut Value, id_map: &HashMap<String, String>) {
    let portals = match root.get_mut("portals").and_then(|v| v.as_array_mut()) {
        Some(arr) => arr,
        None => return,
    };
    for portal in portals.iter_mut() {
        rewrite_field(portal, "endpoint_a", id_map);
        rewrite_field(portal, "endpoint_b", id_map);
    }
}

fn rewrite_field(obj: &mut Value, field: &str, id_map: &HashMap<String, String>) {
    if let Some(old_val) = obj.get(field).and_then(|v| v.as_str()).map(|s| s.to_string()) {
        if let Some(new_val) = id_map.get(&old_val) {
            if let Some(o) = obj.as_object_mut() {
                o.insert(field.to_string(), Value::String(new_val.clone()));
            }
        }
    }
}
