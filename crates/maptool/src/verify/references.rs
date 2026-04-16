//! Reference invariants: edges and portals must point to nodes that exist.

use std::collections::HashSet;

use baumhard::mindmap::model::MindMap;

use super::Violation;

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for (i, edge) in map.edges.iter().enumerate() {
        if !map.nodes.contains_key(&edge.from_id) {
            out.push(Violation {
                category: "references",
                location: format!("edge[{}]", i),
                message: format!("from_id {:?} is not a node", edge.from_id),
            });
        }
        if !map.nodes.contains_key(&edge.to_id) {
            out.push(Violation {
                category: "references",
                location: format!("edge[{}]", i),
                message: format!("to_id {:?} is not a node", edge.to_id),
            });
        }
    }

    for (i, portal) in map.portals.iter().enumerate() {
        if !map.nodes.contains_key(&portal.endpoint_a) {
            out.push(Violation {
                category: "references",
                location: format!("portal[{}]", i),
                message: format!("endpoint_a {:?} is not a node", portal.endpoint_a),
            });
        }
        if !map.nodes.contains_key(&portal.endpoint_b) {
            out.push(Violation {
                category: "references",
                location: format!("portal[{}]", i),
                message: format!("endpoint_b {:?} is not a node", portal.endpoint_b),
            });
        }
    }

    // Portal label uniqueness
    let mut seen_labels: HashSet<&str> = HashSet::new();
    for (i, portal) in map.portals.iter().enumerate() {
        if !seen_labels.insert(&portal.label) {
            out.push(Violation {
                category: "references",
                location: format!("portal[{}]", i),
                message: format!("duplicate portal label {:?}", portal.label),
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::{edge, node};

    #[test]
    fn valid_edges_clean() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("1".into(), node("1", None));
        map.edges.push(edge("0", "1"));
        assert!(check(&map).is_empty());
    }

    #[test]
    fn dangling_edge_from_is_flagged() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.edges.push(edge("ghost", "0"));
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "references" && x.message.contains("from_id") && x.message.contains("ghost")));
    }

    #[test]
    fn dangling_edge_to_is_flagged() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.edges.push(edge("0", "ghost"));
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "references" && x.message.contains("to_id")));
    }
}
