//! Applicability predicates migrated from `palette.rs`.
//!
//! These are the "is this command relevant to the current selection"
//! checks. `help` filters the visible command list through them;
//! completion hides commands whose predicate returns false; and
//! commands can re-use them inside `execute` to short-circuit no-ops.
//!
//! They're kept in one place so the predicate vocabulary is scannable
//! — if you're adding a new command and need "edge is selected", the
//! helper already exists here.

use super::ConsoleContext;
use crate::application::document::{EdgeRef, SelectionState};

// ============================================================
// Selection shape
// ============================================================

pub fn always(_: &ConsoleContext) -> bool {
    true
}

pub fn edge_selected(ctx: &ConsoleContext) -> bool {
    matches!(ctx.document.selection, SelectionState::Edge(_))
}

pub fn two_nodes_selected(ctx: &ConsoleContext) -> bool {
    matches!(&ctx.document.selection, SelectionState::Multi(ids) if ids.len() == 2)
}

/// True when the current selection points at an edge (either
/// `SelectionState::Edge` or `SelectionState::PortalLabel`).
/// Commands that target the edge *as a whole* (type change,
/// display mode flip, path reset) use this so they keep working
/// after a click lands on a portal marker — otherwise flipping
/// an edge to portal mode would trap the user (click-to-select
/// on a portal yields `PortalLabel`, and no edge command would
/// apply).
pub fn edge_or_portal_label_selected(ctx: &ConsoleContext) -> bool {
    ctx.document.selection.selected_edge_or_portal_edge().is_some()
}

/// True when the current selection resolves to a portal-mode
/// edge — covers both `Edge(er)` pointing at a portal-mode edge
/// and any `PortalLabel` selection (whose owning edge is
/// definitionally portal-mode). Used by palette entries that
/// should only surface when the user is in portal context.
pub fn portal_edge_selected(ctx: &ConsoleContext) -> bool {
    with_selected_edge(ctx, baumhard::mindmap::model::is_portal_edge)
}

pub fn edge_selected_or_two_nodes(ctx: &ConsoleContext) -> bool {
    edge_selected(ctx) || two_nodes_selected(ctx)
}

/// `color pick` is applicable for both edges and portals — each
/// branch hands off to the appropriate `ColorTarget`.
pub fn edge_selected_with_control_points(ctx: &ConsoleContext) -> bool {
    with_selected_edge(ctx, |e| !e.control_points.is_empty())
}

/// Resolve the currently-targeted edge ref (widens to include
/// `PortalLabel`). Kept as a module helper so every predicate
/// uses the same disambiguation rule.
fn selected_edge_ref(ctx: &ConsoleContext) -> Option<EdgeRef> {
    ctx.document.selection.selected_edge_or_portal_edge()
}

// ============================================================
// Edge resolved-config queries
// ============================================================

fn with_selected_edge<F>(ctx: &ConsoleContext, f: F) -> bool
where
    F: FnOnce(&baumhard::mindmap::model::MindEdge) -> bool,
{
    let er = match selected_edge_ref(ctx) {
        Some(e) => e,
        None => return false,
    };
    ctx.document
        .mindmap
        .edges
        .iter()
        .find(|e| er.matches(e))
        .map(f)
        .unwrap_or(false)
}

fn resolved_for_selected<'a>(
    ctx: &'a ConsoleContext,
) -> Option<std::borrow::Cow<'a, baumhard::mindmap::model::GlyphConnectionConfig>> {
    let er = selected_edge_ref(ctx)?;
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    Some(baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    ))
}

fn effective_body_glyph(ctx: &ConsoleContext) -> Option<String> {
    resolved_for_selected(ctx).map(|r| r.body.clone())
}

fn effective_cap_start(ctx: &ConsoleContext) -> Option<Option<String>> {
    resolved_for_selected(ctx).map(|r| r.cap_start.clone())
}

fn effective_cap_end(ctx: &ConsoleContext) -> Option<Option<String>> {
    resolved_for_selected(ctx).map(|r| r.cap_end.clone())
}

pub fn effective_font_size_pt(ctx: &ConsoleContext) -> Option<f32> {
    resolved_for_selected(ctx).map(|r| r.font_size_pt)
}

pub fn selected_edge_min_font(ctx: &ConsoleContext) -> Option<f32> {
    resolved_for_selected(ctx).map(|r| r.min_font_size_pt)
}

pub fn selected_edge_max_font(ctx: &ConsoleContext) -> Option<f32> {
    resolved_for_selected(ctx).map(|r| r.max_font_size_pt)
}

pub fn effective_spacing(ctx: &ConsoleContext) -> Option<f32> {
    resolved_for_selected(ctx).map(|r| r.spacing)
}

pub fn body_is(ctx: &ConsoleContext, glyph: &str) -> bool {
    effective_body_glyph(ctx).map(|g| g == glyph).unwrap_or(false)
}

pub fn cap_start_is(ctx: &ConsoleContext, glyph: Option<&str>) -> bool {
    match (effective_cap_start(ctx), glyph) {
        (Some(cur), Some(g)) => cur.as_deref() == Some(g),
        (Some(None), None) => true,
        (Some(Some(_)), None) => false,
        _ => false,
    }
}

pub fn cap_end_is(ctx: &ConsoleContext, glyph: Option<&str>) -> bool {
    match (effective_cap_end(ctx), glyph) {
        (Some(cur), Some(g)) => cur.as_deref() == Some(g),
        (Some(None), None) => true,
        (Some(Some(_)), None) => false,
        _ => false,
    }
}

pub fn color_override_present(ctx: &ConsoleContext) -> bool {
    with_selected_edge(ctx, |edge| {
        edge.glyph_connection
            .as_ref()
            .and_then(|c| c.color.as_ref())
            .is_some()
    })
}

pub fn edge_has_label(ctx: &ConsoleContext) -> bool {
    with_selected_edge(ctx, |e| {
        e.label.as_deref().map_or(false, |s| !s.is_empty())
    })
}

pub fn edge_has_style_override(ctx: &ConsoleContext) -> bool {
    with_selected_edge(ctx, |e| e.glyph_connection.is_some())
}

pub fn edge_type_is(ctx: &ConsoleContext, t: &str) -> bool {
    with_selected_edge(ctx, |e| e.edge_type == t)
}

pub fn edge_conversion_would_duplicate(ctx: &ConsoleContext, new_type: &str) -> bool {
    let er = match selected_edge_ref(ctx) {
        Some(e) => e,
        None => return false,
    };
    let current_idx = match ctx.document.mindmap.edges.iter().position(|e| er.matches(e)) {
        Some(i) => i,
        None => return false,
    };
    let from_id = ctx.document.mindmap.edges[current_idx].from_id.clone();
    let to_id = ctx.document.mindmap.edges[current_idx].to_id.clone();
    ctx.document.mindmap.edges.iter().enumerate().any(|(i, e)| {
        i != current_idx
            && e.from_id == from_id
            && e.to_id == to_id
            && e.edge_type == new_type
    })
}

// ============================================================
// Portal-mode edge resolved-config queries
// ============================================================

/// True when the selected edge is a portal-mode edge and its
/// resolved `glyph_connection.body` equals `glyph`. Mirrors
/// `body_is` but targeted at the portal-mode subset so palette
/// entries can highlight the active marker preset.
pub fn portal_marker_is(ctx: &ConsoleContext, glyph: &str) -> bool {
    if !portal_edge_selected(ctx) {
        return false;
    }
    effective_body_glyph(ctx).map(|g| g == glyph).unwrap_or(false)
}
