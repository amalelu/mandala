//! Mindmap data model — the types the loader deserializes from
//! `.mindmap.json` and the document layer mutates. Split across four
//! leaf modules so each concern stays skimmable:
//!
//! - [`canvas`] — [`Canvas`]: the per-map rendering context.
//! - [`node`] — [`MindNode`] and everything that travels with it
//!   ([`NodeStyle`], [`GlyphBorderConfig`], [`ColorSchema`], ...).
//! - [`edge`] — [`MindEdge`], [`GlyphConnectionConfig`],
//!   [`ControlPoint`].
//! - [`portal`] — [`PortalPair`] and the column-letter label generator.
//!
//! This module owns the top-level [`MindMap`] struct plus its impl
//! (root / ancestry / descendant / portal-label queries), and the
//! model-level tests.

pub mod canvas;
pub mod edge;
pub mod node;
pub mod palette;
pub mod portal;

pub use canvas::Canvas;
pub use edge::{ControlPoint, GlyphConnectionConfig, MindEdge};
pub use node::{
    ColorGroup, ColorSchema, CustomBorderGlyphs, GlyphBorderConfig, MindNode, NodeLayout,
    NodeStyle, Position, Size, TextRun,
};
pub use palette::Palette;
pub use portal::{column_letter_label, PortalPair, PORTAL_GLYPH_PRESETS};

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::mindmap::custom_mutation::CustomMutation;

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
    /// Portal pairs — matching glyph markers on two distant nodes used as
    /// a lightweight alternative to cross-link edges when a rendered line
    /// would clutter the map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub portals: Vec<PortalPair>,
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
            portals: Vec::new(),
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

    /// Session 6E: return the lowest unused portal label in column-letter
    /// order: "A", "B", ..., "Z", "AA", "AB", ..., "AZ", "BA", ...
    ///
    /// Walks the existing `portals` vec, collects the used labels into a
    /// set, then emits labels lazily until one is not in the set. Used
    /// by `MindMapDocument::apply_create_portal` so deleting portal "B"
    /// and creating a new one reuses "B" rather than jumping to "D".
    pub fn next_portal_label(&self) -> String {
        use std::collections::HashSet;
        let used: HashSet<&str> = self.portals.iter().map(|p| p.label.as_str()).collect();
        // Lazy column-letter generator: 1 → "A", 26 → "Z", 27 → "AA", ...
        // (matching the Excel column naming scheme).
        let mut n: u64 = 1;
        loop {
            let label = column_letter_label(n);
            if !used.contains(label.as_str()) {
                return label;
            }
            n += 1;
        }
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
