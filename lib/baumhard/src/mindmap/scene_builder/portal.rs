//! Portal marker emission. One `PortalElement` per endpoint per
//! pair (so two markers per `PortalPair`), floated above each
//! endpoint node's top-right corner. Portals whose endpoints are
//! missing or hidden by fold are skipped silently. Color resolves
//! through the theme variable map.
//!
//! Color-preview override wins on the previewed portal so live
//! color-picker feedback is visible on both markers; selection
//! override is the second-priority source (cyan highlight).

use std::collections::HashMap;

use crate::mindmap::model::MindMap;
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
use crate::util::color::resolve_var;

use super::{PortalColorPreview, PortalElement, PortalRefKey};

/// Emit two portal markers per visible `PortalPair`.
pub(super) fn build_portal_elements(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_portal: Option<(&str, &str, &str)>,
    portal_color_preview: Option<PortalColorPreview<'_>>,
) -> Vec<PortalElement> {
    let vars = &map.canvas.theme_variables;
    let mut portal_elements: Vec<PortalElement> = Vec::new();

    for portal in &map.portals {
        let node_a = match map.nodes.get(&portal.endpoint_a) {
            Some(n) => n,
            None => continue,
        };
        let node_b = match map.nodes.get(&portal.endpoint_b) {
            Some(n) => n,
            None => continue,
        };
        if map.is_hidden_by_fold(node_a) || map.is_hidden_by_fold(node_b) {
            continue;
        }
        let is_selected = selected_portal.map_or(false, |(l, a, b)| {
            l == portal.label && a == portal.endpoint_a && b == portal.endpoint_b
        });
        let key = PortalRefKey::from_portal(portal);
        // Color picker preview beats selection on the previewed
        // portal so the user's live feedback is visible on both
        // markers — the same rule as the edge body path.
        let preview_for_this_portal: Option<&str> = portal_color_preview.and_then(|p| {
            if *p.portal_key == key {
                Some(p.color)
            } else {
                None
            }
        });
        let raw_color: &str = if let Some(p) = preview_for_this_portal {
            p
        } else if is_selected {
            SELECTION_HIGHLIGHT_HEX
        } else {
            portal.color.as_str()
        };
        let color = resolve_var(raw_color, vars).to_string();

        for endpoint in [node_a, node_b] {
            let (ox, oy) = offsets.get(&endpoint.id).copied().unwrap_or((0.0, 0.0));
            let node_x = endpoint.position.x as f32 + ox;
            let node_y = endpoint.position.y as f32 + oy;
            let node_w = endpoint.size.width as f32;

            // Loose square AABB sized from the glyph font; matches the
            // connection-label sizing heuristic (≈0.6 × font_size per
            // char, one char wide for the single marker glyph).
            let bounds_w = portal.font_size_pt * 1.4;
            let bounds_h = portal.font_size_pt * 1.4;
            // Float the marker just above the node's top-right corner.
            let top_left = (node_x + node_w - bounds_w * 0.9, node_y - bounds_h - 8.0);

            portal_elements.push(PortalElement {
                portal_ref: key.clone(),
                endpoint_node_id: endpoint.id.clone(),
                glyph: portal.glyph.clone(),
                position: top_left,
                bounds: (bounds_w, bounds_h),
                color: color.clone(),
                font: portal.font.clone(),
                font_size_pt: portal.font_size_pt,
            });
        }
    }

    portal_elements
}
