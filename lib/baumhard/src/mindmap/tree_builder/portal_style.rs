// SPDX-License-Identifier: MPL-2.0

//! Portal endpoint style + layout resolution — the per-endpoint
//! cascade that turns a `display_mode = "portal"` edge into the
//! icon glyph, the text glyph, and the two canvas-space AABBs
//! [`super::portal`] projects into the `Portals` canvas tree.
//!
//! Split from the tree builder because the cascade is a
//! self-contained pure function over `(edge, endpoint_state,
//! canvas, override, zoom)`: the clipboard resolver, the console
//! verbs, and the tests all want the resolved style without
//! building an arena.
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

use glam::Vec2;

use crate::mindmap::model::{
    Canvas, GlyphConnectionConfig, MindEdge, PortalEndpointState, PORTAL_GLYPH_PRESETS,
};
use crate::mindmap::portal_geometry::{border_outward_normal, border_point_at, default_border_t};
use crate::mindmap::scene_cache::EdgeKey;
use crate::util::color::resolve_var;
use crate::util::geometry::aabb_center;
use crate::util::grapheme_chad::count_grapheme_clusters;

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
/// per-endpoint override > edge-level override > source-node
/// palette stroke > edge.color),
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
/// `themed` is the palette tier of the edge color cascade —
/// resolve it with
/// [`MindMap::edge_theme_stroke_color`](crate::mindmap::model::MindMap::edge_theme_stroke_color).
pub fn resolve_portal_endpoint_style(
    edge: &MindEdge,
    endpoint_state: Option<&PortalEndpointState>,
    canvas: &Canvas,
    raw_color_override: Option<&str>,
    themed: Option<&str>,
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
    let target_screen = crate::font::fonts::clamp_to_font_window(
        base_font_size * z,
        cfg.min_font_size_pt,
        cfg.max_font_size_pt,
    );
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
    // `raw_color_override`) always win so live feedback is
    // visible; the committed cascade below them is
    // `MindEdge::portal_endpoint_color`'s.
    let raw_color: &str =
        raw_color_override.unwrap_or_else(|| edge.portal_endpoint_color(canvas, endpoint_state, themed));

    ResolvedPortalStyle {
        glyph,
        color: resolve_var(raw_color, &canvas.theme_variables).to_string(),
        font: cfg.font.clone(),
        font_size_pt,
    }
}

/// Resolve the text-channel style for one portal endpoint. Sibling
/// of [`resolve_portal_endpoint_style`] — the text label carries
/// its own color + size cascade so a colored badge can hold a
/// differently-colored annotation beside it (parity with
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
///
/// - Base: `endpoint_state.text_font_size_pt` → edge's
///   `glyph_connection.font_size_pt` (or the hardcoded portal
///   default when the edge carries no glyph_connection, matching
///   the icon's fallback).
/// - Clamps: `endpoint_state.text_min_font_size_pt` /
///   `text_max_font_size_pt` → the edge's `glyph_connection` clamps.
///
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
    let target_screen = crate::font::fonts::clamp_to_font_window(base_font_size * z, min, max);
    let font_size_pt = target_screen / z;

    // Text color: transient overrides first, then the per-endpoint
    // `text_color`, then the already-resolved icon color. Falling
    // back to the icon color (as a fully-resolved hex) rather than
    // re-running the icon cascade keeps the two channels in sync
    // for portals the user has only half-styled.
    //
    // Deliberately *not*
    // [`MindEdge::portal_endpoint_text_color`](crate::mindmap::model::MindEdge::portal_endpoint_text_color):
    // that helper resolves the committed model cascade, while
    // `icon_color` here may carry a preview / selection override
    // the text is supposed to follow. The model helper is what
    // the document layer's clipboard resolver reads, where no
    // transient override exists.
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
    // Drag-authored perpendicular slide. Sums into the outset so the
    // user can pull the label further away from the border (positive)
    // or back toward it (negative). `None` falls through to a flush
    // outset, matching the pre-field behavior.
    let perp = endpoint_state.and_then(|s| s.perpendicular_offset).unwrap_or(0.0);
    // Translate from anchor to AABB top-left: shift by half-extent
    // toward the label origin, then outward along the normal so the
    // label sits just outside the border.
    let top_left = Vec2::new(
        anchor.x - bounds.x * 0.5 + normal.x * (bounds.x * 0.5 + outset + perp),
        anchor.y - bounds.y * 0.5 + normal.y * (bounds.y * 0.5 + outset + perp),
    );
    PortalLabelLayout { top_left, bounds }
}

/// Padding between a portal icon and its adjacent text label,
/// as a fraction of the icon font size. Tuned so the text sits
/// slightly outside the icon AABB without colliding with it.
pub(crate) const PORTAL_TEXT_PADDING_FRAC: f32 = 0.25;

/// Layout result for a portal text label: top-left AABB corner
/// and extent in canvas space. Sits outward of the icon along
/// the border normal so the text always extends away from the
/// owning node rather than toward it. `bounds` is
/// [`Vec2::ZERO`] for empty text — see [`layout_portal_text`] for
/// why that is the phantom-hot-zone invariant and not an accident.
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
// `clippy::too_many_arguments`: the geometry needs both AABBs, both
// font sizes, the owning rect, the partner center, and the text —
// every one an independent input to the placement math.
#[allow(clippy::too_many_arguments)]
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
    // Grapheme-cluster count as a cheap proxy for shaped width —
    // cosmic-text will reshape on render anyway. Counting graphemes
    // (not Unicode scalars) keeps a family-ZWJ emoji at one slot
    // wide instead of eleven (§B3).
    //
    // Empty text collapses to a **zero-extent** box, and that is
    // load-bearing rather than cosmetic. The portal tree always
    // emits a text slot (the channel layout has to stay stable for
    // the §B2 in-place mutator path), so a text-less endpoint would
    // otherwise carry a ~30×65 px rectangle beside its icon that
    // renders nothing but still answers hit-tests — a phantom hot
    // zone stealing clicks from whatever sits beneath. Both
    // consumers already skip zero-extent areas: the renderer's
    // tree walker returns early on empty text, and
    // `Tree::descendant_at`'s BVH requires strictly positive
    // bounds. Suppression therefore lives in the geometry itself,
    // with no side table to keep in sync.
    let grapheme_count = count_grapheme_clusters(text) as f32;
    let bounds = if grapheme_count == 0.0 {
        Vec2::ZERO
    } else {
        Vec2::new(grapheme_count * text_font_size_pt * 0.6, text_font_size_pt * 1.3)
    };
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
    let icon_center = aabb_center(icon.top_left, icon.bounds);
    // Distance along the outward normal needed to keep the text
    // AABB entirely outside the icon AABB. Both AABBs are world-
    // axis-aligned; their half-extent along an arbitrary normal is
    // the "support function" of the rectangle —
    // `|half.x * normal.x| + |half.y * normal.y|`.
    //
    // `border_outward_normal` returns only the four axis-aligned
    // unit vectors today, so this collapses to the half-width or
    // half-height and the general form is not currently load-
    // bearing. It is written in the general form anyway because it
    // is the *reason* icon and text can never overlap — with
    // `padding` strictly positive, the normal is a separating axis
    // between the two boxes for any normal, not just the cardinal
    // four. That separation is what lets click routing resolve the
    // portal's two sub-parts in a single BVH descent instead of
    // needing a text-before-icon precedence rule.
    let icon_half = icon.bounds * 0.5;
    let text_half = bounds * 0.5;
    let abs_normal = Vec2::new(normal.x.abs(), normal.y.abs());
    let icon_support = icon_half.x * abs_normal.x + icon_half.y * abs_normal.y;
    let text_support = text_half.x * abs_normal.x + text_half.y * abs_normal.y;
    let outward_offset = icon_support + padding + text_support;
    let text_center = icon_center + normal * outward_offset;
    let top_left = Vec2::new(text_center.x - bounds.x * 0.5, text_center.y - bounds.y * 0.5);
    PortalTextLayout { top_left, bounds }
}
