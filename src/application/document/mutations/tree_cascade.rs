//! `tree-cascade` — arranges the target node's descendants as a
//! top-down tree. Each level sits below the previous at a configurable
//! row height; siblings spread horizontally, centered under their
//! parent. Handler mutates `MindMap.nodes[*].position` in a BFS pass
//! so each level sees its parent's final position before placing
//! itself.

use std::collections::VecDeque;

use super::MindMapDocument;

const ROW_GAP: f64 = 80.0;
const SIBLING_GAP: f64 = 40.0;

pub fn apply(doc: &mut MindMapDocument, target_id: &str) {
    // BFS queue of (parent_id, depth).
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(target_id.to_string());

    while let Some(current) = queue.pop_front() {
        let children: Vec<String> = doc
            .mindmap
            .children_of(&current)
            .iter()
            .map(|n| n.id.clone())
            .collect();
        if children.is_empty() {
            continue;
        }
        let Some(parent) = doc.mindmap.nodes.get(&current).cloned() else {
            continue;
        };
        // Gather child sizes once so we can compute the row width
        // without re-borrowing inside the placement loop.
        let sizes: Vec<(f64, f64)> = children
            .iter()
            .filter_map(|id| doc.mindmap.nodes.get(id))
            .map(|n| (n.size.width, n.size.height))
            .collect();
        let total_width: f64 =
            sizes.iter().map(|(w, _)| *w).sum::<f64>() + SIBLING_GAP * (children.len() as f64 - 1.0).max(0.0);
        let parent_cx = parent.position.x + parent.size.width / 2.0;
        let row_y = parent.position.y + parent.size.height + ROW_GAP;

        let mut cursor_x = parent_cx - total_width / 2.0;
        for (child_id, (cw, _ch)) in children.iter().zip(sizes.iter()) {
            if let Some(child) = doc.mindmap.nodes.get_mut(child_id) {
                child.position.x = cursor_x;
                child.position.y = row_y;
            }
            cursor_x += cw + SIBLING_GAP;
            queue.push_back(child_id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::MindMapDocument;

    /// Pinned testament fixture. Node `"1"` has four direct children;
    /// reuses the same pin as the flower_layout tests so a fixture
    /// restructure surfaces consistently. §T4 fixture discipline.
    const FIXTURE_PARENT_ID: &str = "1";
    const FIXTURE_MIN_CHILDREN: usize = 4;

    fn test_doc() -> (MindMapDocument, String) {
        let path = format!(
            "{}/maps/testament.mindmap.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let doc = MindMapDocument::load(&path).expect("testament loads");
        let count = doc.mindmap.children_of(FIXTURE_PARENT_ID).len();
        assert!(
            count >= FIXTURE_MIN_CHILDREN,
            "fixture drift: testament node '{}' expected to have \u{2265}{} \
             direct children, found {}. Pin a different parent id or \
             update FIXTURE_MIN_CHILDREN.",
            FIXTURE_PARENT_ID,
            FIXTURE_MIN_CHILDREN,
            count
        );
        (doc, FIXTURE_PARENT_ID.to_string())
    }

    #[test]
    fn descendants_sit_below_parent_after_apply() {
        let (mut doc, target_id) = test_doc();
        let parent = doc.mindmap.nodes.get(&target_id).cloned().unwrap();
        apply(&mut doc, &target_id);
        for child in doc.mindmap.children_of(&target_id) {
            let c = doc.mindmap.nodes.get(&child.id).unwrap();
            assert!(
                c.position.y > parent.position.y,
                "child '{}' landed above parent",
                child.id
            );
        }
    }

    #[test]
    fn descendants_have_no_horizontal_overlap_within_a_row() {
        let (mut doc, target_id) = test_doc();
        apply(&mut doc, &target_id);
        let children: Vec<_> = doc
            .mindmap
            .children_of(&target_id)
            .iter()
            .map(|n| n.id.clone())
            .collect();
        // Children are on the same row and ordered left-to-right; each
        // should start at or after the previous child's right edge.
        let mut last_right: Option<f64> = None;
        for id in &children {
            let n = doc.mindmap.nodes.get(id).unwrap();
            if let Some(prev) = last_right {
                assert!(
                    n.position.x + 1e-6 >= prev,
                    "child '{}' starts at {} but previous ended at {}",
                    id,
                    n.position.x,
                    prev
                );
            }
            last_right = Some(n.position.x + n.size.width);
        }
    }

    #[test]
    fn apply_on_leaf_is_noop() {
        let path = format!(
            "{}/maps/testament.mindmap.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut doc = MindMapDocument::load(&path).unwrap();
        let leaf = doc
            .mindmap
            .nodes
            .keys()
            .find(|id| doc.mindmap.children_of(id).is_empty())
            .unwrap()
            .clone();
        let before = doc.mindmap.nodes.get(&leaf).unwrap().position.clone();
        apply(&mut doc, &leaf);
        let after = doc.mindmap.nodes.get(&leaf).unwrap().position.clone();
        assert_eq!(before.x, after.x);
        assert_eq!(before.y, after.y);
    }
}
