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
    /// become roots with fresh sibling indices), and removing every edge
    /// that touched the node (parent_child, cross_link, etc.). Returns an
    /// `UndoAction::DeleteNode` payload that fully reverses the operation
    /// on undo, or `None` if the node doesn't exist.
    ///
    /// Orphaning is shallow — only direct children are promoted. Each
    /// grand-child stays attached to its parent, so entire subtrees
    /// survive the delete intact, just one level higher in the hierarchy.
    /// Matches the user request "orphan children" at Session 7A follow-up.
    ///
    /// The caller is expected to push the returned undo payload onto the
    /// stack and trigger a `rebuild_all`.
    pub fn delete_node(&mut self, node_id: &str) -> Option<UndoAction> {
        // Remove the node itself. Bail early if the id doesn't exist so
        // we don't leave the model in a half-mutated state.
        let node = self.mindmap.nodes.remove(node_id)?;

        // Orphan immediate children: clear `parent_id`, assign fresh root
        // indices one past the current maximum so they sort last among
        // roots. Mirrors the indexing in `apply_create_orphan_node`.
        let next_root_index = self
            .mindmap
            .root_nodes()
            .iter()
            .map(|n| n.index)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        let child_ids: Vec<String> = self
            .mindmap
            .nodes
            .values()
            .filter(|n| n.parent_id.as_deref() == Some(node_id))
            .map(|n| n.id.clone())
            .collect();
        let mut orphaned_children: Vec<(String, i32)> = Vec::new();
        for (i, cid) in child_ids.iter().enumerate() {
            if let Some(child) = self.mindmap.nodes.get_mut(cid) {
                orphaned_children.push((cid.clone(), child.index));
                child.parent_id = None;
                child.index = next_root_index + i as i32;
            }
        }

        // Collect every edge that touches the deleted node, paired
        // with its index in the **pre-removal** edge vec. The indices
        // matter for undo: ascending-order re-insertion at the original
        // positions correctly reconstructs the original edge order
        // because each earlier-index re-insert shifts later elements
        // right by exactly one, so the next stored original index is
        // still the right slot.
        //
        // We must NOT compute the index during an in-place remove
        // loop — by the time we reach the second touching edge, the
        // prior removal has already shifted its index down by one, so
        // storing the loop index would record a stale post-removal
        // position. Undo would then re-insert at the wrong slot and
        // silently reorder edges the caller never touched. Collect
        // first (using `enumerate()` on the original vec), then drop
        // the touching edges with `retain()`.
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

    /// Create a new unattached (orphan) node at the given canvas position
    /// and insert it into the map. The node has `parent_id == None` so it
    /// renders as a root, giving users a way to start a subtree in
    /// isolation and attach it later (via reparent mode, Ctrl+P).
    ///
    /// Returns the new node's id. The caller is expected to push a
    /// `UndoAction::CreateNode { node_id }` entry so Ctrl+Z removes it.
    pub fn apply_create_orphan_node(&mut self, position: Vec2) -> String {
        let id = self.fresh_node_id();
        // Index among roots: one past the current maximum so the new node
        // sorts last (it was just created, after all).
        let next_root_index = self.mindmap.root_nodes()
            .iter()
            .map(|n| n.index)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        let node = default_orphan_node(&id, position, next_root_index);
        self.mindmap.nodes.insert(id.clone(), node);
        id
    }

    /// Detach each node in `node_ids` from its parent, promoting it to a
    /// root node. Each node's entire subtree stays attached to it — only
    /// the link between the node and its former parent is severed.
    ///
    /// This is a thin wrapper around `apply_reparent(ids, None)`, which
    /// already handles the promote-to-root case (updating `parent_id`,
    /// `index`, and removing the corresponding `parent_child` edge). The
    /// wrapper exists so keybind dispatch can call a self-documenting
    /// method name.
    ///
    /// Returns the same `ReparentUndoData` as `apply_reparent`, which the
    /// caller should wrap in `UndoAction::ReparentNodes` for undo.
    pub fn apply_orphan_selection(&mut self, node_ids: &[String]) -> ReparentUndoData {
        self.apply_reparent(node_ids, None)
    }

    /// Create an orphan node at `canvas_pos`, push the `CreateNode` undo
    /// entry, select the new node, and mark the document dirty. Returns
    /// the new node's id. Shared between the Ctrl+N / double-click-empty
    /// paths on native and WASM — each caller then typically opens the
    /// text editor on the returned id.
    pub fn create_orphan_and_select(&mut self, canvas_pos: Vec2) -> String {
        let new_id = self.apply_create_orphan_node(canvas_pos);
        self.undo_stack.push(UndoAction::CreateNode { node_id: new_id.clone() });
        self.selection = SelectionState::Single(new_id.clone());
        self.dirty = true;
        new_id
    }

    /// Detach every currently-selected node from its parent (promote to
    /// root), push the `ReparentNodes` undo entry, and mark dirty.
    /// Returns `true` if anything was actually orphaned — callers gate a
    /// rebuild on this. No-op on empty selection or when no nodes
    /// actually moved.
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

    /// Delete whatever is currently selected (edge / portal / single node /
    /// multiple nodes), push the appropriate undo entries, clear the
    /// selection, and mark dirty. Returns `true` if anything was actually
    /// deleted so the caller can gate a rebuild; `false` on empty selection
    /// or no-op removes. Node deletion orphans immediate children (they
    /// become roots) and strips every edge that touched the deleted node.
    /// For multi-select, one undo entry is pushed per node so Ctrl+Z
    /// unwinds them in reverse order.
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

    /// Generate a fresh node id that doesn't collide with any existing
    /// node. The format is `new-<n>` where `n` starts at 1 and increments
    /// until the id is free. Deterministic for testing.
    fn fresh_node_id(&self) -> String {
        let mut n: usize = 1;
        loop {
            let candidate = format!("new-{}", n);
            if !self.mindmap.nodes.contains_key(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// Create a default-styled `cross_link` edge between two nodes and push
    /// it onto `mindmap.edges`. Returns the index where it was inserted so
    /// the caller can push a `CreateEdge` undo action.
    ///
    /// Returns `None` if:
    /// - `source_id == target_id` (self-links are rejected)
    /// - either node doesn't exist in the map
    /// - a `cross_link` edge from source to target already exists
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
        // Duplicate check: reject if a cross_link already exists between
        // these two nodes (in this direction).
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
    pub(super) fn dedup_subtree_roots(&self, node_ids: &[String]) -> Vec<String> {
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

    /// Reparent a set of nodes under `new_parent_id` (None = promote to root),
    /// making them the last children of the new parent in their given order.
    ///
    /// Silently skips any source node that would create a cycle (i.e. the target
    /// parent is the source itself or one of its descendants). Also skips source
    /// nodes whose ID doesn't exist in the map.
    ///
    /// Positions are absolute world-space coordinates, so no position recalculation
    /// is needed — only `parent_id`, `index`, and `parent_child` edges change.
    ///
    /// Returns `ReparentUndoData` containing:
    /// - `entries`: `(node_id, old_parent_id, old_index)` for each successfully
    ///   reparented node.
    /// - `old_edges`: A full snapshot of `mindmap.edges` from before the operation,
    ///   so edge mutations can be reversed on undo.
    ///
    /// If no nodes were reparented (all rejected), `entries` is empty and the
    /// caller should not push any undo action.
    pub fn apply_reparent(
        &mut self,
        node_ids: &[String],
        new_parent_id: Option<&str>,
    ) -> ReparentUndoData {
        // Snapshot edges before any mutation so undo can restore them wholesale.
        let old_edges = self.mindmap.edges.clone();

        // Compute the starting index: one greater than the current max sibling index
        // under the new parent (or 0 if no siblings).
        let mut next_index: i32 = match new_parent_id {
            Some(pid) => {
                self.mindmap.children_of(pid)
                    .iter()
                    .map(|n| n.index)
                    .max()
                    .map(|m| m + 1)
                    .unwrap_or(0)
            }
            None => {
                self.mindmap.root_nodes()
                    .iter()
                    .map(|n| n.index)
                    .max()
                    .map(|m| m + 1)
                    .unwrap_or(0)
            }
        };

        let mut entries: Vec<(String, Option<String>, i32)> = Vec::new();
        for source_id in node_ids {
            // Skip nonexistent nodes
            if !self.mindmap.nodes.contains_key(source_id) {
                continue;
            }
            // Cycle check: if the target is the source itself or a descendant
            // of the source, reparenting would create a cycle. Skip.
            if let Some(target) = new_parent_id {
                if self.mindmap.is_ancestor_or_self(source_id, target) {
                    continue;
                }
            }
            // A node whose parent is already the target is a no-op that would
            // still reassign its index and push it to last-child position. That
            // is a valid (user-intended) "move to end", so we allow it.
            let node = match self.mindmap.nodes.get_mut(source_id) {
                Some(n) => n,
                None => continue,
            };
            entries.push((source_id.clone(), node.parent_id.clone(), node.index));
            node.parent_id = new_parent_id.map(|s| s.to_string());
            node.index = next_index;
            next_index += 1;

            // Update parent_child edges for this source: find any existing
            // parent_child edge where to_id == source_id (the edge coming from
            // the old parent) and either update its from_id or remove it.
            // If no such edge exists and we're reparenting to a new parent,
            // create a new default-styled parent_child edge.
            let old_edge_pos = self.mindmap.edges.iter().position(|e| {
                e.edge_type == "parent_child" && e.to_id == *source_id
            });
            match (old_edge_pos, new_parent_id) {
                (Some(idx), Some(new_parent)) => {
                    // Repoint the existing edge to the new parent, preserving
                    // its styling (color, anchors, control points, etc.)
                    self.mindmap.edges[idx].from_id = new_parent.to_string();
                    // Clear control points — the old curve was computed for
                    // the old parent's position and would look wrong.
                    self.mindmap.edges[idx].control_points.clear();
                }
                (Some(idx), None) => {
                    // Promoted to root — remove the old parent_child edge
                    self.mindmap.edges.remove(idx);
                }
                (None, Some(new_parent)) => {
                    // No prior edge (e.g. a prior root now being parented).
                    // Create a default parent_child edge.
                    self.mindmap.edges.push(default_parent_child_edge(
                        new_parent, source_id,
                    ));
                }
                (None, None) => {
                    // Root to root — no edge changes needed.
                }
            }
        }
        ReparentUndoData { entries, old_edges }
    }
}
