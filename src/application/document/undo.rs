//! `MindMapDocument::undo()` — the dispatch that matches on the
//! `UndoAction` the stack just popped and reverses the mutation
//! that pushed it. Each variant of `UndoAction` has a matching
//! branch here and nowhere else.

use baumhard::mindmap::custom_mutation::apply_mutations_to_element;

use super::types::{PortalRef, SelectionState};
use super::undo_action::UndoAction;
use super::MindMapDocument;

impl MindMapDocument {

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
                UndoAction::ReparentNodes { entries, old_edges } => {
                    for (id, old_parent, old_index) in entries {
                        if let Some(node) = self.mindmap.nodes.get_mut(&id) {
                            node.parent_id = old_parent;
                            node.index = old_index;
                        }
                    }
                    // Restore the full edges snapshot — this reverses any
                    // parent_child edge additions, removals, and from_id
                    // repointing that apply_reparent performed.
                    self.mindmap.edges = old_edges;
                }
                UndoAction::DeleteEdge { index, edge } => {
                    // Reinsert at the original index, clamped to current length
                    // in case other undo actions have shifted the Vec.
                    let idx = index.min(self.mindmap.edges.len());
                    self.mindmap.edges.insert(idx, edge);
                }
                UndoAction::CreateEdge { index } => {
                    if index < self.mindmap.edges.len() {
                        self.mindmap.edges.remove(index);
                    }
                }
                UndoAction::EditEdge { index, before } => {
                    // Restore the pre-edit edge. If the index is stale
                    // (structural change since the action was recorded),
                    // clamp to the current length — a best-effort
                    // fallback that matches the DeleteEdge pattern.
                    if index < self.mindmap.edges.len() {
                        self.mindmap.edges[index] = before;
                    }
                }
                UndoAction::CreateNode { node_id } => {
                    // Remove the node. It has no parent_child edge (it was
                    // created as an orphan) and no children (fresh node),
                    // so there's nothing else to clean up. If the current
                    // selection referenced it, clear the selection.
                    self.mindmap.nodes.remove(&node_id);
                    if self.selection.is_selected(&node_id) {
                        self.selection = SelectionState::None;
                    }
                }
                UndoAction::EditNodeText { node_id, before_text, before_runs } => {
                    // Restore the pre-edit text and text_runs. If the node
                    // has been removed since the action was recorded (e.g.
                    // a later delete that hasn't been undone yet), silently
                    // skip — matches the `EditEdge` clamp-on-missing pattern.
                    if let Some(node) = self.mindmap.nodes.get_mut(&node_id) {
                        node.text = before_text;
                        node.text_runs = before_runs;
                    }
                }
                UndoAction::EditNodeStyle { node_id, before_style, before_runs } => {
                    // Same clamp-on-missing rule as EditNodeText.
                    if let Some(node) = self.mindmap.nodes.get_mut(&node_id) {
                        node.style = before_style;
                        node.text_runs = before_runs;
                    }
                }
                UndoAction::CanvasSnapshot { canvas } => {
                    self.mindmap.canvas = canvas;
                }
                UndoAction::CreatePortal { index } => {
                    // Mirror `CreateEdge`: pop the created portal off
                    // the vec. Clear portal selection if it referenced
                    // the portal we just removed.
                    if index < self.mindmap.portals.len() {
                        let removed = self.mindmap.portals.remove(index);
                        if let SelectionState::Portal(ref pref) = self.selection {
                            if pref.matches(&removed) {
                                self.selection = SelectionState::None;
                            }
                        }
                    }
                }
                UndoAction::DeletePortal { index, portal } => {
                    // Mirror `DeleteEdge`: re-insert at the original
                    // index, clamped to the current length.
                    let idx = index.min(self.mindmap.portals.len());
                    self.mindmap.portals.insert(idx, portal);
                }
                UndoAction::EditPortal { index, before } => {
                    // Mirror `EditEdge`: replace in place, clamped.
                    if index < self.mindmap.portals.len() {
                        self.mindmap.portals[index] = before;
                    }
                }
                UndoAction::DeleteNode { node, removed_edges, orphaned_children } => {
                    // Re-insert the node itself.
                    let restored_id = node.id.clone();
                    self.mindmap.nodes.insert(restored_id.clone(), node);
                    // Re-insert edges at their original pre-delete
                    // indices. `delete_node` stores each index relative
                    // to the *original* edge vec (via `enumerate()` on
                    // the live vec before `retain()` drops the matches),
                    // so ascending-order re-insertion correctly slots
                    // each one into its original position — each earlier
                    // re-insert shifts all later elements right by
                    // exactly one, so the next stored index still
                    // points at the correct slot.
                    for (idx, edge) in removed_edges {
                        let idx = idx.min(self.mindmap.edges.len());
                        self.mindmap.edges.insert(idx, edge);
                    }
                    // Re-attach orphaned children: restore `parent_id`
                    // and the pre-delete sibling `index`.
                    for (cid, old_index) in orphaned_children {
                        if let Some(child) = self.mindmap.nodes.get_mut(&cid) {
                            child.parent_id = Some(restored_id.clone());
                            child.index = old_index;
                        }
                    }
                }
            }
            true
        } else {
            false
        }
    }

}
