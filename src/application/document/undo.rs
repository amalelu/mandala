// SPDX-License-Identifier: MPL-2.0

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
        // A `DeleteNode` restore whose id is already occupied must NOT
        // consume the action: dropping it would lose the node payload +
        // removed edges + orphan mapping (permanent data loss) while `undo()`
        // still returns `true`, so the caller reports a successful undo on a
        // no-op and the next Ctrl-Z applies the action *below* it to a
        // one-delete-out-of-sync state. Peek first, leave the action on the
        // stack, and return `false` so undo visibly stalls instead of lying.
        // Unreachable given the `delete_node` minting fix; the `debug_assert!`
        // makes any future regression fail `./test.sh` instead of silently
        // corrupting the history.
        if let Some(UndoAction::DeleteNode { node, .. }) = self.undo_stack.last() {
            if self.mindmap.nodes.contains_key(node.id.as_str()) {
                log::error!(
                    "undo DeleteNode: id '{}' is already occupied; refusing to overwrite a \
                     live node and leaving the action on the undo stack",
                    node.id
                );
                debug_assert!(false, "undo DeleteNode landed on an occupied id");
                return false;
            }
        }

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
                        let removed = self.mindmap.edges.remove(index);
                        // If the selection points at the edge we just
                        // removed, clear it — otherwise the selection
                        // dangles at a triple no edge in the map
                        // matches, and subsequent scene builds silently
                        // render nothing highlighted while the
                        // `selected_edge()` lookup keeps returning the
                        // stale ref. Mirrors the `CreateNode` branch
                        // below.
                        if let SelectionState::Edge(ref er) = self.selection {
                            if er.matches(&removed) {
                                self.selection = SelectionState::None;
                            }
                        }
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
                UndoAction::EditNodeText {
                    node_id,
                    before_sections,
                    before_position,
                    before_size,
                    before_selection,
                } => {
                    if let Some(node) = self.mindmap.nodes.get_mut(&node_id) {
                        node.sections = before_sections;
                        node.position = before_position;
                        node.size = before_size;
                    }
                    self.selection = before_selection;
                }
                UndoAction::EditNodeStyle {
                    node_id,
                    before_style,
                    before_sections,
                    before_position,
                    before_size,
                    before_selection,
                } => {
                    if let Some(node) = self.mindmap.nodes.get_mut(&node_id) {
                        node.style = before_style;
                        node.sections = before_sections;
                        node.position = before_position;
                        node.size = before_size;
                    }
                    self.selection = before_selection;
                }
                UndoAction::EditNodeAabb {
                    node_id,
                    before_position,
                    before_size,
                } => {
                    if let Some(node) = self.mindmap.nodes.get_mut(&node_id) {
                        node.position = before_position;
                        node.size = before_size;
                    }
                }
                UndoAction::EditNodeZoom {
                    node_id,
                    before_min,
                    before_max,
                } => {
                    if let Some(node) = self.mindmap.nodes.get_mut(&node_id) {
                        node.min_zoom_to_render = before_min;
                        node.max_zoom_to_render = before_max;
                    }
                }
                UndoAction::CanvasSnapshot { canvas } => {
                    self.mindmap.canvas = canvas;
                }
                UndoAction::DeleteNode {
                    node,
                    removed_edges,
                    orphaned_children,
                } => {
                    let restored_id = node.id.clone();
                    // Precondition (`restored_id` free) is verified by the
                    // peek-before-pop guard at the top of `undo()`, so the
                    // insert never clobbers a live node.
                    self.mindmap.nodes.insert(restored_id.clone(), node);
                    for (idx, edge) in removed_edges {
                        let idx = idx.min(self.mindmap.edges.len());
                        self.mindmap.edges.insert(idx, edge);
                    }
                    // Reverse the cascade rename for each orphaned child:
                    // rename from root-level ID back to original subtree ID,
                    // then restore parent_id to the deleted node — but only
                    // when the reverse rename actually applied. If it were
                    // refused (`old_id` occupied by an unrelated node), a blind
                    // `get_mut(&old_id)` would find that foreign occupant and
                    // re-hang its subtree under the restored node. Gating on
                    // the `bool` avoids re-parenting a stranger (§9).
                    for (old_id, root_id) in orphaned_children {
                        if self.cascade_rename(&root_id, &old_id) {
                            if let Some(child) = self.mindmap.nodes.get_mut(&old_id) {
                                child.parent_id = Some(restored_id.clone());
                            }
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
