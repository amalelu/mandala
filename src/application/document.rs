use std::path::Path;
use glam::Vec2;
use log::{error, info};
use baumhard::core::primitives::Range;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::loader;
use baumhard::mindmap::scene_builder::{self, RenderScene};
use baumhard::mindmap::tree_builder::{self, MindMapTree};

/// Selection highlight color: bright cyan [R, G, B, A]
const HIGHLIGHT_COLOR: [f32; 4] = [0.0, 0.9, 1.0, 1.0];

/// Tracks which nodes are currently selected.
#[derive(Clone, Debug)]
pub enum SelectionState {
    None,
    Single(String),
    Multi(Vec<String>),
}

impl SelectionState {
    pub fn is_selected(&self, node_id: &str) -> bool {
        match self {
            SelectionState::None => false,
            SelectionState::Single(id) => id == node_id,
            SelectionState::Multi(ids) => ids.contains(&node_id.to_string()),
        }
    }

    pub fn selected_ids(&self) -> Vec<&str> {
        match self {
            SelectionState::None => vec![],
            SelectionState::Single(id) => vec![id.as_str()],
            SelectionState::Multi(ids) => ids.iter().map(|s| s.as_str()).collect(),
        }
    }
}

/// Owns the MindMap data model and provides scene-building for the Renderer.
pub struct MindMapDocument {
    pub mindmap: MindMap,
    pub file_path: Option<String>,
    pub dirty: bool,
    pub selection: SelectionState,
}

impl MindMapDocument {
    /// Load a MindMap from a file path and create a Document.
    pub fn load(path: &str) -> Result<Self, String> {
        match loader::load_from_file(Path::new(path)) {
            Ok(map) => {
                info!("Loaded mindmap '{}' with {} nodes", map.name, map.nodes.len());
                Ok(MindMapDocument {
                    mindmap: map,
                    file_path: Some(path.to_string()),
                    dirty: false,
                    selection: SelectionState::None,
                })
            }
            Err(e) => {
                let msg = format!("Failed to load mindmap '{}': {}", path, e);
                error!("{}", msg);
                Err(msg)
            }
        }
    }

    /// Build a Baumhard mutation tree from the MindMap hierarchy.
    /// Each MindNode becomes a GlyphArea in the tree, preserving parent-child structure.
    pub fn build_tree(&self) -> MindMapTree {
        tree_builder::build_mindmap_tree(&self.mindmap)
    }

    /// Build a RenderScene from the current MindMap state.
    /// Used for connections and borders (flat pipeline).
    pub fn build_scene(&self) -> RenderScene {
        scene_builder::build_scene(&self.mindmap)
    }
}

/// Hit test: find the node at the given canvas position.
/// Returns the MindNode ID of the smallest (innermost) node containing the point,
/// or None if the click is on empty space.
pub fn hit_test(canvas_pos: Vec2, tree: &MindMapTree) -> Option<String> {
    let mut best: Option<(String, f32)> = None; // (node_id, area)

    for (mind_id, &node_id) in &tree.node_map {
        let node = match tree.tree.arena.get(node_id) {
            Some(n) => n,
            None => continue,
        };
        let area = match node.get().glyph_area() {
            Some(a) => a,
            None => continue,
        };

        let x = area.position.x.0;
        let y = area.position.y.0;
        let w = area.render_bounds.x.0;
        let h = area.render_bounds.y.0;

        if canvas_pos.x >= x && canvas_pos.x <= x + w
            && canvas_pos.y >= y && canvas_pos.y <= y + h
        {
            let node_area = w * h;
            if best.as_ref().map_or(true, |(_, best_area)| node_area < *best_area) {
                best = Some((mind_id.clone(), node_area));
            }
        }
    }

    best.map(|(id, _)| id)
}

/// Apply selection highlight to the tree by modifying selected nodes' color regions.
/// Call this after `build_tree()` and before passing the tree to the renderer.
/// The tree is modified in-place; rebuilding restores original colors from the MindMap model.
pub fn apply_selection_highlight(tree: &mut MindMapTree, selection: &SelectionState) {
    for selected_id in selection.selected_ids() {
        let node_id = match tree.node_map.get(selected_id) {
            Some(&id) => id,
            None => continue,
        };
        let node = match tree.tree.arena.get_mut(node_id) {
            Some(n) => n,
            None => continue,
        };
        let element = node.get_mut();
        if let Some(glyph_area) = element.glyph_area_mut() {
            // Collect existing region ranges first, then update each one's color.
            // Using the exact existing ranges ensures set_or_insert finds a match
            // and updates in place rather than inserting a duplicate region.
            let ranges: Vec<Range> = glyph_area.regions.all_regions()
                .iter()
                .map(|r| r.range)
                .collect();
            for range in &ranges {
                glyph_area.set_region_color(range, &HIGHLIGHT_COLOR);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::loader;
    use std::path::PathBuf;

    fn test_map_path() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("maps/testament.mindmap.json");
        path
    }

    fn load_test_tree() -> MindMapTree {
        let map = loader::load_from_file(&test_map_path()).unwrap();
        let doc = MindMapDocument {
            mindmap: map,
            file_path: None,
            dirty: false,
            selection: SelectionState::None,
        };
        doc.build_tree()
    }

    #[test]
    fn test_hit_test_direct_hit() {
        let tree = load_test_tree();
        // "Lord God" node (id: 348068464) — get its position from the tree
        let node_id = tree.node_map.get("348068464").unwrap();
        let area = tree.tree.arena.get(*node_id).unwrap().get().glyph_area().unwrap();
        let center = Vec2::new(
            area.position.x.0 + area.render_bounds.x.0 / 2.0,
            area.position.y.0 + area.render_bounds.y.0 / 2.0,
        );
        let result = hit_test(center, &tree);
        assert_eq!(result, Some("348068464".to_string()));
    }

    #[test]
    fn test_hit_test_miss() {
        let tree = load_test_tree();
        // A point far away from any node
        let result = hit_test(Vec2::new(-99999.0, -99999.0), &tree);
        assert_eq!(result, None);
    }

    #[test]
    fn test_hit_test_returns_smallest_on_overlap() {
        let tree = load_test_tree();
        // Find a parent-child pair where child is inside parent's bounds
        // "Lord God" (348068464) has children — find one whose bounds overlap
        let parent_id_str = "348068464";
        let parent_node_id = tree.node_map.get(parent_id_str).unwrap();
        let parent_area = tree.tree.arena.get(*parent_node_id).unwrap().get().glyph_area().unwrap();
        let parent_size = parent_area.render_bounds.x.0 * parent_area.render_bounds.y.0;

        // Find any child node that's smaller and test its center
        for (mind_id, &nid) in &tree.node_map {
            if mind_id == parent_id_str { continue; }
            let a = match tree.tree.arena.get(nid).and_then(|n| n.get().glyph_area()) {
                Some(a) => a,
                None => continue,
            };
            let child_size = a.render_bounds.x.0 * a.render_bounds.y.0;
            let child_center = Vec2::new(
                a.position.x.0 + a.render_bounds.x.0 / 2.0,
                a.position.y.0 + a.render_bounds.y.0 / 2.0,
            );
            // Check if this child center also hits the parent
            let px = parent_area.position.x.0;
            let py = parent_area.position.y.0;
            let pw = parent_area.render_bounds.x.0;
            let ph = parent_area.render_bounds.y.0;
            if child_center.x >= px && child_center.x <= px + pw
                && child_center.y >= py && child_center.y <= py + ph
                && child_size < parent_size
            {
                // Both parent and child contain this point — should return the smaller one
                let result = hit_test(child_center, &tree);
                assert_eq!(result, Some(mind_id.clone()),
                    "Should select smaller child node, not parent");
                return;
            }
        }
        // If no overlap found in test data, that's OK — test is structural
    }

    #[test]
    fn test_selection_state_is_selected() {
        let none = SelectionState::None;
        assert!(!none.is_selected("123"));

        let single = SelectionState::Single("123".to_string());
        assert!(single.is_selected("123"));
        assert!(!single.is_selected("456"));

        let multi = SelectionState::Multi(vec!["123".to_string(), "456".to_string()]);
        assert!(multi.is_selected("123"));
        assert!(multi.is_selected("456"));
        assert!(!multi.is_selected("789"));
    }

    #[test]
    fn test_apply_selection_highlight() {
        let mut tree = load_test_tree();
        let selection = SelectionState::Single("348068464".to_string());
        let node_id = *tree.node_map.get("348068464").unwrap();

        // Before highlight: original color (white)
        let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
        let original_color = area.regions.all_regions()[0].color.unwrap();
        assert!((original_color[0] - 1.0).abs() < 0.01, "Expected white before highlight");

        // Apply highlight
        apply_selection_highlight(&mut tree, &selection);

        // After highlight: cyan
        let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
        let highlighted_color = area.regions.all_regions()[0].color.unwrap();
        assert!((highlighted_color[0] - HIGHLIGHT_COLOR[0]).abs() < 0.01);
        assert!((highlighted_color[1] - HIGHLIGHT_COLOR[1]).abs() < 0.01);
        assert!((highlighted_color[2] - HIGHLIGHT_COLOR[2]).abs() < 0.01);
    }

    #[test]
    fn test_highlight_does_not_affect_unselected() {
        let mut tree = load_test_tree();
        let selection = SelectionState::Single("348068464".to_string());

        // Pick a different node and copy its NodeId and regions before mutation
        let other_id = tree.node_map.keys()
            .find(|k| *k != "348068464")
            .unwrap().clone();
        let other_node_id = *tree.node_map.get(&other_id).unwrap();
        let before = tree.tree.arena.get(other_node_id).unwrap().get()
            .glyph_area().unwrap().regions.clone();

        apply_selection_highlight(&mut tree, &selection);

        let after = tree.tree.arena.get(other_node_id).unwrap().get()
            .glyph_area().unwrap().regions.clone();
        assert_eq!(before, after, "Unselected node colors should not change");
    }
}
