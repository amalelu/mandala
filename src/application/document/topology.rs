//! Topology mutations — creating / deleting / moving / reparenting
//! nodes. Everything that reshapes which nodes exist, where they
//! sit, and who their parent is. Also carries `delete_node` —
//! the node-centric remove that also rips touching edges.

use glam::Vec2;

use baumhard::mindmap::model::{MindEdge, Position};

use super::defaults::{default_cross_link_edge, default_orphan_node, default_parent_child_edge};
use super::types::{EdgeRef, PortalRef, ReparentUndoData, SelectionState};
use super::undo_action::UndoAction;
use super::MindMapDocument;

impl MindMapDocument {
    /// Remove a node from the map, orphaning its immediate children (they
    /// become roots), and removing every edge that touched the node.
    pub fn delete_node(&mut self, node_id: &str) -> Option<UndoAction> {
        let node = self.mindmap.nodes.remove(node_id)?;

        // Orphan immediate children: promote each to a root with a fresh
        // root-level Dewey ID.
        let child_ids: Vec<String> = self
            .mindmap
            .nodes
            .values()
            .filter(|n| n.parent_id.as_deref() == Some(node_id))
            .map(|n| n.id.clone())
            .collect();
        let mut orphaned_children: Vec<(String, String)> = Vec::new();
        for cid in &child_ids {
            let new_root_id = self.fresh_child_id(None);
            if let Some(mut child) = self.mindmap.nodes.remove(cid) {
                orphaned_children.push((cid.clone(), new_root_id.clone()));
                child.parent_id = None;
                child.id = new_root_id.clone();
                self.mindmap.nodes.insert(new_root_id, child);
            }
        }

        // Collect every edge that touches the deleted node.
        let removed_edges: Vec<(usize, MindEdge)> = self
            .mindmap
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.from_id == node_id || e.to_id == node_id)
            .map(|(i, e)| (i, e.clone()))
            .collect();
        self.mindmap
            .edges
            .retain(|e| e.from_id != node_id && e.to_id != node_id);

        self.dirty = true;
        Some(UndoAction::DeleteNode {
            node,
            removed_edges,
            orphaned_children,
        })
    }

    /// Create a new unattached (orphan) node at the given canvas position.
    /// Returns the new node's Dewey-decimal id.
    pub fn apply_create_orphan_node(&mut self, position: Vec2) -> String {
        let id = self.fresh_child_id(None);
        let node = default_orphan_node(&id, position);
        self.mindmap.nodes.insert(id.clone(), node);
        id
    }

    /// Detach each node in `node_ids` from its parent, promoting it to root.
    pub fn apply_orphan_selection(&mut self, node_ids: &[String]) -> ReparentUndoData {
        self.apply_reparent(node_ids, None)
    }

    /// Create an orphan node, push undo, select it, mark dirty.
    pub fn create_orphan_and_select(&mut self, canvas_pos: Vec2) -> String {
        let new_id = self.apply_create_orphan_node(canvas_pos);
        self.undo_stack.push(UndoAction::CreateNode { node_id: new_id.clone() });
        self.selection = SelectionState::Single(new_id.clone());
        self.dirty = true;
        new_id
    }

    /// Orphan every selected node with undo support.
    pub fn apply_orphan_selection_with_undo(&mut self) -> bool {
        let sel: Vec<String> = self.selection
            .selected_ids().iter().map(|s| s.to_string()).collect();
        if sel.is_empty() {
            return false;
        }
        let undo_data = self.apply_orphan_selection(&sel);
        if undo_data.entries.is_empty() {
            return false;
        }
        self.undo_stack.push(UndoAction::ReparentNodes {
            entries: undo_data.entries,
            old_edges: undo_data.old_edges,
        });
        self.dirty = true;
        true
    }

    /// Delete whatever is currently selected.
    pub fn apply_delete_selection(&mut self) -> bool {
        enum DelKind {
            Edge(EdgeRef),
            Portal(PortalRef),
            Node(String),
            Nodes(Vec<String>),
        }
        let kind = match &self.selection {
            SelectionState::Edge(e) => Some(DelKind::Edge(e.clone())),
            SelectionState::Portal(p) => Some(DelKind::Portal(p.clone())),
            SelectionState::Single(id) => Some(DelKind::Node(id.clone())),
            SelectionState::Multi(ids) => Some(DelKind::Nodes(ids.clone())),
            SelectionState::None => None,
        };
        match kind {
            Some(DelKind::Edge(edge_ref)) => {
                if let Some((index, edge)) = self.remove_edge(&edge_ref) {
                    self.undo_stack.push(UndoAction::DeleteEdge { index, edge });
                    self.selection = SelectionState::None;
                    self.dirty = true;
                    return true;
                }
            }
            Some(DelKind::Portal(pref)) => {
                if self.apply_delete_portal(&pref).is_some() {
                    self.selection = SelectionState::None;
                    return true;
                }
            }
            Some(DelKind::Node(id)) => {
                if let Some(undo) = self.delete_node(&id) {
                    self.undo_stack.push(undo);
                    self.selection = SelectionState::None;
                    return true;
                }
            }
            Some(DelKind::Nodes(ids)) => {
                let mut any = false;
                for id in ids {
                    if let Some(undo) = self.delete_node(&id) {
                        self.undo_stack.push(undo);
                        any = true;
                    }
                }
                if any {
                    self.selection = SelectionState::None;
                    return true;
                }
            }
            None => {}
        }
        false
    }

    /// Generate the next available Dewey-decimal child ID under `parent`.
    /// `None` = root level. E.g. if roots "0", "1", "2" exist, returns "3".
    /// If parent "1" has children "1.0", "1.2", returns "1.3".
    pub(super) fn fresh_child_id(&self, parent: Option<&str>) -> String {
        let prefix = match parent {
            Some(p) => format!("{}.", p),
            None => String::new(),
        };
        let max_segment: Option<usize> = self
            .mindmap
            .nodes
            .keys()
            .filter_map(|id| {
                let suffix = id.strip_prefix(&prefix)?;
                // Only direct children — no further dots
                if suffix.contains('.') {
                    return None;
                }
                suffix.parse::<usize>().ok()
            })
            .max();
        let next = max_segment.map(|m| m + 1).unwrap_or(0);
        format!("{}{}", prefix, next)
    }

    /// Create a default-styled cross_link edge between two nodes.
    pub fn create_cross_link_edge(
        &mut self,
        source_id: &str,
        target_id: &str,
    ) -> Option<usize> {
        if source_id == target_id {
            return None;
        }
        if !self.mindmap.nodes.contains_key(source_id)
            || !self.mindmap.nodes.contains_key(target_id)
        {
            return None;
        }
        let exists = self.mindmap.edges.iter().any(|e| {
            e.edge_type == "cross_link"
                && e.from_id == source_id
                && e.to_id == target_id
        });
        if exists {
            return None;
        }
        let edge = default_cross_link_edge(source_id, target_id);
        self.mindmap.edges.push(edge);
        Some(self.mindmap.edges.len() - 1)
    }

    /// Apply a position delta to a node and all its descendants.
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

    /// Apply a position delta to a single node only.
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
    pub fn apply_move_multiple(&mut self, node_ids: &[String], dx: f64, dy: f64, individual: bool) -> Vec<(String, Position)> {
        if individual {
            let mut undo_data = Vec::new();
            for nid in node_ids {
                undo_data.extend(self.apply_move_single(nid, dx, dy));
            }
            return undo_data;
        }
        let roots = self.dedup_subtree_roots(node_ids);
        let mut undo_data = Vec::new();
        for nid in &roots {
            undo_data.extend(self.apply_move_subtree(nid, dx, dy));
        }
        undo_data
    }

    /// Filter a list of node IDs to only the "roots" — nodes not
    /// descendant of any other node in the list.
    pub(super) fn dedup_subtree_roots(&self, node_ids: &[String]) -> Vec<String> {
        let id_set: std::collections::HashSet<&str> = node_ids.iter().map(|s| s.as_str()).collect();
        node_ids.iter().filter(|id| {
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

    /// Reparent a set of nodes under `new_parent_id` (None = promote to root).
    /// Updates `parent_id` and parent_child edges. Returns undo data.
    pub fn apply_reparent(
        &mut self,
        node_ids: &[String],
        new_parent_id: Option<&str>,
    ) -> ReparentUndoData {
        let old_edges = self.mindmap.edges.clone();

        let mut entries: Vec<(String, Option<String>)> = Vec::new();
        for source_id in node_ids {
            if !self.mindmap.nodes.contains_key(source_id) {
                continue;
            }
            if let Some(target) = new_parent_id {
                if self.mindmap.is_ancestor_or_self(source_id, target) {
                    continue;
                }
            }
            let node = match self.mindmap.nodes.get_mut(source_id) {
                Some(n) => n,
                None => continue,
            };
            entries.push((source_id.clone(), node.parent_id.clone()));
            node.parent_id = new_parent_id.map(|s| s.to_string());

            // Update parent_child edges
            let old_edge_pos = self.mindmap.edges.iter().position(|e| {
                e.edge_type == "parent_child" && e.to_id == *source_id
            });
            match (old_edge_pos, new_parent_id) {
                (Some(idx), Some(new_parent)) => {
                    self.mindmap.edges[idx].from_id = new_parent.to_string();
                    self.mindmap.edges[idx].control_points.clear();
                }
                (Some(idx), None) => {
                    self.mindmap.edges.remove(idx);
                }
                (None, Some(new_parent)) => {
                    self.mindmap.edges.push(default_parent_child_edge(
                        new_parent, source_id,
                    ));
                }
                (None, None) => {}
            }
        }
        ReparentUndoData { entries, old_edges }
    }
}
