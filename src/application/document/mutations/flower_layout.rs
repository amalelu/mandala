//! `flower-layout` — arranges the target node's direct children in a
//! circle around it. The radius scales with both the parent's size
//! and the children's bounding box so the result reads as a flower,
//! not an overlap. Handler mutates `MindMap.nodes[child_id].position`
//! for each child; the tree rebuild on the next frame picks up the
//! new geometry.

use super::MindMapDocument;

/// Padding added to the computed radius so children don't kiss the
/// parent's border.
const RADIAL_PADDING: f64 = 40.0;
/// Starting angle in radians (0 = right, π/2 = down). We rotate a
/// quarter turn so the first child lands at the top of the circle,
/// matching the convention in mind-mapping tools.
const START_ANGLE: f64 = -std::f64::consts::FRAC_PI_2;

pub fn apply(doc: &mut MindMapDocument, target_id: &str) {
    let Some(target) = doc.mindmap.nodes.get(target_id).cloned() else {
        return;
    };
    let children: Vec<String> = doc
        .mindmap
        .children_of(target_id)
        .iter()
        .map(|n| n.id.clone())
        .collect();
    if children.is_empty() {
        return;
    }

    // Radius fits the largest child's bounding diagonal plus the
    // parent's half-span — sized so a child's border never crosses
    // the parent's. O(children) in size reads, no allocations.
    let parent_span = (target.size.width.max(target.size.height)) / 2.0;
    let max_child_span = children
        .iter()
        .filter_map(|id| doc.mindmap.nodes.get(id))
        .map(|n| (n.size.width.max(n.size.height)) / 2.0)
        .fold(0.0_f64, f64::max);
    let radius = parent_span + max_child_span + RADIAL_PADDING;

    let n = children.len() as f64;
    let cx = target.position.x + target.size.width / 2.0;
    let cy = target.position.y + target.size.height / 2.0;

    for (i, child_id) in children.iter().enumerate() {
        let theta = START_ANGLE + (i as f64) * std::f64::consts::TAU / n;
        let Some(child) = doc.mindmap.nodes.get_mut(child_id) else {
            continue;
        };
        // Position is the top-left corner; centre the child on the
        // computed point.
        child.position.x = cx + radius * theta.cos() - child.size.width / 2.0;
        child.position.y = cy + radius * theta.sin() - child.size.height / 2.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::MindMapDocument;

    fn test_doc_with_children() -> (MindMapDocument, String, Vec<String>) {
        let path = format!(
            "{}/maps/testament.mindmap.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let doc = MindMapDocument::load(&path).expect("testament loads");
        // Find a node with at least 2 direct children.
        let target_id = doc
            .mindmap
            .nodes
            .keys()
            .find(|id| doc.mindmap.children_of(id).len() >= 2)
            .expect("testament has a node with children")
            .clone();
        let child_ids: Vec<String> = doc
            .mindmap
            .children_of(&target_id)
            .iter()
            .map(|n| n.id.clone())
            .collect();
        (doc, target_id, child_ids)
    }

    #[test]
    fn apply_moves_every_child_to_a_unique_position() {
        let (mut doc, target_id, child_ids) = test_doc_with_children();
        apply(&mut doc, &target_id);
        // Every child has a position; no two children share one.
        let mut positions: Vec<(f64, f64)> = Vec::new();
        for id in &child_ids {
            let n = doc.mindmap.nodes.get(id).unwrap();
            positions.push((n.position.x, n.position.y));
        }
        for i in 0..positions.len() {
            for j in i + 1..positions.len() {
                let (a, b) = (positions[i], positions[j]);
                assert!(
                    (a.0 - b.0).abs() > 1e-3 || (a.1 - b.1).abs() > 1e-3,
                    "children {} and {} landed on the same point",
                    child_ids[i],
                    child_ids[j]
                );
            }
        }
    }

    #[test]
    fn apply_centers_children_roughly_on_parent_midpoint() {
        let (mut doc, target_id, child_ids) = test_doc_with_children();
        let parent = doc.mindmap.nodes.get(&target_id).cloned().unwrap();
        let cx = parent.position.x + parent.size.width / 2.0;
        let cy = parent.position.y + parent.size.height / 2.0;
        apply(&mut doc, &target_id);
        // Centroid of child centres should be near the parent's centre
        // (within one padding-radius — the layout is rotationally
        // symmetric around the parent so this holds as a sanity check).
        let (mut sx, mut sy) = (0.0_f64, 0.0_f64);
        for id in &child_ids {
            let n = doc.mindmap.nodes.get(id).unwrap();
            sx += n.position.x + n.size.width / 2.0;
            sy += n.position.y + n.size.height / 2.0;
        }
        let n = child_ids.len() as f64;
        let (mx, my) = (sx / n, sy / n);
        assert!(
            (mx - cx).abs() < 2.0 * RADIAL_PADDING,
            "centroid x {} drifted from parent centre {}",
            mx,
            cx
        );
        assert!(
            (my - cy).abs() < 2.0 * RADIAL_PADDING,
            "centroid y {} drifted from parent centre {}",
            my,
            cy
        );
    }

    #[test]
    fn apply_on_leaf_node_is_noop() {
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
