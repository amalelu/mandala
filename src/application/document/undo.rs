//! `MindMapDocument::undo()` — the dispatch that matches on the
//! `UndoAction` the stack just popped and reverses the mutation
//! that pushed it. Each variant of `UndoAction` has a matching
//! branch here and nowhere else.

use super::types::SelectionState;
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
                    for (id, old_parent) in entries {
                        if let Some(node) = self.mindmap.nodes.get_mut(&id) {
                            node.parent_id = old_parent;
                        }
                    }
                    self.mindmap.edges = old_edges;
                }
                UndoAction::DeleteEdge { index, edge } => {
                    let idx = index.min(self.mindmap.edges.len());
                    self.mindmap.edges.insert(idx, edge);
                }
                UndoAction::CreateEdge { index } => {
                    if index < self.mindmap.edges.len() {
                        self.mindmap.edges.remove(index);
                    }
                }
                UndoAction::EditEdge { index, before } => {
                    if index < self.mindmap.edges.len() {
                        self.mindmap.edges[index] = before;
                    }
                }
                UndoAction::CreateNode { node_id } => {
                    self.mindmap.nodes.remove(&node_id);
                    if self.selection.is_selected(&node_id) {
                        self.selection = SelectionState::None;
                    }
                }
                UndoAction::EditNodeText { node_id, before_text, before_runs } => {
                    if let Some(node) = self.mindmap.nodes.get_mut(&node_id) {
                        node.text = before_text;
                        node.text_runs = before_runs;
                    }
                }
                UndoAction::EditNodeStyle { node_id, before_style, before_runs } => {
                    if let Some(node) = self.mindmap.nodes.get_mut(&node_id) {
                        node.style = before_style;
                        node.text_runs = before_runs;
                    }
                }
                UndoAction::CanvasSnapshot { canvas } => {
                    self.mindmap.canvas = canvas;
                }
                UndoAction::DeleteNode { node, removed_edges, orphaned_children } => {
                    let restored_id = node.id.clone();
                    self.mindmap.nodes.insert(restored_id.clone(), node);
                    for (idx, edge) in removed_edges {
                        let idx = idx.min(self.mindmap.edges.len());
                        self.mindmap.edges.insert(idx, edge);
                    }
                    // Reverse the cascade rename for each orphaned child:
                    // rename from root-level ID back to original subtree ID,
                    // then restore parent_id to the deleted node.
                    for (old_id, root_id) in orphaned_children {
                        self.cascade_rename(&root_id, &old_id);
                        if let Some(child) = self.mindmap.nodes.get_mut(&old_id) {
                            child.parent_id = Some(restored_id.clone());
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
