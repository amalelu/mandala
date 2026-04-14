use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::mindmap::custom_mutation::{CustomMutation, TriggerBinding};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindMap {
    pub version: String,
    pub name: String,
    pub canvas: Canvas,
    pub nodes: HashMap<String, MindNode>,
    pub edges: Vec<MindEdge>,
    /// Map-level custom mutation definitions, available to all nodes in this map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_mutations: Vec<CustomMutation>,
    /// Session 6E: portal pairs — matching glyph markers on two distant
    /// nodes used as a lightweight alternative to cross-link edges when a
    /// rendered line would clutter the map. Each pair contributes two
    /// rendered markers (one per endpoint). Backward-compatible via
    /// serde default: maps authored before 6E parse with an empty vec.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub portals: Vec<PortalPair>,
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
    /// The live map of theme variables, each keyed by its CSS-style name
    /// (including the leading `--`, e.g. `"--bg"`). Any color string in the
    /// map can reference these via `var(--name)` and will be resolved at
    /// scene-build time. This is the single source of truth for the "current
    /// theme"; switching themes copies a preset from `theme_variants` into
    /// this map.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub theme_variables: HashMap<String, String>,
    /// Named theme presets. Values are whole variable maps that can be
    /// copied into `theme_variables` via a `SetThemeVariant` document
    /// action. Editing a variant here does nothing at runtime until it's
    /// activated — these are authoring state, not the live theme.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub theme_variants: HashMap<String, HashMap<String, String>>,
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
    /// Trigger bindings attached to this specific node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_bindings: Vec<TriggerBinding>,
    /// Inline custom mutations defined on this node (not shared with other nodes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_mutations: Vec<CustomMutation>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Parameter-space position of the label along the connection
    /// path. `0.0` sits at the from-anchor, `1.0` at the to-anchor,
    /// `0.5` (or `None`) at the midpoint. Introduced in Session 6D
    /// for labeled edges; absent on older maps, which render their
    /// labels at the midpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_position_t: Option<f32>,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Font size in points. Interpreted as the *target* on-screen glyph
    /// size at `camera.zoom == 1.0`. At other zoom levels the effective
    /// canvas-space size is derived from this base and clamped into
    /// `[min_font_size_pt, max_font_size_pt]` in screen space — see
    /// [`GlyphConnectionConfig::effective_font_size_pt`].
    #[serde(default = "default_connection_font_size")]
    pub font_size_pt: f32,
    /// Lower bound (in points) on the on-screen glyph size. When zooming
    /// out, this clamp kicks in so glyphs don't collapse into an
    /// unreadable dust cloud; the canvas-space font size is inflated to
    /// keep the on-screen size ≥ this value, which also reduces the
    /// number of sampled glyphs along the connection path.
    #[serde(default = "default_connection_min_font_size")]
    pub min_font_size_pt: f32,
    /// Upper bound (in points) on the on-screen glyph size. When zooming
    /// in, this clamp caps how large individual glyphs can get so a
    /// heavily-magnified connection doesn't render as a few enormous
    /// boulders; the canvas-space font size shrinks to compensate, so
    /// more densely-sampled glyphs follow the path.
    #[serde(default = "default_connection_max_font_size")]
    pub max_font_size_pt: f32,
    /// Color override as #RRGGBB. None = inherit from edge color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Spacing between repeated body glyphs (0 = tight packing).
    #[serde(default)]
    pub spacing: f32,
}

fn default_connection_body() -> String { "\u{00B7}".to_string() } // middle dot ·
fn default_connection_font_size() -> f32 { 12.0 }
fn default_connection_min_font_size() -> f32 { 8.0 }
fn default_connection_max_font_size() -> f32 { 24.0 }

impl Default for GlyphConnectionConfig {
    fn default() -> Self {
        GlyphConnectionConfig {
            body: default_connection_body(),
            cap_start: None,
            cap_end: None,
            font: None,
            font_size_pt: default_connection_font_size(),
            min_font_size_pt: default_connection_min_font_size(),
            max_font_size_pt: default_connection_max_font_size(),
            color: None,
            spacing: 0.0,
        }
    }
}

impl GlyphConnectionConfig {
    /// Effective canvas-space font size for this connection at the given
    /// camera zoom. The renderer applies `TextArea.scale = camera.zoom`
    /// to every connection glyph, so a canvas-space `S` pt glyph ends
    /// up `S * camera_zoom` on screen. To keep the on-screen size inside
    /// `[min_font_size_pt, max_font_size_pt]`, we clamp the target
    /// screen size and divide back through the zoom.
    ///
    /// Because the scene builder uses this value to compute sample
    /// spacing (`effective_font * 0.6 + spacing`), the glyph count along
    /// a connection automatically drops when zoomed out and rises when
    /// zoomed in — the key LOD lever that prevents the dust-cloud
    /// failure mode at extreme zoom levels.
    pub fn effective_font_size_pt(&self, camera_zoom: f32) -> f32 {
        let z = camera_zoom.max(f32::EPSILON);
        let target_screen = (self.font_size_pt * z)
            .clamp(self.min_font_size_pt, self.max_font_size_pt);
        target_screen / z
    }

    /// Return the effective `GlyphConnectionConfig` for `edge`, resolved
    /// through the standard precedence: per-edge override (`edge.glyph_connection`)
    /// > canvas-level default (`canvas.default_connection`) > hardcoded default.
    ///
    /// Session 6D uses this helper from the document mutation layer when
    /// forking an inherited-default edge into a concrete per-edge copy on
    /// the first style edit. The returned `Cow::Owned` case carries a
    /// freshly-cloned value the caller can install into
    /// `edge.glyph_connection`.
    pub fn resolved_for<'a>(edge: &'a MindEdge, canvas: &'a Canvas) -> std::borrow::Cow<'a, GlyphConnectionConfig> {
        if let Some(cfg) = edge.glyph_connection.as_ref() {
            std::borrow::Cow::Borrowed(cfg)
        } else if let Some(cfg) = canvas.default_connection.as_ref() {
            std::borrow::Cow::Borrowed(cfg)
        } else {
            std::borrow::Cow::Owned(GlyphConnectionConfig::default())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlPoint {
    pub x: f64,
    pub y: f64,
}

/// Session 6E: a portal pair — two matching glyph markers placed on
/// two distant nodes so users can visually link them without rendering
/// a connection line across the canvas. Portals are a lightweight
/// alternative to cross-link edges for very far-apart nodes.
///
/// Each pair produces *two* rendered markers (one per endpoint node),
/// both showing the same `glyph` and `color`. The `label` is an
/// auto-assigned identifier ("A", "B", ..., "AA"...) used for stable
/// identity in selection/undo; it is not currently drawn next to the
/// glyph — the glyph alone is the visual cue. Labels are immutable in
/// Session 6E; a rename action is deferred.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortalPair {
    /// Node id of the first endpoint.
    pub endpoint_a: String,
    /// Node id of the second endpoint.
    pub endpoint_b: String,
    /// Auto-assigned stable identifier: "A", "B", ..., "Z", "AA", "AB"...
    /// Picked by `MindMap::next_portal_label` to be the lowest unused
    /// letter in column-letter order at creation time.
    pub label: String,
    /// The visible marker glyph, e.g. "◈", "◆", "⬡". Rotated from
    /// `PORTAL_GLYPH_PRESETS` at creation time, editable via the
    /// Session 6E palette actions.
    pub glyph: String,
    /// Marker color — `#RRGGBB` hex or `var(--name)`. Resolved through
    /// the theme variable map at scene-build time, so `var(--accent)`
    /// auto-restyles on theme swap (see `connection labels` for the
    /// parallel pattern at `scene_builder.rs` Session 6D).
    pub color: String,
    /// Point size of the rendered glyph. Defaults to 16.0 so markers
    /// are a hair larger than body text for legibility without
    /// dominating the node they sit next to.
    #[serde(default = "default_portal_font_size")]
    pub font_size_pt: f32,
    /// Optional font family override. `None` falls back to the
    /// renderer's default font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

fn default_portal_font_size() -> f32 { 16.0 }

/// Convert a 1-indexed ordinal into an Excel-style column letter label:
/// `1 → "A"`, `26 → "Z"`, `27 → "AA"`, `28 → "AB"`, `702 → "ZZ"`,
/// `703 → "AAA"`, and so on. Used by `MindMap::next_portal_label` to
/// walk a lazy sequence of candidate portal labels. Panics on `0`.
pub fn column_letter_label(mut n: u64) -> String {
    assert!(n > 0, "column_letter_label is 1-indexed");
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    s
}

/// Rotation palette used by `MindMapDocument::apply_create_portal`
/// to pick a distinct default glyph for each new portal without
/// requiring the user to choose up front. Indexed by
/// `portals.len() % PORTAL_GLYPH_PRESETS.len()` at creation time.
pub const PORTAL_GLYPH_PRESETS: &[&str] = &[
    "\u{25C8}", // ◈ white diamond containing black small diamond
    "\u{25C6}", // ◆ black diamond
    "\u{2B21}", // ⬡ white hexagon
    "\u{2B22}", // ⬢ black hexagon
    "\u{25C9}", // ◉ fisheye
    "\u{2756}", // ❖ black diamond minus white X
    "\u{2726}", // ✦ black four pointed star
    "\u{2727}", // ✧ white four pointed star
];

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
            nodes: HashMap::new(),
            edges: Vec::new(),
            custom_mutations: Vec::new(),
            portals: Vec::new(),
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mindmap::loader;
    use std::path::PathBuf;

    fn test_map_path() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop(); // lib/baumhard -> lib
        path.pop(); // lib -> root
        path.push("maps/testament.mindmap.json");
        path
    }

    #[test]
    fn test_all_descendants() {
        let path = test_map_path();
        let map = loader::load_from_file(&path).unwrap();

        // "Lord God" (348068464) has children — descendants should include them all
        let children = map.children_of("348068464");
        assert!(!children.is_empty(), "Lord God should have children");

        let descendants = map.all_descendants("348068464");
        // Every direct child should appear in descendants
        for child in &children {
            assert!(descendants.contains(&child.id), "Child {} missing from descendants", child.id);
        }
        // Descendants should be >= children (includes grandchildren etc.)
        assert!(descendants.len() >= children.len());
    }

    #[test]
    fn test_all_descendants_leaf_node() {
        let path = test_map_path();
        let map = loader::load_from_file(&path).unwrap();

        // Find a leaf node (no children)
        let leaf = map.nodes.values()
            .find(|n| map.children_of(&n.id).is_empty())
            .expect("Should have at least one leaf node");

        let descendants = map.all_descendants(&leaf.id);
        assert!(descendants.is_empty(), "Leaf node should have no descendants");
    }

    /// Find a (root_id, child_id, grandchild_id) triple in the testament map.
    /// Used by the ancestor tests below.
    fn find_hierarchy_triple(map: &MindMap) -> (String, String, String) {
        for root in map.root_nodes() {
            for child in map.children_of(&root.id) {
                let grands = map.children_of(&child.id);
                if let Some(grand) = grands.first() {
                    return (root.id.clone(), child.id.clone(), grand.id.clone());
                }
            }
        }
        panic!("testament map should contain a root -> child -> grandchild chain");
    }

    #[test]
    fn test_is_ancestor_or_self_reflexive() {
        let map = loader::load_from_file(&test_map_path()).unwrap();
        let (root, child, grand) = find_hierarchy_triple(&map);
        assert!(map.is_ancestor_or_self(&root, &root));
        assert!(map.is_ancestor_or_self(&child, &child));
        assert!(map.is_ancestor_or_self(&grand, &grand));
    }

    #[test]
    fn test_is_ancestor_or_self_direct_parent() {
        let map = loader::load_from_file(&test_map_path()).unwrap();
        let (root, child, grand) = find_hierarchy_triple(&map);
        // root is a direct ancestor of child; child is a direct ancestor of grand
        assert!(map.is_ancestor_or_self(&root, &child));
        assert!(map.is_ancestor_or_self(&child, &grand));
    }

    #[test]
    fn test_is_ancestor_or_self_deep_descendant() {
        let map = loader::load_from_file(&test_map_path()).unwrap();
        let (root, _child, grand) = find_hierarchy_triple(&map);
        // root is a transitive ancestor of grand (two hops away)
        assert!(map.is_ancestor_or_self(&root, &grand));
    }

    #[test]
    fn test_is_ancestor_or_self_reversed_is_false() {
        let map = loader::load_from_file(&test_map_path()).unwrap();
        let (root, child, grand) = find_hierarchy_triple(&map);
        // A descendant is never the ancestor of its own parent chain.
        assert!(!map.is_ancestor_or_self(&child, &root));
        assert!(!map.is_ancestor_or_self(&grand, &child));
        assert!(!map.is_ancestor_or_self(&grand, &root));
    }

    #[test]
    fn test_is_ancestor_or_self_sibling_is_unrelated() {
        let map = loader::load_from_file(&test_map_path()).unwrap();
        // Find two sibling roots (they share parent_id = None but are not
        // ancestors of each other).
        let roots = map.root_nodes();
        if roots.len() >= 2 {
            let a = roots[0].id.clone();
            let b = roots[1].id.clone();
            assert!(!map.is_ancestor_or_self(&a, &b));
            assert!(!map.is_ancestor_or_self(&b, &a));
        }
        // Also check: the first root and some node whose parent chain does not
        // include it (pick an unrelated subtree if available).
        // The above two-sibling-roots case is sufficient for testament.
    }

    /// Tiny tolerance for floating-point comparisons in the
    /// `effective_font_size_pt` tests below — the formula is just two
    /// multiplies and a divide, so anything tighter than this means a
    /// real bug.
    const EFFECTIVE_FONT_EPSILON: f32 = 1.0e-4;

    #[test]
    fn effective_font_size_unity_zoom_returns_base() {
        let cfg = GlyphConnectionConfig::default(); // 12 / 8 / 24
        // At zoom = 1.0 the base 12 is inside [8, 24], so screen size
        // = 12 and canvas size = 12 / 1 = 12.
        assert!(
            (cfg.effective_font_size_pt(1.0) - 12.0).abs() < EFFECTIVE_FONT_EPSILON,
            "expected 12.0 at zoom 1.0, got {}",
            cfg.effective_font_size_pt(1.0)
        );
    }

    #[test]
    fn effective_font_size_zoomed_out_floors_to_min() {
        let cfg = GlyphConnectionConfig::default();
        // At zoom = 0.1: base * zoom = 1.2 → clamp up to 8 → canvas
        // = 8 / 0.1 = 80.
        let got = cfg.effective_font_size_pt(0.1);
        assert!(
            (got - 80.0).abs() < EFFECTIVE_FONT_EPSILON,
            "expected 80.0 at zoom 0.1, got {got}"
        );

        // At zoom = 0.5: base * zoom = 6 → clamp up to 8 → canvas
        // = 8 / 0.5 = 16.
        let got = cfg.effective_font_size_pt(0.5);
        assert!(
            (got - 16.0).abs() < EFFECTIVE_FONT_EPSILON,
            "expected 16.0 at zoom 0.5, got {got}"
        );
    }

    #[test]
    fn effective_font_size_zoomed_in_ceils_to_max() {
        let cfg = GlyphConnectionConfig::default();
        // At zoom = 2.0: base * zoom = 24 (right at the cap) → canvas
        // = 24 / 2 = 12.
        let got = cfg.effective_font_size_pt(2.0);
        assert!(
            (got - 12.0).abs() < EFFECTIVE_FONT_EPSILON,
            "expected 12.0 at zoom 2.0, got {got}"
        );

        // At zoom = 5.0: base * zoom = 60 → clamp down to 24 → canvas
        // = 24 / 5 = 4.8.
        let got = cfg.effective_font_size_pt(5.0);
        assert!(
            (got - 4.8).abs() < EFFECTIVE_FONT_EPSILON,
            "expected 4.8 at zoom 5.0, got {got}"
        );
    }

    #[test]
    fn effective_font_size_handles_zero_and_negative_zoom() {
        // Zero or negative zoom would divide by zero / produce a
        // negative font; the implementation guards with EPSILON. Just
        // assert it returns a finite, positive value rather than
        // panicking or returning NaN.
        let cfg = GlyphConnectionConfig::default();
        let z0 = cfg.effective_font_size_pt(0.0);
        assert!(z0.is_finite() && z0 > 0.0, "expected finite > 0, got {z0}");
        let zn = cfg.effective_font_size_pt(-1.0);
        assert!(zn.is_finite() && zn > 0.0, "expected finite > 0, got {zn}");
    }

    #[test]
    fn effective_font_size_respects_custom_bounds() {
        // Tighter clamp: [10, 14] with the same base.
        let cfg = GlyphConnectionConfig {
            min_font_size_pt: 10.0,
            max_font_size_pt: 14.0,
            ..GlyphConnectionConfig::default()
        };
        // zoom = 1.0: 12 in [10, 14] → canvas 12.
        assert!((cfg.effective_font_size_pt(1.0) - 12.0).abs() < EFFECTIVE_FONT_EPSILON);
        // zoom = 0.5: 6 → up to 10 → canvas 20.
        assert!((cfg.effective_font_size_pt(0.5) - 20.0).abs() < EFFECTIVE_FONT_EPSILON);
        // zoom = 2.0: 24 → down to 14 → canvas 7.
        assert!((cfg.effective_font_size_pt(2.0) - 7.0).abs() < EFFECTIVE_FONT_EPSILON);
    }

    // Session 6D Phase 1: label_position_t + resolved_for helper.

    fn synthetic_edge_with_label(label: Option<&str>, pos: Option<f32>) -> MindEdge {
        MindEdge {
            from_id: "a".to_string(),
            to_id: "b".to_string(),
            edge_type: "cross_link".to_string(),
            color: "#fff".to_string(),
            width: 1,
            line_style: 0,
            visible: true,
            label: label.map(|s| s.to_string()),
            label_position_t: pos,
            anchor_from: 0,
            anchor_to: 0,
            control_points: Vec::new(),
            glyph_connection: None,
        }
    }

    #[test]
    fn label_position_t_round_trips_through_json() {
        // Explicit value is preserved.
        let edge = synthetic_edge_with_label(Some("hello"), Some(0.25));
        let json = serde_json::to_string(&edge).unwrap();
        assert!(json.contains("label_position_t"), "json should include the field: {json}");
        let back: MindEdge = serde_json::from_str(&json).unwrap();
        assert_eq!(back.label.as_deref(), Some("hello"));
        assert_eq!(back.label_position_t, Some(0.25));
    }

    #[test]
    fn label_position_t_missing_defaults_to_none() {
        // Older maps without the field must still deserialize.
        let json = r##"{
            "from_id":"a","to_id":"b","type":"cross_link",
            "color":"#fff","width":1,"line_style":0,"visible":true,
            "label":null,"anchor_from":0,"anchor_to":0,"control_points":[]
        }"##;
        let edge: MindEdge = serde_json::from_str(json).unwrap();
        assert_eq!(edge.label_position_t, None);
        // And round-trips back without the field (skip_serializing_if).
        let back_json = serde_json::to_string(&edge).unwrap();
        assert!(
            !back_json.contains("label_position_t"),
            "None should not serialize: {back_json}"
        );
    }

    #[test]
    fn resolved_for_returns_borrowed_from_edge_when_present() {
        let mut edge = synthetic_edge_with_label(None, None);
        let custom = GlyphConnectionConfig {
            body: "◆".to_string(),
            ..GlyphConnectionConfig::default()
        };
        edge.glyph_connection = Some(custom);
        let canvas = Canvas {
            background_color: "#000".to_string(),
            default_border: None,
            default_connection: None,
            theme_variables: HashMap::new(),
            theme_variants: HashMap::new(),
        };
        let resolved = GlyphConnectionConfig::resolved_for(&edge, &canvas);
        assert_eq!(resolved.body, "◆");
        // It's borrowed, not owned — clone-count unchanged.
        assert!(matches!(resolved, std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn resolved_for_falls_back_to_canvas_default() {
        let edge = synthetic_edge_with_label(None, None);
        let canvas_cfg = GlyphConnectionConfig {
            body: "═".to_string(),
            ..GlyphConnectionConfig::default()
        };
        let canvas = Canvas {
            background_color: "#000".to_string(),
            default_border: None,
            default_connection: Some(canvas_cfg),
            theme_variables: HashMap::new(),
            theme_variants: HashMap::new(),
        };
        let resolved = GlyphConnectionConfig::resolved_for(&edge, &canvas);
        assert_eq!(resolved.body, "═");
        assert!(matches!(resolved, std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn resolved_for_falls_back_to_hardcoded_default() {
        let edge = synthetic_edge_with_label(None, None);
        let canvas = Canvas {
            background_color: "#000".to_string(),
            default_border: None,
            default_connection: None,
            theme_variables: HashMap::new(),
            theme_variants: HashMap::new(),
        };
        let resolved = GlyphConnectionConfig::resolved_for(&edge, &canvas);
        assert_eq!(resolved.body, GlyphConnectionConfig::default().body);
        // Owned — the caller got a freshly-built default.
        assert!(matches!(resolved, std::borrow::Cow::Owned(_)));
    }

    // ============================================================
    // Session 6E — portal data model tests
    // ============================================================

    fn synthetic_empty_map() -> MindMap {
        MindMap {
            version: "1".to_string(),
            name: "test".to_string(),
            canvas: Canvas {
                background_color: "#000".to_string(),
                default_border: None,
                default_connection: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            nodes: HashMap::new(),
            edges: Vec::new(),
            custom_mutations: Vec::new(),
            portals: Vec::new(),
        }
    }

    #[test]
    fn column_letter_label_sequence() {
        assert_eq!(column_letter_label(1), "A");
        assert_eq!(column_letter_label(2), "B");
        assert_eq!(column_letter_label(26), "Z");
        assert_eq!(column_letter_label(27), "AA");
        assert_eq!(column_letter_label(28), "AB");
        assert_eq!(column_letter_label(52), "AZ");
        assert_eq!(column_letter_label(53), "BA");
        assert_eq!(column_letter_label(702), "ZZ");
        assert_eq!(column_letter_label(703), "AAA");
    }

    #[test]
    fn portal_pair_round_trips_through_json() {
        let portal = PortalPair {
            endpoint_a: "node-1".to_string(),
            endpoint_b: "node-2".to_string(),
            label: "A".to_string(),
            glyph: "\u{25C8}".to_string(),
            color: "var(--accent)".to_string(),
            font_size_pt: 18.0,
            font: Some("LiberationSans".to_string()),
        };
        let json = serde_json::to_string(&portal).unwrap();
        assert!(json.contains("node-1"));
        assert!(json.contains("\"label\":\"A\""));
        let back: PortalPair = serde_json::from_str(&json).unwrap();
        assert_eq!(back.endpoint_a, "node-1");
        assert_eq!(back.endpoint_b, "node-2");
        assert_eq!(back.label, "A");
        assert_eq!(back.color, "var(--accent)");
        assert_eq!(back.font_size_pt, 18.0);
        assert_eq!(back.font.as_deref(), Some("LiberationSans"));
    }

    #[test]
    fn portal_pair_font_size_defaults_when_missing() {
        // A portal authored without `font_size_pt` must deserialize with the
        // default 16.0 so older saved maps keep working when this field is
        // added post-hoc.
        let json = r##"{
            "endpoint_a":"a","endpoint_b":"b",
            "label":"A","glyph":"\u25C8","color":"#aa88cc"
        }"##;
        let portal: PortalPair = serde_json::from_str(json).unwrap();
        assert_eq!(portal.font_size_pt, 16.0);
        assert_eq!(portal.font, None);
    }

    #[test]
    fn portals_missing_deserializes_empty() {
        // Maps authored before Session 6E omit the `portals` field
        // entirely. `#[serde(default)]` must give them an empty vec so
        // they keep loading cleanly.
        let map = loader::load_from_file(&test_map_path()).unwrap();
        assert!(map.portals.is_empty(), "pre-6E maps should have no portals");
    }

    #[test]
    fn portals_empty_vec_skipped_in_serialize() {
        // A fresh map with no portals must not write the field so the
        // on-disk JSON shape for existing maps is byte-stable.
        let map = synthetic_empty_map();
        let json = serde_json::to_string(&map).unwrap();
        assert!(
            !json.contains("\"portals\""),
            "empty portals should not appear in JSON: {json}"
        );
    }

    #[test]
    fn next_portal_label_picks_lowest_unused() {
        let mut map = synthetic_empty_map();
        assert_eq!(map.next_portal_label(), "A");

        map.portals.push(PortalPair {
            endpoint_a: "x".to_string(), endpoint_b: "y".to_string(),
            label: "A".to_string(), glyph: "\u{25C8}".to_string(),
            color: "#aa88cc".to_string(), font_size_pt: 16.0, font: None,
        });
        assert_eq!(map.next_portal_label(), "B");

        // Fill in "B" — next should be "C".
        map.portals.push(PortalPair {
            endpoint_a: "x".to_string(), endpoint_b: "y".to_string(),
            label: "B".to_string(), glyph: "\u{25C6}".to_string(),
            color: "#aa88cc".to_string(), font_size_pt: 16.0, font: None,
        });
        assert_eq!(map.next_portal_label(), "C");

        // Skip "C", use "D" — the gap at "C" should be reused first.
        map.portals.last_mut().unwrap().label = "D".to_string();
        assert_eq!(map.next_portal_label(), "B");
    }

    #[test]
    fn next_portal_label_wraps_to_double_letter() {
        let mut map = synthetic_empty_map();
        // Fill A..Z.
        for n in 1u64..=26 {
            map.portals.push(PortalPair {
                endpoint_a: "x".to_string(), endpoint_b: "y".to_string(),
                label: column_letter_label(n),
                glyph: "\u{25C8}".to_string(),
                color: "#aa88cc".to_string(),
                font_size_pt: 16.0,
                font: None,
            });
        }
        assert_eq!(map.next_portal_label(), "AA");
    }

    #[test]
    fn portal_glyph_presets_are_nonempty_and_unique() {
        assert!(!PORTAL_GLYPH_PRESETS.is_empty());
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for g in PORTAL_GLYPH_PRESETS {
            assert!(seen.insert(*g), "glyph preset {g} duplicated");
        }
    }
}
