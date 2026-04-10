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

use crate::application::document::{MindMapDocument, SelectionState};

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
    fn palette_actions_have_all_eleven_session_6c_entries() {
        assert_eq!(PALETTE_ACTIONS.len(), 11);
    }
}
