// SPDX-License-Identifier: MPL-2.0

//! Section-target resolution — the single cascade that turns a
//! selection (plus an optional `section=K` kv) into the section
//! index a per-section operation writes to.
//!
//! Three call sites need this answer and each used to carry its
//! own copy of the rule table: the `section …` verb family
//! (`super::execute_section`), the `section frame …` sibling
//! (`super::frame`), and the keybind / macro `Action` arms in
//! `crate::application::app::dispatch::cross_dispatch::style`.
//! The copies drifted at least once (review-fix CRIT-2: the
//! frame resolver was missing the single-section auto-resolve),
//! which is exactly what `CODE_CONVENTIONS.md` §5 forbids. The
//! cascade now lives here once and the three legitimate
//! divergences are named by [`SectionTargetPolicy`].

use crate::application::console::parser::Args;
use crate::application::document::SelectionState;

/// The rule-table variant a resolution follows. Every policy walks
/// the same cascade — explicit `section=K` kv, then a live
/// `Section` / `SectionRange` selection naming the node, then the
/// single-section auto-resolve — and differs only in how a
/// `MultiSection` selection is treated and in the wording of the
/// resulting errors.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SectionTargetPolicy {
    /// `section move|resize|show|text|edit|delete|split …`.
    /// Single-target by construction: the node comes from
    /// [`SelectionState::primary_node_id`], so a `MultiSection`
    /// selection is rejected outright with the "pass
    /// `section=<idx>`" hint. (`section move dx=… dy=…` fans out
    /// over multiple sections, but that arm resolves its targets
    /// itself before reaching this resolver.)
    Verb,
    /// `section frame …`. The caller fans out over the
    /// deduplicated owning nodes of the selection, so `node_id`
    /// is one of possibly several candidates and a `MultiSection`
    /// selection falls through to the single-section
    /// auto-resolve rather than being rejected up front.
    Frame,
    /// Keybind / macro `Action` arms. These have no
    /// `MindMapDocument` in hand when they resolve, so they pass
    /// `section_count = None` and the single-section
    /// auto-resolve is unavailable — a bare `Single(id)` cannot
    /// be resolved and is rejected. A `MultiSection` holding
    /// exactly one entry still resolves (no ambiguity); two or
    /// more is rejected rather than silently collapsed to the
    /// first entry.
    Action,
}

impl SectionTargetPolicy {
    /// Error-message prefix — the verb the user typed, or the
    /// dispatch surface the rejection came from.
    fn label(self) -> &'static str {
        match self {
            SectionTargetPolicy::Verb => "section",
            SectionTargetPolicy::Frame => "section frame",
            SectionTargetPolicy::Action => "section Action",
        }
    }
}

/// Read the optional `section=K` kv off `args`. `verb` prefixes
/// the parse error so `section=abc` reports which verb rejected
/// it. Returns `Ok(None)` when the kv is absent.
///
/// Single source for what used to be two byte-identical private
/// `parse_section_kv` helpers (one in `section/mod.rs`, one in
/// `section/frame.rs`).
pub(crate) fn parse_section_target_kv(args: &Args, verb: &str) -> Result<Option<usize>, String> {
    for (k, v) in args.kvs() {
        if k == "section" {
            return super::super::range_kv::parse_section_kv(verb, v).map(Some);
        }
    }
    Ok(None)
}

/// Resolve which section of `node_id` an operation targets.
///
/// `section_count` is the node's section count when the caller can
/// see the document (`Some`), or `None` on the `Action` path where
/// only the selection is in hand. `kv_idx` is the parsed
/// `section=K` kv, if any — it is *not* bounds-checked here; the
/// caller validates the resolved index against the live node so an
/// explicit out-of-range `section=99` reports "not found" rather
/// than a resolution failure.
pub(crate) fn resolve_section_index(
    selection: &SelectionState,
    node_id: &str,
    kv_idx: Option<usize>,
    section_count: Option<usize>,
    policy: SectionTargetPolicy,
) -> Result<usize, String> {
    // 1. An explicit `section=K` always wins — the user named the
    //    target, whatever the selection says.
    if let Some(idx) = kv_idx {
        return Ok(idx);
    }
    // 2. A live `Section` / `SectionRange` selection on this very
    //    node names the target directly.
    if let Some(sel) = selection.selected_section() {
        if sel.node_id == node_id {
            return Ok(sel.section_idx);
        }
    }
    // 3. `MultiSection` — the one axis the three policies disagree
    //    on, so the divergence is spelled out here instead of being
    //    re-derived per call site.
    if let SelectionState::MultiSection(secs) = selection {
        match policy {
            SectionTargetPolicy::Verb => {
                return Err(multi_section_single_target_error(policy.label()));
            }
            SectionTargetPolicy::Action => match secs.as_slice() {
                // Exactly one section selected is unambiguous —
                // `MultiSection` upholds `len() >= 2` through
                // `SelectionState::from_sections`, so this arm is
                // defensive rather than routine.
                [only] if only.node_id == node_id => return Ok(only.section_idx),
                // Two or more is a real multi-target selection.
                // Collapsing it to the first entry would throw the
                // user's signal away silently — the macro / keybind
                // path is single-target, so it says so instead.
                [_, _, ..] => return Err(action_multi_section_error(secs.len())),
                _ => {}
            },
            SectionTargetPolicy::Frame => {}
        }
    }
    // 4. A single-section node has exactly one answer, so the user
    //    doesn't have to spell it out.
    if section_count == Some(1) {
        return Ok(0);
    }
    // 5. Genuinely ambiguous.
    Err(match section_count {
        Some(n) => format!(
            "{}: node '{}' has {} sections — pick one (click) or pass section=<idx>",
            policy.label(),
            node_id,
            n
        ),
        None => format!(
            "{}: no section selected — click a section (or pick one with \
             the `section` verb's section=<idx> kv) before firing a \
             section-targeted binding",
            policy.label()
        ),
    })
}

/// Resolve `(node_id, section_idx)` for the keybind / macro
/// `Action` arms, which carry no `section=K` kv and no document.
/// `None` means "nothing to act on"; the reason is logged so a
/// silent no-op never reaches the user unexplained.
pub(crate) fn resolve_action_section_target(selection: &SelectionState) -> Option<(String, usize)> {
    let node_id = match selection.primary_node_id() {
        Some(id) => id.to_string(),
        // `MultiSection` has no primary node — take the first
        // entry's owner and let the resolver decide whether the
        // set is unambiguous for that node.
        None => match selection {
            SelectionState::MultiSection(secs) => secs.first()?.node_id.clone(),
            _ => return None,
        },
    };
    match resolve_section_index(selection, &node_id, None, None, SectionTargetPolicy::Action) {
        Ok(idx) => Some((node_id, idx)),
        Err(msg) => {
            log::warn!("{}", msg);
            None
        }
    }
}

/// The `section …` verb family's rejection for a `MultiSection`
/// selection. Shared by the resolver and by the node-id resolver
/// in `section/mod.rs` so both halves of a `section resize` on a
/// multi-section selection report the same sentence.
pub(crate) fn multi_section_single_target_error(label: &str) -> String {
    format!(
        "{}: multi-section selection — single-target only; pass \
         section=<idx> or click one section first",
        label
    )
}

/// The `Action` path's rejection for a real multi-target
/// `MultiSection` selection. Pre-dedup this silently collapsed to
/// the first entry, losing the user's multi-target signal.
fn action_multi_section_error(len: usize) -> String {
    format!(
        "section Action: MultiSection({} entries) rejected; \
         macro/keybind path is single-target — script multiple \
         Action steps for fan-out, or use the console verb \
         `section move dx=… dy=…` (delta form fans out)",
        len
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::{GraphemeRange, SectionSel, SectionSpan};

    fn section_sel(node_id: &str, section_idx: usize) -> SectionSel {
        SectionSel {
            node_id: node_id.to_string(),
            section_idx,
        }
    }

    /// Rule 1 — an explicit `section=K` wins over every other
    /// signal, on every policy.
    #[test]
    fn resolve_section_index_kv_wins_over_selection() {
        let sel = SelectionState::Section(section_sel("n", 2));
        for policy in [
            SectionTargetPolicy::Verb,
            SectionTargetPolicy::Frame,
            SectionTargetPolicy::Action,
        ] {
            assert_eq!(
                resolve_section_index(&sel, "n", Some(7), Some(3), policy),
                Ok(7),
                "policy {:?} must honor an explicit section= kv",
                policy
            );
        }
    }

    /// Rule 2 — a `Section` / `SectionRange` selection naming the
    /// node resolves to its index; one naming a *different* node
    /// does not.
    #[test]
    fn resolve_section_index_selection_must_name_the_node() {
        let sel = SelectionState::Section(section_sel("a", 1));
        assert_eq!(
            resolve_section_index(&sel, "a", None, Some(4), SectionTargetPolicy::Frame),
            Ok(1)
        );
        // A different node with 4 sections is ambiguous.
        assert!(resolve_section_index(&sel, "b", None, Some(4), SectionTargetPolicy::Frame).is_err());
        // …unless that node happens to have exactly one section.
        assert_eq!(
            resolve_section_index(&sel, "b", None, Some(1), SectionTargetPolicy::Frame),
            Ok(0)
        );
    }

    /// Rule 4 — the single-section auto-resolve (CRIT-2's fix)
    /// applies to both document-aware policies.
    #[test]
    fn resolve_section_index_single_section_auto_resolves() {
        let sel = SelectionState::Single("n".into());
        assert_eq!(
            resolve_section_index(&sel, "n", None, Some(1), SectionTargetPolicy::Verb),
            Ok(0)
        );
        assert_eq!(
            resolve_section_index(&sel, "n", None, Some(1), SectionTargetPolicy::Frame),
            Ok(0)
        );
    }

    /// A multi-section node with a bare `Single` selection is
    /// ambiguous; the error carries the count and the kv hint.
    #[test]
    fn resolve_section_index_multi_section_node_reports_count_and_hint() {
        let sel = SelectionState::Single("n".into());
        let err = resolve_section_index(&sel, "n", None, Some(3), SectionTargetPolicy::Verb)
            .expect_err("3-section node with no section selected is ambiguous");
        assert!(err.contains("has 3 sections"), "missing count: {}", err);
        assert!(err.contains("section=<idx>"), "missing hint: {}", err);
    }

    /// The `Action` policy has no section count, so a bare
    /// `Single` cannot resolve — the auto-resolve needs the
    /// document the Action path doesn't hold.
    #[test]
    fn resolve_section_index_action_rejects_single() {
        let sel = SelectionState::Single("n".into());
        assert!(resolve_section_index(&sel, "n", None, None, SectionTargetPolicy::Action).is_err());
        assert_eq!(resolve_action_section_target(&sel), None);
    }

    /// `MultiSection` diverges by policy: rejected outright for
    /// the `section …` verb, transparent for `section frame …`
    /// (which fans out over owning nodes), collapsed when
    /// unambiguous for the Action path.
    #[test]
    fn resolve_section_index_multi_section_diverges_by_policy() {
        let sel = SelectionState::MultiSection(vec![section_sel("a", 0), section_sel("b", 3)]);
        let verb_err = resolve_section_index(&sel, "a", None, Some(2), SectionTargetPolicy::Verb)
            .expect_err("verb policy rejects MultiSection");
        assert!(verb_err.contains("single-target only"), "{}", verb_err);
        // Frame falls through to the count-based rules.
        assert_eq!(
            resolve_section_index(&sel, "a", None, Some(1), SectionTargetPolicy::Frame),
            Ok(0)
        );
        // Action rejects a genuine multi-target selection rather
        // than silently picking the first entry — even when the
        // entries name different nodes.
        assert_eq!(resolve_action_section_target(&sel), None);
    }

    /// Two entries on the *same* node is ambiguous too — the
    /// Action path must not silently pick the first.
    #[test]
    fn resolve_action_section_target_rejects_two_entries_on_one_node() {
        let sel = SelectionState::MultiSection(vec![section_sel("a", 0), section_sel("a", 1)]);
        assert_eq!(resolve_action_section_target(&sel), None);
    }

    /// A `MultiSection` of exactly one entry is unambiguous.
    #[test]
    fn resolve_action_section_target_collapses_single_entry() {
        let sel = SelectionState::MultiSection(vec![section_sel("n", 3)]);
        assert_eq!(resolve_action_section_target(&sel), Some(("n".into(), 3)));
    }

    /// `SectionRange` resolves through the same rule 2 as
    /// `Section` — both surface via `selected_section()`.
    #[test]
    fn resolve_action_section_target_handles_section_range() {
        let sel = SelectionState::SectionRange {
            sel: section_sel("n", 2),
            section_span: SectionSpan::single(2),
            grapheme_range: GraphemeRange::new(0, 1),
        };
        assert_eq!(resolve_action_section_target(&sel), Some(("n".into(), 2)));
    }

    /// Non-section-bearing selections resolve to nothing on the
    /// Action path.
    #[test]
    fn resolve_action_section_target_rejects_non_section_selections() {
        assert_eq!(resolve_action_section_target(&SelectionState::None), None);
        assert_eq!(
            resolve_action_section_target(&SelectionState::Multi(vec!["a".into(), "b".into()])),
            None
        );
    }
}
