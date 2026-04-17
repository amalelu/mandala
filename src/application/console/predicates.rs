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
use crate::application::document::SelectionState;

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

/// True when the current selection is an edge with
/// `display_mode = "portal"`. Used by the console to scope
/// portal-only actions (like `edge portal` create) and filter
/// completion.
pub fn portal_edge_selected(ctx: &ConsoleContext) -> bool {
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return false,
    };
    ctx.document
        .mindmap
        .edges
        .iter()
        .find(|e| er.matches(e))
        .map(baumhard::mindmap::model::is_portal_edge)
        .unwrap_or(false)
}

pub fn edge_selected_or_two_nodes(ctx: &ConsoleContext) -> bool {
    edge_selected(ctx) || two_nodes_selected(ctx)
}

/// `color pick` is applicable for both edges and portals — each
/// branch hands off to the appropriate `ColorTarget`.
pub fn edge_selected_with_control_points(ctx: &ConsoleContext) -> bool {
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return false,
    };
    ctx.document
        .mindmap
        .edges
        .iter()
        .find(|e| er.matches(e))
        .map(|e| !e.control_points.is_empty())
        .unwrap_or(false)
}

// ============================================================
// Edge resolved-config queries
// ============================================================

fn with_selected_edge<F>(ctx: &ConsoleContext, f: F) -> bool
where
    F: FnOnce(&baumhard::mindmap::model::MindEdge) -> bool,
{
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return false,
    };
    ctx.document
        .mindmap
        .edges
        .iter()
        .find(|e| er.matches(e))
        .map(f)
        .unwrap_or(false)
}

fn effective_body_glyph(ctx: &ConsoleContext) -> Option<String> {
    let er = ctx.document.selection.selected_edge()?;
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.body.clone())
}

fn effective_cap_start(ctx: &ConsoleContext) -> Option<Option<String>> {
    let er = ctx.document.selection.selected_edge()?;
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.cap_start.clone())
}

fn effective_cap_end(ctx: &ConsoleContext) -> Option<Option<String>> {
    let er = ctx.document.selection.selected_edge()?;
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.cap_end.clone())
}

pub fn effective_font_size_pt(ctx: &ConsoleContext) -> Option<f32> {
    let er = ctx.document.selection.selected_edge()?;
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.font_size_pt)
}

pub fn selected_edge_min_font(ctx: &ConsoleContext) -> Option<f32> {
    let er = ctx.document.selection.selected_edge()?;
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.min_font_size_pt)
}

pub fn selected_edge_max_font(ctx: &ConsoleContext) -> Option<f32> {
    let er = ctx.document.selection.selected_edge()?;
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.max_font_size_pt)
}

pub fn effective_spacing(ctx: &ConsoleContext) -> Option<f32> {
    let er = ctx.document.selection.selected_edge()?;
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.spacing)
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
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return false,
    };
    let current_idx = match ctx.document.mindmap.edges.iter().position(|e| er.matches(e)) {
        Some(i) => i,
        None => return false,
    };
    let from_id = &ctx.document.mindmap.edges[current_idx].from_id;
    let to_id = &ctx.document.mindmap.edges[current_idx].to_id;
    ctx.document.mindmap.edges.iter().enumerate().any(|(i, e)| {
        i != current_idx
            && &e.from_id == from_id
            && &e.to_id == to_id
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
