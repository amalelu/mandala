//! Portal marker emission. One `PortalElement` per endpoint per
//! edge with `display_mode = "portal"` (so two markers per such
//! edge), attached to its owning node's border at the point that
//! faces the opposite endpoint (the directional default, overridden
//! by a user-dragged `PortalEndpointState.border_t`). Edges whose
//! endpoints are missing or hidden by fold are skipped silently.
//!
//! Color resolution cascade, per-endpoint:
//!
//! 1. Color-picker live preview on this edge (wins over everything
//!    else so the wheel drag is visible).
//! 2. Selection highlight (cyan) — applied either to both markers
//!    when the whole edge is selected, or to just one marker when
//!    a single portal label is selected via `selected_portal_label`.
//! 3. `PortalEndpointState.color` — per-endpoint override set by
//!    the wheel / paste / console when just this label is the
//!    target.
//! 4. `GlyphConnectionConfig.color` (edge-level override).
//! 5. `MindEdge.color` (final fallback, always present in the
//!    model).
//!
//! All five stages go through `resolve_var` so `var(--name)`
//! references render correctly.

use std::collections::HashMap;

use glam::Vec2;

use crate::mindmap::model::{
    is_portal_edge, portal_endpoint_state, Canvas, GlyphConnectionConfig, MindEdge, MindMap,
    PortalEndpointState, PORTAL_GLYPH_PRESETS,
};
use crate::mindmap::portal_geometry::{
    border_outward_normal, border_point_at, default_border_t,
};
use crate::mindmap::scene_cache::EdgeKey;
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
use crate::util::color::resolve_var;

use super::{PortalColorPreview, PortalElement};

/// Default portal marker font size when no `glyph_connection`
/// override is set. Matches the creation-time default in
/// `document::defaults::default_portal_edge` so an edge flipped
/// from line to portal mode (inheriting the canvas / hardcoded
/// default) and a freshly-created portal edge read at the same
/// visual scale.
pub(crate) const DEFAULT_PORTAL_MARKER_FONT_SIZE_PT: f32 = 50.0;

/// Padding between a portal label and the owning node's border,
/// expressed as a fraction of the marker's font size. Tuned so the
/// label sits just outside the border glyph without visually
/// merging into it.
pub(crate) const PORTAL_OUTSET_FRAC: f32 = 0.35;

/// Default line-body glyph shape — a literal middle dot. When an
/// edge is flipped to portal mode without an explicit glyph, the
/// resolved body is this character, which renders as a hairline
/// dot at portal scale. Detecting it lets us substitute a visible
/// portal-marker preset glyph instead.
const LINE_BODY_DEFAULT_GLYPH: &str = "\u{00B7}";

/// Identifies the currently selected portal label, if any. Passed
/// through the scene / tree build so the selected marker picks up
/// the cyan highlight independently of its sibling on the same
/// edge. Distinct from `selected_edge`: whole-edge selection
/// highlights *both* markers, per-label selection highlights just
/// one.
#[derive(Debug, Clone, Copy)]
pub struct SelectedPortalLabel<'a> {
    pub edge_key: &'a EdgeKey,
    pub endpoint_node_id: &'a str,
}

/// Resolved rendering params for one portal-mode edge's marker on
/// one endpoint. The per-endpoint `color` cascade is materialized
/// into an absolute string here; position math happens in the
/// caller so it can compose geometry from the owning node + partner.
#[derive(Debug, Clone)]
pub struct ResolvedPortalStyle {
    pub glyph: String,
    pub color: String,
    pub font: Option<String>,
    pub font_size_pt: f32,
}

/// Resolved rendering params for a portal endpoint's **text**
/// label — the glyph area that sits alongside the icon. Split out
/// from [`ResolvedPortalStyle`] so per-endpoint overrides
/// (`text_color`, `text_font_size_pt`, `text_min_font_size_pt`,
/// `text_max_font_size_pt`) route only to the text channel while
/// the icon keeps reading its own cascade.
///
/// No `font` field: text always inherits the icon's font (which
/// already routes through `glyph_connection.font`); a
/// per-endpoint text-font override isn't a current requirement
/// and the icon's resolved font reaches the tree builder via
/// `ResolvedPortalStyle::font`.
#[derive(Debug, Clone)]
pub struct ResolvedPortalTextStyle {
    pub color: String,
    pub font_size_pt: f32,
}

/// Resolve the per-endpoint portal marker style. Merges the color
/// cascade (preview > whole-edge-select > per-label-select >
/// per-endpoint override > edge-level override > edge.color),
/// picks a visible glyph, and produces a **canvas-space**
/// font size already compensated for camera zoom the same way
/// line-mode connections do (see
/// [`GlyphConnectionConfig::effective_font_size_pt`]): the
/// renderer scales every glyph by `camera.zoom` at draw time, so
/// at zoom = 0.5 a portal the user wants to read at 50pt on
/// screen needs a 100pt canvas-space glyph. The clamp into
/// `[min_font_size_pt, max_font_size_pt]` runs on the
/// screen-space size, then we divide back through zoom — same
/// formula line connections use so portals LOD identically as
/// the user zooms out.
///
/// `raw_color_override` is the preview / selection hex already
/// resolved by the caller; `None` means "no transient override".
pub fn resolve_portal_endpoint_style(
    edge: &MindEdge,
    endpoint_state: Option<&PortalEndpointState>,
    canvas: &Canvas,
    raw_color_override: Option<&str>,
    camera_zoom: f32,
) -> ResolvedPortalStyle {
    let cfg = GlyphConnectionConfig::resolved_for(edge, canvas);

    // Base (unclamped, pre-zoom) font size. When the edge carries
    // no `glyph_connection` override, fall back to the portal
    // default so markers read at a consistent badge size even on
    // edges flipped from line to portal mode without an explicit
    // marker font setting.
    let base_font_size = if edge.glyph_connection.is_none() {
        DEFAULT_PORTAL_MARKER_FONT_SIZE_PT
    } else {
        cfg.font_size_pt
    };
    // Zoom-clamp — identical to `GlyphConnectionConfig::effective_font_size_pt`,
    // inlined so we can substitute the portal default when there's
    // no per-edge glyph_connection config.
    let z = camera_zoom.max(f32::EPSILON);
    let target_screen =
        (base_font_size * z).clamp(cfg.min_font_size_pt, cfg.max_font_size_pt);
    let font_size_pt = target_screen / z;

    // Glyph fallback. The line-body default (middle dot) renders
    // as a hairline at any reasonable marker size, so an edge
    // flipped to portal mode without a chosen glyph would appear
    // invisible. Substitute the first preset so every portal label
    // has a recognizable shape out of the box.
    let glyph = if cfg.body == LINE_BODY_DEFAULT_GLYPH {
        PORTAL_GLYPH_PRESETS
            .first()
            .copied()
            .unwrap_or(LINE_BODY_DEFAULT_GLYPH)
            .to_string()
    } else {
        cfg.body.clone()
    };

    // Color cascade. Preview and selection overrides (passed via
    // `raw_color_override`) always win so live feedback is visible.
    let raw_color: &str = raw_color_override
        .or_else(|| endpoint_state.and_then(|s| s.color.as_deref()))
        .or(cfg.color.as_deref())
        .unwrap_or(&edge.color);

    ResolvedPortalStyle {
        glyph,
        color: resolve_var(raw_color, &canvas.theme_variables).to_string(),
        font: cfg.font.clone(),
        font_size_pt,
    }
}

/// Resolve the text-channel style for one portal endpoint. Sibling
/// of [`resolve_portal_endpoint_style`] — the text label carries
/// its own color + size cascade so a coloured badge can hold a
/// differently-coloured annotation beside it (parity with
/// line-mode edge labels).
///
/// Color cascade, in order of precedence:
/// 1. `raw_color_override` (preview / whole-edge highlight / per-label
///    highlight) — wins so live wheel feedback and selection cyan
///    remain visible.
/// 2. `endpoint_state.text_color` — per-endpoint text override.
/// 3. `icon_color` — falls back to the already-resolved icon cascade
///    so a portal whose user has only set `color` gets a text
///    channel that matches the icon automatically.
///
/// Font size inheritance:
/// - Base: `endpoint_state.text_font_size_pt` → edge's
///   `glyph_connection.font_size_pt` (or the hardcoded portal
///   default when the edge carries no glyph_connection, matching
///   the icon's fallback).
/// - Clamps: `endpoint_state.text_min_font_size_pt` /
///   `text_max_font_size_pt` → the edge's `glyph_connection` clamps.
/// The clamping formula mirrors
/// [`GlyphConnectionConfig::effective_font_size_pt`]: clamp the
/// target-screen size into `[min, max]` and divide back through
/// `camera_zoom`, so the text LODs the same way the icon does.
pub fn resolve_portal_endpoint_text_style(
    edge: &MindEdge,
    endpoint_state: Option<&PortalEndpointState>,
    canvas: &Canvas,
    raw_color_override: Option<&str>,
    icon_color: &str,
    camera_zoom: f32,
) -> ResolvedPortalTextStyle {
    let cfg = GlyphConnectionConfig::resolved_for(edge, canvas);
    let body_base = if edge.glyph_connection.is_none() {
        DEFAULT_PORTAL_MARKER_FONT_SIZE_PT
    } else {
        cfg.font_size_pt
    };
    let base_font_size = endpoint_state
        .and_then(|s| s.text_font_size_pt)
        .unwrap_or(body_base);
    let min = endpoint_state
        .and_then(|s| s.text_min_font_size_pt)
        .unwrap_or(cfg.min_font_size_pt);
    let max = endpoint_state
        .and_then(|s| s.text_max_font_size_pt)
        .unwrap_or(cfg.max_font_size_pt);
    let z = camera_zoom.max(f32::EPSILON);
    let target_screen = (base_font_size * z).clamp(min, max);
    let font_size_pt = target_screen / z;

    // Text color: transient overrides first, then the per-endpoint
    // `text_color`, then the already-resolved icon color. Falling
    // back to the icon color (as a fully-resolved hex) rather than
    // re-running the icon cascade keeps the two channels in sync
    // for portals the user has only half-styled.
    let resolved_text_color: String = if let Some(hex) = raw_color_override {
        resolve_var(hex, &canvas.theme_variables).to_string()
    } else if let Some(hex) = endpoint_state.and_then(|s| s.text_color.as_deref()) {
        resolve_var(hex, &canvas.theme_variables).to_string()
    } else {
        icon_color.to_string()
    };

    ResolvedPortalTextStyle {
        color: resolved_text_color,
        font_size_pt,
    }
}

/// Per-endpoint layout result: the top-left AABB corner plus its
/// extent, derived from `border_t` (user override) or the
/// directional default. The owning node's position + size have
/// already been offset-adjusted by the caller.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PortalLabelLayout {
    pub top_left: Vec2,
    pub bounds: Vec2,
}

/// Compute the portal label position for one endpoint. `owner_pos`
/// / `owner_size` are the owning node's canvas-space rectangle
/// (with any in-progress drag offset already applied).
/// `partner_center` is used to compute the directional default
/// when `endpoint_state.border_t` is absent.
pub(crate) fn layout_portal_label(
    owner_pos: Vec2,
    owner_size: Vec2,
    partner_center: Vec2,
    endpoint_state: Option<&PortalEndpointState>,
    font_size_pt: f32,
) -> PortalLabelLayout {
    let bounds = Vec2::new(font_size_pt * 1.4, font_size_pt * 1.4);
    let t = endpoint_state
        .and_then(|s| s.border_t)
        .unwrap_or_else(|| default_border_t(owner_pos, owner_size, partner_center));
    let anchor = border_point_at(owner_pos, owner_size, t);
    let normal = border_outward_normal(t);
    let outset = font_size_pt * PORTAL_OUTSET_FRAC;
    // Translate from anchor to AABB top-left: shift by half-extent
    // toward the label origin, then outward along the normal so the
    // label sits just outside the border.
    let top_left = Vec2::new(
        anchor.x - bounds.x * 0.5 + normal.x * (bounds.x * 0.5 + outset),
        anchor.y - bounds.y * 0.5 + normal.y * (bounds.y * 0.5 + outset),
    );
    PortalLabelLayout { top_left, bounds }
}

/// Center of a node in canvas space, used as the partner reference
/// for the directional-default computation.
pub(crate) fn node_center(pos: Vec2, size: Vec2) -> Vec2 {
    Vec2::new(pos.x + size.x * 0.5, pos.y + size.y * 0.5)
}

/// Padding between a portal icon and its adjacent text label,
/// as a fraction of the icon font size. Tuned so the text sits
/// slightly outside the icon AABB without colliding with it.
pub(crate) const PORTAL_TEXT_PADDING_FRAC: f32 = 0.25;

/// Layout result for a portal text label: top-left AABB corner
/// and extent in canvas space. Sits outward of the icon along
/// the border normal so the text always extends away from the
/// owning node rather than toward it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PortalTextLayout {
    pub top_left: Vec2,
    pub bounds: Vec2,
}

/// Compute the AABB for a portal text label, given the icon
/// layout, the border parameter driving the outward normal, and
/// the icon + text font sizes. Text extends from the icon's
/// outward edge away from the node along the normal, with width
/// scaled by grapheme count using the same
/// `char_count × font_size × 0.6` heuristic connection labels
/// use.
///
/// `icon_font_size_pt` drives the padding between icon and text
/// (matches [`PORTAL_TEXT_PADDING_FRAC`]'s contract — "fraction
/// of the **icon** font size") so the visible gap stays stable
/// when the text is resized independently. `text_font_size_pt`
/// drives only the text AABB dimensions.
pub(crate) fn layout_portal_text(
    icon: PortalLabelLayout,
    owner_pos: Vec2,
    owner_size: Vec2,
    partner_center: Vec2,
    endpoint_state: Option<&PortalEndpointState>,
    icon_font_size_pt: f32,
    text_font_size_pt: f32,
    text: &str,
) -> PortalTextLayout {
    // Approximate grapheme count (cheap proxy for shaped
    // width — cosmic-text will reshape on render anyway). Empty
    // strings get a minimum 1-char-wide slot so the buffer is
    // never zero-sized, matching the connection-label helper.
    let char_count = text.chars().count().max(1) as f32;
    let bounds = Vec2::new(char_count * text_font_size_pt * 0.6, text_font_size_pt * 1.3);
    let t = endpoint_state
        .and_then(|s| s.border_t)
        .unwrap_or_else(|| default_border_t(owner_pos, owner_size, partner_center));
    let normal = border_outward_normal(t);
    // Padding is driven by the **icon** size so the visible gap
    // between icon and text stays stable when the user shrinks or
    // grows the text independently — a 6pt annotation beside a
    // 50pt badge still sits at a consistent distance from the badge.
    let padding = icon_font_size_pt * PORTAL_TEXT_PADDING_FRAC;
    // Icon center as the anchor for text placement.
    let icon_center = Vec2::new(
        icon.top_left.x + icon.bounds.x * 0.5,
        icon.top_left.y + icon.bounds.y * 0.5,
    );
    // Distance along the outward normal needed to keep the text
    // AABB entirely outside the icon AABB. Both AABBs are world-
    // axis-aligned; their half-extent along an arbitrary normal
    // is the "support function" of the rectangle —
    // `|half.x * normal.x| + |half.y * normal.y|`. For cardinal
    // normals (top/right/bottom/left sides) this collapses to the
    // half-width and the old `icon.bounds.x * 0.5 + bounds.x * 0.5`
    // formula. For the cardinal-corner transitions where
    // `border_outward_normal` briefly returns a diagonal
    // (Y-down canvas, normal from a corner), the old formula
    // under-estimated the clearance and the text AABB could
    // cross into the icon AABB — mis-routing icon clicks to
    // `ClickHit::PortalText`.
    let icon_half = icon.bounds * 0.5;
    let text_half = bounds * 0.5;
    let abs_normal = Vec2::new(normal.x.abs(), normal.y.abs());
    let icon_support = icon_half.x * abs_normal.x + icon_half.y * abs_normal.y;
    let text_support = text_half.x * abs_normal.x + text_half.y * abs_normal.y;
    let outward_offset = icon_support + padding + text_support;
    let text_center = icon_center + normal * outward_offset;
    let top_left = Vec2::new(
        text_center.x - bounds.x * 0.5,
        text_center.y - bounds.y * 0.5,
    );
    PortalTextLayout { top_left, bounds }
}

/// Emit one `PortalElement` per endpoint of every visible
/// portal-mode edge. Only covers the marker icon — per-endpoint
/// text labels (the adjacent-glyph concept) render exclusively
/// through the tree-builder path (`tree_builder::portal`), which
/// is the live portal render pipeline; the scene-level path
/// exists for tests and stringly-typed inspection, not for the
/// GPU.
pub(super) fn build_portal_elements(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<(&str, &str, &str)>,
    selected_portal_label: Option<SelectedPortalLabel<'_>>,
    portal_color_preview: Option<PortalColorPreview<'_>>,
    camera_zoom: f32,
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
        let is_edge_selected = selected_edge.map_or(false, |(f, t, ty)| {
            f == edge.from_id && t == edge.to_id && ty == edge.edge_type
        });
        let preview_for_this_edge: Option<&str> = portal_color_preview.and_then(|p| {
            if *p.edge_key == edge_key {
                Some(p.color)
            } else {
                None
            }
        });

        let endpoints = [(node_a, node_b), (node_b, node_a)];
        for (owner, partner) in endpoints {
            let (ox, oy) = offsets.get(&owner.id).copied().unwrap_or((0.0, 0.0));
            let owner_pos = Vec2::new(owner.position.x as f32 + ox, owner.position.y as f32 + oy);
            let owner_size = Vec2::new(owner.size.width as f32, owner.size.height as f32);
            let (px, py) = offsets.get(&partner.id).copied().unwrap_or((0.0, 0.0));
            let partner_pos = Vec2::new(
                partner.position.x as f32 + px,
                partner.position.y as f32 + py,
            );
            let partner_size = Vec2::new(partner.size.width as f32, partner.size.height as f32);

            let endpoint_state = portal_endpoint_state(edge, &owner.id);
            let is_this_label_selected = selected_portal_label.map_or(false, |s| {
                *s.edge_key == edge_key && s.endpoint_node_id == owner.id
            });
            let raw_color_override: Option<&str> = if let Some(p) = preview_for_this_edge {
                Some(p)
            } else if is_edge_selected || is_this_label_selected {
                Some(SELECTION_HIGHLIGHT_HEX)
            } else {
                None
            };

            let style = resolve_portal_endpoint_style(
                edge,
                endpoint_state,
                &map.canvas,
                raw_color_override,
                camera_zoom,
            );
            let layout = layout_portal_label(
                owner_pos,
                owner_size,
                node_center(partner_pos, partner_size),
                endpoint_state,
                style.font_size_pt,
            );

            portal_elements.push(PortalElement {
                edge_key: edge_key.clone(),
                endpoint_node_id: owner.id.clone(),
                glyph: style.glyph.clone(),
                position: (layout.top_left.x, layout.top_left.y),
                bounds: (layout.bounds.x, layout.bounds.y),
                color: style.color.clone(),
                font: style.font.clone(),
                font_size_pt: style.font_size_pt,
            });
        }
    }

    portal_elements
}
