//! Tree-structure invariants: parent_id references and cycle detection.

use baumhard::mindmap::model::MindMap;
use std::collections::HashSet;

use super::Violation;

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for node in map.nodes.values() {
        if let Some(ref pid) = node.parent_id {
            if !map.nodes.contains_key(pid) {
                out.push(Violation {
                    category: "tree",
                    location: node.id.clone(),
                    message: format!("parent_id {:?} references a node that does not exist", pid),
                });
                continue;
            }
        }
    }

    // Cycle detection: walk each node's parent chain, flag if we revisit.
    for node in map.nodes.values() {
        let mut seen: HashSet<&str> = HashSet::new();
        seen.insert(node.id.as_str());
        let mut current = node.parent_id.as_deref();
        while let Some(pid) = current {
            if !seen.insert(pid) {
                out.push(Violation {
                    category: "tree",
                    location: node.id.clone(),
                    message: format!("cycle detected in parent_id chain (revisited {:?})", pid),
                });
                break;
            }
            current = map.nodes.get(pid).and_then(|n| n.parent_id.as_deref());
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::node;

    #[test]
    fn empty_map_has_no_violations() {
        assert!(check(&MindMap::new_blank("t")).is_empty());
    }

    #[test]
    fn missing_parent_is_flagged() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", Some("ghost")));
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "tree" && x.message.contains("ghost")));
    }

    #[test]
    fn cycle_is_flagged() {
        let mut map = MindMap::new_blank("t");
        // a → b → a
        map.nodes.insert("a".into(), node("a", Some("b")));
        map.nodes.insert("b".into(), node("b", Some("a")));
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "tree" && x.message.contains("cycle")));
    }

    #[test]
    fn valid_tree_clean() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("0.0".into(), node("0.0", Some("0")));
        assert!(check(&map).is_empty());
    }
}
