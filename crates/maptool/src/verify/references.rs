//! Reference invariants: every edge's endpoints must point to nodes
//! that exist in the map. Applies uniformly to line-mode and
//! portal-mode edges (since portals are now an edge display mode).

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
        assert!(v.iter().any(|x| x.category == "references"
            && x.message.contains("from_id")
            && x.message.contains("ghost")));
    }

    #[test]
    fn dangling_edge_to_is_flagged() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.edges.push(edge("0", "ghost"));
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "references" && x.message.contains("to_id")));
    }

    /// A portal-mode edge with a dangling endpoint is flagged the same
    /// way as any other edge — `display_mode` doesn't affect
    /// reference validation.
    #[test]
    fn dangling_portal_mode_edge_endpoint_flagged() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        let mut e = edge("0", "ghost");
        e.display_mode = Some(baumhard::mindmap::model::DISPLAY_MODE_PORTAL.to_string());
        map.edges.push(e);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "references" && x.message.contains("to_id")));
    }
}
