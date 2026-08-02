// SPDX-License-Identifier: MPL-2.0

//! Edge-identity invariants. Edges are identified by the tuple
//! `(from_id, to_id, edge_type)`; duplicates break `EdgeRef` lookups
//! and scene-cache keys, so verify flags them even though the loader
//! keeps the map loadable with a runtime warning.

use std::collections::HashMap;

use baumhard::mindmap::model::MindMap;

use super::Violation;

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut seen: HashMap<(String, String, String), usize> = HashMap::new();

    for (i, edge) in map.edges.iter().enumerate() {
        let key = (edge.from_id.clone(), edge.to_id.clone(), edge.edge_type.clone());
        if let Some(&first) = seen.get(&key) {
            out.push(Violation::edge(
                "edges",
                i,
                format!(
                    "duplicate edge (from_id={:?}, to_id={:?}, type={:?}) \
                     first seen at edge[{}]",
                    edge.from_id, edge.to_id, edge.edge_type, first
                ),
            ));
        } else {
            seen.insert(key, i);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::{edge, node};

    #[test]
    fn distinct_edges_clean() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("1".into(), node("1", None));
        map.edges.push(edge("0", "1"));
        map.edges.push(edge("1", "0"));
        assert!(check(&map).is_empty());
    }

    #[test]
    fn different_types_between_same_nodes_clean() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("1".into(), node("1", None));
        let mut parent = edge("0", "1");
        parent.edge_type = "parent_child".into();
        map.edges.push(parent);
        map.edges.push(edge("0", "1")); // cross_link
        assert!(check(&map).is_empty());
    }

    #[test]
    fn duplicate_tuple_flagged() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("1".into(), node("1", None));
        map.edges.push(edge("0", "1"));
        map.edges.push(edge("0", "1"));
        let v = check(&map);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].category, "edges");
        assert_eq!(v[0].location, "edge[1]");
        assert!(v[0].message.contains("first seen at edge[0]"));
    }
}
