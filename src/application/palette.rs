//! Context-aware command palette for Mandala (Session 6C).
//!
//! Pressing `/` opens a glyph-rendered modal that lists actions
//! applicable to the current selection. Users type to fuzzy-filter
//! the list, Up/Down to navigate, Enter to execute. Built as an
//! alternative to burning a Ctrl-chord for every new action — the
//! palette is the long-tail discovery path, existing hotkeys stay.
//!
//! An action is a static `PaletteAction` carrying an applicability
//! predicate and an execute callback. Actions are declared as a
//! `const` slice so the registry is zero-cost at startup. All
//! execute callbacks mutate the document; the dispatcher in
//! `app.rs` handles cache invalidation + rebuild_all after.
//!
//! The session 6C surface ships eleven actions: reset connection to
//! straight, and two sets of five anchor-side actions (Auto / Top /
//! Right / Bottom / Left for both `from` and `to` anchors).

use crate::application::document::{EdgeRef, MindMapDocument, SelectionState};

/// A single command palette entry. Kept small and `'static` so the
/// whole registry can live in a const slice.
#[derive(Clone, Copy)]
pub struct PaletteAction {
    /// Stable identifier used by tests and potentially by a future
    /// keybind-to-action layer. Not shown to the user.
    pub id: &'static str,
    /// Display text shown in the filtered list.
    pub label: &'static str,
    /// One-line subtitle shown dim below the label. Also folded into
    /// the fuzzy-search haystack so users can match on description
    /// words.
    pub description: &'static str,
    /// Extra match tokens (e.g. "anchor", "side"). Folded into the
    /// fuzzy-search haystack so "anchor top" can match a
    /// "Set from-anchor: Top" action even though "anchor" isn't in
    /// the label.
    pub tags: &'static [&'static str],
    /// Returns `true` when the action should appear in the current
    /// selection context. Keeps the list focused: a straight edge
    /// never offers "Reset connection to straight".
    pub applicable: fn(&PaletteContext) -> bool,
    /// Mutates the document. The dispatcher in `app.rs` clears the
    /// scene cache and rebuilds after every successful execute.
    pub execute: fn(&mut PaletteEffects),
}

/// Read-only view of app state visible to the applicability
/// predicate. A thin wrapper so tests and the dispatcher can build
/// one cheaply.
pub struct PaletteContext<'a> {
    pub document: &'a MindMapDocument,
}

/// Mutable handles handed to `execute`. Kept intentionally small —
/// anything that needs a renderer or the active tree lives in the
/// dispatcher wrapper in `app.rs`, not inside actions.
pub struct PaletteEffects<'a> {
    pub document: &'a mut MindMapDocument,
    /// Session 6D: if an action wants to hand control to the inline
    /// label editor after running, it sets this to `Some(edge_ref)`.
    /// The dispatcher in `app.rs` drains the field after `execute`
    /// returns and opens the `LabelEditState` modal. Keeps actions
    /// pure-function: no renderer access, no modal state.
    pub open_label_edit: Option<EdgeRef>,
}

/// Case-insensitive subsequence fuzzy match. Returns `None` when any
/// char of `query` can't be matched in `haystack`, or `Some(score)`
/// otherwise. Higher scores are better; the scoring rewards dense
/// matches and penalises gaps + late first-match.
///
/// The algorithm is intentionally simple — with a 30-item palette
/// the perf floor is nanoseconds, so a real fuzzy library would be
/// over-engineered. The score heuristic is:
///
///     +10 per matched char
///     -1  per `haystack` char skipped before the first match
///     -1  per `haystack` char skipped between consecutive matches
///     +2  bonus per match immediately following a word boundary
pub fn fuzzy_score(query: &str, haystack: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let h: Vec<char> = haystack.chars().flat_map(|c| c.to_lowercase()).collect();
    let mut qi = 0usize;
    let mut score: i32 = 0;
    let mut first_match: Option<usize> = None;
    let mut prev_match: Option<usize> = None;
    for (hi, &hc) in h.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if hc == q[qi] {
            score += 10;
            if first_match.is_none() {
                first_match = Some(hi);
                score -= hi as i32;
            }
            if let Some(pm) = prev_match {
                let gap = (hi - pm - 1) as i32;
                score -= gap;
            }
            // Word-boundary bonus: previous char is a space / non-alnum.
            if hi == 0 || !h[hi - 1].is_alphanumeric() {
                score += 2;
            }
            prev_match = Some(hi);
            qi += 1;
        }
    }
    if qi == q.len() {
        Some(score)
    } else {
        None
    }
}

/// Build the fuzzy-search haystack string for an action. Joins the
/// label, description, and tags into a single space-separated blob
/// that `fuzzy_score` walks.
pub fn action_haystack(action: &PaletteAction) -> String {
    let mut s = String::with_capacity(64);
    s.push_str(action.label);
    s.push(' ');
    s.push_str(action.description);
    for t in action.tags {
        s.push(' ');
        s.push_str(t);
    }
    s
}

/// Filter the global action registry against the given query +
/// context. Returns indices into `PALETTE_ACTIONS`, sorted
/// descending by fuzzy score. Non-applicable actions are dropped.
/// An empty query returns every applicable action in registry
/// order.
pub fn filter_actions(query: &str, ctx: &PaletteContext) -> Vec<usize> {
    let mut scored: Vec<(usize, i32)> = Vec::new();
    for (idx, action) in PALETTE_ACTIONS.iter().enumerate() {
        if !(action.applicable)(ctx) {
            continue;
        }
        if query.is_empty() {
            scored.push((idx, 0));
            continue;
        }
        let hay = action_haystack(action);
        if let Some(score) = fuzzy_score(query, &hay) {
            scored.push((idx, score));
        }
    }
    if !query.is_empty() {
        scored.sort_by(|a, b| b.1.cmp(&a.1));
    }
    scored.into_iter().map(|(i, _)| i).collect()
}

// ============================================================
// Applicability predicates
// ============================================================

fn edge_selected(ctx: &PaletteContext) -> bool {
    matches!(ctx.document.selection, SelectionState::Edge(_))
}

fn edge_selected_with_control_points(ctx: &PaletteContext) -> bool {
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

/// Helper used by several Session 6D applicability predicates: run
/// `f` against the currently selected edge, returning `false` if the
/// selection is not an edge or the edge can't be found.
fn with_selected_edge<F>(ctx: &PaletteContext, f: F) -> bool
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

/// Return the effective body glyph of the selected edge, walking the
/// edge override → canvas default → hardcoded default chain. Used by
/// the body-glyph setter actions to self-hide when the current body
/// already matches.
fn effective_body_glyph(ctx: &PaletteContext) -> Option<String> {
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return None,
    };
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.body.clone())
}

fn effective_cap_start(ctx: &PaletteContext) -> Option<Option<String>> {
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return None,
    };
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.cap_start.clone())
}

fn effective_cap_end(ctx: &PaletteContext) -> Option<Option<String>> {
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return None,
    };
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.cap_end.clone())
}

fn effective_font_size_pt(ctx: &PaletteContext) -> Option<f32> {
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return None,
    };
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.font_size_pt)
}

fn effective_spacing(ctx: &PaletteContext) -> Option<f32> {
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return None,
    };
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge,
        &ctx.document.mindmap.canvas,
    );
    Some(resolved.spacing)
}

// ============================================================
// Execute callbacks
// ============================================================

fn exec_reset_edge_to_straight(eff: &mut PaletteEffects) {
    if let SelectionState::Edge(er) = eff.document.selection.clone() {
        eff.document.reset_edge_to_straight(&er);
    }
}

fn make_set_anchor_exec(is_from: bool, value: i32) -> fn(&mut PaletteEffects) {
    // Rust won't let us capture `is_from` / `value` into an `fn`
    // pointer, so we expand one concrete function per combination.
    // The ten combinations below are generated by hand — small
    // and static, so no macro is worth the indirection.
    match (is_from, value) {
        (true, 0) => exec_set_anchor_from_auto,
        (true, 1) => exec_set_anchor_from_top,
        (true, 2) => exec_set_anchor_from_right,
        (true, 3) => exec_set_anchor_from_bottom,
        (true, 4) => exec_set_anchor_from_left,
        (false, 0) => exec_set_anchor_to_auto,
        (false, 1) => exec_set_anchor_to_top,
        (false, 2) => exec_set_anchor_to_right,
        (false, 3) => exec_set_anchor_to_bottom,
        (false, 4) => exec_set_anchor_to_left,
        _ => exec_noop,
    }
}

fn exec_noop(_eff: &mut PaletteEffects) {}

macro_rules! def_set_anchor_exec {
    ($name:ident, $is_from:expr, $value:expr) => {
        fn $name(eff: &mut PaletteEffects) {
            if let SelectionState::Edge(er) = eff.document.selection.clone() {
                eff.document.set_edge_anchor(&er, $is_from, $value);
            }
        }
    };
}

def_set_anchor_exec!(exec_set_anchor_from_auto, true, 0);
def_set_anchor_exec!(exec_set_anchor_from_top, true, 1);
def_set_anchor_exec!(exec_set_anchor_from_right, true, 2);
def_set_anchor_exec!(exec_set_anchor_from_bottom, true, 3);
def_set_anchor_exec!(exec_set_anchor_from_left, true, 4);
def_set_anchor_exec!(exec_set_anchor_to_auto, false, 0);
def_set_anchor_exec!(exec_set_anchor_to_top, false, 1);
def_set_anchor_exec!(exec_set_anchor_to_right, false, 2);
def_set_anchor_exec!(exec_set_anchor_to_bottom, false, 3);
def_set_anchor_exec!(exec_set_anchor_to_left, false, 4);

// ============================================================
// Session 6D execute callbacks
// ============================================================

macro_rules! def_set_body_exec {
    ($name:ident, $glyph:expr) => {
        fn $name(eff: &mut PaletteEffects) {
            if let SelectionState::Edge(er) = eff.document.selection.clone() {
                eff.document.set_edge_body_glyph(&er, $glyph);
            }
        }
    };
}

def_set_body_exec!(exec_set_body_dot, "\u{00B7}"); // ·
def_set_body_exec!(exec_set_body_dash, "\u{2500}"); // ─
def_set_body_exec!(exec_set_body_double, "\u{2550}"); // ═
def_set_body_exec!(exec_set_body_wave, "\u{223C}"); // ∼
def_set_body_exec!(exec_set_body_chain, "\u{22EF}"); // ⋯

macro_rules! def_set_cap_start_exec {
    ($name:ident, $glyph:expr) => {
        fn $name(eff: &mut PaletteEffects) {
            if let SelectionState::Edge(er) = eff.document.selection.clone() {
                eff.document.set_edge_cap_start(&er, $glyph);
            }
        }
    };
}

def_set_cap_start_exec!(exec_set_cap_start_arrow, Some("\u{25C0}")); // ◀
def_set_cap_start_exec!(exec_set_cap_start_circle, Some("\u{25CF}")); // ●
def_set_cap_start_exec!(exec_set_cap_start_diamond, Some("\u{25C6}")); // ◆
def_set_cap_start_exec!(exec_set_cap_start_none, None);

macro_rules! def_set_cap_end_exec {
    ($name:ident, $glyph:expr) => {
        fn $name(eff: &mut PaletteEffects) {
            if let SelectionState::Edge(er) = eff.document.selection.clone() {
                eff.document.set_edge_cap_end(&er, $glyph);
            }
        }
    };
}

def_set_cap_end_exec!(exec_set_cap_end_arrow, Some("\u{25B6}")); // ▶
def_set_cap_end_exec!(exec_set_cap_end_circle, Some("\u{25CF}")); // ●
def_set_cap_end_exec!(exec_set_cap_end_diamond, Some("\u{25C6}")); // ◆
def_set_cap_end_exec!(exec_set_cap_end_none, None);

macro_rules! def_set_color_exec {
    ($name:ident, $color:expr) => {
        fn $name(eff: &mut PaletteEffects) {
            if let SelectionState::Edge(er) = eff.document.selection.clone() {
                eff.document.set_edge_color(&er, $color);
            }
        }
    };
}

// Theme-var-aware color presets. The resolver in `util/color.rs`
// expands `var(--name)` at scene-build time, so these automatically
// restyle when the user switches themes.
def_set_color_exec!(exec_color_accent, Some("var(--accent)"));
def_set_color_exec!(exec_color_edge, Some("var(--edge)"));
def_set_color_exec!(exec_color_fg, Some("var(--fg)"));
def_set_color_exec!(exec_color_reset, None);

fn exec_font_size_smaller(eff: &mut PaletteEffects) {
    if let SelectionState::Edge(er) = eff.document.selection.clone() {
        eff.document.set_edge_font_size_step(&er, -2.0);
    }
}

fn exec_font_size_larger(eff: &mut PaletteEffects) {
    if let SelectionState::Edge(er) = eff.document.selection.clone() {
        eff.document.set_edge_font_size_step(&er, 2.0);
    }
}

fn exec_font_size_reset(eff: &mut PaletteEffects) {
    if let SelectionState::Edge(er) = eff.document.selection.clone() {
        eff.document.reset_edge_font_size(&er);
    }
}

macro_rules! def_set_spacing_exec {
    ($name:ident, $value:expr) => {
        fn $name(eff: &mut PaletteEffects) {
            if let SelectionState::Edge(er) = eff.document.selection.clone() {
                eff.document.set_edge_spacing(&er, $value);
            }
        }
    };
}

def_set_spacing_exec!(exec_spacing_tight, 0.0);
def_set_spacing_exec!(exec_spacing_normal, 2.0);
def_set_spacing_exec!(exec_spacing_wide, 6.0);

macro_rules! def_set_edge_type_exec {
    ($name:ident, $type_str:expr) => {
        fn $name(eff: &mut PaletteEffects) {
            if let SelectionState::Edge(er) = eff.document.selection.clone() {
                eff.document.set_edge_type(&er, $type_str);
            }
        }
    };
}

def_set_edge_type_exec!(exec_convert_to_cross_link, "cross_link");
def_set_edge_type_exec!(exec_convert_to_parent_child, "parent_child");

fn exec_edit_label(eff: &mut PaletteEffects) {
    if let SelectionState::Edge(er) = eff.document.selection.clone() {
        eff.open_label_edit = Some(er);
    }
}

fn exec_clear_label(eff: &mut PaletteEffects) {
    if let SelectionState::Edge(er) = eff.document.selection.clone() {
        eff.document.set_edge_label(&er, None);
    }
}

macro_rules! def_label_position_exec {
    ($name:ident, $t:expr) => {
        fn $name(eff: &mut PaletteEffects) {
            if let SelectionState::Edge(er) = eff.document.selection.clone() {
                eff.document.set_edge_label_position(&er, $t);
            }
        }
    };
}

def_label_position_exec!(exec_label_position_start, 0.0);
def_label_position_exec!(exec_label_position_middle, 0.5);
def_label_position_exec!(exec_label_position_end, 1.0);

fn exec_reset_edge_style(eff: &mut PaletteEffects) {
    if let SelectionState::Edge(er) = eff.document.selection.clone() {
        eff.document.reset_edge_style_to_default(&er);
    }
}

// ============================================================
// Session 6D applicability predicates
// ============================================================

fn edge_selected_and_not_body(glyph: &'static str) -> impl Fn(&PaletteContext) -> bool {
    move |ctx: &PaletteContext| {
        effective_body_glyph(ctx)
            .map(|b| b != glyph)
            .unwrap_or(false)
    }
}

fn edge_selected_and_not_body_dot(ctx: &PaletteContext) -> bool {
    effective_body_glyph(ctx).map(|b| b != "\u{00B7}").unwrap_or(false)
}
fn edge_selected_and_not_body_dash(ctx: &PaletteContext) -> bool {
    effective_body_glyph(ctx).map(|b| b != "\u{2500}").unwrap_or(false)
}
fn edge_selected_and_not_body_double(ctx: &PaletteContext) -> bool {
    effective_body_glyph(ctx).map(|b| b != "\u{2550}").unwrap_or(false)
}
fn edge_selected_and_not_body_wave(ctx: &PaletteContext) -> bool {
    effective_body_glyph(ctx).map(|b| b != "\u{223C}").unwrap_or(false)
}
fn edge_selected_and_not_body_chain(ctx: &PaletteContext) -> bool {
    effective_body_glyph(ctx).map(|b| b != "\u{22EF}").unwrap_or(false)
}

fn cap_start_not_arrow(ctx: &PaletteContext) -> bool {
    effective_cap_start(ctx).map(|c| c.as_deref() != Some("\u{25C0}")).unwrap_or(false)
}
fn cap_start_not_circle(ctx: &PaletteContext) -> bool {
    effective_cap_start(ctx).map(|c| c.as_deref() != Some("\u{25CF}")).unwrap_or(false)
}
fn cap_start_not_diamond(ctx: &PaletteContext) -> bool {
    effective_cap_start(ctx).map(|c| c.as_deref() != Some("\u{25C6}")).unwrap_or(false)
}
fn cap_start_not_none(ctx: &PaletteContext) -> bool {
    effective_cap_start(ctx).map(|c| c.is_some()).unwrap_or(false)
}

fn cap_end_not_arrow(ctx: &PaletteContext) -> bool {
    effective_cap_end(ctx).map(|c| c.as_deref() != Some("\u{25B6}")).unwrap_or(false)
}
fn cap_end_not_circle(ctx: &PaletteContext) -> bool {
    effective_cap_end(ctx).map(|c| c.as_deref() != Some("\u{25CF}")).unwrap_or(false)
}
fn cap_end_not_diamond(ctx: &PaletteContext) -> bool {
    effective_cap_end(ctx).map(|c| c.as_deref() != Some("\u{25C6}")).unwrap_or(false)
}
fn cap_end_not_none(ctx: &PaletteContext) -> bool {
    effective_cap_end(ctx).map(|c| c.is_some()).unwrap_or(false)
}

fn color_override_present(ctx: &PaletteContext) -> bool {
    with_selected_edge(ctx, |edge| {
        edge.glyph_connection.as_ref().and_then(|c| c.color.as_ref()).is_some()
    })
}

fn font_size_not_at_min(ctx: &PaletteContext) -> bool {
    match (effective_font_size_pt(ctx), ctx_selected_edge_min_font(ctx)) {
        (Some(cur), Some(min)) => cur > min + 0.5,
        _ => false,
    }
}

fn font_size_not_at_max(ctx: &PaletteContext) -> bool {
    match (effective_font_size_pt(ctx), ctx_selected_edge_max_font(ctx)) {
        (Some(cur), Some(max)) => cur < max - 0.5,
        _ => false,
    }
}

fn font_size_not_default(ctx: &PaletteContext) -> bool {
    let default = baumhard::mindmap::model::GlyphConnectionConfig::default().font_size_pt;
    effective_font_size_pt(ctx).map(|s| (s - default).abs() > 0.5).unwrap_or(false)
}

fn ctx_selected_edge_min_font(ctx: &PaletteContext) -> Option<f32> {
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return None,
    };
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge, &ctx.document.mindmap.canvas,
    );
    Some(resolved.min_font_size_pt)
}

fn ctx_selected_edge_max_font(ctx: &PaletteContext) -> Option<f32> {
    let er = match &ctx.document.selection {
        SelectionState::Edge(e) => e,
        _ => return None,
    };
    let edge = ctx.document.mindmap.edges.iter().find(|e| er.matches(e))?;
    let resolved = baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(
        edge, &ctx.document.mindmap.canvas,
    );
    Some(resolved.max_font_size_pt)
}

fn spacing_not(ctx: &PaletteContext, target: f32) -> bool {
    effective_spacing(ctx).map(|s| (s - target).abs() > f32::EPSILON).unwrap_or(false)
}
fn spacing_not_tight(ctx: &PaletteContext) -> bool { spacing_not(ctx, 0.0) }
fn spacing_not_normal(ctx: &PaletteContext) -> bool { spacing_not(ctx, 2.0) }
fn spacing_not_wide(ctx: &PaletteContext) -> bool { spacing_not(ctx, 6.0) }

fn edge_type_not_cross_link(ctx: &PaletteContext) -> bool {
    with_selected_edge(ctx, |e| e.edge_type != "cross_link")
        && !edge_conversion_would_duplicate(ctx, "cross_link")
}

fn edge_type_not_parent_child(ctx: &PaletteContext) -> bool {
    with_selected_edge(ctx, |e| e.edge_type != "parent_child")
        && !edge_conversion_would_duplicate(ctx, "parent_child")
}

fn edge_conversion_would_duplicate(ctx: &PaletteContext, new_type: &str) -> bool {
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

fn edge_has_label(ctx: &PaletteContext) -> bool {
    with_selected_edge(ctx, |e| e.label.as_deref().map_or(false, |s| !s.is_empty()))
}

fn label_position_not(ctx: &PaletteContext, target: f32) -> bool {
    with_selected_edge(ctx, |e| {
        let cur = e.label_position_t.unwrap_or(0.5);
        (cur - target).abs() > f32::EPSILON && e.label.as_deref().map_or(false, |s| !s.is_empty())
    })
}
fn label_position_not_start(ctx: &PaletteContext) -> bool { label_position_not(ctx, 0.0) }
fn label_position_not_middle(ctx: &PaletteContext) -> bool { label_position_not(ctx, 0.5) }
fn label_position_not_end(ctx: &PaletteContext) -> bool { label_position_not(ctx, 1.0) }

fn edge_has_style_override(ctx: &PaletteContext) -> bool {
    with_selected_edge(ctx, |e| e.glyph_connection.is_some())
}

/// Silence `#[allow(dead_code)]` chatter on the higher-order
/// `edge_selected_and_not_body` helper — it's exposed in case a
/// future session wants to add more body-glyph presets without
/// hand-rolling a predicate each time. Tied down with a trivial
/// no-op reference so rustc sees the symbol.
#[allow(dead_code)]
fn _touch_body_helper() {
    let _ = edge_selected_and_not_body("·");
}

// ============================================================
// The global action registry
// ============================================================

/// Session 6C action set. Eleven entries: reset-to-straight and ten
/// anchor setters. Grown as later sessions land.
pub const PALETTE_ACTIONS: &[PaletteAction] = &[
    PaletteAction {
        id: "reset_edge_to_straight",
        label: "Reset connection to straight",
        description: "Remove all control points from the selected edge",
        tags: &["edge", "connection", "straight", "clear"],
        applicable: edge_selected_with_control_points,
        execute: exec_reset_edge_to_straight,
    },
    PaletteAction {
        id: "edge_set_anchor_from_auto",
        label: "Set from-anchor: Auto",
        description: "Let the edge pick its own attachment side at the source",
        tags: &["edge", "connection", "anchor", "side", "auto"],
        applicable: edge_selected,
        execute: exec_set_anchor_from_auto,
    },
    PaletteAction {
        id: "edge_set_anchor_from_top",
        label: "Set from-anchor: Top",
        description: "Attach the source of the edge to the top of its node",
        tags: &["edge", "connection", "anchor", "side", "top"],
        applicable: edge_selected,
        execute: exec_set_anchor_from_top,
    },
    PaletteAction {
        id: "edge_set_anchor_from_right",
        label: "Set from-anchor: Right",
        description: "Attach the source of the edge to the right of its node",
        tags: &["edge", "connection", "anchor", "side", "right"],
        applicable: edge_selected,
        execute: exec_set_anchor_from_right,
    },
    PaletteAction {
        id: "edge_set_anchor_from_bottom",
        label: "Set from-anchor: Bottom",
        description: "Attach the source of the edge to the bottom of its node",
        tags: &["edge", "connection", "anchor", "side", "bottom"],
        applicable: edge_selected,
        execute: exec_set_anchor_from_bottom,
    },
    PaletteAction {
        id: "edge_set_anchor_from_left",
        label: "Set from-anchor: Left",
        description: "Attach the source of the edge to the left of its node",
        tags: &["edge", "connection", "anchor", "side", "left"],
        applicable: edge_selected,
        execute: exec_set_anchor_from_left,
    },
    PaletteAction {
        id: "edge_set_anchor_to_auto",
        label: "Set to-anchor: Auto",
        description: "Let the edge pick its own attachment side at the target",
        tags: &["edge", "connection", "anchor", "side", "auto"],
        applicable: edge_selected,
        execute: exec_set_anchor_to_auto,
    },
    PaletteAction {
        id: "edge_set_anchor_to_top",
        label: "Set to-anchor: Top",
        description: "Attach the target of the edge to the top of its node",
        tags: &["edge", "connection", "anchor", "side", "top"],
        applicable: edge_selected,
        execute: exec_set_anchor_to_top,
    },
    PaletteAction {
        id: "edge_set_anchor_to_right",
        label: "Set to-anchor: Right",
        description: "Attach the target of the edge to the right of its node",
        tags: &["edge", "connection", "anchor", "side", "right"],
        applicable: edge_selected,
        execute: exec_set_anchor_to_right,
    },
    PaletteAction {
        id: "edge_set_anchor_to_bottom",
        label: "Set to-anchor: Bottom",
        description: "Attach the target of the edge to the bottom of its node",
        tags: &["edge", "connection", "anchor", "side", "bottom"],
        applicable: edge_selected,
        execute: exec_set_anchor_to_bottom,
    },
    PaletteAction {
        id: "edge_set_anchor_to_left",
        label: "Set to-anchor: Left",
        description: "Attach the target of the edge to the left of its node",
        tags: &["edge", "connection", "anchor", "side", "left"],
        applicable: edge_selected,
        execute: exec_set_anchor_to_left,
    },
    // ====================================================================
    // Session 6D — connection style and label actions
    // ====================================================================
    // Body glyph presets
    PaletteAction {
        id: "edge_set_body_dot",
        label: "Set body glyph: dot (·)",
        description: "Repeat a middle dot along the connection path",
        tags: &["edge", "connection", "body", "glyph", "dot", "style"],
        applicable: edge_selected_and_not_body_dot,
        execute: exec_set_body_dot,
    },
    PaletteAction {
        id: "edge_set_body_dash",
        label: "Set body glyph: dash (─)",
        description: "Repeat a horizontal light dash along the connection path",
        tags: &["edge", "connection", "body", "glyph", "dash", "line", "style"],
        applicable: edge_selected_and_not_body_dash,
        execute: exec_set_body_dash,
    },
    PaletteAction {
        id: "edge_set_body_double",
        label: "Set body glyph: double (═)",
        description: "Repeat a horizontal double line along the connection path",
        tags: &["edge", "connection", "body", "glyph", "double", "line", "style"],
        applicable: edge_selected_and_not_body_double,
        execute: exec_set_body_double,
    },
    PaletteAction {
        id: "edge_set_body_wave",
        label: "Set body glyph: wave (∼)",
        description: "Repeat a tilde wave along the connection path",
        tags: &["edge", "connection", "body", "glyph", "wave", "tilde", "style"],
        applicable: edge_selected_and_not_body_wave,
        execute: exec_set_body_wave,
    },
    PaletteAction {
        id: "edge_set_body_chain",
        label: "Set body glyph: chain (⋯)",
        description: "Repeat a mid-line ellipsis along the connection path",
        tags: &["edge", "connection", "body", "glyph", "chain", "dots", "style"],
        applicable: edge_selected_and_not_body_chain,
        execute: exec_set_body_chain,
    },
    // Cap start presets
    PaletteAction {
        id: "edge_set_cap_start_arrow",
        label: "Set from-cap: arrow (◀)",
        description: "Place a left-pointing triangle at the source anchor",
        tags: &["edge", "connection", "cap", "start", "from", "arrow"],
        applicable: cap_start_not_arrow,
        execute: exec_set_cap_start_arrow,
    },
    PaletteAction {
        id: "edge_set_cap_start_circle",
        label: "Set from-cap: circle (●)",
        description: "Place a filled circle at the source anchor",
        tags: &["edge", "connection", "cap", "start", "from", "circle"],
        applicable: cap_start_not_circle,
        execute: exec_set_cap_start_circle,
    },
    PaletteAction {
        id: "edge_set_cap_start_diamond",
        label: "Set from-cap: diamond (◆)",
        description: "Place a filled diamond at the source anchor",
        tags: &["edge", "connection", "cap", "start", "from", "diamond"],
        applicable: cap_start_not_diamond,
        execute: exec_set_cap_start_diamond,
    },
    PaletteAction {
        id: "edge_set_cap_start_none",
        label: "Clear from-cap",
        description: "Remove the source-anchor cap glyph",
        tags: &["edge", "connection", "cap", "start", "from", "clear", "none"],
        applicable: cap_start_not_none,
        execute: exec_set_cap_start_none,
    },
    // Cap end presets
    PaletteAction {
        id: "edge_set_cap_end_arrow",
        label: "Set to-cap: arrow (▶)",
        description: "Place a right-pointing triangle at the target anchor",
        tags: &["edge", "connection", "cap", "end", "to", "arrow"],
        applicable: cap_end_not_arrow,
        execute: exec_set_cap_end_arrow,
    },
    PaletteAction {
        id: "edge_set_cap_end_circle",
        label: "Set to-cap: circle (●)",
        description: "Place a filled circle at the target anchor",
        tags: &["edge", "connection", "cap", "end", "to", "circle"],
        applicable: cap_end_not_circle,
        execute: exec_set_cap_end_circle,
    },
    PaletteAction {
        id: "edge_set_cap_end_diamond",
        label: "Set to-cap: diamond (◆)",
        description: "Place a filled diamond at the target anchor",
        tags: &["edge", "connection", "cap", "end", "to", "diamond"],
        applicable: cap_end_not_diamond,
        execute: exec_set_cap_end_diamond,
    },
    PaletteAction {
        id: "edge_set_cap_end_none",
        label: "Clear to-cap",
        description: "Remove the target-anchor cap glyph",
        tags: &["edge", "connection", "cap", "end", "to", "clear", "none"],
        applicable: cap_end_not_none,
        execute: exec_set_cap_end_none,
    },
    // Color presets (theme-var-aware)
    PaletteAction {
        id: "edge_color_accent",
        label: "Use color: accent",
        description: "Theme the connection with var(--accent)",
        tags: &["edge", "connection", "color", "accent", "theme"],
        applicable: edge_selected,
        execute: exec_color_accent,
    },
    PaletteAction {
        id: "edge_color_edge",
        label: "Use color: edge",
        description: "Theme the connection with var(--edge)",
        tags: &["edge", "connection", "color", "edge", "theme"],
        applicable: edge_selected,
        execute: exec_color_edge,
    },
    PaletteAction {
        id: "edge_color_fg",
        label: "Use color: foreground",
        description: "Theme the connection with var(--fg)",
        tags: &["edge", "connection", "color", "foreground", "fg", "theme"],
        applicable: edge_selected,
        execute: exec_color_fg,
    },
    PaletteAction {
        id: "edge_color_reset",
        label: "Reset color (inherit edge color)",
        description: "Clear the glyph-connection color override",
        tags: &["edge", "connection", "color", "reset", "inherit"],
        applicable: color_override_present,
        execute: exec_color_reset,
    },
    // Font size
    PaletteAction {
        id: "edge_font_size_smaller",
        label: "Smaller connection glyphs",
        description: "Shrink the connection glyph font size by 2pt",
        tags: &["edge", "connection", "font", "size", "smaller", "shrink"],
        applicable: font_size_not_at_min,
        execute: exec_font_size_smaller,
    },
    PaletteAction {
        id: "edge_font_size_larger",
        label: "Larger connection glyphs",
        description: "Grow the connection glyph font size by 2pt",
        tags: &["edge", "connection", "font", "size", "larger", "grow"],
        applicable: font_size_not_at_max,
        execute: exec_font_size_larger,
    },
    PaletteAction {
        id: "edge_font_size_reset",
        label: "Reset connection font size",
        description: "Restore the connection glyph font size to the default (12pt)",
        tags: &["edge", "connection", "font", "size", "reset", "default"],
        applicable: font_size_not_default,
        execute: exec_font_size_reset,
    },
    // Spacing
    PaletteAction {
        id: "edge_spacing_tight",
        label: "Spacing: tight",
        description: "Pack connection glyphs flush together",
        tags: &["edge", "connection", "spacing", "tight", "dense"],
        applicable: spacing_not_tight,
        execute: exec_spacing_tight,
    },
    PaletteAction {
        id: "edge_spacing_normal",
        label: "Spacing: normal",
        description: "Moderate gap between connection glyphs",
        tags: &["edge", "connection", "spacing", "normal"],
        applicable: spacing_not_normal,
        execute: exec_spacing_normal,
    },
    PaletteAction {
        id: "edge_spacing_wide",
        label: "Spacing: wide",
        description: "Airy gap between connection glyphs",
        tags: &["edge", "connection", "spacing", "wide", "airy"],
        applicable: spacing_not_wide,
        execute: exec_spacing_wide,
    },
    // Edge type
    PaletteAction {
        id: "edge_convert_to_cross_link",
        label: "Convert to cross-link",
        description: "Change the edge type to cross_link",
        tags: &["edge", "connection", "type", "cross_link", "convert"],
        applicable: edge_type_not_cross_link,
        execute: exec_convert_to_cross_link,
    },
    PaletteAction {
        id: "edge_convert_to_parent_child",
        label: "Convert to parent-child",
        description: "Change the edge type to parent_child",
        tags: &["edge", "connection", "type", "parent_child", "convert", "hierarchy"],
        applicable: edge_type_not_parent_child,
        execute: exec_convert_to_parent_child,
    },
    // Label editing
    PaletteAction {
        id: "edge_edit_label",
        label: "Edit connection label",
        description: "Open the inline label editor on the selected edge",
        tags: &["edge", "connection", "label", "edit", "text"],
        applicable: edge_selected,
        execute: exec_edit_label,
    },
    PaletteAction {
        id: "edge_clear_label",
        label: "Clear connection label",
        description: "Remove the label from the selected edge",
        tags: &["edge", "connection", "label", "clear", "delete"],
        applicable: edge_has_label,
        execute: exec_clear_label,
    },
    // Label position
    PaletteAction {
        id: "edge_label_position_start",
        label: "Position label at start",
        description: "Move the label to the from-anchor end of the path",
        tags: &["edge", "connection", "label", "position", "start", "from"],
        applicable: label_position_not_start,
        execute: exec_label_position_start,
    },
    PaletteAction {
        id: "edge_label_position_middle",
        label: "Position label at middle",
        description: "Move the label to the middle of the path",
        tags: &["edge", "connection", "label", "position", "middle"],
        applicable: label_position_not_middle,
        execute: exec_label_position_middle,
    },
    PaletteAction {
        id: "edge_label_position_end",
        label: "Position label at end",
        description: "Move the label to the to-anchor end of the path",
        tags: &["edge", "connection", "label", "position", "end", "to"],
        applicable: label_position_not_end,
        execute: exec_label_position_end,
    },
    // Reset style
    PaletteAction {
        id: "edge_reset_style",
        label: "Reset connection style to default",
        description: "Clear the per-edge glyph override, falling back to the canvas default",
        tags: &["edge", "connection", "reset", "default", "style"],
        applicable: edge_has_style_override,
        execute: exec_reset_edge_style,
    },
];

/// Look up an action by its stable id. Returns `None` if no action
/// has that id. Primarily used by tests to exercise known actions
/// without relying on registry order.
pub fn action_by_id(id: &str) -> Option<(usize, &'static PaletteAction)> {
    PALETTE_ACTIONS
        .iter()
        .enumerate()
        .find(|(_, a)| a.id == id)
}

// make_set_anchor_exec is public-in-module for potential future use
// (e.g. declarative keybind bindings). Suppress the dead-code warning
// when nothing in this crate calls it yet.
#[allow(dead_code)]
fn _touch_make_set_anchor_exec() {
    let _ = make_set_anchor_exec;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_score_empty_query_returns_zero() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn fuzzy_score_subsequence_match() {
        assert!(fuzzy_score("rst", "reset").is_some());
        assert!(fuzzy_score("rst", "reset connection to straight").is_some());
    }

    #[test]
    fn fuzzy_score_missing_char_none() {
        assert_eq!(fuzzy_score("xyz", "reset connection"), None);
    }

    #[test]
    fn fuzzy_score_case_insensitive() {
        assert!(fuzzy_score("TOP", "set from-anchor: top").is_some());
        assert!(fuzzy_score("top", "SET FROM-ANCHOR: TOP").is_some());
    }

    #[test]
    fn fuzzy_score_prefers_earlier_match() {
        let early = fuzzy_score("top", "top of list").unwrap();
        let late = fuzzy_score("top", "this is near the top").unwrap();
        assert!(early > late, "early={early} late={late}");
    }

    #[test]
    fn fuzzy_score_word_boundary_bonus() {
        // "anchor" matches at a word boundary in "set anchor" but
        // not in a word-internal position; both should score but
        // the boundary one should win.
        let a = fuzzy_score("anchor", "set anchor side").unwrap();
        let b = fuzzy_score("anchor", "setanchorside").unwrap();
        assert!(a > b, "a={a} b={b}");
    }

    #[test]
    fn action_haystack_includes_tags() {
        let action = PaletteAction {
            id: "test",
            label: "Label",
            description: "Desc",
            tags: &["tagone", "tagtwo"],
            applicable: |_| true,
            execute: |_| {},
        };
        let hay = action_haystack(&action);
        assert!(hay.contains("Label"));
        assert!(hay.contains("Desc"));
        assert!(hay.contains("tagone"));
        assert!(hay.contains("tagtwo"));
    }

    #[test]
    fn action_by_id_finds_reset() {
        let hit = action_by_id("reset_edge_to_straight");
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().1.id, "reset_edge_to_straight");
    }

    #[test]
    fn action_by_id_finds_set_anchor_from_top() {
        assert!(action_by_id("edge_set_anchor_from_top").is_some());
    }

    #[test]
    fn action_by_id_unknown_returns_none() {
        assert!(action_by_id("nope").is_none());
    }

    #[test]
    fn palette_actions_session_6c_entries_present() {
        // Session 6C shipped 11 entries; Session 6D added 31 more.
        assert!(
            PALETTE_ACTIONS.len() >= 11,
            "expected at least 11 (session 6C minimum), got {}",
            PALETTE_ACTIONS.len()
        );
    }

    #[test]
    fn palette_actions_session_6d_count_matches_plan() {
        // Session 6D target: 11 (carried over from 6C) + 31 new = 42.
        // Tight assertion so adding/removing actions shows up in CI.
        assert_eq!(
            PALETTE_ACTIONS.len(),
            42,
            "expected 42 palette actions (11 from 6C + 31 new in 6D)"
        );
    }

    #[test]
    fn palette_actions_session_6d_ids_all_resolve() {
        let ids: &[&str] = &[
            "edge_set_body_dot",
            "edge_set_body_dash",
            "edge_set_body_double",
            "edge_set_body_wave",
            "edge_set_body_chain",
            "edge_set_cap_start_arrow",
            "edge_set_cap_start_circle",
            "edge_set_cap_start_diamond",
            "edge_set_cap_start_none",
            "edge_set_cap_end_arrow",
            "edge_set_cap_end_circle",
            "edge_set_cap_end_diamond",
            "edge_set_cap_end_none",
            "edge_color_accent",
            "edge_color_edge",
            "edge_color_fg",
            "edge_color_reset",
            "edge_font_size_smaller",
            "edge_font_size_larger",
            "edge_font_size_reset",
            "edge_spacing_tight",
            "edge_spacing_normal",
            "edge_spacing_wide",
            "edge_convert_to_cross_link",
            "edge_convert_to_parent_child",
            "edge_edit_label",
            "edge_clear_label",
            "edge_label_position_start",
            "edge_label_position_middle",
            "edge_label_position_end",
            "edge_reset_style",
        ];
        assert_eq!(ids.len(), 31, "expected 31 new 6D action ids");
        for id in ids {
            assert!(action_by_id(id).is_some(), "action id '{id}' not registered");
        }
    }
}
