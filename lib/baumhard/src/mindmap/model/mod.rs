//! Mindmap data model — the types the loader deserializes from
//! `.mindmap.json` and the document layer mutates. Split across four
//! leaf modules so each concern stays skimmable:
//!
//! - `canvas` — `Canvas`: the per-map rendering context.
//! - `node` — `MindNode` and everything that travels with it
//!   (`NodeStyle`, `GlyphBorderConfig`, `ColorSchema`, ...).
//! - `edge` — `MindEdge`, `GlyphConnectionConfig`, `ControlPoint`,
//!   plus portal-mode helpers (portals are now a `display_mode` on
//!   edges rather than a separate entity).
//! - `palette` — named colour palettes referenced by nodes'
//!   `color_schema.palette` field.
//!
//! This module owns the top-level `MindMap` struct plus its impl
//! (root / ancestry / descendant queries), and the model-level tests.

/// `Canvas` — per-map rendering context: background, default
/// border/connection styles, theme variables.
pub mod canvas;
/// `MindEdge`, `ControlPoint`, `GlyphConnectionConfig`, plus
/// portal-mode edge helpers (portals are a `display_mode` on edges).
pub mod edge;
/// `MindNode` and its travelling-companion structs (position,
/// size, text runs, node style, layout, colour schema, border).
pub mod node;
/// Named colour palettes referenced by nodes' `color_schema.palette`
/// field.
pub mod palette;

pub use canvas::Canvas;
pub use edge::{
    is_portal_edge, portal_endpoint_state, portal_endpoint_state_mut, ControlPoint,
    EdgeLabelConfig, GlyphConnectionConfig, MindEdge, PortalEndpointState,
    DEFAULT_LABEL_SIZE_FACTOR, DISPLAY_MODE_LINE, DISPLAY_MODE_PORTAL, PORTAL_GLYPH_PRESETS,
};
pub use node::{
    ColorGroup, ColorSchema, CustomBorderGlyphs, GlyphBorderConfig, MindNode, NodeLayout,
    NodeStyle, Position, Size, TextRun,
};
pub use palette::Palette;

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::mindmap::custom_mutation::CustomMutation;

/// The whole-map value type — what [`crate::mindmap::loader`]
/// deserializes from a `.mindmap.json` file and what the document
/// layer mutates and persists. Carries the version, name, shared
/// canvas state, named palettes, the node map (keyed by Dewey-decimal
/// id), the edge list, and any map-level custom mutations.
///
/// Plain data; no runtime cost beyond the `HashMap` / `Vec`
/// allocations serde performs. Tree-shape queries
/// ([`Self::root_nodes`], [`Self::children_of`],
/// [`Self::is_ancestor_or_self`], etc.) walk the node map lazily —
/// see each method for its per-call cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindMap {
    pub version: String,
    pub name: String,
    pub canvas: Canvas,
    /// Named color palettes referenced by nodes' color_schema.palette field.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub palettes: HashMap<String, Palette>,
    pub nodes: HashMap<String, MindNode>,
    pub edges: Vec<MindEdge>,
    /// Map-level custom mutation definitions, available to all nodes in this map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_mutations: Vec<CustomMutation>,
}

impl MindMap {
    /// Construct an empty `MindMap` with the given name. The canvas
    /// uses the same default background as fixture maps (`#000000`)
    /// and no theme variants. Nodes and edges start empty — ready to
    /// be populated by the `new` console command (or by direct user
    /// editing once a save target is bound).
    pub fn new_blank(name: impl Into<String>) -> Self {
        MindMap {
            version: "1.0".to_string(),
            name: name.into(),
            canvas: Canvas {
                background_color: "#000000".to_string(),
                default_border: None,
                default_connection: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            palettes: HashMap::new(),
            nodes: HashMap::new(),
            edges: Vec::new(),
            custom_mutations: Vec::new(),
        }
    }

    /// Returns root nodes (nodes with no parent), sorted by ID segment.
    pub fn root_nodes(&self) -> Vec<&MindNode> {
        let mut roots: Vec<&MindNode> = self.nodes.values()
            .filter(|n| n.parent_id.is_none())
            .collect();
        roots.sort_by_key(|n| id_sort_key(&n.id));
        roots
    }

    /// Returns children of a given node, sorted by ID segment.
    pub fn children_of(&self, parent_id: &str) -> Vec<&MindNode> {
        let mut children: Vec<&MindNode> = self.nodes.values()
            .filter(|n| n.parent_id.as_deref() == Some(parent_id))
            .collect();
        children.sort_by_key(|n| id_sort_key(&n.id));
        children
    }

    /// Returns true if any ancestor of this node is folded, meaning
    /// this node should be hidden from view.
    pub fn is_hidden_by_fold(&self, node: &MindNode) -> bool {
        let mut current_id = node.parent_id.as_deref();
        while let Some(pid) = current_id {
            match self.nodes.get(pid) {
                Some(parent) => {
                    if parent.folded {
                        return true;
                    }
                    current_id = parent.parent_id.as_deref();
                }
                None => return false,
            }
        }
        false
    }

    /// Collect all descendant IDs of a node (recursive), not including the node itself.
    pub fn all_descendants(&self, node_id: &str) -> Vec<String> {
        let mut result = Vec::new();
        self.collect_descendants(node_id, &mut result);
        result
    }

    fn collect_descendants(&self, node_id: &str, result: &mut Vec<String>) {
        for child in self.children_of(node_id) {
            result.push(child.id.clone());
            self.collect_descendants(&child.id, result);
        }
    }

    /// Returns true if `candidate_ancestor` equals `node_id` or is a (transitive)
    /// ancestor of it. Used to prevent reparenting a node under itself or under
    /// one of its own descendants (which would create a cycle).
    pub fn is_ancestor_or_self(&self, candidate_ancestor: &str, node_id: &str) -> bool {
        if candidate_ancestor == node_id {
            return true;
        }
        let mut current = self.nodes.get(node_id).and_then(|n| n.parent_id.as_deref());
        while let Some(pid) = current {
            if pid == candidate_ancestor {
                return true;
            }
            current = self.nodes.get(pid).and_then(|n| n.parent_id.as_deref());
        }
        false
    }

    /// Resolves the effective colors for a themed node by looking up
    /// the palette from the top-level palettes map.
    pub fn resolve_theme_colors<'a>(&'a self, node: &'a MindNode) -> Option<&'a ColorGroup> {
        let schema = node.color_schema.as_ref()?;
        let palette = self.palettes.get(&schema.palette)?;
        let level = schema.level as usize;
        if level < palette.groups.len() {
            Some(&palette.groups[level])
        } else {
            palette.groups.last()
        }
    }
}

/// Extract the last segment of a Dewey-decimal ID as a numeric sort key.
/// `"1.2.3"` → `3`, `"0"` → `0`. Falls back to 0 for non-numeric IDs.
pub fn id_sort_key(id: &str) -> usize {
    id.rsplit('.').next()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
}

/// Derive the parent ID from a Dewey-decimal node ID.
/// `"1.2.3"` → `Some("1.2")`, `"0"` → `None` (root node).
pub fn derive_parent_id(id: &str) -> Option<String> {
    let dot = id.rfind('.')?;
    Some(id[..dot].to_string())
}

#[cfg(test)]
mod tests;
