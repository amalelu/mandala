//! Edge data model: `MindEdge` (the serialized edge record),
//! `ControlPoint` (its Bezier anchors), and `GlyphConnectionConfig`
//! (the per-edge / per-canvas glyph-connection rendering config plus
//! its `effective_font_size_pt` and `resolved_for` helpers). The
//! sampling / rendering math lives in [`crate::mindmap::connection`];
//! this module owns only the config surface the loader and mutator
//! layers manipulate.

use serde::{Deserialize, Serialize};

use super::Canvas;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MindEdge {
    pub from_id: String,
    pub to_id: String,
    #[serde(rename = "type")]
    pub edge_type: String,
    pub color: String,
    pub width: i32,
    pub line_style: String,
    pub visible: bool,
    pub label: Option<String>,
    /// Parameter-space position of the label along the connection
    /// path. `0.0` sits at the from-anchor, `1.0` at the to-anchor,
    /// `0.5` (or `None`) at the midpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_position_t: Option<f32>,
    pub anchor_from: String,
    pub anchor_to: String,
    pub control_points: Vec<ControlPoint>,
    /// Glyph-based connection rendering. Optional — if absent, the renderer
    /// composes a connection from default glyphs based on the edge direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph_connection: Option<GlyphConnectionConfig>,
    /// How the edge renders. `None` or `Some("line")` → the usual path
    /// from one endpoint to the other. `Some("portal")` → two floating
    /// glyph markers, one above each endpoint node, without a line
    /// between them. Portal mode is the lightweight visual link for
    /// far-apart nodes — clicking a marker selects the edge, double-
    /// clicking navigates the camera to the opposite endpoint. Absent
    /// in serialized JSON when the default holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_mode: Option<String>,
}

/// Sentinel `display_mode` value for portal-mode rendering. Stored
/// as a named string in JSON (per `format/enums.md`) so the vocabulary
/// can grow without breaking old readers.
pub const DISPLAY_MODE_PORTAL: &str = "portal";

/// Sentinel `display_mode` value for standard line rendering — the
/// default when the field is absent. Emitted only when callers need a
/// non-`None` opt-in to the default; normal creation leaves
/// `display_mode = None`.
pub const DISPLAY_MODE_LINE: &str = "line";

/// True if this edge renders as a portal (two glyph markers) rather
/// than a line. Portals reuse `edge.glyph_connection.body` as the
/// marker glyph, `edge.color` (and optionally `glyph_connection.color`)
/// as the marker color, and `glyph_connection.{font, font_size_pt}`
/// for typography — no portal-specific fields exist.
pub fn is_portal_edge(edge: &MindEdge) -> bool {
    matches!(edge.display_mode.as_deref(), Some(DISPLAY_MODE_PORTAL))
}

/// Rotation palette used by `MindMapDocument::create_portal_edge` to
/// pick a distinct default marker glyph for each new portal edge
/// without forcing the user to choose up front. Indexed by
/// `(visible portal-edge count) % PORTAL_GLYPH_PRESETS.len()` at
/// creation time.
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
