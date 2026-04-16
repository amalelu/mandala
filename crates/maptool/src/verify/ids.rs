//! ID-consistency invariants: HashMap key matches node.id, and Dewey
//! structure agrees with parent_id.

use baumhard::mindmap::model::{derive_parent_id, MindMap};

use super::Violation;

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for (key, node) in &map.nodes {
        if &node.id != key {
            out.push(Violation {
                category: "ids",
                location: key.clone(),
                message: format!("HashMap key {:?} does not match node.id {:?}", key, node.id),
            });
        }

        let derived = derive_parent_id(&node.id);
        match (&derived, &node.parent_id) {
            (Some(d), Some(p)) if d == p => {}
            (None, None) => {}
            (Some(d), Some(p)) => {
                out.push(Violation {
                    category: "ids",
                    location: node.id.clone(),
                    message: format!(
                        "parent_id {:?} does not match derived parent {:?}",
                        p, d
                    ),
                });
            }
            (None, Some(p)) => {
                out.push(Violation {
                    category: "ids",
                    location: node.id.clone(),
                    message: format!(
                        "root-style id has parent_id {:?} (expected null)",
                        p
                    ),
                });
            }
            (Some(d), None) => {
                out.push(Violation {
                    category: "ids",
                    location: node.id.clone(),
                    message: format!(
                        "id implies parent {:?} but parent_id is null",
                        d
                    ),
                });
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::node;

    #[test]
    fn valid_dewey_is_clean() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("0.0".into(), node("0.0", Some("0")));
        map.nodes.insert("0.1".into(), node("0.1", Some("0")));
        assert!(check(&map).is_empty());
    }

    #[test]
    fn mismatched_parent_flagged() {
        let mut map = MindMap::new_blank("t");
        // id says parent is "0", but parent_id says "1"
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("1".into(), node("1", None));
        map.nodes.insert("0.0".into(), node("0.0", Some("1")));
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "ids" && x.message.contains("does not match")));
    }

    #[test]
    fn key_mismatch_flagged() {
        let mut map = MindMap::new_blank("t");
        // HashMap key "0" but node.id is "xyz"
        map.nodes.insert("0".into(), node("xyz", None));
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "ids" && x.message.contains("does not match node.id")));
    }
}
