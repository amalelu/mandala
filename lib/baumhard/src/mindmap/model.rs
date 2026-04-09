use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindMap {
    pub version: String,
    pub name: String,
    pub canvas: Canvas,
    pub nodes: HashMap<String, MindNode>,
    pub edges: Vec<MindEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Canvas {
    pub background_color: String,
    /// Default border style applied to all nodes unless overridden per-node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_border: Option<GlyphBorderConfig>,
    /// Default connection style applied to all edges unless overridden per-edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_connection: Option<GlyphConnectionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub index: i32,
    pub position: Position,
    pub size: Size,
    pub text: String,
    pub text_runs: Vec<TextRun>,
    pub style: NodeStyle,
    pub layout: NodeLayout,
    pub folded: bool,
    pub notes: String,
    pub color_schema: Option<ColorSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextRun {
    pub start: usize,
    pub end: usize,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub font: String,
    pub size_pt: u32,
    pub color: String,
    pub hyperlink: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStyle {
    pub background_color: String,
    pub frame_color: String,
    pub text_color: String,
    pub shape_type: i32,
    pub corner_radius_percent: f64,
    pub frame_thickness: f64,
    pub show_frame: bool,
    pub show_shadow: bool,
    /// Glyph-based border configuration. Optional — if absent, the renderer
    /// applies a default border style based on the node's frame_color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<GlyphBorderConfig>,
}

/// Configures how a node's border is rendered using font glyphs.
/// All fields are optional with sensible defaults so the format stays forgiving.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlyphBorderConfig {
    /// Which glyph preset to use: "light", "heavy", "double", "rounded", or "custom"
    #[serde(default = "default_border_preset")]
    pub preset: String,
    /// Font family name for border glyphs. None = system default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font size in points for border glyphs.
    #[serde(default = "default_border_font_size")]
    pub font_size_pt: f32,
    /// Border color override as #RRGGBB. None = inherit from frame_color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Custom glyph definitions. Only used when preset = "custom".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyphs: Option<CustomBorderGlyphs>,
    /// Padding between border and content (in pixels).
    #[serde(default = "default_border_padding")]
    pub padding: f32,
}

fn default_border_preset() -> String { "rounded".to_string() }
fn default_border_font_size() -> f32 { 14.0 }
fn default_border_padding() -> f32 { 4.0 }

/// Custom glyphs for each part of the border.
/// Each field is a string (single char or multi-char glyph).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomBorderGlyphs {
    #[serde(default = "default_h_glyph")]
    pub top: String,
    #[serde(default = "default_h_glyph")]
    pub bottom: String,
    #[serde(default = "default_v_glyph")]
    pub left: String,
    #[serde(default = "default_v_glyph")]
    pub right: String,
    #[serde(default = "default_tl_glyph")]
    pub top_left: String,
    #[serde(default = "default_tr_glyph")]
    pub top_right: String,
    #[serde(default = "default_bl_glyph")]
    pub bottom_left: String,
    #[serde(default = "default_br_glyph")]
    pub bottom_right: String,
}

fn default_h_glyph() -> String { "\u{2500}".to_string() }
fn default_v_glyph() -> String { "\u{2502}".to_string() }
fn default_tl_glyph() -> String { "\u{256D}".to_string() }
fn default_tr_glyph() -> String { "\u{256E}".to_string() }
fn default_bl_glyph() -> String { "\u{2570}".to_string() }
fn default_br_glyph() -> String { "\u{256F}".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLayout {
    #[serde(rename = "type")]
    pub layout_type: i32,
    pub direction: i32,
    pub spacing: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSchema {
    pub level: i32,
    pub palette: String,
    pub variant: i32,
    pub starts_at_root: bool,
    pub connections_colored: bool,
    pub theme_id: String,
    pub groups: Vec<ColorGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorGroup {
    pub background: String,
    pub frame: String,
    pub text: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindEdge {
    pub from_id: String,
    pub to_id: String,
    #[serde(rename = "type")]
    pub edge_type: String,
    pub color: String,
    pub width: i32,
    pub line_style: i32,
    pub visible: bool,
    pub label: Option<String>,
    pub anchor_from: i32,
    pub anchor_to: i32,
    pub control_points: Vec<ControlPoint>,
    /// Glyph-based connection rendering. Optional — if absent, the renderer
    /// composes a connection from default glyphs based on the edge direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph_connection: Option<GlyphConnectionConfig>,
}

/// Configures how a connection between nodes is rendered using font glyphs.
/// Connections are composed of repeating body glyphs and optional end caps,
/// laid out along the path from source to target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlyphConnectionConfig {
    /// The glyph(s) used for the body/middle of the connection, repeated to fill length.
    #[serde(default = "default_connection_body")]
    pub body: String,
    /// Glyph for the start of the connection (near the source node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_start: Option<String>,
    /// Glyph for the end of the connection (near the target node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_end: Option<String>,
    /// Font family name for connection glyphs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font size in points.
    #[serde(default = "default_connection_font_size")]
    pub font_size_pt: f32,
    /// Color override as #RRGGBB. None = inherit from edge color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Spacing between repeated body glyphs (0 = tight packing).
    #[serde(default)]
    pub spacing: f32,
}

fn default_connection_body() -> String { "\u{00B7}".to_string() } // middle dot ·
fn default_connection_font_size() -> f32 { 12.0 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlPoint {
    pub x: f64,
    pub y: f64,
}

impl MindMap {
    /// Returns root nodes (nodes with no parent), sorted by index.
    pub fn root_nodes(&self) -> Vec<&MindNode> {
        let mut roots: Vec<&MindNode> = self.nodes.values()
            .filter(|n| n.parent_id.is_none())
            .collect();
        roots.sort_by_key(|n| n.index);
        roots
    }

    /// Returns children of a given node, sorted by index.
    pub fn children_of(&self, parent_id: &str) -> Vec<&MindNode> {
        let mut children: Vec<&MindNode> = self.nodes.values()
            .filter(|n| n.parent_id.as_deref() == Some(parent_id))
            .collect();
        children.sort_by_key(|n| n.index);
        children
    }

    /// Finds the color schema root for a themed node by walking up the parent chain.
    /// Returns the schema root node (level 0 with non-empty groups).
    pub fn find_schema_root<'a>(&'a self, node: &'a MindNode) -> Option<&'a MindNode> {
        if let Some(ref schema) = node.color_schema {
            if schema.level == 0 && !schema.groups.is_empty() {
                return Some(node);
            }
        }
        // Walk up the parent chain
        let mut current = node;
        loop {
            match current.parent_id.as_deref() {
                None => return None,
                Some(pid) => {
                    match self.nodes.get(pid) {
                        None => return None,
                        Some(parent) => {
                            if let Some(ref schema) = parent.color_schema {
                                if schema.level == 0 && !schema.groups.is_empty() {
                                    return Some(parent);
                                }
                            }
                            current = parent;
                        }
                    }
                }
            }
        }
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

    /// Resolves the effective colors for a themed node.
    /// Returns (background, frame, text, title) hex color strings.
    pub fn resolve_theme_colors<'a>(&'a self, node: &'a MindNode) -> Option<&'a ColorGroup> {
        let schema = node.color_schema.as_ref()?;
        let schema_root = self.find_schema_root(node)?;
        let root_schema = schema_root.color_schema.as_ref()?;
        let level = schema.level as usize;
        if level < root_schema.groups.len() {
            Some(&root_schema.groups[level])
        } else {
            // Wrap around if level exceeds groups
            root_schema.groups.last()
        }
    }
}
