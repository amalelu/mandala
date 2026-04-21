//! `UndoAction` — the tagged union the undo stack stores. One
//! variant per user-facing mutation the document can perform; the
//! `undo()` dispatch lives in `undo.rs` and branches on these
//! variants.

use baumhard::mindmap::model::{Canvas, MindEdge, MindNode, NodeStyle, Position, TextRun};

/// An undoable action that can be reversed.
#[derive(Clone, Debug)]
pub enum UndoAction {
    /// Stores original positions of moved nodes for restoration.
    MoveNodes { original_positions: Vec<(String, Position)> },
    /// Stores full node snapshots before a custom mutation was applied.
    CustomMutation { node_snapshots: Vec<(String, MindNode)> },
    /// Stores original parent_id for each reparented node, plus a full
    /// snapshot of `mindmap.edges` from before the reparent so that
    /// parent_child edge rewrites can be reversed on undo.
    ReparentNodes {
        entries: Vec<(String, Option<String>)>,
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
    /// The `text` (and possibly `text_runs`) of a node was edited in
    /// place via `set_node_text`. Undo restores the pre-edit `text` and
    /// `text_runs` on the node, if it still exists.
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
    /// A node's zoom-visibility window (`min_zoom_to_render` /
    /// `max_zoom_to_render`) was edited. Kept as its own variant —
    /// not folded into `EditNodeStyle` — because the zoom pair is
    /// not part of `NodeStyle`; it sits on `MindNode` directly as a
    /// presence gate orthogonal to visual styling.
    EditNodeZoom {
        node_id: String,
        before_min: Option<f32>,
        before_max: Option<f32>,
    },
    /// Snapshot of the entire `Canvas` taken before a document action
    /// (theme switch, etc.) mutated it. The canvas is small and cloning
    /// the whole thing is cheaper than tracking field-level diffs, and
    /// trivially correct.
    CanvasSnapshot { canvas: Canvas },
    /// A node was deleted. Restored by re-inserting the node, re-inserting
    /// every edge that touched it at its original `mindmap.edges` index,
    /// and restoring the `parent_id` of every child that was orphaned by
    /// the delete.
    DeleteNode {
        node: MindNode,
        /// Edges that referenced the deleted node (parent_child, cross_link,
        /// etc.), paired with their original index in `mindmap.edges`.
        removed_edges: Vec<(usize, MindEdge)>,
        /// For each child that was orphaned: its original id (before
        /// orphaning assigned it a root-level id) and the root-level id
        /// it was given. Undo restores the original id and parent_id.
        orphaned_children: Vec<(String, String)>,
    },
}
