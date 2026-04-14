//! `UndoAction` — the tagged union the undo stack stores. One
//! variant per user-facing mutation the document can perform; the
//! `undo()` dispatch lives in `undo.rs` and branches on these
//! variants.

use baumhard::mindmap::custom_mutation::CustomMutation;
use baumhard::mindmap::model::{Canvas, MindEdge, MindNode, NodeStyle, PortalPair, Position, TextRun};

use super::types::ReparentUndoData;

/// An undoable action that can be reversed.
#[derive(Clone, Debug)]
pub enum UndoAction {
    /// Stores original positions of moved nodes for restoration.
    MoveNodes { original_positions: Vec<(String, Position)> },
    /// Stores full node snapshots before a custom mutation was applied.
    CustomMutation { node_snapshots: Vec<(String, MindNode)> },
    /// Stores original parent_id and index for each reparented node, plus a
    /// full snapshot of `mindmap.edges` from before the reparent so that
    /// parent_child edge rewrites can be reversed on undo.
    ReparentNodes {
        entries: Vec<(String, Option<String>, i32)>,
        old_edges: Vec<MindEdge>,
    },
    /// Edge removed via the Delete key on a selected connection. Restored
    /// by re-inserting `edge` at `index` in `mindmap.edges`.
    DeleteEdge { index: usize, edge: MindEdge },
    /// Edge created via connect mode (Ctrl+D). Reversed by removing the
    /// edge at `index` (assumes LIFO undo order so the index is still valid).
    CreateEdge { index: usize },
    /// Full in-place edit of an existing edge — control-point drag,
    /// anchor change, reset-to-straight via palette, etc. The `before`
    /// snapshot is the pre-edit edge; undo replaces
    /// `mindmap.edges[index]` with it. Assumes the edge was not
    /// removed or reordered since the action was recorded (LIFO undo
    /// order makes this safe in practice).
    EditEdge { index: usize, before: MindEdge },
    /// A new node was created (via `apply_create_orphan_node`). Undo
    /// removes the node from `mindmap.nodes` by id.
    CreateNode { node_id: String },
    /// Session 7A: the `text` (and possibly `text_runs`) of a node was
    /// edited in place via `set_node_text`. Undo restores the pre-edit
    /// `text` and `text_runs` on the node, if it still exists.
    EditNodeText {
        node_id: String,
        before_text: String,
        before_runs: Vec<TextRun>,
    },
    /// A node's visual style was edited in place (bg / border / text
    /// color / font size). Captures the pre-edit `NodeStyle` plus the
    /// `text_runs` snapshot, since `set_node_text_color` and
    /// `set_node_font_size` may rewrite run colors / sizes on top of
    /// the style change. Undo restores both fields. Separate from
    /// `EditNodeText` because the round-trip contract is different
    /// (text is untouched; runs are touched only on text-color /
    /// font-size edits).
    EditNodeStyle {
        node_id: String,
        before_style: NodeStyle,
        before_runs: Vec<TextRun>,
    },
    /// Snapshot of the entire `Canvas` taken before a document action
    /// (theme switch, etc.) mutated it. The canvas is small and cloning
    /// the whole thing is cheaper than tracking field-level diffs, and
    /// trivially correct.
    CanvasSnapshot { canvas: Canvas },
    /// Session 6E: a new portal pair was created via
    /// `apply_create_portal`. Undo removes the portal at `index`
    /// (assumes LIFO undo order so the index is still valid).
    CreatePortal { index: usize },
    /// Session 6E: a portal pair was deleted via
    /// `apply_delete_portal`. Undo re-inserts `portal` at `index` in
    /// `mindmap.portals`.
    DeletePortal { index: usize, portal: PortalPair },
    /// Session 6E: a portal pair was edited in place (glyph, color,
    /// or any other field change via `apply_edit_portal`). Undo
    /// replaces `mindmap.portals[index]` with the `before` snapshot.
    EditPortal { index: usize, before: PortalPair },
    /// A node was deleted. Restored by re-inserting the node, re-inserting
    /// every edge that touched it at its original `mindmap.edges` index,
    /// and restoring the `parent_id`/`index` of every child that was
    /// orphaned by the delete. Mirrors the `DeleteEdge`/`DeletePortal`
    /// pattern, extended for the extra bookkeeping node deletion requires.
    DeleteNode {
        node: MindNode,
        /// Edges that referenced the deleted node (parent_child, cross_link,
        /// etc.), paired with their original index in `mindmap.edges`.
        /// Stored in ascending index order so the insertion loop on undo
        /// re-inserts them in the order they were removed.
        removed_edges: Vec<(usize, MindEdge)>,
        /// For each child that was orphaned by the delete, its id and
        /// pre-delete sibling `index`. `parent_id` is always the deleted
        /// node's id so it doesn't need to be stored separately.
        orphaned_children: Vec<(String, i32)>,
    },
}
