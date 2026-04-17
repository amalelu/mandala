//! Portal marker emission. One `PortalElement` per endpoint per
//! edge with `display_mode = "portal"` (so two markers per such edge),
//! floated above each endpoint node's top-right corner. Edges whose
//! endpoints are missing or hidden by fold are skipped silently.
//! Color resolves through the theme variable map.
//!
//! Color-preview override wins on the previewed edge so live
//! color-picker feedback is visible on both markers; selection
//! override is the second-priority source (cyan highlight).

use std::collections::HashMap;

use crate::mindmap::model::{is_portal_edge, GlyphConnectionConfig, MindMap};
use crate::mindmap::scene_cache::EdgeKey;
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
use crate::util::color::resolve_var;

use super::{PortalColorPreview, PortalElement};

/// Default portal marker font size when no `glyph_connection` override
/// is set. A hair larger than body text (which defaults to 12pt) so
/// the marker reads clearly next to the node it sits above without
/// dominating it.
pub(crate) const DEFAULT_PORTAL_MARKER_FONT_SIZE_PT: f32 = 16.0;

/// Resolved rendering params for one portal-mode edge's markers. The
/// connection cascade (edge-override → canvas default → hardcoded
/// default) gives us `glyph_connection.body` as the marker glyph, and
/// `font` / `font_size_pt` / `color` for typography. When neither
/// override nor canvas default sets a size, fall back to
/// `DEFAULT_PORTAL_MARKER_FONT_SIZE_PT` rather than the line body
/// default (12pt), since markers read better a bit larger.
pub(crate) struct ResolvedPortalStyle {
    pub glyph: String,
    pub color: String,
    pub font: Option<String>,
    pub font_size_pt: f32,
}

pub(crate) fn resolve_portal_style(
    edge: &crate::mindmap::model::MindEdge,
    canvas: &crate::mindmap::model::Canvas,
    raw_color_override: Option<&str>,
) -> ResolvedPortalStyle {
    let cfg = GlyphConnectionConfig::resolved_for(edge, canvas);
    // Marker font-size fallback: when an edge is flipped to portal-mode
    // and has *no* `glyph_connection` override (so the resolved size
    // came from the canvas default or the hardcoded 12pt line default),
    // bump the marker to 16pt so it reads clearly next to the node.
    // Any explicit `edge.glyph_connection.font_size_pt` — including a
    // user-chosen 12pt — is respected as-is; the heuristic does not
    // second-guess an explicit value.
    let font_size_pt = if edge.glyph_connection.is_none() {
        DEFAULT_PORTAL_MARKER_FONT_SIZE_PT
    } else {
        cfg.font_size_pt
    };
    let raw_color: &str = raw_color_override
        .or(cfg.color.as_deref())
        .unwrap_or(&edge.color);
    ResolvedPortalStyle {
        glyph: cfg.body.clone(),
        color: resolve_var(raw_color, &canvas.theme_variables).to_string(),
        font: cfg.font.clone(),
        font_size_pt,
    }
}

/// Emit two portal markers per visible portal-mode edge.
pub(super) fn build_portal_elements(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<(&str, &str, &str)>,
    portal_color_preview: Option<PortalColorPreview<'_>>,
) -> Vec<PortalElement> {
    let mut portal_elements: Vec<PortalElement> = Vec::new();

    for edge in &map.edges {
        if !is_portal_edge(edge) {
            continue;
        }
        if !edge.visible {
            continue;
        }
        let node_a = match map.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => continue,
        };
        let node_b = match map.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => continue,
        };
        if map.is_hidden_by_fold(node_a) || map.is_hidden_by_fold(node_b) {
            continue;
        }
        let edge_key = EdgeKey::from_edge(edge);
        let is_selected = selected_edge.map_or(false, |(f, t, ty)| {
            f == edge.from_id && t == edge.to_id && ty == edge.edge_type
        });
        // Color-picker preview beats selection on the previewed edge
        // so the user's live feedback is visible on both markers —
        // the same rule as the edge body path.
        let preview_for_this_edge: Option<&str> = portal_color_preview.and_then(|p| {
            if *p.edge_key == edge_key {
                Some(p.color)
            } else {
                None
            }
        });
        let raw_color_override: Option<&str> = if let Some(p) = preview_for_this_edge {
            Some(p)
        } else if is_selected {
            Some(SELECTION_HIGHLIGHT_HEX)
        } else {
            None
        };
        let style = resolve_portal_style(edge, &map.canvas, raw_color_override);

        for endpoint in [node_a, node_b] {
            let (ox, oy) = offsets.get(&endpoint.id).copied().unwrap_or((0.0, 0.0));
            let node_x = endpoint.position.x as f32 + ox;
            let node_y = endpoint.position.y as f32 + oy;
            let node_w = endpoint.size.width as f32;

            // Loose square AABB sized from the glyph font; matches the
            // connection-label sizing heuristic (≈0.6 × font_size per
            // char, one char wide for the single marker glyph).
            let bounds_w = style.font_size_pt * 1.4;
            let bounds_h = style.font_size_pt * 1.4;
            // Float the marker just above the node's top-right corner.
            let top_left = (node_x + node_w - bounds_w * 0.9, node_y - bounds_h - 8.0);

            portal_elements.push(PortalElement {
                edge_key: edge_key.clone(),
                endpoint_node_id: endpoint.id.clone(),
                glyph: style.glyph.clone(),
                position: top_left,
                bounds: (bounds_w, bounds_h),
                color: style.color.clone(),
                font: style.font.clone(),
                font_size_pt: style.font_size_pt,
            });
        }
    }

    portal_elements
}
