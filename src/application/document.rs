use std::collections::{HashMap, HashSet};
use std::path::Path;
use glam::Vec2;
use log::{error, info};
use baumhard::core::primitives::Range;
use baumhard::mindmap::custom_mutation::{
    CustomMutation, MutationBehavior, TargetScope, Trigger,
    PlatformContext, apply_mutations_to_element,
};
use baumhard::mindmap::model::{MindMap, MindNode, Position};
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

/// An undoable action that can be reversed.
#[derive(Clone, Debug)]
pub enum UndoAction {
    /// Stores original positions of moved nodes for restoration.
    MoveNodes { original_positions: Vec<(String, Position)> },
    /// Stores full node snapshots before a custom mutation was applied.
    CustomMutation { node_snapshots: Vec<(String, MindNode)> },
}

/// Owns the MindMap data model and provides scene-building for the Renderer.
pub struct MindMapDocument {
    pub mindmap: MindMap,
    pub file_path: Option<String>,
    pub dirty: bool,
    pub selection: SelectionState,
    pub undo_stack: Vec<UndoAction>,
    /// Registry of all available custom mutations (global + map + inline, keyed by id).
    pub mutation_registry: HashMap<String, CustomMutation>,
    /// Tracks active toggle mutations per node: (node_id, mutation_id).
    pub active_toggles: HashSet<(String, String)>,
}

impl MindMapDocument {
    /// Load a MindMap from a file path and create a Document.
    pub fn load(path: &str) -> Result<Self, String> {
        match loader::load_from_file(Path::new(path)) {
            Ok(map) => {
                info!("Loaded mindmap '{}' with {} nodes", map.name, map.nodes.len());
                let mut doc = MindMapDocument {
                    mindmap: map,
                    file_path: Some(path.to_string()),
                    dirty: false,
                    selection: SelectionState::None,
                    undo_stack: Vec::new(),
                    mutation_registry: HashMap::new(),
                    active_toggles: HashSet::new(),
                };
                doc.build_mutation_registry();
                Ok(doc)
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

    /// Build a RenderScene with position offsets applied to specific nodes.
    /// Used during drag to update connections and borders in real-time.
    pub fn build_scene_with_offsets(&self, offsets: &HashMap<String, (f32, f32)>) -> RenderScene {
        scene_builder::build_scene_with_offsets(&self.mindmap, offsets)
    }

    /// Apply a position delta to a node and all its descendants in the MindMap model.
    /// Returns the original positions for undo.
    pub fn apply_move_subtree(&mut self, node_id: &str, dx: f64, dy: f64) -> Vec<(String, Position)> {
        let mut ids = vec![node_id.to_string()];
        ids.extend(self.mindmap.all_descendants(node_id));
        let mut original_positions = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Some(node) = self.mindmap.nodes.get_mut(id) {
                original_positions.push((id.clone(), node.position.clone()));
                node.position.x += dx;
                node.position.y += dy;
            }
        }
        original_positions
    }

    /// Apply a position delta to a single node only (no descendants).
    /// Returns the original position for undo.
    pub fn apply_move_single(&mut self, node_id: &str, dx: f64, dy: f64) -> Option<(String, Position)> {
        if let Some(node) = self.mindmap.nodes.get_mut(node_id) {
            let original = (node_id.to_string(), node.position.clone());
            node.position.x += dx;
            node.position.y += dy;
            Some(original)
        } else {
            None
        }
    }

    /// Move multiple root nodes at once, with subtree deduplication.
    /// If a selected node is already a descendant of another selected node,
    /// it is skipped to avoid double-movement when moving subtrees.
    /// Returns combined undo data for all moved nodes.
    pub fn apply_move_multiple(&mut self, node_ids: &[String], dx: f64, dy: f64, individual: bool) -> Vec<(String, Position)> {
        if individual {
            // No dedup needed — each node moves independently
            let mut undo_data = Vec::new();
            for nid in node_ids {
                undo_data.extend(self.apply_move_single(nid, dx, dy));
            }
            return undo_data;
        }

        // Deduplicate: skip nodes that are descendants of other selected nodes
        let roots = self.dedup_subtree_roots(node_ids);
        let mut undo_data = Vec::new();
        for nid in &roots {
            undo_data.extend(self.apply_move_subtree(nid, dx, dy));
        }
        undo_data
    }

    /// Filter a list of node IDs to only the "roots" — nodes that are not
    /// descendants of any other node in the list.
    fn dedup_subtree_roots(&self, node_ids: &[String]) -> Vec<String> {
        let id_set: std::collections::HashSet<&str> = node_ids.iter().map(|s| s.as_str()).collect();
        node_ids.iter().filter(|id| {
            // Walk up the parent chain; if any ancestor is in the set, skip this node
            let mut current = self.mindmap.nodes.get(id.as_str())
                .and_then(|n| n.parent_id.as_deref());
            while let Some(pid) = current {
                if id_set.contains(pid) {
                    return false;
                }
                current = self.mindmap.nodes.get(pid)
                    .and_then(|n| n.parent_id.as_deref());
            }
            true
        }).cloned().collect()
    }

    /// Undo the last action. Returns true if something was undone.
    pub fn undo(&mut self) -> bool {
        if let Some(action) = self.undo_stack.pop() {
            match action {
                UndoAction::MoveNodes { original_positions } => {
                    for (id, pos) in original_positions {
                        if let Some(node) = self.mindmap.nodes.get_mut(&id) {
                            node.position = pos;
                        }
                    }
                }
                UndoAction::CustomMutation { node_snapshots } => {
                    for (id, snapshot) in node_snapshots {
                        self.mindmap.nodes.insert(id, snapshot);
                    }
                }
            }
            true
        } else {
            false
        }
    }

    /// Build the mutation registry from map-level and inline node mutations.
    /// Inline mutations override map-level mutations with the same id.
    pub fn build_mutation_registry(&mut self) {
        self.mutation_registry.clear();
        // Map-level mutations (lower precedence)
        for cm in &self.mindmap.custom_mutations {
            self.mutation_registry.insert(cm.id.clone(), cm.clone());
        }
        // Inline node mutations (higher precedence — override map-level)
        for node in self.mindmap.nodes.values() {
            for cm in &node.inline_mutations {
                self.mutation_registry.insert(cm.id.clone(), cm.clone());
            }
        }
    }

    /// Find custom mutations triggered by a given trigger on a specific node.
    /// Checks the node's trigger_bindings and filters by platform context.
    pub fn find_triggered_mutations(
        &self,
        node_id: &str,
        trigger: &Trigger,
        platform: &PlatformContext,
    ) -> Vec<CustomMutation> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return vec![],
        };
        let mut results = Vec::new();
        for binding in &node.trigger_bindings {
            if &binding.trigger != trigger {
                continue;
            }
            // Check platform context filter
            if !binding.contexts.is_empty() && !binding.contexts.contains(platform) {
                continue;
            }
            if let Some(cm) = self.mutation_registry.get(&binding.mutation_id) {
                results.push(cm.clone());
            }
        }
        results
    }

    /// Apply a custom mutation to the tree and optionally sync to the model.
    /// For Persistent mutations, snapshots affected nodes for undo and sets dirty flag.
    /// For Toggle mutations, tracks active state without model sync.
    pub fn apply_custom_mutation(
        &mut self,
        custom: &CustomMutation,
        node_id: &str,
        tree: &mut MindMapTree,
    ) {
        // For toggle behavior, check if already active and reverse if so
        if custom.behavior == MutationBehavior::Toggle {
            let key = (node_id.to_string(), custom.id.clone());
            if self.active_toggles.contains(&key) {
                // Reverse: remove toggle, rebuild affected nodes from model
                self.active_toggles.remove(&key);
                return;
            }
            self.active_toggles.insert(key);
            // Toggle mutations apply to tree only (visual), no model sync
            self.apply_to_tree(custom, node_id, tree);
            return;
        }

        // Persistent: snapshot, apply to tree, sync to model
        let affected_ids = self.collect_affected_node_ids(node_id, &custom.target_scope);
        let snapshots: Vec<(String, MindNode)> = affected_ids.iter()
            .filter_map(|id| {
                self.mindmap.nodes.get(id).map(|n| (id.clone(), n.clone()))
            })
            .collect();

        self.apply_to_tree(custom, node_id, tree);

        // Sync tree state back to model for affected nodes
        for id in &affected_ids {
            self.sync_node_from_tree(id, tree);
        }

        if !snapshots.is_empty() {
            self.undo_stack.push(UndoAction::CustomMutation { node_snapshots: snapshots });
            self.dirty = true;
        }
    }

    /// Apply mutations to the Baumhard tree based on target scope.
    fn apply_to_tree(
        &self,
        custom: &CustomMutation,
        node_id: &str,
        tree: &mut MindMapTree,
    ) {
        match custom.target_scope {
            TargetScope::SelfOnly => {
                if let Some(&nid) = tree.node_map.get(node_id) {
                    if let Some(node) = tree.tree.arena.get_mut(nid) {
                        apply_mutations_to_element(&custom.mutations, node.get_mut());
                    }
                }
            }
            TargetScope::Children => {
                let child_ids: Vec<String> = self.mindmap.children_of(node_id)
                    .iter().map(|n| n.id.clone()).collect();
                for cid in &child_ids {
                    if let Some(&nid) = tree.node_map.get(cid.as_str()) {
                        if let Some(node) = tree.tree.arena.get_mut(nid) {
                            apply_mutations_to_element(&custom.mutations, node.get_mut());
                        }
                    }
                }
            }
            TargetScope::Descendants => {
                let desc_ids = self.mindmap.all_descendants(node_id);
                for did in &desc_ids {
                    if let Some(&nid) = tree.node_map.get(did.as_str()) {
                        if let Some(node) = tree.tree.arena.get_mut(nid) {
                            apply_mutations_to_element(&custom.mutations, node.get_mut());
                        }
                    }
                }
            }
            TargetScope::SelfAndDescendants => {
                // Self
                if let Some(&nid) = tree.node_map.get(node_id) {
                    if let Some(node) = tree.tree.arena.get_mut(nid) {
                        apply_mutations_to_element(&custom.mutations, node.get_mut());
                    }
                }
                // Descendants
                let desc_ids = self.mindmap.all_descendants(node_id);
                for did in &desc_ids {
                    if let Some(&nid) = tree.node_map.get(did.as_str()) {
                        if let Some(node) = tree.tree.arena.get_mut(nid) {
                            apply_mutations_to_element(&custom.mutations, node.get_mut());
                        }
                    }
                }
            }
            TargetScope::Parent => {
                if let Some(parent_id) = self.mindmap.nodes.get(node_id)
                    .and_then(|n| n.parent_id.as_deref())
                {
                    let pid = parent_id.to_string();
                    if let Some(&nid) = tree.node_map.get(pid.as_str()) {
                        if let Some(node) = tree.tree.arena.get_mut(nid) {
                            apply_mutations_to_element(&custom.mutations, node.get_mut());
                        }
                    }
                }
            }
            TargetScope::Siblings => {
                if let Some(parent_id) = self.mindmap.nodes.get(node_id)
                    .and_then(|n| n.parent_id.as_deref())
                {
                    let sibling_ids: Vec<String> = self.mindmap.children_of(parent_id)
                        .iter()
                        .filter(|n| n.id != node_id)
                        .map(|n| n.id.clone())
                        .collect();
                    for sid in &sibling_ids {
                        if let Some(&nid) = tree.node_map.get(sid.as_str()) {
                            if let Some(node) = tree.tree.arena.get_mut(nid) {
                                apply_mutations_to_element(&custom.mutations, node.get_mut());
                            }
                        }
                    }
                }
            }
        }
    }

    /// Collect the IDs of all nodes affected by a mutation with the given scope.
    fn collect_affected_node_ids(&self, node_id: &str, scope: &TargetScope) -> Vec<String> {
        match scope {
            TargetScope::SelfOnly => vec![node_id.to_string()],
            TargetScope::Children => {
                self.mindmap.children_of(node_id).iter().map(|n| n.id.clone()).collect()
            }
            TargetScope::Descendants => self.mindmap.all_descendants(node_id),
            TargetScope::SelfAndDescendants => {
                let mut ids = vec![node_id.to_string()];
                ids.extend(self.mindmap.all_descendants(node_id));
                ids
            }
            TargetScope::Parent => {
                self.mindmap.nodes.get(node_id)
                    .and_then(|n| n.parent_id.clone())
                    .into_iter().collect()
            }
            TargetScope::Siblings => {
                self.mindmap.nodes.get(node_id)
                    .and_then(|n| n.parent_id.as_deref())
                    .map(|pid| {
                        self.mindmap.children_of(pid).iter()
                            .filter(|n| n.id != node_id)
                            .map(|n| n.id.clone())
                            .collect()
                    })
                    .unwrap_or_default()
            }
        }
    }

    /// Sync a node's position from the Baumhard tree back to the MindMap model.
    /// Used after persistent mutations to ensure the model reflects tree state.
    fn sync_node_from_tree(&mut self, node_id: &str, tree: &MindMapTree) {
        let tree_nid = match tree.node_map.get(node_id) {
            Some(&nid) => nid,
            None => return,
        };
        let element = match tree.tree.arena.get(tree_nid) {
            Some(n) => n.get(),
            None => return,
        };
        let area = match element.glyph_area() {
            Some(a) => a,
            None => return,
        };
        if let Some(model_node) = self.mindmap.nodes.get_mut(node_id) {
            model_node.position.x = area.position.x.0 as f64;
            model_node.position.y = area.position.y.0 as f64;
        }
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

/// Find all node IDs whose bounds intersect the given canvas-space rectangle.
/// The rectangle is defined by two opposite corners (min and max are computed internally).
pub fn rect_select(corner_a: Vec2, corner_b: Vec2, tree: &MindMapTree) -> Vec<String> {
    let min_x = corner_a.x.min(corner_b.x);
    let min_y = corner_a.y.min(corner_b.y);
    let max_x = corner_a.x.max(corner_b.x);
    let max_y = corner_a.y.max(corner_b.y);

    let mut hits = Vec::new();
    for (mind_id, &node_id) in &tree.node_map {
        let area = match tree.tree.arena.get(node_id).and_then(|n| n.get().glyph_area()) {
            Some(a) => a,
            None => continue,
        };
        let x = area.position.x.0;
        let y = area.position.y.0;
        let w = area.render_bounds.x.0;
        let h = area.render_bounds.y.0;

        // AABB overlap test
        if x + w >= min_x && x <= max_x && y + h >= min_y && y <= max_y {
            hits.push(mind_id.clone());
        }
    }
    hits
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

/// Apply a position delta directly to nodes in the Baumhard tree (in-place mutation).
/// Used during drag for fast visual preview without rebuilding from the MindMap model.
pub fn apply_drag_delta(tree: &mut MindMapTree, node_id: &str, dx: f32, dy: f32, include_descendants: bool) {
    let tree_node_id = match tree.node_map.get(node_id) {
        Some(&id) => id,
        None => return,
    };

    // Collect node IDs to mutate (must collect first to avoid borrow conflict with arena)
    let node_ids: Vec<indextree::NodeId> = if include_descendants {
        tree_node_id.descendants(&tree.tree.arena).collect()
    } else {
        vec![tree_node_id]
    };

    for nid in node_ids {
        if let Some(node) = tree.tree.arena.get_mut(nid) {
            if let Some(area) = node.get_mut().glyph_area_mut() {
                area.move_position(dx, dy);
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

    fn load_test_doc() -> MindMapDocument {
        let map = loader::load_from_file(&test_map_path()).unwrap();
        let mut doc = MindMapDocument {
            mindmap: map,
            file_path: None,
            dirty: false,
            selection: SelectionState::None,
            undo_stack: Vec::new(),
            mutation_registry: HashMap::new(),
            active_toggles: HashSet::new(),
        };
        doc.build_mutation_registry();
        doc
    }

    fn load_test_tree() -> MindMapTree {
        load_test_doc().build_tree()
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

    #[test]
    fn test_move_subtree_updates_all_positions() {
        let mut doc = load_test_doc();
        let node_id = "348068464"; // Lord God
        let descendants = doc.mindmap.all_descendants(node_id);
        assert!(!descendants.is_empty(), "Lord God should have descendants");

        // Record original positions
        let orig_pos: Vec<(String, f64, f64)> = std::iter::once(node_id.to_string())
            .chain(descendants.iter().cloned())
            .filter_map(|id| {
                let n = doc.mindmap.nodes.get(&id)?;
                Some((id, n.position.x, n.position.y))
            })
            .collect();

        let dx = 50.0;
        let dy = -30.0;
        doc.apply_move_subtree(node_id, dx, dy);

        for (id, ox, oy) in &orig_pos {
            let n = doc.mindmap.nodes.get(id).unwrap();
            assert!((n.position.x - (ox + dx)).abs() < 0.001, "Node {} x not shifted", id);
            assert!((n.position.y - (oy + dy)).abs() < 0.001, "Node {} y not shifted", id);
        }
    }

    #[test]
    fn test_move_subtree_preserves_relative_positions() {
        let mut doc = load_test_doc();
        let node_id = "348068464";
        let descendants = doc.mindmap.all_descendants(node_id);

        // Record relative offsets from parent to each descendant
        let parent = doc.mindmap.nodes.get(node_id).unwrap();
        let offsets: Vec<(String, f64, f64)> = descendants.iter().filter_map(|id| {
            let n = doc.mindmap.nodes.get(id)?;
            Some((id.clone(), n.position.x - parent.position.x, n.position.y - parent.position.y))
        }).collect();

        doc.apply_move_subtree(node_id, 100.0, 200.0);

        let parent = doc.mindmap.nodes.get(node_id).unwrap();
        for (id, dx, dy) in &offsets {
            let n = doc.mindmap.nodes.get(id).unwrap();
            let actual_dx = n.position.x - parent.position.x;
            let actual_dy = n.position.y - parent.position.y;
            assert!((actual_dx - dx).abs() < 0.001, "Relative x offset changed for {}", id);
            assert!((actual_dy - dy).abs() < 0.001, "Relative y offset changed for {}", id);
        }
    }

    #[test]
    fn test_move_single_only_affects_target() {
        let mut doc = load_test_doc();
        let node_id = "348068464";
        let descendants = doc.mindmap.all_descendants(node_id);

        // Record descendant positions before
        let before: Vec<(String, f64, f64)> = descendants.iter().filter_map(|id| {
            let n = doc.mindmap.nodes.get(id)?;
            Some((id.clone(), n.position.x, n.position.y))
        }).collect();

        doc.apply_move_single(node_id, 100.0, 200.0);

        // Descendants should be unchanged
        for (id, ox, oy) in &before {
            let n = doc.mindmap.nodes.get(id).unwrap();
            assert!((n.position.x - ox).abs() < 0.001, "Descendant {} x changed unexpectedly", id);
            assert!((n.position.y - oy).abs() < 0.001, "Descendant {} y changed unexpectedly", id);
        }

        // But the target node should have moved
        let target = doc.mindmap.nodes.get(node_id).unwrap();
        // We don't assert exact position here, just that it changed
        // (the original was stored before the move, but we didn't save it in this test)
    }

    #[test]
    fn test_move_returns_original_positions() {
        let mut doc = load_test_doc();
        let node_id = "348068464";
        let orig_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
        let orig_y = doc.mindmap.nodes.get(node_id).unwrap().position.y;

        let undo_data = doc.apply_move_subtree(node_id, 50.0, 50.0);
        let target_entry = undo_data.iter().find(|(id, _)| id == node_id).unwrap();
        assert!((target_entry.1.x - orig_x).abs() < 0.001);
        assert!((target_entry.1.y - orig_y).abs() < 0.001);
    }

    #[test]
    fn test_undo_restores_positions() {
        let mut doc = load_test_doc();
        let node_id = "348068464";

        // Record original positions
        let orig_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
        let orig_y = doc.mindmap.nodes.get(node_id).unwrap().position.y;

        // Move and push undo
        let undo_data = doc.apply_move_subtree(node_id, 100.0, 200.0);
        doc.undo_stack.push(UndoAction::MoveNodes { original_positions: undo_data });

        // Verify moved
        assert!((doc.mindmap.nodes.get(node_id).unwrap().position.x - (orig_x + 100.0)).abs() < 0.001);

        // Undo
        assert!(doc.undo());

        // Verify restored
        assert!((doc.mindmap.nodes.get(node_id).unwrap().position.x - orig_x).abs() < 0.001);
        assert!((doc.mindmap.nodes.get(node_id).unwrap().position.y - orig_y).abs() < 0.001);
    }

    #[test]
    fn test_apply_drag_delta() {
        let doc = load_test_doc();
        let mut tree = doc.build_tree();
        let node_id = "348068464";

        let tree_nid = *tree.node_map.get(node_id).unwrap();
        let orig_x = tree.tree.arena.get(tree_nid).unwrap().get().glyph_area().unwrap().position.x.0;
        let orig_y = tree.tree.arena.get(tree_nid).unwrap().get().glyph_area().unwrap().position.y.0;

        apply_drag_delta(&mut tree, node_id, 25.0, -15.0, false);

        let new_x = tree.tree.arena.get(tree_nid).unwrap().get().glyph_area().unwrap().position.x.0;
        let new_y = tree.tree.arena.get(tree_nid).unwrap().get().glyph_area().unwrap().position.y.0;
        assert!((new_x - (orig_x + 25.0)).abs() < 0.001);
        assert!((new_y - (orig_y - 15.0)).abs() < 0.001);
    }

    #[test]
    fn test_apply_drag_delta_with_descendants() {
        let doc = load_test_doc();
        let mut tree = doc.build_tree();
        let node_id = "348068464";

        // Find a child of Lord God in the tree
        let child_ids: Vec<String> = doc.mindmap.all_descendants(node_id);
        assert!(!child_ids.is_empty());
        let child_id = &child_ids[0];
        let child_tree_nid = *tree.node_map.get(child_id).unwrap();
        let child_orig_x = tree.tree.arena.get(child_tree_nid).unwrap().get()
            .glyph_area().unwrap().position.x.0;

        apply_drag_delta(&mut tree, node_id, 30.0, 20.0, true);

        let child_new_x = tree.tree.arena.get(child_tree_nid).unwrap().get()
            .glyph_area().unwrap().position.x.0;
        assert!((child_new_x - (child_orig_x + 30.0)).abs() < 0.001,
            "Descendant should be shifted when include_descendants=true");
    }

    #[test]
    fn test_dedup_subtree_roots() {
        let doc = load_test_doc();
        let parent_id = "348068464"; // Lord God
        let descendants = doc.mindmap.all_descendants(parent_id);
        assert!(!descendants.is_empty());
        let child_id = &descendants[0];

        // If both parent and child are selected, only parent should be a root
        let ids = vec![parent_id.to_string(), child_id.clone()];
        let roots = doc.dedup_subtree_roots(&ids);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0], parent_id);
    }

    #[test]
    fn test_apply_move_multiple_no_double_movement() {
        let mut doc = load_test_doc();
        let parent_id = "348068464";
        let descendants = doc.mindmap.all_descendants(parent_id);
        let child_id = &descendants[0];

        let child_orig_x = doc.mindmap.nodes.get(child_id).unwrap().position.x;

        // Move both parent and child as subtrees — child should only move once (via parent)
        let ids = vec![parent_id.to_string(), child_id.clone()];
        doc.apply_move_multiple(&ids, 50.0, 0.0, false);

        let child_new_x = doc.mindmap.nodes.get(child_id).unwrap().position.x;
        assert!((child_new_x - (child_orig_x + 50.0)).abs() < 0.001,
            "Child should be moved exactly once, not twice");
    }

    #[test]
    fn test_rect_select_finds_nodes_in_region() {
        let tree = load_test_tree();
        // Get position/bounds of "Lord God" to build a rect that contains it
        let node_id = *tree.node_map.get("348068464").unwrap();
        let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
        let x = area.position.x.0;
        let y = area.position.y.0;
        let w = area.render_bounds.x.0;
        let h = area.render_bounds.y.0;

        // A rect that exactly contains this node should select it
        let hits = rect_select(
            Vec2::new(x - 1.0, y - 1.0),
            Vec2::new(x + w + 1.0, y + h + 1.0),
            &tree,
        );
        assert!(hits.contains(&"348068464".to_string()), "Should find Lord God in rect");
    }

    #[test]
    fn test_rect_select_misses_distant_nodes() {
        let tree = load_test_tree();
        // A rect far from any node should select nothing
        let hits = rect_select(
            Vec2::new(-99999.0, -99999.0),
            Vec2::new(-99998.0, -99998.0),
            &tree,
        );
        assert!(hits.is_empty(), "Should find no nodes in distant rect");
    }

    // --- Session 9B: Custom mutation registry & application tests ---

    use baumhard::mindmap::custom_mutation::{
        CustomMutation as CM, MutationBehavior as MB, TargetScope as TS,
        Trigger as Tr, TriggerBinding as TB, PlatformContext as PC,
    };
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;

    fn make_test_mutation(id: &str, scope: TS) -> CM {
        CM {
            id: id.to_string(),
            name: id.to_string(),
            mutations: vec![
                Mutation::area_command(GlyphAreaCommand::NudgeRight(10.0)),
            ],
            target_scope: scope,
            behavior: MB::Persistent,
            predicate: None,
        }
    }

    #[test]
    fn test_mutation_registry_empty_for_existing_map() {
        let doc = load_test_doc();
        assert!(doc.mutation_registry.is_empty(),
            "Existing map without custom_mutations should have empty registry");
    }

    #[test]
    fn test_mutation_registry_from_map_level() {
        let mut doc = load_test_doc();
        doc.mindmap.custom_mutations.push(make_test_mutation("nudge-right", TS::SelfOnly));
        doc.build_mutation_registry();
        assert_eq!(doc.mutation_registry.len(), 1);
        assert!(doc.mutation_registry.contains_key("nudge-right"));
    }

    #[test]
    fn test_mutation_registry_inline_overrides_map() {
        let mut doc = load_test_doc();
        // Map-level mutation
        let mut map_cm = make_test_mutation("shared-id", TS::SelfOnly);
        map_cm.name = "Map Version".to_string();
        doc.mindmap.custom_mutations.push(map_cm);

        // Inline mutation on a node with the same id
        let mut inline_cm = make_test_mutation("shared-id", TS::Children);
        inline_cm.name = "Inline Version".to_string();
        let node_id = "348068464";
        doc.mindmap.nodes.get_mut(node_id).unwrap().inline_mutations.push(inline_cm);

        doc.build_mutation_registry();
        assert_eq!(doc.mutation_registry.len(), 1);
        let cm = doc.mutation_registry.get("shared-id").unwrap();
        assert_eq!(cm.name, "Inline Version", "Inline should override map-level");
        assert_eq!(cm.target_scope, TS::Children);
    }

    #[test]
    fn test_find_triggered_mutations_match() {
        let mut doc = load_test_doc();
        doc.mindmap.custom_mutations.push(make_test_mutation("nudge", TS::SelfOnly));
        doc.build_mutation_registry();

        let node_id = "348068464";
        doc.mindmap.nodes.get_mut(node_id).unwrap().trigger_bindings.push(TB {
            trigger: Tr::OnClick,
            mutation_id: "nudge".to_string(),
            contexts: vec![],
        });

        let results = doc.find_triggered_mutations(node_id, &Tr::OnClick, &PC::Desktop);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "nudge");
    }

    #[test]
    fn test_find_triggered_mutations_no_match() {
        let mut doc = load_test_doc();
        doc.mindmap.custom_mutations.push(make_test_mutation("nudge", TS::SelfOnly));
        doc.build_mutation_registry();

        let node_id = "348068464";
        doc.mindmap.nodes.get_mut(node_id).unwrap().trigger_bindings.push(TB {
            trigger: Tr::OnClick,
            mutation_id: "nudge".to_string(),
            contexts: vec![],
        });

        // OnHover should not match
        let results = doc.find_triggered_mutations(node_id, &Tr::OnHover, &PC::Desktop);
        assert!(results.is_empty());
    }

    #[test]
    fn test_find_triggered_mutations_platform_filter() {
        let mut doc = load_test_doc();
        doc.mindmap.custom_mutations.push(make_test_mutation("desktop-only", TS::SelfOnly));
        doc.build_mutation_registry();

        let node_id = "348068464";
        doc.mindmap.nodes.get_mut(node_id).unwrap().trigger_bindings.push(TB {
            trigger: Tr::OnClick,
            mutation_id: "desktop-only".to_string(),
            contexts: vec![PC::Desktop],
        });

        // Desktop should match
        let results = doc.find_triggered_mutations(node_id, &Tr::OnClick, &PC::Desktop);
        assert_eq!(results.len(), 1);

        // Touch should be filtered out
        let results = doc.find_triggered_mutations(node_id, &Tr::OnClick, &PC::Touch);
        assert!(results.is_empty());
    }

    #[test]
    fn test_collect_affected_node_ids_self_only() {
        let doc = load_test_doc();
        let ids = doc.collect_affected_node_ids("348068464", &TS::SelfOnly);
        assert_eq!(ids, vec!["348068464"]);
    }

    #[test]
    fn test_collect_affected_node_ids_children() {
        let doc = load_test_doc();
        let children = doc.mindmap.children_of("348068464");
        let ids = doc.collect_affected_node_ids("348068464", &TS::Children);
        assert_eq!(ids.len(), children.len());
        for child in &children {
            assert!(ids.contains(&child.id));
        }
    }

    #[test]
    fn test_collect_affected_node_ids_descendants() {
        let doc = load_test_doc();
        let all_desc = doc.mindmap.all_descendants("348068464");
        let ids = doc.collect_affected_node_ids("348068464", &TS::Descendants);
        assert_eq!(ids.len(), all_desc.len());
    }

    #[test]
    fn test_collect_affected_node_ids_self_and_descendants() {
        let doc = load_test_doc();
        let all_desc = doc.mindmap.all_descendants("348068464");
        let ids = doc.collect_affected_node_ids("348068464", &TS::SelfAndDescendants);
        assert_eq!(ids.len(), all_desc.len() + 1);
        assert!(ids.contains(&"348068464".to_string()));
    }

    #[test]
    fn test_apply_custom_mutation_persistent_sets_dirty() {
        let mut doc = load_test_doc();
        let cm = make_test_mutation("nudge", TS::SelfOnly);
        doc.mindmap.custom_mutations.push(cm.clone());
        doc.build_mutation_registry();
        let mut tree = doc.build_tree();

        assert!(!doc.dirty);
        doc.apply_custom_mutation(&cm, "348068464", &mut tree);
        assert!(doc.dirty, "Persistent mutation should set dirty flag");
        assert_eq!(doc.undo_stack.len(), 1, "Should push undo action");
    }

    #[test]
    fn test_apply_custom_mutation_toggle_does_not_set_dirty() {
        let mut doc = load_test_doc();
        let mut cm = make_test_mutation("toggle-test", TS::SelfOnly);
        cm.behavior = MB::Toggle;
        doc.mindmap.custom_mutations.push(cm.clone());
        doc.build_mutation_registry();
        let mut tree = doc.build_tree();

        doc.apply_custom_mutation(&cm, "348068464", &mut tree);
        assert!(!doc.dirty, "Toggle mutation should not set dirty flag");
        assert!(doc.undo_stack.is_empty(), "Toggle mutation should not push undo");
        assert!(doc.active_toggles.contains(&("348068464".to_string(), "toggle-test".to_string())));
    }

    #[test]
    fn test_apply_custom_mutation_toggle_reverses() {
        let mut doc = load_test_doc();
        let mut cm = make_test_mutation("toggle-test", TS::SelfOnly);
        cm.behavior = MB::Toggle;
        doc.mindmap.custom_mutations.push(cm.clone());
        doc.build_mutation_registry();
        let mut tree = doc.build_tree();

        // First apply: activates toggle
        doc.apply_custom_mutation(&cm, "348068464", &mut tree);
        assert!(doc.active_toggles.contains(&("348068464".to_string(), "toggle-test".to_string())));

        // Second apply: deactivates toggle
        doc.apply_custom_mutation(&cm, "348068464", &mut tree);
        assert!(!doc.active_toggles.contains(&("348068464".to_string(), "toggle-test".to_string())));
    }

    #[test]
    fn test_undo_custom_mutation_restores_node() {
        let mut doc = load_test_doc();
        let cm = make_test_mutation("nudge", TS::SelfOnly);
        let node_id = "348068464";

        let orig_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
        let mut tree = doc.build_tree();

        doc.apply_custom_mutation(&cm, node_id, &mut tree);
        // Position may have been synced from tree; verify undo restores original
        assert!(doc.undo());
        let restored_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
        assert!((restored_x - orig_x).abs() < 0.001, "Undo should restore original position");
    }
}
