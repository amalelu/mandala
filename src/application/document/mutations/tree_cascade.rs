//! `tree-cascade` — arranges the target node's descendants as a
//! top-down tree. Each level sits below the previous at a configurable
//! row height; siblings spread horizontally, centered under their
//! parent. Handler mutates `MindMap.nodes[*].position` in a BFS pass
//! so each level sees its parent's final position before placing
//! itself.

use std::collections::VecDeque;

use log::debug;

use super::MindMapDocument;

const ROW_GAP: f64 = 80.0;
const SIBLING_GAP: f64 = 40.0;

/// `true` iff `f` is finite and not negative. Guards the layout
/// against NaN / Infinity / negative sizes propagating into every
/// downstream row — see `flower_layout::is_safe_coord` for the
/// same rationale.
fn is_safe_coord(f: f64) -> bool {
    f.is_finite() && f >= 0.0
}

pub fn apply(doc: &mut MindMapDocument, target_id: &str) {
    // BFS queue, seeded with the anchor. MindMap's single-parent
    // invariant (`format/ids.md`, enforced at load by the verifier)
    // rules out cycles, so the BFS is guaranteed to terminate.
    // `iteration_budget` converts a future invariant violation
    // (cycle, duplicate parent link introduced by a new mutation)
    // from an infinite loop that freezes the UI into an immediate
    // panic whose stack trace points at this function. The bound
    // is `2 * nodes.len()`: a cycle-free BFS visits each node at
    // most once and enqueues each child at most once, so exceeding
    // twice the node count is definitive proof of a cycle.
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(target_id.to_string());
    let iteration_budget = doc.mindmap.nodes.len().saturating_mul(2).max(2);
    let mut iterations = 0usize;

    while let Some(current) = queue.pop_front() {
        iterations += 1;
        assert!(
            iterations <= iteration_budget,
            "tree-cascade BFS exceeded {} iterations starting from '{}': \
             the single-parent invariant appears to be violated (cycle?). \
             Aborting to avoid an infinite freeze.",
            iteration_budget,
            target_id
        );
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
        if !is_safe_coord(parent.size.width)
            || !is_safe_coord(parent.size.height)
            || !parent.position.x.is_finite()
            || !parent.position.y.is_finite()
        {
            debug!(
                "tree-cascade: parent '{}' has non-finite size/position; skipping row",
                current
            );
            // Still enqueue children so deeper levels get placed
            // relative to their own parents if those are well-formed.
            for child_id in &children {
                queue.push_back(child_id.clone());
            }
            continue;
        }
        // Gather child sizes once so we can compute the row width
        // without re-borrowing inside the placement loop. Skip
        // children with non-finite sizes — they aren't placed this
        // pass but are still walked as potential parents of their
        // own sub-rows.
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
            if is_safe_coord(*cw) {
                if let Some(child) = doc.mindmap.nodes.get_mut(child_id) {
                    child.position.x = cursor_x;
                    child.position.y = row_y;
                }
            } else {
                debug!(
                    "tree-cascade: child '{}' has non-finite width; leaving in place",
                    child_id
                );
            }
            cursor_x += cw.max(0.0) + SIBLING_GAP;
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

    /// Freeze-hardening regression: if the single-parent invariant
    /// is violated (e.g. a bug elsewhere introduces a two-node
    /// parent cycle), the BFS must abort with a diagnostic panic
    /// rather than loop forever and freeze the UI. Without the
    /// iteration guard this test would hang.
    #[test]
    #[should_panic(expected = "tree-cascade BFS exceeded")]
    fn cycle_in_parent_links_panics_instead_of_freezing() {
        let path = format!(
            "{}/maps/testament.mindmap.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut doc = MindMapDocument::load(&path).unwrap();
        // Pick any two distinct nodes and wire them into a cycle by
        // pointing each at the other as parent. `children_of` scans
        // `nodes.values().filter(|n| n.parent_id == Some(parent))`,
        // so this creates `A → B → A → ...` for the BFS.
        let mut ids = doc.mindmap.nodes.keys().cloned();
        let a = ids.next().expect("fixture has at least one node");
        let b = ids.next().expect("fixture has at least two nodes");
        doc.mindmap.nodes.get_mut(&a).unwrap().parent_id = Some(b.clone());
        doc.mindmap.nodes.get_mut(&b).unwrap().parent_id = Some(a.clone());
        // Seed the BFS at one of the cycle nodes so the loop can
        // actually enter the cycle.
        apply(&mut doc, &a);
    }
}
