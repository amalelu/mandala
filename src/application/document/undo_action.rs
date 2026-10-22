// SPDX-License-Identifier: MPL-2.0

//! `UndoAction` — the tagged union the undo stack stores. One
//! variant per user-facing mutation the document can perform; the
//! `undo()` dispatch lives in `undo.rs` and branches on these
//! variants.

use super::types::SelectionState;
use baumhard::mindmap::model::{
    Canvas, ColorSchema, MindEdge, MindNode, MindSection, NodeStyle, Position, Size,
};

/// An undoable action that can be reversed.
#[derive(Clone, Debug)]
pub enum UndoAction {
    /// Stores original positions of moved nodes for restoration.
    // Native-driver-only, with `CreateEdge`: pushed by the native
    // drag-commit and edge-create paths.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    MoveNodes {
        original_positions: Vec<(String, Position)>,
    },
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
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
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
    /// A node's text content was edited in place via `set_node_text`
    /// (post-section-refactor: writes through to the node's first
    /// [`MindSection`]). The full pre-edit `sections` array is
    /// snapshotted so undo restores text *and* per-run formatting on
    /// every section, not just the one that was edited.
    ///
    /// `before_position` / `before_size` mirror the `EditNodeStyle`
    /// fields: text setters trigger `grow_one_node_to_fit_text` /
    /// `grow_one_node_to_fit_border` floor passes that can grow the
    /// node's AABB; without restoring them, undo of a long-text-
    /// replace leaves the node visibly inflated.
    EditNodeText {
        node_id: String,
        before_sections: Vec<MindSection>,
        before_position: Position,
        before_size: Size,
        /// Pre-mutation `doc.selection`. Text setters don't
        /// fire `cleanup_after_structural_mutation` (sections.len
        /// doesn't change), so this rarely shifts on text-only
        /// changes — captured anyway for symmetry with
        /// `EditNodeStyle` and to defend against future text
        /// setters that might trigger selection cleanup.
        before_selection: SelectionState,
    },
    /// A node's visual style was edited in place (bg / border / text
    /// color / font size). Captures the pre-edit `NodeStyle` and
    /// `color_schema` plus the full `sections` snapshot — `set_node_text_color`,
    /// `set_node_font_size`, and `set_node_font_family` rewrite per-
    /// run colors / sizes / fonts on top of the style change, and
    /// the section-resident runs need to come back wholesale on undo.
    /// Separate from `EditNodeText` because the round-trip contract
    /// is different (text is untouched here; only the section
    /// formatting is).
    EditNodeStyle {
        node_id: String,
        before_style: NodeStyle,
        /// Pre-mutation `node.color_schema`. The three per-node
        /// color setters write the node's own
        /// [`ColorOverrides`](baumhard::mindmap::model::ColorOverrides)
        /// when the node is themed — that is where the read path
        /// looks, so it is where the write has to land — and the
        /// override lives on the schema rather than in `NodeStyle`.
        /// Without this field a `color bg=` on a themed node would
        /// be unundoable.
        before_color_schema: Option<ColorSchema>,
        before_sections: Vec<MindSection>,
        /// Pre-mutation `node.position`. Captured because some
        /// style setters (and the structural section mutators
        /// added in Batch 5) trigger `grow_one_node_to_fit_text`
        /// / `grow_one_node_to_fit_border` floor passes that can
        /// shift the node's AABB; without restoring `position`
        /// on undo, the node would visibly drift after style →
        /// undo.
        before_position: Position,
        /// Pre-mutation `node.size`. Same rationale as
        /// `before_position` — the floor passes can grow `size`,
        /// and undo must restore it or the node stays inflated
        /// after the section/style mutation is undone.
        before_size: Size,
        /// Pre-mutation `doc.selection`. Captured because the
        /// structural-mutation cleanup
        /// (`cleanup_after_structural_mutation`) actively
        /// rewrites selection (e.g. `Section(idx=2)` →
        /// `Single(node)` after `delete_section(2)`); without
        /// restoring on undo, the user's pre-mutation selection
        /// is lost forever. Pre-Tier-4 review surfaced this as
        /// the missing leg.
        before_selection: SelectionState,
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
    /// (theme switch, etc.) mutated it. Cloning the whole thing is
    /// cheaper than tracking field-level diffs, and trivially correct.
    ///
    /// **Boxed.** Held inline, the canvas alone sized this enum at over
    /// a kilobyte (clippy's `large_enum_variant` measured 1216 bytes),
    /// and the undo stack is a `Vec<UndoAction>`, so every entry — a
    /// one-field zoom edit included — would occupy that much. Boxing
    /// leaves the cost with the snapshots that actually need it.
    CanvasSnapshot { canvas: Box<Canvas> },
    /// A node's AABB (`position` + `size`) was atomically rewritten
    /// — the release-commit of a node-resize drag, or the
    /// `node resize` console verb. Captures the pre-edit pair so
    /// undo restores both fields in lockstep. Separate from
    /// `MoveNodes` (which captures multiple node positions but not
    /// sizes) and from `EditNodeStyle` (which captures style +
    /// sections but not the node's own AABB).
    EditNodeAabb {
        node_id: String,
        before_position: Position,
        before_size: Size,
    },
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
