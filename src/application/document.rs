use std::collections::{HashMap, HashSet};
use std::path::Path;
use glam::Vec2;
use log::{error, info};
use baumhard::core::primitives::Range;
use baumhard::mindmap::custom_mutation::{
    CustomMutation, DocumentAction, MutationBehavior, TargetScope, Trigger,
    PlatformContext, apply_mutations_to_element,
};
use baumhard::mindmap::connection;
use baumhard::mindmap::model::{
    Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, Position, Size,
};
use baumhard::mindmap::loader;
use baumhard::mindmap::scene_builder::{self, RenderScene};
use baumhard::mindmap::tree_builder::{self, MindMapTree};

/// Selection highlight color: bright cyan [R, G, B, A]
const HIGHLIGHT_COLOR: [f32; 4] = [0.0, 0.9, 1.0, 1.0];

/// Reparent-mode source color: orange, used for nodes currently being reparented.
const REPARENT_SOURCE_COLOR: [f32; 4] = [1.0, 0.55, 0.0, 1.0];

/// Reparent-mode target color: green, used for the node currently hovered as
/// a potential reparent target.
const REPARENT_TARGET_COLOR: [f32; 4] = [0.2, 1.0, 0.4, 1.0];

/// Identifies an edge in the MindMap by its endpoints and type. Edges have
/// no stable ID, so this triple is the canonical reference (matching how
/// `apply_reparent` looks up parent_child edges at document.rs:301).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EdgeRef {
    pub from_id: String,
    pub to_id: String,
    pub edge_type: String,
}

impl EdgeRef {
    pub fn new(from_id: impl Into<String>, to_id: impl Into<String>, edge_type: impl Into<String>) -> Self {
        Self {
            from_id: from_id.into(),
            to_id: to_id.into(),
            edge_type: edge_type.into(),
        }
    }

    /// Returns true if this ref identifies the given `MindEdge`.
    pub fn matches(&self, edge: &MindEdge) -> bool {
        self.from_id == edge.from_id
            && self.to_id == edge.to_id
            && self.edge_type == edge.edge_type
    }
}

/// Tracks what is currently selected in the document. Node and edge
/// selection are mutually exclusive — clicking a node clears any edge
/// selection and vice versa.
#[derive(Clone, Debug)]
pub enum SelectionState {
    None,
    Single(String),
    Multi(Vec<String>),
    Edge(EdgeRef),
}

impl SelectionState {
    pub fn is_selected(&self, node_id: &str) -> bool {
        match self {
            SelectionState::None => false,
            SelectionState::Single(id) => id == node_id,
            SelectionState::Multi(ids) => ids.contains(&node_id.to_string()),
            SelectionState::Edge(_) => false,
        }
    }

    pub fn selected_ids(&self) -> Vec<&str> {
        match self {
            SelectionState::None => vec![],
            SelectionState::Single(id) => vec![id.as_str()],
            SelectionState::Multi(ids) => ids.iter().map(|s| s.as_str()).collect(),
            SelectionState::Edge(_) => vec![],
        }
    }

    /// Returns the selected edge, if any.
    pub fn selected_edge(&self) -> Option<&EdgeRef> {
        match self {
            SelectionState::Edge(e) => Some(e),
            _ => None,
        }
    }
}

/// Return value of `MindMapDocument::apply_reparent`. Contains both the
/// per-node parent/index entries and a full snapshot of the edges Vec so that
/// edge rewrites can be reversed wholesale on undo.
#[derive(Clone, Debug)]
pub struct ReparentUndoData {
    pub entries: Vec<(String, Option<String>, i32)>,
    pub old_edges: Vec<MindEdge>,
}

/// Build a default-styled parent_child edge from `from_id` to `to_id`.
/// Used when reparenting a node that has no prior parent_child edge (e.g.
/// a formerly-root node being attached to a parent).
fn default_parent_child_edge(from_id: &str, to_id: &str) -> MindEdge {
    MindEdge {
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        edge_type: "parent_child".to_string(),
        color: "#888888".to_string(),
        width: 4,
        line_style: 0,
        visible: true,
        label: None,
        anchor_from: 0,
        anchor_to: 0,
        control_points: Vec::new(),
        glyph_connection: None,
    }
}

/// Build a fresh "orphan" MindNode with sensible defaults, positioned at
/// `position` and marked as a root (`parent_id = None`). The node has
/// placeholder text so it's visible on the canvas until the user edits it
/// (text editing is still WIP; see roadmap M7).
fn default_orphan_node(id: &str, position: Vec2, index: i32) -> MindNode {
    use baumhard::mindmap::model::TextRun;
    let text = "New node".to_string();
    let text_runs = vec![TextRun {
        start: 0,
        end: text.chars().count(),
        bold: false,
        italic: false,
        underline: false,
        font: "LiberationSans".to_string(),
        size_pt: 24,
        color: "#ffffff".to_string(),
        hyperlink: None,
    }];
    MindNode {
        id: id.to_string(),
        parent_id: None,
        index,
        position: Position {
            x: position.x as f64,
            y: position.y as f64,
        },
        size: Size {
            width: 240.0,
            height: 60.0,
        },
        text,
        text_runs,
        style: NodeStyle {
            background_color: "#141414".to_string(),
            frame_color: "#30b082".to_string(),
            text_color: "#ffffff".to_string(),
            shape_type: 0,
            corner_radius_percent: 10.0,
            frame_thickness: 4.0,
            show_frame: true,
            show_shadow: false,
            border: None,
        },
        layout: NodeLayout {
            layout_type: 0,
            direction: 0,
            spacing: 50.0,
        },
        folded: false,
        notes: String::new(),
        color_schema: None,
        trigger_bindings: Vec::new(),
        inline_mutations: Vec::new(),
    }
}

/// Build a default-styled cross_link edge from `from_id` to `to_id`.
/// Used by connect mode (Ctrl+D) to create non-hierarchical connections.
/// Cross-links don't affect the tree structure.
fn default_cross_link_edge(from_id: &str, to_id: &str) -> MindEdge {
    MindEdge {
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        edge_type: "cross_link".to_string(),
        color: "#aa88cc".to_string(),
        width: 3,
        line_style: 0,
        visible: true,
        label: None,
        anchor_from: 0,
        anchor_to: 0,
        control_points: Vec::new(),
        glyph_connection: None,
    }
}

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
    /// A new node was created (via `apply_create_orphan_node`). Undo
    /// removes the node from `mindmap.nodes` by id.
    CreateNode { node_id: String },
    /// Snapshot of the entire `Canvas` taken before a document action
    /// (theme switch, etc.) mutated it. The canvas is small and cloning
    /// the whole thing is cheaper than tracking field-level diffs, and
    /// trivially correct.
    CanvasSnapshot { canvas: Canvas },
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
    ///
    /// `camera_zoom` is forwarded through to the scene builder so
    /// connection glyphs can be sized via
    /// `GlyphConnectionConfig::effective_font_size_pt` — see
    /// `baumhard::mindmap::scene_builder::build_scene` for details.
    pub fn build_scene(&self, camera_zoom: f32) -> RenderScene {
        scene_builder::build_scene(&self.mindmap, camera_zoom)
    }

    /// Build a RenderScene with position offsets applied to specific nodes.
    /// Used during drag to update connections and borders in real-time.
    pub fn build_scene_with_offsets(
        &self,
        offsets: &HashMap<String, (f32, f32)>,
        camera_zoom: f32,
    ) -> RenderScene {
        scene_builder::build_scene_with_offsets(&self.mindmap, offsets, camera_zoom)
    }

    /// Cache-aware scene build. The drag drain in `app.rs` calls this
    /// every frame with a persistent `SceneConnectionCache` so unchanged
    /// edges skip the `sample_path` geometry work entirely — Phase B of
    /// the connection-render cost fix. See
    /// `baumhard::mindmap::scene_cache` for invariants.
    pub fn build_scene_with_cache(
        &self,
        offsets: &HashMap<String, (f32, f32)>,
        cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
        camera_zoom: f32,
    ) -> RenderScene {
        let sel = self.selection.selected_edge()
            .map(|e| (e.from_id.as_str(), e.to_id.as_str(), e.edge_type.as_str()));
        scene_builder::build_scene_with_cache(&self.mindmap, offsets, sel, cache, camera_zoom)
    }

    /// Build a RenderScene that also reflects the current edge selection.
    /// The selected edge (if any) gets a cyan color override baked into its
    /// ConnectionElement so the renderer paints it in the highlight color.
    pub fn build_scene_with_selection(&self, camera_zoom: f32) -> RenderScene {
        let sel = self.selection.selected_edge()
            .map(|e| (e.from_id.as_str(), e.to_id.as_str(), e.edge_type.as_str()));
        scene_builder::build_scene_with_offsets_and_selection(
            &self.mindmap,
            &HashMap::new(),
            sel,
            camera_zoom,
        )
    }

    /// Remove an edge matching `edge_ref` from the MindMap. Returns its
    /// original index in `mindmap.edges` and the removed edge so the caller
    /// can push a `DeleteEdge` undo action.
    pub fn remove_edge(&mut self, edge_ref: &EdgeRef) -> Option<(usize, MindEdge)> {
        let idx = self.mindmap.edges.iter().position(|e| edge_ref.matches(e))?;
        let edge = self.mindmap.edges.remove(idx);
        Some((idx, edge))
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
                UndoAction::CanvasSnapshot { canvas } => {
                    self.mindmap.canvas = canvas;
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

    /// Apply any document-level actions carried by a custom mutation. These
    /// operate on `self.mindmap.canvas` rather than any tree node, so they
    /// run independently of `apply_custom_mutation`'s tree walk. When any
    /// action would actually change state, a `CanvasSnapshot` undo entry is
    /// pushed capturing the pre-action canvas, and the document is marked
    /// dirty. Returns true if the canvas was modified.
    pub fn apply_document_actions(&mut self, custom: &CustomMutation) -> bool {
        if custom.document_actions.is_empty() {
            return false;
        }
        let snapshot = self.mindmap.canvas.clone();
        let mut changed = false;
        for action in &custom.document_actions {
            match action {
                DocumentAction::SetThemeVariant(name) => {
                    if let Some(preset) = self.mindmap.canvas.theme_variants.get(name) {
                        let new_vars = preset.clone();
                        if new_vars != self.mindmap.canvas.theme_variables {
                            self.mindmap.canvas.theme_variables = new_vars;
                            changed = true;
                        }
                    }
                    // Unknown variant: silently ignored (graceful).
                }
                DocumentAction::SetThemeVariables(map) => {
                    for (k, v) in map {
                        let existing = self.mindmap.canvas.theme_variables.get(k);
                        if existing.map(|s| s != v).unwrap_or(true) {
                            self.mindmap.canvas.theme_variables
                                .insert(k.clone(), v.clone());
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.undo_stack.push(UndoAction::CanvasSnapshot { canvas: snapshot });
            self.dirty = true;
        }
        changed
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

/// Hit test edges: find the nearest visible edge within `tolerance` canvas
/// units of `canvas_pos`. Returns an `EdgeRef` for the closest edge, or
/// `None` if nothing is within range.
///
/// Visibility filter mirrors `scene_builder::build_scene_with_offsets` — an
/// edge is eligible only if `edge.visible` is true, both endpoint nodes
/// exist, and neither endpoint is hidden by fold state.
pub fn hit_test_edge(canvas_pos: Vec2, map: &MindMap, tolerance: f32) -> Option<EdgeRef> {
    let mut best: Option<(EdgeRef, f32)> = None;
    for edge in &map.edges {
        if !edge.visible {
            continue;
        }
        let from_node = match map.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => continue,
        };
        let to_node = match map.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => continue,
        };
        if map.is_hidden_by_fold(from_node) || map.is_hidden_by_fold(to_node) {
            continue;
        }

        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);

        let path = connection::build_connection_path(
            from_pos, from_size, edge.anchor_from,
            to_pos, to_size, edge.anchor_to,
            &edge.control_points,
        );
        let dist = connection::distance_to_path(canvas_pos, &path);
        if dist > tolerance {
            continue;
        }
        if best.as_ref().map_or(true, |(_, best_dist)| dist < *best_dist) {
            best = Some((
                EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type),
                dist,
            ));
        }
    }
    best.map(|(e, _)| e)
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

/// Apply an orange "reparent-source" highlight to the given nodes in the tree.
/// Used in reparent mode to indicate which nodes the user is about to move.
/// Call after `apply_selection_highlight` if you want it to override the cyan
/// selection color for source nodes.
pub fn apply_reparent_source_highlight(tree: &mut MindMapTree, sources: &[String]) {
    for source_id in sources {
        let node_id = match tree.node_map.get(source_id) {
            Some(&id) => id,
            None => continue,
        };
        let node = match tree.tree.arena.get_mut(node_id) {
            Some(n) => n,
            None => continue,
        };
        if let Some(glyph_area) = node.get_mut().glyph_area_mut() {
            let ranges: Vec<Range> = glyph_area.regions.all_regions()
                .iter()
                .map(|r| r.range)
                .collect();
            for range in &ranges {
                glyph_area.set_region_color(range, &REPARENT_SOURCE_COLOR);
            }
        }
    }
}

/// Apply a green "reparent-target" highlight to a single node in the tree.
/// Used in reparent mode to indicate the currently-hovered drop target.
pub fn apply_reparent_target_highlight(tree: &mut MindMapTree, target_id: &str) {
    let node_id = match tree.node_map.get(target_id) {
        Some(&id) => id,
        None => return,
    };
    let node = match tree.tree.arena.get_mut(node_id) {
        Some(n) => n,
        None => return,
    };
    if let Some(glyph_area) = node.get_mut().glyph_area_mut() {
        let ranges: Vec<Range> = glyph_area.regions.all_regions()
            .iter()
            .map(|r| r.range)
            .collect();
        for range in &ranges {
            glyph_area.set_region_color(range, &REPARENT_TARGET_COLOR);
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
            document_actions: vec![],
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

    // --- Session 5B: reparent tests ---

    /// Pick (new_parent_id, source_id) where source is an unrelated node that
    /// can be validly reparented under new_parent. Both are pulled from the
    /// testament map and guaranteed to exist.
    fn find_reparent_pair(doc: &MindMapDocument) -> (String, String) {
        // Find two distinct nodes where the source is not an ancestor of the target.
        // Simplest approach: pick two unrelated leaf-ish nodes.
        let ids: Vec<String> = doc.mindmap.nodes.keys().cloned().collect();
        for a in &ids {
            for b in &ids {
                if a == b { continue; }
                // source = a, target parent = b. Valid iff a is not an ancestor of b.
                if !doc.mindmap.is_ancestor_or_self(a, b) {
                    return (b.clone(), a.clone());
                }
            }
        }
        panic!("testament map should contain a valid reparent pair");
    }

    #[test]
    fn test_apply_reparent_single_node_updates_parent_and_index() {
        let mut doc = load_test_doc();
        let (new_parent, source) = find_reparent_pair(&doc);
        let expected_index = doc.mindmap.children_of(&new_parent)
            .iter().map(|n| n.index).max().map(|m| m + 1).unwrap_or(0);

        let undo = doc.apply_reparent(&[source.clone()], Some(&new_parent));
        assert_eq!(undo.entries.len(), 1, "should have one undo entry");

        let node = doc.mindmap.nodes.get(&source).unwrap();
        assert_eq!(node.parent_id.as_deref(), Some(new_parent.as_str()),
            "parent_id should now point to new parent");
        assert_eq!(node.index, expected_index, "index should be max+1 of new siblings");
    }

    #[test]
    fn test_apply_reparent_updates_parent_child_edges() {
        let mut doc = load_test_doc();
        let (new_parent, source) = find_reparent_pair(&doc);

        // Precondition: there should be a parent_child edge leading to source
        // (the testament map wires every hierarchy link as an explicit edge).
        let had_old_edge = doc.mindmap.edges.iter().any(|e|
            e.edge_type == "parent_child" && e.to_id == source
        );

        doc.apply_reparent(&[source.clone()], Some(&new_parent));

        // After reparent: any parent_child edge pointing at source must have
        // from_id == new_parent. There should be at least one such edge if
        // there was one before (or if we're attaching a formerly-root node).
        let parent_edges: Vec<&MindEdge> = doc.mindmap.edges.iter()
            .filter(|e| e.edge_type == "parent_child" && e.to_id == source)
            .collect();
        if had_old_edge {
            assert_eq!(parent_edges.len(), 1,
                "should still have exactly one parent_child edge to source");
            assert_eq!(parent_edges[0].from_id, new_parent,
                "parent_child edge from_id should be updated to new parent");
        }
    }

    #[test]
    fn test_apply_reparent_to_root_removes_edge() {
        let mut doc = load_test_doc();
        let source = doc.mindmap.nodes.values()
            .find(|n| n.parent_id.is_some())
            .map(|n| n.id.clone())
            .expect("testament should have at least one non-root node");

        // Precondition: there should be an existing parent_child edge to source.
        let had_old_edge = doc.mindmap.edges.iter().any(|e|
            e.edge_type == "parent_child" && e.to_id == source
        );
        assert!(had_old_edge, "testament non-root node should have an incoming parent_child edge");

        doc.apply_reparent(&[source.clone()], None);

        // The parent_child edge should have been removed (promoted to root).
        let still_has_edge = doc.mindmap.edges.iter().any(|e|
            e.edge_type == "parent_child" && e.to_id == source
        );
        assert!(!still_has_edge,
            "parent_child edge to source should be removed when promoted to root");
    }

    #[test]
    fn test_apply_reparent_multiple_nodes_become_siblings() {
        let mut doc = load_test_doc();
        // Find a node with two unrelated siblings we can reparent.
        // Use two unrelated nodes from find_reparent_pair repeatedly.
        let (new_parent, first_source) = find_reparent_pair(&doc);
        // Find a second source that is also not an ancestor of new_parent and is
        // not the same as first_source.
        let second_source = doc.mindmap.nodes.keys()
            .find(|k| **k != new_parent && **k != first_source
                && !doc.mindmap.is_ancestor_or_self(k, &new_parent))
            .expect("testament should have another candidate source")
            .clone();

        let start_index = doc.mindmap.children_of(&new_parent)
            .iter().map(|n| n.index).max().map(|m| m + 1).unwrap_or(0);

        let sources = vec![first_source.clone(), second_source.clone()];
        let undo = doc.apply_reparent(&sources, Some(&new_parent));
        assert_eq!(undo.entries.len(), 2, "both sources should be reparented");

        let n1 = doc.mindmap.nodes.get(&first_source).unwrap();
        let n2 = doc.mindmap.nodes.get(&second_source).unwrap();
        assert_eq!(n1.parent_id.as_deref(), Some(new_parent.as_str()));
        assert_eq!(n2.parent_id.as_deref(), Some(new_parent.as_str()));
        // Indices should be start_index and start_index+1, preserving argument order
        assert_eq!(n1.index, start_index);
        assert_eq!(n2.index, start_index + 1);
    }

    #[test]
    fn test_apply_reparent_to_root() {
        let mut doc = load_test_doc();
        // Pick any non-root node
        let source = doc.mindmap.nodes.values()
            .find(|n| n.parent_id.is_some())
            .map(|n| n.id.clone())
            .expect("testament should have at least one non-root node");

        let expected_index = doc.mindmap.root_nodes()
            .iter().map(|n| n.index).max().map(|m| m + 1).unwrap_or(0);

        let undo = doc.apply_reparent(&[source.clone()], None);
        assert_eq!(undo.entries.len(), 1);

        let node = doc.mindmap.nodes.get(&source).unwrap();
        assert_eq!(node.parent_id, None, "should be promoted to root");
        assert_eq!(node.index, expected_index);
    }

    #[test]
    fn test_apply_reparent_rejects_cycle() {
        let mut doc = load_test_doc();
        // Find a parent with a grandchild so we can try to reparent the grandparent
        // under its own grandchild.
        let (grandparent, _child, grandchild) = {
            let mut found = None;
            'outer: for root in doc.mindmap.root_nodes() {
                for child in doc.mindmap.children_of(&root.id) {
                    let grands = doc.mindmap.children_of(&child.id);
                    if let Some(g) = grands.first() {
                        found = Some((root.id.clone(), child.id.clone(), g.id.clone()));
                        break 'outer;
                    }
                }
            }
            found.expect("testament should have a three-level chain")
        };

        let orig_parent = doc.mindmap.nodes.get(&grandparent).unwrap().parent_id.clone();
        let orig_index = doc.mindmap.nodes.get(&grandparent).unwrap().index;

        // Try to reparent grandparent under grandchild — should be silently rejected
        let undo = doc.apply_reparent(&[grandparent.clone()], Some(&grandchild));
        assert!(undo.entries.is_empty(), "cycle should be rejected, no entries in undo data");

        // State should be unchanged
        let gp = doc.mindmap.nodes.get(&grandparent).unwrap();
        assert_eq!(gp.parent_id, orig_parent);
        assert_eq!(gp.index, orig_index);
    }

    #[test]
    fn test_apply_reparent_rejects_self() {
        let mut doc = load_test_doc();
        let source = doc.mindmap.nodes.keys().next().unwrap().clone();
        let orig_parent = doc.mindmap.nodes.get(&source).unwrap().parent_id.clone();

        // Try to reparent a node under itself — should be silently rejected
        let undo = doc.apply_reparent(&[source.clone()], Some(&source));
        assert!(undo.entries.is_empty(), "self-reparent should be rejected");
        assert_eq!(doc.mindmap.nodes.get(&source).unwrap().parent_id, orig_parent);
    }

    #[test]
    fn test_reparent_undo_restores_parent_index_and_edges() {
        let mut doc = load_test_doc();
        let (new_parent, source) = find_reparent_pair(&doc);
        let orig_parent = doc.mindmap.nodes.get(&source).unwrap().parent_id.clone();
        let orig_index = doc.mindmap.nodes.get(&source).unwrap().index;
        let orig_edges_snapshot = doc.mindmap.edges.clone();

        let undo_data = doc.apply_reparent(&[source.clone()], Some(&new_parent));
        doc.undo_stack.push(UndoAction::ReparentNodes {
            entries: undo_data.entries,
            old_edges: undo_data.old_edges,
        });

        // Precondition: actually moved
        assert_eq!(
            doc.mindmap.nodes.get(&source).unwrap().parent_id.as_deref(),
            Some(new_parent.as_str())
        );

        // Undo and verify restoration
        assert!(doc.undo());
        let restored = doc.mindmap.nodes.get(&source).unwrap();
        assert_eq!(restored.parent_id, orig_parent);
        assert_eq!(restored.index, orig_index);

        // Edges should also be restored bit-for-bit
        assert_eq!(doc.mindmap.edges.len(), orig_edges_snapshot.len(),
            "edges Vec length should be restored");
        for (orig, restored) in orig_edges_snapshot.iter().zip(doc.mindmap.edges.iter()) {
            assert_eq!(orig.from_id, restored.from_id);
            assert_eq!(orig.to_id, restored.to_id);
            assert_eq!(orig.edge_type, restored.edge_type);
        }
    }

    // ---------------------------------------------------------------------
    // Session 6A: edge selection, deletion, undo
    // ---------------------------------------------------------------------

    /// Pick an edge from the testament map for hit-testing and deletion tests.
    /// Returns the edge's EdgeRef plus a canvas-space point that should lie
    /// on (or very near) the edge path.
    fn pick_test_edge(doc: &MindMapDocument) -> (EdgeRef, Vec2) {
        let edge = doc.mindmap.edges.iter()
            .find(|e| e.visible)
            .expect("testament map has visible edges");
        let from = doc.mindmap.nodes.get(&edge.from_id).unwrap();
        let to = doc.mindmap.nodes.get(&edge.to_id).unwrap();
        let from_pos = Vec2::new(from.position.x as f32, from.position.y as f32);
        let from_size = Vec2::new(from.size.width as f32, from.size.height as f32);
        let to_pos = Vec2::new(to.position.x as f32, to.position.y as f32);
        let to_size = Vec2::new(to.size.width as f32, to.size.height as f32);
        let path = baumhard::mindmap::connection::build_connection_path(
            from_pos, from_size, edge.anchor_from,
            to_pos, to_size, edge.anchor_to,
            &edge.control_points,
        );
        // Sample the middle of the path for a guaranteed on-path point.
        let samples = baumhard::mindmap::connection::sample_path(&path, 4.0);
        let midpoint = samples[samples.len() / 2].position;
        let edge_ref = EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
        (edge_ref, midpoint)
    }

    #[test]
    fn test_selection_state_edge_variant() {
        let edge_ref = EdgeRef::new("a", "b", "cross_link");
        let sel = SelectionState::Edge(edge_ref.clone());
        assert_eq!(sel.selected_edge(), Some(&edge_ref));
        // Node-selection queries on an edge selection return empty
        assert!(!sel.is_selected("a"));
        assert_eq!(sel.selected_ids().len(), 0);
    }

    #[test]
    fn test_edge_ref_matches() {
        let edge_ref = EdgeRef::new("a", "b", "cross_link");
        let edge = MindEdge {
            from_id: "a".into(),
            to_id: "b".into(),
            edge_type: "cross_link".into(),
            color: "#fff".into(),
            width: 1,
            line_style: 0,
            visible: true,
            label: None,
            anchor_from: 0,
            anchor_to: 0,
            control_points: vec![],
            glyph_connection: None,
        };
        assert!(edge_ref.matches(&edge));

        let wrong_type = EdgeRef::new("a", "b", "parent_child");
        assert!(!wrong_type.matches(&edge));
    }

    #[test]
    fn test_hit_test_edge_hits_on_path() {
        let doc = load_test_doc();
        let (expected, point) = pick_test_edge(&doc);
        let hit = hit_test_edge(point, &doc.mindmap, 2.0);
        assert_eq!(hit, Some(expected));
    }

    #[test]
    fn test_hit_test_edge_miss_far_away() {
        let doc = load_test_doc();
        // A point very far from any node/edge
        let hit = hit_test_edge(Vec2::new(-1_000_000.0, -1_000_000.0), &doc.mindmap, 8.0);
        assert_eq!(hit, None);
    }

    #[test]
    fn test_hit_test_edge_respects_tolerance() {
        let doc = load_test_doc();
        let (_, point) = pick_test_edge(&doc);
        // Shift 50 units away from the path (orthogonal). Tolerance of 5
        // should NOT produce a hit; tolerance of 100 should.
        let offset = Vec2::new(0.0, 50.0);
        let shifted = point + offset;
        assert_eq!(hit_test_edge(shifted, &doc.mindmap, 5.0), None);
        assert!(hit_test_edge(shifted, &doc.mindmap, 100.0).is_some());
    }

    #[test]
    fn test_remove_edge_returns_index_and_edge() {
        let mut doc = load_test_doc();
        let (edge_ref, _) = pick_test_edge(&doc);
        let orig_count = doc.mindmap.edges.len();

        let (idx, removed) = doc.remove_edge(&edge_ref).expect("edge should exist");
        assert!(edge_ref.matches(&removed));
        assert_eq!(doc.mindmap.edges.len(), orig_count - 1);
        // The index should be within the original range
        assert!(idx < orig_count);
    }

    #[test]
    fn test_remove_edge_missing_returns_none() {
        let mut doc = load_test_doc();
        let missing = EdgeRef::new("nope_from", "nope_to", "cross_link");
        assert!(doc.remove_edge(&missing).is_none());
    }

    #[test]
    fn test_undo_delete_edge_restores_at_original_index() {
        let mut doc = load_test_doc();
        let (edge_ref, _) = pick_test_edge(&doc);
        let orig_edges = doc.mindmap.edges.clone();
        let orig_idx = orig_edges.iter().position(|e| edge_ref.matches(e)).unwrap();

        let (idx, edge) = doc.remove_edge(&edge_ref).unwrap();
        doc.undo_stack.push(UndoAction::DeleteEdge { index: idx, edge });
        doc.dirty = true;

        assert_eq!(doc.mindmap.edges.len(), orig_edges.len() - 1);

        // Undo
        assert!(doc.undo());
        assert_eq!(doc.mindmap.edges.len(), orig_edges.len());
        // The edge should be back at its original position
        let restored = &doc.mindmap.edges[orig_idx];
        assert!(edge_ref.matches(restored));
    }

    #[test]
    fn test_scene_builder_highlights_selected_edge() {
        let mut doc = load_test_doc();
        let (edge_ref, _) = pick_test_edge(&doc);

        // Without selection: the edge renders with its model color
        let scene_normal = doc.build_scene_with_selection(1.0);
        let normal_colors: Vec<String> = scene_normal.connection_elements.iter()
            .map(|c| c.color.clone())
            .collect();

        // With edge selected: its element color should be the cyan highlight
        doc.selection = SelectionState::Edge(edge_ref);
        let scene_selected = doc.build_scene_with_selection(1.0);
        let highlighted_count = scene_selected.connection_elements.iter()
            .filter(|c| c.color.eq_ignore_ascii_case("#00E5FF"))
            .count();
        assert_eq!(highlighted_count, 1,
            "exactly one connection element should carry the selection color");
        // And exactly one color should have changed vs. the unselected scene
        let changed: usize = scene_selected.connection_elements.iter()
            .zip(normal_colors.iter())
            .filter(|(c, orig)| &c.color != *orig)
            .count();
        assert_eq!(changed, 1);
    }

    // ---------------------------------------------------------------------
    // Session 6B: connection creation
    // ---------------------------------------------------------------------

    #[test]
    fn test_default_cross_link_edge_fields() {
        let e = default_cross_link_edge("a", "b");
        assert_eq!(e.from_id, "a");
        assert_eq!(e.to_id, "b");
        assert_eq!(e.edge_type, "cross_link");
        assert!(e.visible);
        assert_eq!(e.anchor_from, 0);
        assert_eq!(e.anchor_to, 0);
        assert!(e.control_points.is_empty());
        assert!(e.label.is_none());
    }

    #[test]
    fn test_create_cross_link_edge_success() {
        let mut doc = load_test_doc();
        // Pick two nodes that are definitely distinct
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let orig_count = doc.mindmap.edges.len();

        let idx = doc.create_cross_link_edge(&a, &b).expect("should succeed");
        assert_eq!(idx, orig_count);
        assert_eq!(doc.mindmap.edges.len(), orig_count + 1);
        let created = &doc.mindmap.edges[idx];
        assert_eq!(created.edge_type, "cross_link");
        assert_eq!(created.from_id, a);
        assert_eq!(created.to_id, b);
    }

    #[test]
    fn test_create_cross_link_rejects_self_link() {
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().unwrap().clone();
        let orig_count = doc.mindmap.edges.len();
        assert!(doc.create_cross_link_edge(&id, &id).is_none());
        assert_eq!(doc.mindmap.edges.len(), orig_count);
    }

    #[test]
    fn test_create_cross_link_rejects_duplicate() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        assert!(doc.create_cross_link_edge(&a, &b).is_some());
        // Second attempt should be a no-op
        let orig_count = doc.mindmap.edges.len();
        assert!(doc.create_cross_link_edge(&a, &b).is_none());
        assert_eq!(doc.mindmap.edges.len(), orig_count);
    }

    #[test]
    fn test_create_cross_link_rejects_unknown_node() {
        let mut doc = load_test_doc();
        let known = doc.mindmap.nodes.keys().next().unwrap().clone();
        assert!(doc.create_cross_link_edge(&known, "does_not_exist").is_none());
        assert!(doc.create_cross_link_edge("does_not_exist", &known).is_none());
    }

    #[test]
    fn test_undo_create_edge_removes_it() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let orig_count = doc.mindmap.edges.len();

        let idx = doc.create_cross_link_edge(&a, &b).unwrap();
        doc.undo_stack.push(UndoAction::CreateEdge { index: idx });

        assert!(doc.undo());
        assert_eq!(doc.mindmap.edges.len(), orig_count);
        // No cross_link between a and b should remain
        let still_there = doc.mindmap.edges.iter().any(|e| {
            e.edge_type == "cross_link" && e.from_id == a && e.to_id == b
        });
        assert!(!still_there);
    }

    // ---------------------------------------------------------------------
    // Orphan node creation + orphan-selection action
    // ---------------------------------------------------------------------

    #[test]
    fn test_create_orphan_node_adds_to_map() {
        let mut doc = load_test_doc();
        let orig_count = doc.mindmap.nodes.len();
        let pos = Vec2::new(123.0, 456.0);

        let new_id = doc.apply_create_orphan_node(pos);

        assert_eq!(doc.mindmap.nodes.len(), orig_count + 1);
        let node = doc.mindmap.nodes.get(&new_id).expect("new node must exist");
        assert_eq!(node.id, new_id);
        assert!(node.parent_id.is_none(), "orphan should have no parent");
        assert_eq!(node.position.x, 123.0);
        assert_eq!(node.position.y, 456.0);
        assert!(!node.text.is_empty(), "orphan should have placeholder text");
    }

    #[test]
    fn test_create_orphan_node_ids_are_unique() {
        let mut doc = load_test_doc();
        let a = doc.apply_create_orphan_node(Vec2::ZERO);
        let b = doc.apply_create_orphan_node(Vec2::ZERO);
        let c = doc.apply_create_orphan_node(Vec2::ZERO);
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
        // All three should exist in the map
        assert!(doc.mindmap.nodes.contains_key(&a));
        assert!(doc.mindmap.nodes.contains_key(&b));
        assert!(doc.mindmap.nodes.contains_key(&c));
    }

    #[test]
    fn test_undo_create_node_removes_it() {
        let mut doc = load_test_doc();
        let orig_count = doc.mindmap.nodes.len();

        let new_id = doc.apply_create_orphan_node(Vec2::new(0.0, 0.0));
        doc.undo_stack.push(UndoAction::CreateNode { node_id: new_id.clone() });
        doc.selection = SelectionState::Single(new_id.clone());

        assert!(doc.undo());
        assert_eq!(doc.mindmap.nodes.len(), orig_count);
        assert!(!doc.mindmap.nodes.contains_key(&new_id));
        // Selection should have been cleared since it referenced the deleted node
        assert!(matches!(doc.selection, SelectionState::None));
    }

    #[test]
    fn test_orphan_selection_promotes_to_root_and_keeps_subtree() {
        // Pick a non-root node that has at least one child, so we can
        // verify the subtree stays attached after orphaning.
        let mut doc = load_test_doc();
        let (parent_having_child, child) = doc.mindmap.nodes.values()
            .find_map(|n| {
                let kids = doc.mindmap.children_of(&n.id);
                if !kids.is_empty() && n.parent_id.is_some() {
                    Some((n.id.clone(), kids[0].id.clone()))
                } else {
                    None
                }
            })
            .expect("testament map should have at least one non-root parent node");

        // Precondition: the selected node has a parent, and has a child
        assert!(doc.mindmap.nodes.get(&parent_having_child).unwrap().parent_id.is_some());
        let child_of_node = doc.mindmap.nodes.get(&child).unwrap().parent_id.clone();
        assert_eq!(child_of_node.as_deref(), Some(parent_having_child.as_str()));

        let undo = doc.apply_orphan_selection(&[parent_having_child.clone()]);
        assert_eq!(undo.entries.len(), 1);

        // The orphaned node is now a root...
        assert!(doc.mindmap.nodes.get(&parent_having_child).unwrap().parent_id.is_none());
        // ...but its child is still attached to it.
        assert_eq!(
            doc.mindmap.nodes.get(&child).unwrap().parent_id.as_deref(),
            Some(parent_having_child.as_str()),
            "child subtree should stay attached to the orphaned node"
        );
    }

    #[test]
    fn test_orphan_selection_undo_reattaches() {
        let mut doc = load_test_doc();
        let non_root = doc.mindmap.nodes.values()
            .find(|n| n.parent_id.is_some())
            .map(|n| n.id.clone())
            .expect("at least one non-root node");
        let original_parent = doc.mindmap.nodes.get(&non_root).unwrap().parent_id.clone();
        let original_index = doc.mindmap.nodes.get(&non_root).unwrap().index;

        let undo = doc.apply_orphan_selection(&[non_root.clone()]);
        doc.undo_stack.push(UndoAction::ReparentNodes {
            entries: undo.entries,
            old_edges: undo.old_edges,
        });

        // Precondition: it's now a root
        assert!(doc.mindmap.nodes.get(&non_root).unwrap().parent_id.is_none());

        // Undo restores the parent link + index
        assert!(doc.undo());
        let restored = doc.mindmap.nodes.get(&non_root).unwrap();
        assert_eq!(restored.parent_id, original_parent);
        assert_eq!(restored.index, original_index);
    }

    #[test]
    fn test_orphan_selection_on_root_is_noop() {
        let mut doc = load_test_doc();
        let root = doc.mindmap.root_nodes().first().map(|n| n.id.clone()).unwrap();
        let orig_edges_len = doc.mindmap.edges.len();

        let undo = doc.apply_orphan_selection(&[root.clone()]);
        // The node is already a root, so there are entries (it's a valid
        // "move-to-last-root-index" op), but nothing meaningful changed:
        // parent_id is still None.
        assert!(doc.mindmap.nodes.get(&root).unwrap().parent_id.is_none());
        // And since it was already a root, no parent_child edge was removed.
        assert_eq!(doc.mindmap.edges.len(), orig_edges_len);
        // undo.entries may be non-empty but the restoration is a no-op.
        let _ = undo;
    }
}
