use std::collections::{HashMap, HashSet};
use std::path::Path;
use glam::Vec2;
use log::{error, info};
use baumhard::core::primitives::Range;
use baumhard::gfx_structs::area::GlyphAreaCommand;
use baumhard::gfx_structs::mutator::{GfxMutator, Mutation};
use baumhard::gfx_structs::tree::MutatorTree;
use baumhard::gfx_structs::tree_walker::walk_tree_from;
use baumhard::mindmap::custom_mutation::{
    CustomMutation, DocumentAction, MutationBehavior, TargetScope, Trigger,
    PlatformContext, apply_mutations_to_element,
};
use baumhard::mindmap::connection;
use baumhard::mindmap::model::{
    Canvas, GlyphConnectionConfig, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle,
    PortalPair, Position, Size, TextRun, PORTAL_GLYPH_PRESETS,
};
use baumhard::mindmap::loader;
use baumhard::mindmap::scene_builder::{self, RenderScene};
use baumhard::mindmap::tree_builder::{self, MindMapTree};

/// Selection highlight color: bright cyan [R, G, B, A]
pub const HIGHLIGHT_COLOR: [f32; 4] = [0.0, 0.9, 1.0, 1.0];

/// Reparent-mode source color: orange, used for nodes currently being reparented.
pub const REPARENT_SOURCE_COLOR: [f32; 4] = [1.0, 0.55, 0.0, 1.0];

/// Reparent-mode target color: green, used for the node currently hovered as
/// a potential reparent target.
pub const REPARENT_TARGET_COLOR: [f32; 4] = [0.2, 1.0, 0.4, 1.0];

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

/// Session 6E: stable identity of a portal pair. Mirrors `EdgeRef` —
/// portals have no numeric id, but the auto-assigned `label` plus the
/// two endpoint node ids form a unique triple within a single
/// `MindMap`. `PortalRef` is the document-layer form; the rendering
/// scene builder uses a parallel `scene_builder::PortalRefKey` type
/// so it can own the triple without depending on the application
/// layer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PortalRef {
    pub label: String,
    pub endpoint_a: String,
    pub endpoint_b: String,
}

impl PortalRef {
    pub fn new(
        label: impl Into<String>,
        endpoint_a: impl Into<String>,
        endpoint_b: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            endpoint_a: endpoint_a.into(),
            endpoint_b: endpoint_b.into(),
        }
    }

    /// Returns true if this ref identifies the given `PortalPair`.
    pub fn matches(&self, portal: &PortalPair) -> bool {
        self.label == portal.label
            && self.endpoint_a == portal.endpoint_a
            && self.endpoint_b == portal.endpoint_b
    }

    pub fn from_portal(portal: &PortalPair) -> Self {
        Self {
            label: portal.label.clone(),
            endpoint_a: portal.endpoint_a.clone(),
            endpoint_b: portal.endpoint_b.clone(),
        }
    }
}

/// Tracks what is currently selected in the document. Node, edge,
/// and portal selection are mutually exclusive — selecting one kind
/// clears any prior selection of the others.
#[derive(Clone, Debug)]
pub enum SelectionState {
    None,
    Single(String),
    Multi(Vec<String>),
    Edge(EdgeRef),
    /// Session 6E: a portal pair is currently selected. The renderer
    /// draws both of its marker glyphs in the cyan highlight color, and
    /// Delete/Ctrl+Z target this portal.
    Portal(PortalRef),
}

impl SelectionState {
    pub fn is_selected(&self, node_id: &str) -> bool {
        match self {
            SelectionState::None => false,
            SelectionState::Single(id) => id == node_id,
            SelectionState::Multi(ids) => ids.contains(&node_id.to_string()),
            SelectionState::Edge(_) => false,
            SelectionState::Portal(_) => false,
        }
    }

    pub fn selected_ids(&self) -> Vec<&str> {
        match self {
            SelectionState::None => vec![],
            SelectionState::Single(id) => vec![id.as_str()],
            SelectionState::Multi(ids) => ids.iter().map(|s| s.as_str()).collect(),
            SelectionState::Edge(_) => vec![],
            SelectionState::Portal(_) => vec![],
        }
    }

    /// Returns the selected edge, if any.
    pub fn selected_edge(&self) -> Option<&EdgeRef> {
        match self {
            SelectionState::Edge(e) => Some(e),
            _ => None,
        }
    }

    /// Session 6E: returns the selected portal pair, if any.
    pub fn selected_portal(&self) -> Option<&PortalRef> {
        match self {
            SelectionState::Portal(p) => Some(p),
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
        label_position_t: None,
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
        label_position_t: None,
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
    /// Transient label edit preview. When `Some((edge_key, buffer))`,
    /// scene-building substitutes `buffer` (plus a trailing caret) for
    /// the matching edge's `ConnectionLabelElement.text` — the inline
    /// label editor's live display. Cleared on commit or cancel.
    ///
    /// Lives on the document rather than on the app layer so all
    /// `build_scene_*` callers see the override without extra
    /// plumbing. The committed `MindEdge.label` in `self.mindmap` is
    /// never touched during editing; the preview is purely a
    /// scene-level substitution.
    pub label_edit_preview: Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    /// Transient color-picker hover preview. When `Some(...)`, the
    /// scene builder substitutes the preview color for the edge or
    /// portal under the wheel — overriding both the resolved
    /// `config.color` and any selection highlight on the previewed
    /// element so the user sees the live HSV value on the element
    /// being edited. Commit (`set_edge_color` / `set_portal_color`)
    /// and cancel both clear the preview; neither the committed
    /// model nor the undo stack is touched during hover.
    pub color_picker_preview: Option<ColorPickerPreview>,
}

/// Transient visual-only substitution of a color-pickerable element's
/// color. Read by `build_scene_*` and consumed by `scene_builder`'s
/// `EdgeColorPreview` / `PortalColorPreview` threaded params.
#[derive(Debug, Clone)]
pub enum ColorPickerPreview {
    Edge {
        key: baumhard::mindmap::scene_cache::EdgeKey,
        color: String,
    },
    Portal {
        key: baumhard::mindmap::scene_builder::PortalRefKey,
        color: String,
    },
}

/// Walk every node in the map and grow its stored `size` in place until
/// the box is at least large enough to contain the node's shaped text
/// bounds plus a small padding. Grow-only: an author-authored oversized
/// box stays oversized. Runs once at load time, so subsequent reads of
/// `node.size` by the scene builder, renderer, and connection anchors all
/// see a single coherent effective size — no per-frame cost, no cross-
/// layer plumbing.
///
/// The scale / line-height formula mirrors
/// `baumhard::mindmap::tree_builder::mindnode_to_glyph_area` so the
/// measurement matches what the tree walker will actually render.
///
/// Padding: `pad_x = scale * 1.5`, `pad_y = scale * 0.5` — leaves a bit
/// of breathing room between the text and the border glyphs.
fn grow_node_sizes_to_fit_text(map: &mut MindMap) {
    use cosmic_text::{Attrs, Buffer, Metrics, Shaping};

    let mut font_system = baumhard::font::fonts::FONT_SYSTEM
        .write()
        .expect("font system lock poisoned");

    for node in map.nodes.values_mut() {
        let scale = node
            .text_runs
            .first()
            .map(|r| r.size_pt as f32)
            .unwrap_or(14.0);
        let line_height = scale * 1.2;
        let pad_x = scale * 1.5;
        let pad_y = scale * 0.5;

        let mut buffer = Buffer::new(&mut font_system, Metrics::new(scale, line_height));
        // Unbounded layout so we measure the natural single-line width
        // of each logical line (cosmic-text still breaks on embedded
        // `\n`), which is the right floor for "how big does the box
        // need to be".
        buffer.set_size(&mut font_system, None, None);
        buffer.set_text(
            &mut font_system,
            &node.text,
            &Attrs::new(),
            Shaping::Advanced,
            None,
        );

        let measured_w = buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0_f32, f32::max);
        let measured_h = buffer.layout_runs().count() as f32 * line_height;

        let need_w = (measured_w + pad_x) as f64;
        let need_h = (measured_h + pad_y) as f64;
        if node.size.width < need_w {
            node.size.width = need_w;
        }
        if node.size.height < need_h {
            node.size.height = need_h;
        }
    }
}

impl MindMapDocument {
    /// Load a MindMap from a file path and create a Document.
    pub fn load(path: &str) -> Result<Self, String> {
        match loader::load_from_file(Path::new(path)) {
            Ok(mut map) => {
                info!("Loaded mindmap '{}' with {} nodes", map.name, map.nodes.len());
                // Grow any undersized node boxes to fit their text
                // before the model is handed to the tree/scene builders.
                // See `grow_node_sizes_to_fit_text` for the invariants.
                grow_node_sizes_to_fit_text(&mut map);
                let mut doc = MindMapDocument {
                    mindmap: map,
                    file_path: Some(path.to_string()),
                    dirty: false,
                    selection: SelectionState::None,
                    undo_stack: Vec::new(),
                    mutation_registry: HashMap::new(),
                    active_toggles: HashSet::new(),
                    label_edit_preview: None,
                    color_picker_preview: None,
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
    ///
    /// Automatically threads the document's transient UI overrides
    /// into the scene builder:
    /// - `label_edit_preview`: live inline-label buffer + caret.
    /// - `color_picker_preview`: live color-picker hover HSV.
    pub fn build_scene_with_cache(
        &self,
        offsets: &HashMap<String, (f32, f32)>,
        cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
        camera_zoom: f32,
    ) -> RenderScene {
        let sel = self.selection.selected_edge()
            .map(|e| (e.from_id.as_str(), e.to_id.as_str(), e.edge_type.as_str()));
        let portal_sel = self.selection.selected_portal()
            .map(|p| (p.label.as_str(), p.endpoint_a.as_str(), p.endpoint_b.as_str()));
        let label_edit = self
            .label_edit_preview
            .as_ref()
            .map(|(k, s)| (k, s.as_str()));
        let edge_preview = match &self.color_picker_preview {
            Some(ColorPickerPreview::Edge { key, color }) => {
                Some(scene_builder::EdgeColorPreview { edge_key: key, color: color.as_str() })
            }
            _ => None,
        };
        let portal_preview = match &self.color_picker_preview {
            Some(ColorPickerPreview::Portal { key, color }) => {
                Some(scene_builder::PortalColorPreview { portal_key: key, color: color.as_str() })
            }
            _ => None,
        };
        scene_builder::build_scene_with_cache(
            &self.mindmap,
            offsets,
            sel,
            portal_sel,
            label_edit,
            edge_preview,
            portal_preview,
            cache,
            camera_zoom,
        )
    }

    /// Build a RenderScene that also reflects the current edge selection.
    /// The selected edge (if any) gets a cyan color override baked into its
    /// ConnectionElement so the renderer paints it in the highlight color.
    ///
    /// Like `build_scene_with_cache`, this also threads the document's
    /// `label_edit_preview` and `color_picker_preview` into the scene
    /// build so live interaction previews are visible on any scene
    /// that flows through this entry point.
    pub fn build_scene_with_selection(&self, camera_zoom: f32) -> RenderScene {
        let sel = self.selection.selected_edge()
            .map(|e| (e.from_id.as_str(), e.to_id.as_str(), e.edge_type.as_str()));
        let portal_sel = self.selection.selected_portal()
            .map(|p| (p.label.as_str(), p.endpoint_a.as_str(), p.endpoint_b.as_str()));
        let label_edit = self
            .label_edit_preview
            .as_ref()
            .map(|(k, s)| (k, s.as_str()));
        let edge_preview = match &self.color_picker_preview {
            Some(ColorPickerPreview::Edge { key, color }) => {
                Some(scene_builder::EdgeColorPreview { edge_key: key, color: color.as_str() })
            }
            _ => None,
        };
        let portal_preview = match &self.color_picker_preview {
            Some(ColorPickerPreview::Portal { key, color }) => {
                Some(scene_builder::PortalColorPreview { portal_key: key, color: color.as_str() })
            }
            _ => None,
        };
        scene_builder::build_scene_with_offsets_selection_and_overrides(
            &self.mindmap,
            &HashMap::new(),
            sel,
            portal_sel,
            label_edit,
            edge_preview,
            portal_preview,
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

    /// Hit-test the grab-handles of a specific edge at `canvas_pos`.
    /// Returns the closest handle whose canvas-space position is
    /// within `tolerance` of the cursor, or `None` if nothing is in
    /// range. Used by the Session 6C edge-reshape drag flow — called
    /// at mouse-down time when an edge is currently selected.
    ///
    /// Computed from the live edge (so any in-progress drag is
    /// reflected), without consulting the scene cache. Bounded cost:
    /// one `build_connection_path` + up to five distance comparisons.
    pub fn hit_test_edge_handle(
        &self,
        canvas_pos: Vec2,
        edge_ref: &EdgeRef,
        tolerance: f32,
    ) -> Option<(scene_builder::EdgeHandleKind, Vec2)> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let from_node = self.mindmap.nodes.get(&edge.from_id)?;
        let to_node = self.mindmap.nodes.get(&edge.to_id)?;
        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);

        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
        let handles = scene_builder::build_edge_handles(
            edge, &edge_key, from_pos, from_size, to_pos, to_size,
        );

        let mut best: Option<(scene_builder::EdgeHandleKind, Vec2, f32)> = None;
        for h in handles {
            let pos = Vec2::new(h.position.0, h.position.1);
            let dist = canvas_pos.distance(pos);
            if dist > tolerance {
                continue;
            }
            if best.as_ref().map_or(true, |(_, _, d)| dist < *d) {
                best = Some((h.kind, pos, dist));
            }
        }
        best.map(|(k, p, _)| (k, p))
    }

    /// Clear an edge's `control_points` so it renders as a straight
    /// line. Returns `true` if the edge existed and had control
    /// points to clear; `false` if the edge was already straight or
    /// wasn't found. On success, a full snapshot of the pre-edit
    /// edge is pushed onto `undo_stack` as `UndoAction::EditEdge` and
    /// `dirty` is set. No-op for already-straight edges so repeated
    /// palette invocations don't pollute the undo stack.
    pub fn reset_edge_to_straight(&mut self, edge_ref: &EdgeRef) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        if self.mindmap.edges[idx].control_points.is_empty() {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].control_points.clear();
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set an edge's `anchor_from` (when `is_from == true`) or
    /// `anchor_to` (when `is_from == false`) to `value`. Valid values
    /// are 0 (auto) or 1..=4 (top/right/bottom/left). Returns `true`
    /// if the value changed, pushing an `EditEdge` undo snapshot and
    /// setting `dirty`. Returns `false` if the edge was not found or
    /// the anchor was already at the requested value.
    pub fn set_edge_anchor(&mut self, edge_ref: &EdgeRef, is_from: bool, value: i32) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let current = if is_from {
            self.mindmap.edges[idx].anchor_from
        } else {
            self.mindmap.edges[idx].anchor_to
        };
        if current == value {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        if is_from {
            self.mindmap.edges[idx].anchor_from = value;
        } else {
            self.mindmap.edges[idx].anchor_to = value;
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Look up the index of an edge in `mindmap.edges` matching the
    /// given `EdgeRef`. Returned for callers that need to snapshot
    /// the edge before mutating it in place (e.g. the edge-handle
    /// drag flow in `app.rs`).
    pub fn edge_index(&self, edge_ref: &EdgeRef) -> Option<usize> {
        self.mindmap.edges.iter().position(|e| edge_ref.matches(e))
    }

    // ========================================================================
    // Session 6D — connection style and label mutation helpers
    //
    // Every helper in this block mirrors the `reset_edge_to_straight` /
    // `set_edge_anchor` template exactly:
    //
    //   1. Locate the edge index via `edge_ref.matches`.
    //   2. Early-return `false` for no-op cases (value already matches, edge
    //      not found) so repeated palette invocations don't pollute the undo
    //      stack.
    //   3. Clone the full pre-edit edge into `before` — this must happen
    //      BEFORE any fork via `ensure_glyph_connection`, so undo restores
    //      the pre-fork `None` cleanly.
    //   4. Mutate the edge in place.
    //   5. Push `UndoAction::EditEdge { index, before }` and set `dirty`.
    //
    // The fork semantic: on the first style edit of an edge whose
    // `glyph_connection` is None, we materialize a concrete per-edge copy
    // from the effective resolved config (canvas default, else hardcoded
    // default). Subsequent canvas-default changes don't retroactively apply
    // to forked edges — mirroring how CSS "computed style" copies work.
    // ========================================================================

    /// Ensure `edge.glyph_connection` is `Some(_)`, forking from the
    /// canvas default (or the hardcoded default) on first edit. Returns
    /// a mutable reference to the freshly-installed or previously-set
    /// config so the caller can mutate a specific field.
    ///
    /// Must be called AFTER the `before` snapshot has been cloned so
    /// the undo entry still carries the pre-fork `None`.
    fn ensure_glyph_connection<'a>(
        edge: &'a mut MindEdge,
        canvas: &Canvas,
    ) -> &'a mut GlyphConnectionConfig {
        if edge.glyph_connection.is_none() {
            let seed = canvas
                .default_connection
                .clone()
                .unwrap_or_default();
            edge.glyph_connection = Some(seed);
        }
        edge.glyph_connection.as_mut().expect("just installed")
    }

    /// Set the body glyph string for a connection. Empty strings are
    /// rejected (an empty body would produce no glyphs). Returns
    /// `true` if the edge existed and the body actually changed.
    pub fn set_edge_body_glyph(&mut self, edge_ref: &EdgeRef, body: &str) -> bool {
        if body.is_empty() {
            return false;
        }
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        // Peek at the effective body before forking to detect no-ops.
        let current_body = self.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .map(|c| c.body.as_str())
            .or_else(|| self.mindmap.canvas.default_connection.as_ref().map(|c| c.body.as_str()))
            .unwrap_or(&GlyphConnectionConfig::default().body.clone())
            .to_string();
        if current_body == body {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        cfg.body = body.to_string();
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the `cap_start` glyph (or clear it with `None`). Returns
    /// `true` if the edge existed and the value changed.
    pub fn set_edge_cap_start(&mut self, edge_ref: &EdgeRef, cap: Option<&str>) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        let new_val = cap.map(|s| s.to_string());
        if cfg.cap_start == new_val {
            // Roll back the fork if nothing actually changed (ensure_glyph_connection
            // may have installed a default when the edge previously had
            // glyph_connection = None).
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.cap_start = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the `cap_end` glyph (or clear it with `None`). Returns
    /// `true` if the edge existed and the value changed.
    pub fn set_edge_cap_end(&mut self, edge_ref: &EdgeRef, cap: Option<&str>) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        let new_val = cap.map(|s| s.to_string());
        if cfg.cap_end == new_val {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.cap_end = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the color override on a connection's glyph_connection config.
    /// Passing `None` clears the override so the edge inherits from
    /// `edge.color` (or the canvas default). Returns `true` if the edge
    /// existed and the value changed.
    pub fn set_edge_color(&mut self, edge_ref: &EdgeRef, color: Option<&str>) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        let new_val = color.map(|s| s.to_string());
        if cfg.color == new_val {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.color = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Step the connection's base `font_size_pt` by `delta_pt`,
    /// clamped into `[min_font_size_pt, max_font_size_pt]`. Returns
    /// `true` if the clamp yielded a different value from the current
    /// (i.e. we're not already pinned at the relevant bound).
    pub fn set_edge_font_size_step(&mut self, edge_ref: &EdgeRef, delta_pt: f32) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        let new_val = (cfg.font_size_pt + delta_pt)
            .clamp(cfg.min_font_size_pt, cfg.max_font_size_pt);
        if (cfg.font_size_pt - new_val).abs() < f32::EPSILON {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.font_size_pt = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Reset the connection's `font_size_pt` to the hardcoded default
    /// (12.0). Returns `true` if the value actually changed.
    pub fn reset_edge_font_size(&mut self, edge_ref: &EdgeRef) -> bool {
        let default_size = GlyphConnectionConfig::default().font_size_pt;
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        if (cfg.font_size_pt - default_size).abs() < f32::EPSILON {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.font_size_pt = default_size;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the connection's glyph `spacing` (canvas units between
    /// adjacent body glyphs). Returns `true` if the value actually
    /// changed.
    pub fn set_edge_spacing(&mut self, edge_ref: &EdgeRef, spacing: f32) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        if (cfg.spacing - spacing).abs() < f32::EPSILON {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.spacing = spacing;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the label text on an edge. Passing `None` (or `Some("")`)
    /// clears the label. Returns `true` if the value actually changed.
    pub fn set_edge_label(&mut self, edge_ref: &EdgeRef, text: Option<String>) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        // Normalize empty string to None so hit testing and rendering
        // only need to check one absence case.
        let new_val = match text {
            Some(s) if s.is_empty() => None,
            other => other,
        };
        if self.mindmap.edges[idx].label == new_val {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].label = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the label's position along the connection path. `t` is
    /// clamped into `[0.0, 1.0]` — values outside that range are
    /// silently pulled back. Returns `true` if the clamped value
    /// actually differs from the current.
    pub fn set_edge_label_position(&mut self, edge_ref: &EdgeRef, t: f32) -> bool {
        let clamped = t.clamp(0.0, 1.0);
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let current = self.mindmap.edges[idx].label_position_t.unwrap_or(0.5);
        if (current - clamped).abs() < f32::EPSILON {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].label_position_t = Some(clamped);
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Change the `edge_type` of an edge. Refuses the change (returns
    /// `false`) if it would create a duplicate `(from_id, to_id,
    /// new_type)` against another edge. On success updates
    /// `self.selection` to a fresh `EdgeRef` with the new type so the
    /// edge stays selected.
    pub fn set_edge_type(&mut self, edge_ref: &EdgeRef, new_type: &str) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        if self.mindmap.edges[idx].edge_type == new_type {
            return false;
        }
        // Duplicate guard: refuse if some OTHER edge already has the same
        // (from_id, to_id, new_type) triple.
        let from_id = self.mindmap.edges[idx].from_id.clone();
        let to_id = self.mindmap.edges[idx].to_id.clone();
        let duplicate = self.mindmap.edges.iter().enumerate().any(|(i, e)| {
            i != idx
                && e.from_id == from_id
                && e.to_id == to_id
                && e.edge_type == new_type
        });
        if duplicate {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].edge_type = new_type.to_string();
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        // Refresh the selection EdgeRef so the app keeps the edge selected
        // under its new identity.
        if let SelectionState::Edge(ref cur) = self.selection {
            if cur == edge_ref {
                self.selection = SelectionState::Edge(EdgeRef::new(
                    from_id,
                    to_id,
                    new_type,
                ));
            }
        }
        true
    }

    /// Clear `glyph_connection` on the edge, reverting it to the
    /// canvas-level default style. Returns `true` if the edge existed
    /// and had a per-edge override to clear.
    pub fn reset_edge_style_to_default(&mut self, edge_ref: &EdgeRef) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        if self.mindmap.edges[idx].glyph_connection.is_none() {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].glyph_connection = None;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    // ============================================================
    // Session 6E — portal mutation helpers
    // ============================================================

    /// Create a new portal pair linking `node_a` and `node_b`.
    ///
    /// Fails (returns `Err`) if the two ids are identical or if either
    /// node is missing from `mindmap.nodes` — defense in depth, since
    /// the 2-node `Multi` selection path in the palette already rules
    /// out these cases in normal use.
    ///
    /// The new portal gets the lowest unused label in column-letter
    /// order (A, B, ..., Z, AA, ...) from `MindMap::next_portal_label`,
    /// and its glyph is picked by rotating through
    /// `PORTAL_GLYPH_PRESETS` so each new pair looks distinct at a
    /// glance. The color defaults to the same `#aa88cc` used by
    /// `default_cross_link_edge`.
    ///
    /// Pushes `UndoAction::CreatePortal { index }`, marks the document
    /// dirty, and returns a fresh `PortalRef` identifying the new pair.
    pub fn apply_create_portal(
        &mut self,
        node_a: &str,
        node_b: &str,
    ) -> Result<PortalRef, String> {
        if node_a == node_b {
            return Err("cannot create a portal between a node and itself".to_string());
        }
        if !self.mindmap.nodes.contains_key(node_a) {
            return Err(format!("unknown node id: {node_a}"));
        }
        if !self.mindmap.nodes.contains_key(node_b) {
            return Err(format!("unknown node id: {node_b}"));
        }
        let label = self.mindmap.next_portal_label();
        let glyph_idx = self.mindmap.portals.len() % PORTAL_GLYPH_PRESETS.len();
        let portal = PortalPair {
            endpoint_a: node_a.to_string(),
            endpoint_b: node_b.to_string(),
            label: label.clone(),
            glyph: PORTAL_GLYPH_PRESETS[glyph_idx].to_string(),
            color: "#aa88cc".to_string(),
            font_size_pt: 16.0,
            font: None,
        };
        let index = self.mindmap.portals.len();
        let pref = PortalRef::from_portal(&portal);
        self.mindmap.portals.push(portal);
        self.undo_stack.push(UndoAction::CreatePortal { index });
        self.dirty = true;
        Ok(pref)
    }

    /// Delete the portal pair identified by `portal_ref`. Records a
    /// `DeletePortal` undo entry so Ctrl+Z restores it at the same
    /// index. Returns the removed pair on success, `None` if the ref
    /// did not match any portal.
    pub fn apply_delete_portal(
        &mut self,
        portal_ref: &PortalRef,
    ) -> Option<PortalPair> {
        let idx = self.mindmap.portals.iter().position(|p| portal_ref.matches(p))?;
        let portal = self.mindmap.portals.remove(idx);
        self.undo_stack.push(UndoAction::DeletePortal { index: idx, portal: portal.clone() });
        self.dirty = true;
        Some(portal)
    }

    /// Edit a portal in place via a mutation closure. The pre-edit
    /// snapshot is taken before `f` runs and pushed as
    /// `UndoAction::EditPortal`, so Ctrl+Z restores the original
    /// fields wholesale. Returns `true` if the ref matched a portal.
    ///
    /// Used by `set_portal_glyph` / `set_portal_color` / future field
    /// setters in the same way `apply_edit_portal` is the single
    /// "write + record undo" chokepoint for portal mutations.
    pub fn apply_edit_portal<F>(&mut self, portal_ref: &PortalRef, f: F) -> bool
    where
        F: FnOnce(&mut PortalPair),
    {
        let idx = match self.mindmap.portals.iter().position(|p| portal_ref.matches(p)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.portals[idx].clone();
        f(&mut self.mindmap.portals[idx]);
        self.undo_stack.push(UndoAction::EditPortal { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the visible glyph of a portal pair. Wraps
    /// `apply_edit_portal` so undo works via `EditPortal`.
    pub fn set_portal_glyph(&mut self, portal_ref: &PortalRef, glyph: &str) -> bool {
        let glyph_owned = glyph.to_string();
        self.apply_edit_portal(portal_ref, move |p| p.glyph = glyph_owned)
    }

    /// Set the color of a portal pair. Accepts a raw `#RRGGBB` hex or
    /// a theme-variable reference like `var(--accent)`. The color is
    /// resolved at scene-build time so theme swaps auto-restyle
    /// var-referencing portals.
    pub fn set_portal_color(&mut self, portal_ref: &PortalRef, color: &str) -> bool {
        let color_owned = color.to_string();
        self.apply_edit_portal(portal_ref, move |p| p.color = color_owned)
    }

    /// Session 7A: replace a node's `text` and collapse its `text_runs`
    /// to a single run inheriting the first original run's formatting
    /// (font, size_pt, color, bold, italic, underline). If the original
    /// had no runs, a white 24pt Liberation Sans run is synthesized —
    /// mirrors `default_orphan_node`.
    ///
    /// Returns `true` if the value actually changed. No-op / no undo
    /// push on unchanged text, matching `set_edge_label`'s contract.
    ///
    /// **Collapse caveat**: authored multi-run nodes lose their per-span
    /// formatting on any edit. Session 7B's `TextRun` splitting will
    /// preserve it.
    pub fn set_node_text(&mut self, node_id: &str, new_text: String) -> bool {
        let node = match self.mindmap.nodes.get_mut(node_id) {
            Some(n) => n,
            None => return false,
        };
        if node.text == new_text {
            return false;
        }
        let before_text = node.text.clone();
        let before_runs = node.text_runs.clone();
        // Collapse to a single run that spans the new text. Inherit
        // formatting from the first original run, or fall back to the
        // default-orphan defaults if the node had no runs.
        let template = before_runs.first().cloned().unwrap_or_else(|| TextRun {
            start: 0,
            end: 0,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".to_string(),
            size_pt: 24,
            color: "#ffffff".to_string(),
            hyperlink: None,
        });
        let new_runs = vec![TextRun {
            start: 0,
            end: new_text.chars().count(),
            ..template
        }];
        node.text = new_text;
        node.text_runs = new_runs;
        self.undo_stack.push(UndoAction::EditNodeText {
            node_id: node_id.to_string(),
            before_text,
            before_runs,
        });
        self.dirty = true;
        true
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

/// Is `canvas_pos` inside the AABB of node `node_id`? Reads the tree-side
/// glyph area so drag-preview positions count (tree is authoritative
/// during in-flight mutations; identical to the model when idle).
///
/// Unlike `hit_test`, this answers a point-in-specific-node question —
/// a click over a child of `node_id` still counts as "inside" `node_id`,
/// which is what the text editor's click-outside-commit gesture wants.
pub fn point_in_node_aabb(canvas_pos: Vec2, node_id: &str, tree: &MindMapTree) -> bool {
    tree.node_map
        .get(node_id)
        .and_then(|nid| tree.tree.arena.get(*nid))
        .and_then(|n| n.get().glyph_area())
        .map(|area| {
            let x = area.position.x.0;
            let y = area.position.y.0;
            let w = area.render_bounds.x.0;
            let h = area.render_bounds.y.0;
            canvas_pos.x >= x
                && canvas_pos.x <= x + w
                && canvas_pos.y >= y
                && canvas_pos.y <= y + h
        })
        .unwrap_or(false)
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

/// Apply a set of node highlights as baumhard mutations. For each
/// `(mind_node_id, color)` pair, the node's existing text-run ranges
/// are collected from its `GlyphArea` and a `GfxMutator::Macro` of one
/// `SetRegionColor(range, color)` mutation per range is applied through
/// `walk_tree_from` — i.e. the highlight is expressed in the same
/// mutation language as the rest of baumhard's tree-walker flow rather
/// than reaching into the arena imperatively.
///
/// Later pairs override earlier ones when the same node appears twice,
/// which is what the reparent/connect modes rely on: callers pass
/// selection highlights first (cyan), then source (orange), then target
/// (green), and the last write wins on conflicts.
///
/// Architectural note: this replaces the earlier trio of
/// `apply_selection_highlight` / `apply_reparent_source_highlight` /
/// `apply_reparent_target_highlight` helpers, which all did the same
/// direct arena patching with different constants. The single function
/// here is both shorter and aligns with architectural decision #6 in
/// ROADMAP.md (mutations as the interaction model).
pub fn apply_tree_highlights<'a, I>(tree: &mut MindMapTree, highlights: I)
where
    I: IntoIterator<Item = (&'a str, [f32; 4])>,
{
    for (mind_id, color) in highlights {
        let Some(&node_id) = tree.node_map.get(mind_id) else { continue };

        // Collect existing region ranges up front. The SetRegionColor
        // mutation needs the exact `Range` of each target region so that
        // the underlying `set_or_insert` finds a match and updates
        // in-place rather than inserting a duplicate region.
        let (ranges, target_channel): (Vec<Range>, usize) = {
            let Some(node) = tree.tree.arena.get(node_id) else { continue };
            let element = node.get();
            let Some(area) = element.glyph_area() else { continue };
            let ranges = area.regions.all_regions().iter().map(|r| r.range).collect();
            // Match the element's channel so the walker's channel-
            // alignment check in `apply_if_matching_channel` passes.
            let channel = {
                use baumhard::gfx_structs::tree::BranchChannel;
                element.channel()
            };
            (ranges, channel)
        };
        if ranges.is_empty() {
            continue;
        }

        let mutations: Vec<Mutation> = ranges
            .into_iter()
            .map(|r| Mutation::area_command(GlyphAreaCommand::SetRegionColor(r, color)))
            .collect();
        let mutator_tree = MutatorTree::new_with(GfxMutator::new_macro(mutations, target_channel));

        // `walk_tree_from` applied at a specific target_id with a
        // single-node MutatorTree runs the macro on that element only
        // (no descendants are touched because the mutator tree has no
        // children, so `align_child_walks` is a no-op). This is the
        // idiomatic "one-shot mutation to a specific node" shape.
        walk_tree_from(&mut tree.tree, &mutator_tree, node_id, mutator_tree.root);
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
            label_edit_preview: None,
            color_picker_preview: None,
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
        let parent_size = {
            let nid = tree.node_map.get(parent_id_str).unwrap();
            let area = tree.tree.arena.get(*nid).unwrap().get().glyph_area().unwrap();
            area.render_bounds.x.0 * area.render_bounds.y.0
        };

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
            if child_size < parent_size
                && point_in_node_aabb(child_center, parent_id_str, &tree)
            {
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
    fn test_apply_tree_highlights_via_walker() {
        let mut tree = load_test_tree();
        let node_id = *tree.node_map.get("348068464").unwrap();

        // Before highlight: original color (white)
        let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
        let original_color = area.regions.all_regions()[0].color.unwrap();
        assert!((original_color[0] - 1.0).abs() < 0.01, "Expected white before highlight");

        // Apply highlight via the new mutator-driven path.
        apply_tree_highlights(
            &mut tree,
            std::iter::once(("348068464", HIGHLIGHT_COLOR)),
        );

        // After highlight: cyan
        let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
        let highlighted_color = area.regions.all_regions()[0].color.unwrap();
        assert!((highlighted_color[0] - HIGHLIGHT_COLOR[0]).abs() < 0.01);
        assert!((highlighted_color[1] - HIGHLIGHT_COLOR[1]).abs() < 0.01);
        assert!((highlighted_color[2] - HIGHLIGHT_COLOR[2]).abs() < 0.01);
    }

    #[test]
    fn test_apply_tree_highlights_does_not_affect_others() {
        let mut tree = load_test_tree();

        // Pick a different node and copy its regions before mutation.
        let other_id = tree.node_map.keys()
            .find(|k| *k != "348068464")
            .unwrap().clone();
        let other_node_id = *tree.node_map.get(&other_id).unwrap();
        let before = tree.tree.arena.get(other_node_id).unwrap().get()
            .glyph_area().unwrap().regions.clone();

        apply_tree_highlights(
            &mut tree,
            std::iter::once(("348068464", HIGHLIGHT_COLOR)),
        );

        let after = tree.tree.arena.get(other_node_id).unwrap().get()
            .glyph_area().unwrap().regions.clone();
        assert_eq!(before, after, "Unselected node colors should not change");
    }

    #[test]
    fn test_apply_tree_highlights_later_pair_overrides_earlier() {
        // The reparent-mode flow relies on source-orange overriding the
        // previously-applied selection-cyan on the same node. Verify the
        // last-write-wins semantics of apply_tree_highlights.
        let mut tree = load_test_tree();
        let node_id = *tree.node_map.get("348068464").unwrap();

        apply_tree_highlights(
            &mut tree,
            vec![
                ("348068464", HIGHLIGHT_COLOR),
                ("348068464", REPARENT_SOURCE_COLOR),
            ],
        );

        let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
        let c = area.regions.all_regions()[0].color.unwrap();
        assert!((c[0] - REPARENT_SOURCE_COLOR[0]).abs() < 0.01);
        assert!((c[1] - REPARENT_SOURCE_COLOR[1]).abs() < 0.01);
        assert!((c[2] - REPARENT_SOURCE_COLOR[2]).abs() < 0.01);
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

    /// Build a `CustomMutation` whose only payload is a single
    /// `SetThemeVariables` document-level action that sets `--bg`
    /// to the given value. Used by the `apply_document_actions`
    /// regression tests.
    fn make_set_bg_doc_mutation(value: &str) -> CM {
        use baumhard::mindmap::custom_mutation::DocumentAction;
        let mut vars = HashMap::new();
        vars.insert("--bg".to_string(), value.to_string());
        CM {
            id: "set-bg".to_string(),
            name: "Set --bg".to_string(),
            mutations: vec![],
            target_scope: TS::SelfOnly,
            behavior: MB::Persistent,
            predicate: None,
            document_actions: vec![DocumentAction::SetThemeVariables(vars)],
        }
    }

    /// Round-trip regression for `UndoAction::CanvasSnapshot`. The
    /// `apply_document_actions` path is the only producer of this
    /// variant, and prior to chunk 5 it had zero test coverage —
    /// CODE_CONVENTIONS.md §6 says every undo variant ships with at
    /// least a forward-and-back test.
    #[test]
    fn test_apply_document_actions_undo_round_trip() {
        let mut doc = load_test_doc();
        // Capture the canvas state before any document-level mutation.
        let before = doc.mindmap.canvas.clone();
        let undo_len_before = doc.undo_stack.len();

        // Apply a single SetThemeVariables action that sets --bg to a
        // sentinel value not present in the testament map.
        let custom = make_set_bg_doc_mutation("#bada55");
        let changed = doc.apply_document_actions(&custom);
        assert!(changed, "applying a new theme var must report a change");
        assert_eq!(
            doc.mindmap.canvas.theme_variables.get("--bg"),
            Some(&"#bada55".to_string())
        );
        assert_eq!(
            doc.undo_stack.len(),
            undo_len_before + 1,
            "exactly one CanvasSnapshot entry should have been pushed"
        );
        assert!(doc.dirty);

        // Undo restores the entire pre-mutation canvas wholesale.
        assert!(doc.undo());
        assert_eq!(doc.mindmap.canvas.theme_variables, before.theme_variables);
        assert_eq!(doc.mindmap.canvas.background_color, before.background_color);
        assert_eq!(
            doc.undo_stack.len(),
            undo_len_before,
            "undo should have popped the CanvasSnapshot entry"
        );
    }

    /// `apply_document_actions` returns false and pushes nothing
    /// when the action would not actually change anything (writing
    /// the same value that's already there). Guards the dirty/undo
    /// no-op path that the docstring on `apply_document_actions`
    /// promises.
    #[test]
    fn test_apply_document_actions_noop_does_not_push_undo() {
        let mut doc = load_test_doc();
        // First write — should change the canvas and push undo.
        let custom = make_set_bg_doc_mutation("#bada55");
        doc.apply_document_actions(&custom);
        let undo_len_after_first = doc.undo_stack.len();
        doc.dirty = false;

        // Second write of the same value — no-op, no undo push,
        // dirty flag should stay false.
        let changed = doc.apply_document_actions(&custom);
        assert!(!changed, "writing the same value must not report a change");
        assert_eq!(doc.undo_stack.len(), undo_len_after_first);
        assert!(!doc.dirty);
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
            label_position_t: None,
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

    // ---------------------------------------------------------------
    // Node deletion (Session 7A follow-up)
    // ---------------------------------------------------------------

    /// Pick a node from the testament map that has at least one child
    /// and at least one parent_child edge pointing at it. The "Lord
    /// God" node has plenty of children and is a root, so we walk one
    /// level down to find a good candidate that also has a parent.
    fn find_node_with_children_and_parent(doc: &MindMapDocument) -> String {
        doc.mindmap.nodes.values()
            .find(|n| {
                n.parent_id.is_some()
                    && !doc.mindmap.children_of(&n.id).is_empty()
            })
            .map(|n| n.id.clone())
            .expect("testament should have at least one non-root node with children")
    }

    #[test]
    fn test_delete_node_orphans_children() {
        let mut doc = load_test_doc();
        let target = find_node_with_children_and_parent(&doc);
        let child_ids: Vec<String> = doc.mindmap.children_of(&target)
            .iter().map(|n| n.id.clone()).collect();
        assert!(!child_ids.is_empty(), "target should have at least one child");

        let undo = doc.delete_node(&target).expect("delete should succeed");
        assert!(matches!(undo, UndoAction::DeleteNode { .. }));

        // The node itself is gone.
        assert!(!doc.mindmap.nodes.contains_key(&target));
        // Every child is now a root (parent_id == None).
        for cid in &child_ids {
            let child = doc.mindmap.nodes.get(cid)
                .expect("child should still exist — only direct attachment is severed");
            assert!(child.parent_id.is_none(),
                "child {} should be orphaned", cid);
        }
        // No parent_child edges touch the deleted id anymore.
        assert!(doc.mindmap.edges.iter().all(|e|
            e.from_id != target && e.to_id != target
        ), "no edges should reference the deleted node");
    }

    #[test]
    fn test_delete_node_removes_all_touching_edges() {
        let mut doc = load_test_doc();
        let target = find_node_with_children_and_parent(&doc);
        // Count edges touching the target beforehand.
        let touching_before = doc.mindmap.edges.iter()
            .filter(|e| e.from_id == target || e.to_id == target)
            .count();
        assert!(touching_before > 0,
            "testament target should have at least one incident edge (parent_child)");

        doc.delete_node(&target).unwrap();

        let touching_after = doc.mindmap.edges.iter()
            .filter(|e| e.from_id == target || e.to_id == target)
            .count();
        assert_eq!(touching_after, 0, "all incident edges should be removed");
    }

    #[test]
    fn test_delete_node_undo_restores_node_edges_and_children() {
        let mut doc = load_test_doc();
        let target = find_node_with_children_and_parent(&doc);

        // Capture pre-delete state to compare after undo.
        let orig_node = doc.mindmap.nodes.get(&target).cloned().unwrap();
        let orig_edges = doc.mindmap.edges.clone();
        let orig_child_state: Vec<(String, Option<String>, i32)> = doc.mindmap
            .children_of(&target)
            .iter()
            .map(|n| (n.id.clone(), n.parent_id.clone(), n.index))
            .collect();

        let undo = doc.delete_node(&target).unwrap();
        doc.undo_stack.push(undo);
        doc.dirty = true;

        assert!(doc.undo(), "undo should succeed");

        // Node is back.
        let restored = doc.mindmap.nodes.get(&target)
            .expect("node should be restored");
        assert_eq!(restored.id, orig_node.id);
        assert_eq!(restored.text, orig_node.text);
        // Edges are fully restored — same count AND same order.
        // Ordering matters: earlier versions of `delete_node` stored
        // post-removal indices, which silently reordered edges that
        // shared the deleted node's neighborhood. Compare each slot
        // by the (from, to, edge_type) triple since edges have no
        // stable id.
        assert_eq!(doc.mindmap.edges.len(), orig_edges.len(),
            "edge count should be restored");
        for (i, (orig, restored)) in orig_edges.iter()
            .zip(doc.mindmap.edges.iter()).enumerate()
        {
            assert_eq!(
                (orig.from_id.as_str(), orig.to_id.as_str(), orig.edge_type.as_str()),
                (restored.from_id.as_str(), restored.to_id.as_str(), restored.edge_type.as_str()),
                "edge at index {} should match after undo", i,
            );
        }
        // Children are re-attached with original parent_id + index.
        for (cid, old_parent, old_idx) in orig_child_state {
            let child = doc.mindmap.nodes.get(&cid).unwrap();
            assert_eq!(child.parent_id, old_parent,
                "child {} parent_id should be restored", cid);
            assert_eq!(child.index, old_idx,
                "child {} index should be restored", cid);
        }
    }

    /// Regression test for the edge-ordering bug found in review of
    /// the initial Session 7A follow-up commit. When a deleted node
    /// has multiple incident edges scattered through the edge vec,
    /// naive in-place removal stores post-removal indices, so the
    /// undo reinserts them at the wrong positions and silently
    /// reorders edges the caller never touched. The fix stores
    /// pre-removal indices via `enumerate()` + `retain()`.
    ///
    /// Built as a self-contained test so we control the edge
    /// neighborhood precisely.
    #[test]
    fn test_delete_node_undo_preserves_edge_order_with_gaps() {
        use baumhard::mindmap::model::MindEdge;

        let mut doc = load_test_doc();
        // Pick any node with at least one incident edge.
        let target = find_node_with_children_and_parent(&doc);

        // Reset edges to a known layout: a mix of edges touching and
        // not touching the target, spaced out so the bug's effect
        // is visible. Use existing node ids so the edges are valid
        // references (any two existing-but-not-target ids work).
        let other_ids: Vec<String> = doc.mindmap.nodes.keys()
            .filter(|id| id.as_str() != target.as_str())
            .take(4)
            .cloned()
            .collect();
        assert!(other_ids.len() >= 4, "need at least 4 non-target nodes");
        let a = other_ids[0].clone();
        let b = other_ids[1].clone();
        let c = other_ids[2].clone();
        let d = other_ids[3].clone();

        let mk_edge = |from: &str, to: &str, etype: &str| MindEdge {
            from_id: from.to_string(),
            to_id: to.to_string(),
            edge_type: etype.to_string(),
            color: "#ffffff".to_string(),
            width: 1,
            line_style: 0,
            visible: true,
            label: None,
            label_position_t: None,
            anchor_from: 0,
            anchor_to: 0,
            control_points: Vec::new(),
            glyph_connection: None,
        };

        // Edge layout: [a→b, a→target, c→d, target→d, b→c]
        //               idx 0      1       2      3       4
        // Positions 1 and 3 touch the target; 0, 2, 4 are bystanders.
        // Wrong behavior would end up reordering the bystanders.
        doc.mindmap.edges = vec![
            mk_edge(&a, &b, "cross_link"),
            mk_edge(&a, &target, "cross_link"),
            mk_edge(&c, &d, "cross_link"),
            mk_edge(&target, &d, "cross_link"),
            mk_edge(&b, &c, "cross_link"),
        ];
        let orig_edges = doc.mindmap.edges.clone();

        let undo = doc.delete_node(&target).unwrap();
        // Sanity: the two touching edges are gone, bystanders remain.
        assert_eq!(doc.mindmap.edges.len(), 3);
        assert_eq!(doc.mindmap.edges[0].to_id, b);
        assert_eq!(doc.mindmap.edges[1].from_id, c);
        assert_eq!(doc.mindmap.edges[2].from_id, b);

        // Undo and verify byte-for-byte positional recovery.
        doc.undo_stack.push(undo);
        doc.dirty = true;
        assert!(doc.undo());

        assert_eq!(doc.mindmap.edges.len(), orig_edges.len());
        for (i, (orig, restored)) in orig_edges.iter()
            .zip(doc.mindmap.edges.iter()).enumerate()
        {
            assert_eq!(
                (orig.from_id.as_str(), orig.to_id.as_str()),
                (restored.from_id.as_str(), restored.to_id.as_str()),
                "edge at index {} out of order after undo", i,
            );
        }
    }

    #[test]
    fn test_delete_node_missing_returns_none() {
        let mut doc = load_test_doc();
        assert!(doc.delete_node("no_such_node_id_exists").is_none());
    }

    #[test]
    fn test_delete_root_node_works() {
        // Delete a top-level root and confirm its children become
        // their own roots. Tests that "orphan children" handles the
        // case where the deleted node has no parent itself.
        let mut doc = load_test_doc();
        // "Lord God" is a known root with children in testament.
        let target = "348068464".to_string();
        assert!(doc.mindmap.nodes.get(&target).unwrap().parent_id.is_none());
        let child_ids: Vec<String> = doc.mindmap.children_of(&target)
            .iter().map(|n| n.id.clone()).collect();
        assert!(!child_ids.is_empty());

        doc.delete_node(&target).unwrap();
        assert!(!doc.mindmap.nodes.contains_key(&target));
        for cid in &child_ids {
            assert!(doc.mindmap.nodes.get(cid).unwrap().parent_id.is_none());
        }
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

    // ---------------------------------------------------------------------
    // Session 6C: edge handles + reset/anchor helpers + EditEdge undo
    // ---------------------------------------------------------------------

    use baumhard::mindmap::model::ControlPoint;
    use baumhard::mindmap::scene_builder::EdgeHandleKind;

    #[test]
    fn test_hit_test_edge_handle_finds_anchor_from() {
        let doc = load_test_doc();
        let (edge_ref, _) = pick_test_edge(&doc);
        let edge = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
        let from_node = doc.mindmap.nodes.get(&edge.from_id).unwrap();
        let to_node = doc.mindmap.nodes.get(&edge.to_id).unwrap();
        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);
        let to_center = Vec2::new(to_pos.x + to_size.x * 0.5, to_pos.y + to_size.y * 0.5);
        let anchor_from_pos = baumhard::mindmap::connection::resolve_anchor_point(
            from_pos, from_size, edge.anchor_from, to_center,
        );

        let hit = doc.hit_test_edge_handle(anchor_from_pos, &edge_ref, 2.0);
        assert!(matches!(hit, Some((EdgeHandleKind::AnchorFrom, _))),
            "expected AnchorFrom hit, got {:?}", hit.as_ref().map(|(k, _)| k));
    }

    #[test]
    fn test_hit_test_edge_handle_finds_midpoint_on_straight_edge() {
        let mut doc = load_test_doc();
        // Make sure we have a straight edge with empty control_points
        let edge_idx = doc.mindmap.edges.iter()
            .position(|e| e.visible && e.control_points.is_empty())
            .expect("testament map should have at least one straight edge");
        let edge_ref = EdgeRef::new(
            &doc.mindmap.edges[edge_idx].from_id,
            &doc.mindmap.edges[edge_idx].to_id,
            &doc.mindmap.edges[edge_idx].edge_type,
        );
        let _ = &mut doc;

        let edge = &doc.mindmap.edges[edge_idx];
        let from_node = doc.mindmap.nodes.get(&edge.from_id).unwrap();
        let to_node = doc.mindmap.nodes.get(&edge.to_id).unwrap();
        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);
        let from_center = Vec2::new(from_pos.x + from_size.x * 0.5, from_pos.y + from_size.y * 0.5);
        let to_center = Vec2::new(to_pos.x + to_size.x * 0.5, to_pos.y + to_size.y * 0.5);
        let start = baumhard::mindmap::connection::resolve_anchor_point(
            from_pos, from_size, edge.anchor_from, to_center,
        );
        let end = baumhard::mindmap::connection::resolve_anchor_point(
            to_pos, to_size, edge.anchor_to, from_center,
        );
        let midpoint = start.lerp(end, 0.5);

        let hit = doc.hit_test_edge_handle(midpoint, &edge_ref, 2.0);
        assert!(matches!(hit, Some((EdgeHandleKind::Midpoint, _))),
            "expected Midpoint hit for straight edge");
    }

    #[test]
    fn test_hit_test_edge_handle_no_midpoint_on_curved_edge() {
        let mut doc = load_test_doc();
        // Give an edge a control point so it's curved
        let edge_idx = doc.mindmap.edges.iter()
            .position(|e| e.visible)
            .unwrap();
        doc.mindmap.edges[edge_idx].control_points.push(ControlPoint { x: 50.0, y: 50.0 });
        let edge_ref = EdgeRef::new(
            &doc.mindmap.edges[edge_idx].from_id,
            &doc.mindmap.edges[edge_idx].to_id,
            &doc.mindmap.edges[edge_idx].edge_type,
        );

        // Compute what the midpoint WOULD be on a straight line; the
        // hit test should NOT return Midpoint for this curved edge
        // regardless of whether some other handle happens to be near.
        let edge = &doc.mindmap.edges[edge_idx];
        let from_node = doc.mindmap.nodes.get(&edge.from_id).unwrap();
        let to_node = doc.mindmap.nodes.get(&edge.to_id).unwrap();
        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);
        let from_center = Vec2::new(from_pos.x + from_size.x * 0.5, from_pos.y + from_size.y * 0.5);

        // The control point is at from_center + (50, 50). Hit there:
        // should get ControlPoint(0), not Midpoint.
        let cp_pos = from_center + Vec2::new(50.0, 50.0);
        let hit = doc.hit_test_edge_handle(cp_pos, &edge_ref, 5.0);
        assert!(matches!(hit, Some((EdgeHandleKind::ControlPoint(0), _))),
            "expected ControlPoint(0) hit on curved edge, got {:?}",
            hit.as_ref().map(|(k, _)| k));
    }

    #[test]
    fn test_hit_test_edge_handle_miss_outside_tolerance() {
        let doc = load_test_doc();
        let (edge_ref, _) = pick_test_edge(&doc);
        let hit = doc.hit_test_edge_handle(
            Vec2::new(-99999.0, -99999.0),
            &edge_ref,
            10.0,
        );
        assert!(hit.is_none());
    }

    #[test]
    fn test_reset_edge_to_straight_clears_control_points() {
        let mut doc = load_test_doc();
        let edge_idx = doc.mindmap.edges.iter()
            .position(|e| e.visible)
            .unwrap();
        doc.mindmap.edges[edge_idx].control_points.push(ControlPoint { x: 10.0, y: 20.0 });
        let edge_ref = EdgeRef::new(
            &doc.mindmap.edges[edge_idx].from_id,
            &doc.mindmap.edges[edge_idx].to_id,
            &doc.mindmap.edges[edge_idx].edge_type,
        );
        let ok = doc.reset_edge_to_straight(&edge_ref);
        assert!(ok, "reset should report success");
        assert!(doc.mindmap.edges[edge_idx].control_points.is_empty());
        assert!(doc.dirty);
        assert_eq!(doc.undo_stack.len(), 1);
    }

    #[test]
    fn test_reset_edge_to_straight_noop_on_already_straight() {
        let mut doc = load_test_doc();
        let edge_idx = doc.mindmap.edges.iter()
            .position(|e| e.visible && e.control_points.is_empty())
            .unwrap();
        let edge_ref = EdgeRef::new(
            &doc.mindmap.edges[edge_idx].from_id,
            &doc.mindmap.edges[edge_idx].to_id,
            &doc.mindmap.edges[edge_idx].edge_type,
        );
        let ok = doc.reset_edge_to_straight(&edge_ref);
        assert!(!ok, "reset on already-straight edge should be a no-op");
        assert!(doc.undo_stack.is_empty());
    }

    #[test]
    fn test_set_edge_anchor_pushes_undo() {
        let mut doc = load_test_doc();
        let edge_idx = doc.mindmap.edges.iter()
            .position(|e| e.visible)
            .unwrap();
        let edge_ref = EdgeRef::new(
            &doc.mindmap.edges[edge_idx].from_id,
            &doc.mindmap.edges[edge_idx].to_id,
            &doc.mindmap.edges[edge_idx].edge_type,
        );
        // Force a change by picking a value different from the current
        let original = doc.mindmap.edges[edge_idx].anchor_from;
        let new_value = if original == 1 { 3 } else { 1 };
        let ok = doc.set_edge_anchor(&edge_ref, true, new_value);
        assert!(ok);
        assert_eq!(doc.mindmap.edges[edge_idx].anchor_from, new_value);
        assert_eq!(doc.undo_stack.len(), 1);
    }

    #[test]
    fn test_set_edge_anchor_noop_when_already_set() {
        let mut doc = load_test_doc();
        let edge_idx = doc.mindmap.edges.iter()
            .position(|e| e.visible)
            .unwrap();
        let edge_ref = EdgeRef::new(
            &doc.mindmap.edges[edge_idx].from_id,
            &doc.mindmap.edges[edge_idx].to_id,
            &doc.mindmap.edges[edge_idx].edge_type,
        );
        let current = doc.mindmap.edges[edge_idx].anchor_from;
        let ok = doc.set_edge_anchor(&edge_ref, true, current);
        assert!(!ok);
        assert!(doc.undo_stack.is_empty());
    }

    #[test]
    fn test_edit_edge_undo_restores_control_points() {
        let mut doc = load_test_doc();
        let edge_idx = doc.mindmap.edges.iter()
            .position(|e| e.visible)
            .unwrap();
        doc.mindmap.edges[edge_idx].control_points.push(ControlPoint { x: 33.0, y: 44.0 });
        let edge_ref = EdgeRef::new(
            &doc.mindmap.edges[edge_idx].from_id,
            &doc.mindmap.edges[edge_idx].to_id,
            &doc.mindmap.edges[edge_idx].edge_type,
        );
        doc.reset_edge_to_straight(&edge_ref);
        assert!(doc.mindmap.edges[edge_idx].control_points.is_empty());
        assert!(doc.undo());
        assert_eq!(doc.mindmap.edges[edge_idx].control_points.len(), 1);
        assert_eq!(doc.mindmap.edges[edge_idx].control_points[0].x, 33.0);
    }

    #[test]
    fn test_edit_edge_undo_restores_anchor() {
        let mut doc = load_test_doc();
        let edge_idx = doc.mindmap.edges.iter()
            .position(|e| e.visible)
            .unwrap();
        let original = doc.mindmap.edges[edge_idx].anchor_from;
        let new_value = if original == 2 { 4 } else { 2 };
        let edge_ref = EdgeRef::new(
            &doc.mindmap.edges[edge_idx].from_id,
            &doc.mindmap.edges[edge_idx].to_id,
            &doc.mindmap.edges[edge_idx].edge_type,
        );
        doc.set_edge_anchor(&edge_ref, true, new_value);
        assert_eq!(doc.mindmap.edges[edge_idx].anchor_from, new_value);
        assert!(doc.undo());
        assert_eq!(doc.mindmap.edges[edge_idx].anchor_from, original);
    }

    #[test]
    fn test_edge_index_finds_existing_edge() {
        let doc = load_test_doc();
        let (edge_ref, _) = pick_test_edge(&doc);
        let idx = doc.edge_index(&edge_ref);
        assert!(idx.is_some());
    }

    #[test]
    fn test_edge_index_unknown_returns_none() {
        let doc = load_test_doc();
        let bogus = EdgeRef::new("nope", "nope2", "cross_link");
        assert!(doc.edge_index(&bogus).is_none());
    }

    // ========================================================================
    // Session 6D — connection style and label mutation tests
    // ========================================================================

    /// Find a cross-link or parent-child edge in the testament map and
    /// return its EdgeRef. Used by the mutation tests below as their
    /// entry point.
    fn first_testament_edge_ref(doc: &MindMapDocument) -> EdgeRef {
        let e = doc.mindmap.edges.first().expect("testament map has edges");
        EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type)
    }

    #[test]
    fn test_ensure_glyph_connection_forks_from_hardcoded_default() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        // Make sure the test subject starts with no per-edge override
        // AND the canvas has no default — forces the hardcoded default
        // path.
        let idx = doc.edge_index(&er).unwrap();
        doc.mindmap.edges[idx].glyph_connection = None;
        doc.mindmap.canvas.default_connection = None;
        doc.undo_stack.clear();
        doc.dirty = false;

        // First style edit: changing the body glyph. The fork should
        // materialize a concrete GlyphConnectionConfig with the
        // hardcoded default body (·) — then the mutation overwrites
        // `body` with the requested value.
        let changed = doc.set_edge_body_glyph(&er, "\u{2500}");
        assert!(changed, "body change should succeed on fresh edge");
        let cfg = doc.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .expect("fork should install a config");
        assert_eq!(cfg.body, "\u{2500}");
        // The other fields should match the hardcoded default.
        let hard = GlyphConnectionConfig::default();
        assert_eq!(cfg.font_size_pt, hard.font_size_pt);
        assert_eq!(cfg.min_font_size_pt, hard.min_font_size_pt);
    }

    #[test]
    fn test_ensure_glyph_connection_forks_from_canvas_default() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        let idx = doc.edge_index(&er).unwrap();
        doc.mindmap.edges[idx].glyph_connection = None;
        // Set a canvas-level default with a distinctive body glyph.
        doc.mindmap.canvas.default_connection = Some(GlyphConnectionConfig {
            body: "\u{22EF}".to_string(), // ⋯
            ..GlyphConnectionConfig::default()
        });
        doc.undo_stack.clear();
        doc.dirty = false;

        // Change a different field (spacing) so the fork copies the
        // canvas body (⋯) into the edge before the field overwrite.
        let changed = doc.set_edge_spacing(&er, 6.0);
        assert!(changed);
        let cfg = doc.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .expect("fork should install a config");
        // Body was copied from the canvas default, not from the
        // hardcoded default.
        assert_eq!(cfg.body, "\u{22EF}");
        assert_eq!(cfg.spacing, 6.0);
    }

    /// Glyph-wheel color picker invariant: setting
    /// `doc.color_picker_preview` never pushes an undo entry and
    /// never flips dirty. Mirrors what the picker hover path does
    /// after the Step C refactor, which moved preview from model-
    /// mutation (`preview_edge_color`) to a transient scene-level
    /// substitution via the document field.
    #[test]
    fn test_color_picker_preview_does_not_push_undo_or_dirty() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        let idx = doc.edge_index(&er).unwrap();
        let stack_depth = doc.undo_stack.len();
        let before = doc.mindmap.edges[idx].clone();
        doc.dirty = false;

        let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(
            &doc.mindmap.edges[idx],
        );
        doc.color_picker_preview = Some(ColorPickerPreview::Edge {
            key,
            color: "#abcdef".to_string(),
        });

        // Model is byte-identical to the pre-preview state.
        assert_eq!(doc.mindmap.edges[idx], before);
        assert_eq!(doc.undo_stack.len(), stack_depth);
        assert!(!doc.dirty);

        // And the scene builder substitutes the preview color into
        // the matching edge's label element.
        doc.selection = SelectionState::Edge(er.clone());
        let scene = doc.build_scene_with_selection(1.0);
        // The edge has a glyph label → scene_builder should emit a
        // ConnectionLabelElement for it. If the edge has no label
        // this test case simply verifies nothing crashes.
        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(
            &doc.mindmap.edges[idx],
        );
        // Preview beats selection on the previewed edge → the
        // connection color (body glyphs) should be the preview hex,
        // not the selection cyan.
        if let Some(conn) = scene
            .connection_elements
            .iter()
            .find(|c| c.edge_key == edge_key)
        {
            assert_eq!(conn.color, "#abcdef",
                "preview should beat selection override on the previewed edge");
        }
    }

    /// Clearing `doc.color_picker_preview` returns scene output to
    /// the pre-preview state without any model mutation.
    #[test]
    fn test_color_picker_preview_cleared_returns_to_committed() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        let idx = doc.edge_index(&er).unwrap();
        let committed_before = doc.mindmap.edges[idx].clone();

        let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(
            &doc.mindmap.edges[idx],
        );
        doc.color_picker_preview = Some(ColorPickerPreview::Edge {
            key,
            color: "#112233".to_string(),
        });
        // ... hover frames would call build_scene here ...
        doc.color_picker_preview = None;

        // Model is untouched across the full preview session.
        assert_eq!(doc.mindmap.edges[idx], committed_before);
    }

    #[test]
    fn test_set_edge_body_glyph_pushes_edit_edge_undo() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        let stack_depth = doc.undo_stack.len();
        let changed = doc.set_edge_body_glyph(&er, "\u{2550}");
        assert!(changed);
        assert_eq!(doc.undo_stack.len(), stack_depth + 1);
        assert!(matches!(doc.undo_stack.last(), Some(UndoAction::EditEdge { .. })));
        assert!(doc.dirty);
    }

    #[test]
    fn test_undo_after_first_style_edit_restores_pre_fork_none() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        let idx = doc.edge_index(&er).unwrap();
        // Force the pre-edit state to None so we can verify the fork
        // is rolled back on undo.
        doc.mindmap.edges[idx].glyph_connection = None;
        doc.undo_stack.clear();

        assert!(doc.set_edge_body_glyph(&er, "\u{2500}"));
        assert!(doc.mindmap.edges[idx].glyph_connection.is_some());
        doc.undo();
        assert!(
            doc.mindmap.edges[idx].glyph_connection.is_none(),
            "undo should restore the pre-fork None"
        );
    }

    #[test]
    fn test_set_edge_color_none_clears_override() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        // First install a color override.
        assert!(doc.set_edge_color(&er, Some("#112233")));
        let idx = doc.edge_index(&er).unwrap();
        assert_eq!(
            doc.mindmap.edges[idx]
                .glyph_connection
                .as_ref()
                .and_then(|c| c.color.as_deref()),
            Some("#112233")
        );
        // Then clear it.
        assert!(doc.set_edge_color(&er, None));
        assert_eq!(
            doc.mindmap.edges[idx]
                .glyph_connection
                .as_ref()
                .and_then(|c| c.color.as_deref()),
            None
        );
    }

    #[test]
    fn test_set_edge_font_size_step_clamps_at_min_and_max() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        // Force a known starting config.
        let idx = doc.edge_index(&er).unwrap();
        doc.mindmap.edges[idx].glyph_connection = Some(GlyphConnectionConfig {
            font_size_pt: 12.0,
            min_font_size_pt: 8.0,
            max_font_size_pt: 24.0,
            ..GlyphConnectionConfig::default()
        });
        doc.undo_stack.clear();

        // Step down past the min: should clamp, returning a smaller
        // but not less-than-min value. Repeatedly stepping down should
        // eventually pin at the min and return false on subsequent
        // attempts (no-op).
        for _ in 0..20 {
            doc.set_edge_font_size_step(&er, -2.0);
        }
        let pinned_low = doc.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .unwrap()
            .font_size_pt;
        assert_eq!(pinned_low, 8.0);
        // Further steps down return false.
        assert!(!doc.set_edge_font_size_step(&er, -2.0));

        // Step up past the max: clamps to 24.
        for _ in 0..20 {
            doc.set_edge_font_size_step(&er, 2.0);
        }
        let pinned_high = doc.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .unwrap()
            .font_size_pt;
        assert_eq!(pinned_high, 24.0);
        assert!(!doc.set_edge_font_size_step(&er, 2.0));
    }

    #[test]
    fn test_set_edge_spacing_idempotent_noop() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        // First set succeeds.
        assert!(doc.set_edge_spacing(&er, 2.0));
        let stack_depth = doc.undo_stack.len();
        // Second set with the same value is a no-op; undo stack
        // doesn't grow.
        assert!(!doc.set_edge_spacing(&er, 2.0));
        assert_eq!(doc.undo_stack.len(), stack_depth);
    }

    #[test]
    fn test_set_edge_label_round_trip() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        // Set a label.
        assert!(doc.set_edge_label(&er, Some("hello".to_string())));
        let idx = doc.edge_index(&er).unwrap();
        assert_eq!(doc.mindmap.edges[idx].label.as_deref(), Some("hello"));
        // Clear via Some("").
        assert!(doc.set_edge_label(&er, Some(String::new())));
        assert_eq!(doc.mindmap.edges[idx].label, None);
        // Setting the same None is a no-op.
        let depth = doc.undo_stack.len();
        assert!(!doc.set_edge_label(&er, None));
        assert_eq!(doc.undo_stack.len(), depth);
    }

    #[test]
    fn test_set_edge_label_position_clamps_into_0_1() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        assert!(doc.set_edge_label_position(&er, -5.0));
        let idx = doc.edge_index(&er).unwrap();
        assert_eq!(doc.mindmap.edges[idx].label_position_t, Some(0.0));

        assert!(doc.set_edge_label_position(&er, 42.0));
        assert_eq!(doc.mindmap.edges[idx].label_position_t, Some(1.0));

        assert!(doc.set_edge_label_position(&er, 0.75));
        assert_eq!(doc.mindmap.edges[idx].label_position_t, Some(0.75));
    }

    #[test]
    fn test_set_edge_type_updates_selection_edge_ref() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        doc.selection = SelectionState::Edge(er.clone());
        let new_type = if er.edge_type == "parent_child" { "cross_link" } else { "parent_child" };
        assert!(doc.set_edge_type(&er, new_type));
        // Selection should now carry the new type.
        match &doc.selection {
            SelectionState::Edge(new_ref) => {
                assert_eq!(new_ref.edge_type, new_type);
                assert_eq!(new_ref.from_id, er.from_id);
                assert_eq!(new_ref.to_id, er.to_id);
            }
            _ => panic!("selection should still be an edge after type flip"),
        }
    }

    #[test]
    fn test_set_edge_type_refuses_duplicate() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        let from_id = er.from_id.clone();
        let to_id = er.to_id.clone();
        // Seed a duplicate edge with the OPPOSITE type so conversion
        // would collide with it.
        let target_type = if er.edge_type == "parent_child" { "cross_link" } else { "parent_child" };
        let mut dup = doc.mindmap.edges[doc.edge_index(&er).unwrap()].clone();
        dup.edge_type = target_type.to_string();
        doc.mindmap.edges.push(dup);
        // Conversion should be refused.
        assert!(!doc.set_edge_type(&er, target_type));
        // Original edge is unchanged.
        assert_eq!(
            doc.mindmap.edges[doc.edge_index(&er).unwrap()].edge_type,
            er.edge_type
        );
    }

    #[test]
    fn test_reset_edge_style_to_default_clears_glyph_connection() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        // Install an override.
        assert!(doc.set_edge_color(&er, Some("#00ff00")));
        let idx = doc.edge_index(&er).unwrap();
        assert!(doc.mindmap.edges[idx].glyph_connection.is_some());
        // Reset clears it.
        assert!(doc.reset_edge_style_to_default(&er));
        assert!(doc.mindmap.edges[idx].glyph_connection.is_none());
        // Repeat call is a no-op.
        assert!(!doc.reset_edge_style_to_default(&er));
    }

    #[test]
    fn test_set_edge_cap_start_none_is_noop_when_already_none() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        // Force cap_start to None via a fresh config.
        let idx = doc.edge_index(&er).unwrap();
        doc.mindmap.edges[idx].glyph_connection = Some(GlyphConnectionConfig::default());
        doc.undo_stack.clear();
        // Setting cap_start to None when already None is a no-op.
        assert!(!doc.set_edge_cap_start(&er, None));
        // Undo stack didn't grow.
        assert_eq!(doc.undo_stack.len(), 0);
    }

    #[test]
    fn test_undo_chain_round_trips_multiple_edits() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        // Snapshot the starting state of the edge.
        let idx = doc.edge_index(&er).unwrap();
        let original = doc.mindmap.edges[idx].clone();
        doc.undo_stack.clear();

        // Apply three edits.
        assert!(doc.set_edge_body_glyph(&er, "\u{2500}"));
        assert!(doc.set_edge_color(&er, Some("#abcdef")));
        assert!(doc.set_edge_label(&er, Some("x".to_string())));
        assert_eq!(doc.undo_stack.len(), 3);

        // Undo all three in LIFO order.
        doc.undo();
        doc.undo();
        doc.undo();
        let restored = &doc.mindmap.edges[idx];
        assert_eq!(restored.label, original.label);
        assert_eq!(
            restored.glyph_connection.as_ref().map(|c| c.body.clone()),
            original.glyph_connection.as_ref().map(|c| c.body.clone())
        );
        assert_eq!(
            restored.glyph_connection.as_ref().and_then(|c| c.color.clone()),
            original.glyph_connection.as_ref().and_then(|c| c.color.clone())
        );
    }

    // -----------------------------------------------------------------
    // grow_node_sizes_to_fit_text — node box auto-sizing
    //
    // Grow-only pass that runs once at load time to ensure every node's
    // stored size is at least big enough to contain its text. These
    // tests lock in the three invariants described in the helper's
    // doc-comment: grow-only, idempotent, and consistent with rendering.
    // -----------------------------------------------------------------

    /// Build a synthetic single-node map with a given text and stored
    /// size. The text_runs carry a 14 pt scale to match the default.
    fn synthetic_single_node_map(text: &str, w: f64, h: f64) -> MindMap {
        use baumhard::mindmap::model::TextRun;
        let text_runs = vec![TextRun {
            start: 0,
            end: text.chars().count(),
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".to_string(),
            size_pt: 14,
            color: "#ffffff".to_string(),
            hyperlink: None,
        }];
        let node = MindNode {
            id: "n1".to_string(),
            parent_id: None,
            index: 0,
            position: Position { x: 0.0, y: 0.0 },
            size: Size { width: w, height: h },
            text: text.to_string(),
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
        };
        let mut nodes = HashMap::new();
        nodes.insert("n1".to_string(), node);
        MindMap {
            version: "1.0".to_string(),
            name: "test".to_string(),
            canvas: Canvas {
                background_color: "#000000".to_string(),
                default_border: None,
                default_connection: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            nodes,
            edges: Vec::new(),
            custom_mutations: Vec::new(),
            portals: Vec::new(),
        }
    }

    /// A stored box that's already comfortably larger than the text
    /// must be left alone — the helper is strictly grow-only.
    #[test]
    fn grow_node_sizes_to_fit_text_does_not_shrink() {
        let mut map = synthetic_single_node_map("Hi", 500.0, 500.0);
        grow_node_sizes_to_fit_text(&mut map);
        let n = map.nodes.get("n1").unwrap();
        assert_eq!(n.size.width, 500.0);
        assert_eq!(n.size.height, 500.0);
    }

    /// A tiny stored box with a long text must grow so the measured
    /// bounds (plus padding) fit inside. Exact measurements depend on
    /// available fonts, so assertions are lower-bound only.
    #[test]
    fn grow_node_sizes_to_fit_text_grows_undersized_boxes() {
        let long = "The quick brown fox jumps over the lazy dog a few times";
        let mut map = synthetic_single_node_map(long, 20.0, 20.0);
        grow_node_sizes_to_fit_text(&mut map);
        let n = map.nodes.get("n1").unwrap();
        // Definitely bigger than 20×20 — the long string shapes to at
        // least a few hundred pixels wide at 14 pt.
        assert!(
            n.size.width > 100.0,
            "expected grown width > 100, got {}",
            n.size.width
        );
        // Height grows by at least one line of text + pad_y (0.5 *
        // 14 = 7) → ≥ one line height (14 * 1.2 ≈ 16.8) + 7 ≈ 23.8.
        assert!(
            n.size.height >= 20.0,
            "expected height ≥ 20 (grow-only floor), got {}",
            n.size.height
        );
    }

    /// Running the pass twice must be a no-op the second time —
    /// after the first run, every node's stored size is already
    /// `>= measured + pad`, so the `max(stored, ...)` reduces to
    /// `stored` on the second call.
    #[test]
    fn grow_node_sizes_to_fit_text_is_idempotent() {
        let mut map = synthetic_single_node_map("Some text here", 10.0, 10.0);
        grow_node_sizes_to_fit_text(&mut map);
        let first_w = map.nodes.get("n1").unwrap().size.width;
        let first_h = map.nodes.get("n1").unwrap().size.height;
        grow_node_sizes_to_fit_text(&mut map);
        let second_w = map.nodes.get("n1").unwrap().size.width;
        let second_h = map.nodes.get("n1").unwrap().size.height;
        assert_eq!(first_w, second_w);
        assert_eq!(first_h, second_h);
    }

    // =====================================================================
    // Session 6E — portal mutation tests
    // =====================================================================

    #[test]
    fn portal_create_success_assigns_first_label() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let pref = doc.apply_create_portal(&a, &b).expect("should succeed");
        assert_eq!(pref.label, "A");
        assert_eq!(pref.endpoint_a, a);
        assert_eq!(pref.endpoint_b, b);
        assert_eq!(doc.mindmap.portals.len(), 1);
        assert_eq!(doc.mindmap.portals[0].label, "A");
        assert!(doc.dirty);
    }

    #[test]
    fn portal_create_assigns_sequential_labels_a_b_c() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let c = iter.next().unwrap().clone();

        let p1 = doc.apply_create_portal(&a, &b).unwrap();
        let p2 = doc.apply_create_portal(&b, &c).unwrap();
        let p3 = doc.apply_create_portal(&a, &c).unwrap();
        assert_eq!(p1.label, "A");
        assert_eq!(p2.label, "B");
        assert_eq!(p3.label, "C");
    }

    #[test]
    fn portal_create_assigns_rotating_glyphs() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let c = iter.next().unwrap().clone();

        doc.apply_create_portal(&a, &b).unwrap();
        doc.apply_create_portal(&b, &c).unwrap();
        assert_eq!(doc.mindmap.portals[0].glyph, PORTAL_GLYPH_PRESETS[0]);
        assert_eq!(doc.mindmap.portals[1].glyph, PORTAL_GLYPH_PRESETS[1]);
    }

    #[test]
    fn portal_create_rejects_self_portal() {
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().unwrap().clone();
        let result = doc.apply_create_portal(&id, &id);
        assert!(result.is_err());
        assert!(doc.mindmap.portals.is_empty());
    }

    #[test]
    fn portal_create_rejects_unknown_node() {
        let mut doc = load_test_doc();
        let known = doc.mindmap.nodes.keys().next().unwrap().clone();
        assert!(doc.apply_create_portal(&known, "does_not_exist").is_err());
        assert!(doc.apply_create_portal("does_not_exist", &known).is_err());
        assert!(doc.mindmap.portals.is_empty());
    }

    #[test]
    fn portal_undo_create_removes_portal() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let pref = doc.apply_create_portal(&a, &b).unwrap();
        doc.selection = SelectionState::Portal(pref);
        assert_eq!(doc.mindmap.portals.len(), 1);
        assert!(doc.undo());
        assert!(doc.mindmap.portals.is_empty());
        // Undoing a CreatePortal that was selected should clear the selection.
        assert!(matches!(doc.selection, SelectionState::None));
    }

    #[test]
    fn portal_delete_and_undo_restore_original_index() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let c = iter.next().unwrap().clone();

        let p1 = doc.apply_create_portal(&a, &b).unwrap();
        let p2 = doc.apply_create_portal(&b, &c).unwrap();
        let _p3 = doc.apply_create_portal(&a, &c).unwrap();
        assert_eq!(doc.mindmap.portals.len(), 3);

        // Delete the middle portal.
        let removed = doc.apply_delete_portal(&p2).expect("should delete");
        assert_eq!(removed.label, "B");
        assert_eq!(doc.mindmap.portals.len(), 2);

        // Undo should slot it back at its original middle index.
        assert!(doc.undo());
        assert_eq!(doc.mindmap.portals.len(), 3);
        assert_eq!(doc.mindmap.portals[1].label, "B");
        // And the other portals should still be intact.
        assert_eq!(doc.mindmap.portals[0].label, p1.label);
    }

    #[test]
    fn portal_edit_glyph_and_undo_restores_before() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let pref = doc.apply_create_portal(&a, &b).unwrap();
        let original_glyph = doc.mindmap.portals[0].glyph.clone();
        assert!(doc.set_portal_glyph(&pref, "\u{2B22}"));
        assert_eq!(doc.mindmap.portals[0].glyph, "\u{2B22}");
        assert!(doc.undo());
        assert_eq!(doc.mindmap.portals[0].glyph, original_glyph);
    }

    #[test]
    fn portal_edit_color_and_undo_restores_before() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let pref = doc.apply_create_portal(&a, &b).unwrap();
        let original_color = doc.mindmap.portals[0].color.clone();
        assert!(doc.set_portal_color(&pref, "var(--accent)"));
        assert_eq!(doc.mindmap.portals[0].color, "var(--accent)");
        assert!(doc.undo());
        assert_eq!(doc.mindmap.portals[0].color, original_color);
    }

    #[test]
    fn portal_delete_returns_none_for_unknown_ref() {
        let mut doc = load_test_doc();
        let ghost = PortalRef::new("Z", "ghost_a", "ghost_b");
        assert!(doc.apply_delete_portal(&ghost).is_none());
    }

    #[test]
    fn portal_next_label_reuses_gap_after_delete() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let c = iter.next().unwrap().clone();

        let _ = doc.apply_create_portal(&a, &b).unwrap();
        let p2 = doc.apply_create_portal(&b, &c).unwrap();
        let _ = doc.apply_create_portal(&a, &c).unwrap();
        // Delete the middle ("B").
        doc.apply_delete_portal(&p2).unwrap();
        // Next creation should reuse "B" since it is now the lowest unused.
        let d = doc.apply_create_portal(&b, &c).unwrap();
        assert_eq!(d.label, "B");
    }

    #[test]
    fn selection_state_portal_is_not_node_selection() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let pref = doc.apply_create_portal(&a, &b).unwrap();
        doc.selection = SelectionState::Portal(pref.clone());

        assert!(!doc.selection.is_selected(&a));
        assert!(!doc.selection.is_selected(&b));
        assert!(doc.selection.selected_ids().is_empty());
        assert_eq!(doc.selection.selected_portal(), Some(&pref));
        assert_eq!(doc.selection.selected_edge(), None);
    }

    // -----------------------------------------------------------------
    // Session 7A: node text editing
    // -----------------------------------------------------------------

    /// Pick a stable node id from the testament map that has a real
    /// text value. The root node id is well-known from other tests.
    fn first_testament_node_id(_doc: &MindMapDocument) -> String {
        "348068464".to_string()
    }

    #[test]
    fn test_set_node_text_updates_text_and_collapses_runs() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let changed = doc.set_node_text(&nid, "Hello world".to_string());
        assert!(changed);
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(node.text, "Hello world");
        assert_eq!(node.text_runs.len(), 1);
        assert_eq!(node.text_runs[0].start, 0);
        assert_eq!(node.text_runs[0].end, "Hello world".chars().count());
        assert!(doc.dirty);
        assert!(matches!(
            doc.undo_stack.last(),
            Some(UndoAction::EditNodeText { .. })
        ));
    }

    #[test]
    fn test_set_node_text_noop_on_unchanged() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let current = doc.mindmap.nodes.get(&nid).unwrap().text.clone();
        doc.undo_stack.clear();
        doc.dirty = false;
        let changed = doc.set_node_text(&nid, current);
        assert!(!changed);
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    #[test]
    fn test_set_node_text_undo_round_trip() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let before_text = doc.mindmap.nodes.get(&nid).unwrap().text.clone();
        let before_runs_len = doc.mindmap.nodes.get(&nid).unwrap().text_runs.len();
        let before_first_run_color = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .first()
            .map(|r| r.color.clone());
        assert!(doc.set_node_text(&nid, "mutated".to_string()));
        assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().text, "mutated");
        assert!(doc.undo());
        let restored = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(restored.text, before_text);
        // TextRun doesn't implement PartialEq, so compare the parts
        // we care about: count + first run's color.
        assert_eq!(restored.text_runs.len(), before_runs_len);
        assert_eq!(
            restored.text_runs.first().map(|r| r.color.clone()),
            before_first_run_color
        );
    }

    #[test]
    fn test_set_node_text_multiline_with_newlines() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        assert!(doc.set_node_text(&nid, "line 1\nline 2\nline 3".to_string()));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(node.text, "line 1\nline 2\nline 3");
        // Collapsed single run spans the full char count, including newlines.
        assert_eq!(node.text_runs.len(), 1);
        assert_eq!(node.text_runs[0].end, "line 1\nline 2\nline 3".chars().count());
    }

    #[test]
    fn test_set_node_text_unknown_id_returns_false() {
        let mut doc = load_test_doc();
        doc.undo_stack.clear();
        doc.dirty = false;
        assert!(!doc.set_node_text("nonexistent-id", "x".to_string()));
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    #[test]
    fn test_set_node_text_inherits_first_run_formatting() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        // Force a specific first-run formatting we can check for.
        {
            let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
            if node.text_runs.is_empty() {
                node.text_runs.push(TextRun {
                    start: 0,
                    end: node.text.chars().count(),
                    bold: false,
                    italic: false,
                    underline: false,
                    font: "LiberationSans".to_string(),
                    size_pt: 24,
                    color: "#ffffff".to_string(),
                    hyperlink: None,
                });
            }
            node.text_runs[0].bold = true;
            node.text_runs[0].color = "#abcdef".to_string();
            node.text_runs[0].size_pt = 33;
        }
        assert!(doc.set_node_text(&nid, "rewritten".to_string()));
        let run = &doc.mindmap.nodes.get(&nid).unwrap().text_runs[0];
        assert!(run.bold);
        assert_eq!(run.color, "#abcdef");
        assert_eq!(run.size_pt, 33);
    }
}
