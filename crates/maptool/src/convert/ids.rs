// SPDX-License-Identifier: MPL-2.0

//! Assign Dewey-decimal IDs to legacy maps. Walk visits parents
//! before children; returns the old→new map for a follow-up
//! reference-rewrite pass.

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

/// Iterative rather than recursive, for the same reason every walk
/// over `parent_id` depth in this workspace is: the depth comes from
/// the file, and `convert --legacy` runs on files from wherever the
/// user got them. A legacy map with a long parent chain is legal and
/// acyclic, so this inherits its depth — and a stack overflow is a
/// `SIGABRT`, not a catchable error, so the conversion would kill
/// the tool rather than report a bad input.
///
/// Children are pushed reversed so `pop()` yields them in the order
/// `sorted_children_of` returns, which is what fixes the assigned
/// Dewey indices. Reverse the visit order and every id changes.
fn assign_children(
    nodes: &serde_json::Map<String, Value>,
    parent_old_id: &str,
    parent_new_id: &str,
    id_map: &mut HashMap<String, String>,
) {
    // (old id of the subtree root, the new id already assigned to it)
    let mut frontier: Vec<(String, String)> = vec![(parent_old_id.to_string(), parent_new_id.to_string())];

    while let Some((old_id, new_id)) = frontier.pop() {
        let children = sorted_children_of(nodes, Some(&old_id));
        let mark = frontier.len();
        for (i, child_old_id) in children.iter().enumerate() {
            let child_new_id = format!("{}.{}", new_id, i);
            id_map.insert(child_old_id.clone(), child_new_id.clone());
            frontier.push((child_old_id.clone(), child_new_id));
        }
        frontier[mark..].reverse();
    }
}

/// Return old IDs of nodes whose parent_id matches `parent`, sorted by
/// ascending index.
fn sorted_children_of(nodes: &serde_json::Map<String, Value>, parent: Option<&str>) -> Vec<String> {
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
                        obj.insert("parent_id".to_string(), Value::String(new_pid.clone()));
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
    let Some(edges) = super::edges_arr_mut(root) else { return };
    for edge in edges.iter_mut() {
        rewrite_field(edge, "from_id", id_map);
        rewrite_field(edge, "to_id", id_map);
    }
}

fn rewrite_portals(root: &mut Value, id_map: &HashMap<String, String>) {
    let Some(portals) = super::portals_arr_mut(root) else { return };
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

#[cfg(test)]
mod deep_chain_tests {
    use super::assign_dewey_ids;
    use serde_json::json;

    /// **`convert --legacy` runs on files from wherever the user got
    /// them.** A legacy map with a long parent chain is legal and
    /// acyclic, so the id assignment inherits its depth. While this
    /// walk recursed, a deep enough chain exhausted the stack and
    /// killed the tool with `SIGABRT` — not an error it could report,
    /// which is the whole difference between "this file is odd" and
    /// "maptool died".
    ///
    /// Run on a 256 KiB stack, the same way the loader's deep-chain
    /// test is: any reintroduced recursion blows a stack that small
    /// long before the chain ends, while the iterative form is
    /// indifferent to it.
    ///
    /// The depth matches the loader's test rather than exceeding it
    /// because `sorted_children_of` rescans every node per parent —
    /// the conversion is O(n²) in the node count, which is fine for
    /// a one-shot migration and would make a larger fixture a
    /// minute-long test for no extra coverage.
    #[test]
    fn test_deep_legacy_chain_does_not_exhaust_the_stack() {
        const DEPTH: usize = 6_000;
        const SMALL_STACK: usize = 256 * 1024;

        let assigned = std::thread::Builder::new()
            .stack_size(SMALL_STACK)
            .spawn(|| {
                let mut nodes = serde_json::Map::new();
                for i in 0..DEPTH {
                    let parent = if i == 0 {
                        serde_json::Value::Null
                    } else {
                        json!(format!("n{}", i - 1))
                    };
                    nodes.insert(format!("n{i}"), json!({ "parent_id": parent, "index": 0 }));
                }
                assign_dewey_ids(&nodes).len()
            })
            .expect("spawn the small-stack conversion")
            .join()
            .expect("the id assignment must not exhaust a 256 KiB stack");

        assert_eq!(assigned, DEPTH, "every node in the chain must get an id");
    }

    /// The iterative rewrite must assign the same ids the recursion
    /// did. Dewey indices come from visit order, so a reversed walk
    /// renames every node in the file.
    #[test]
    fn test_ids_follow_sorted_child_order() {
        let nodes = serde_json::from_value(json!({
            "root":  { "parent_id": null,   "index": 0 },
            "a":     { "parent_id": "root", "index": 0 },
            "b":     { "parent_id": "root", "index": 1 },
            "a1":    { "parent_id": "a",    "index": 0 },
            "a2":    { "parent_id": "a",    "index": 1 },
            "b1":    { "parent_id": "b",    "index": 0 },
        }))
        .expect("fixture parses");
        let map = assign_dewey_ids(&nodes);
        assert_eq!(map.get("root").map(String::as_str), Some("0"));
        assert_eq!(map.get("a").map(String::as_str), Some("0.0"));
        assert_eq!(map.get("b").map(String::as_str), Some("0.1"));
        assert_eq!(map.get("a1").map(String::as_str), Some("0.0.0"));
        assert_eq!(map.get("a2").map(String::as_str), Some("0.0.1"));
        assert_eq!(map.get("b1").map(String::as_str), Some("0.1.0"));
    }
}
