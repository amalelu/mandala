// SPDX-License-Identifier: MPL-2.0

//! Applicability predicates: the "is this command relevant to the
//! current selection" checks.
//!
//! `help` filters the visible command list through them; completion
//! hides commands whose predicate returns false; and commands can
//! re-use them inside `execute` to short-circuit no-ops.
//!
//! They're kept in one place so the predicate vocabulary is scannable
//! — if you're adding a new command and need "edge is selected", the
//! helper already exists here.
//!
//! The vocabulary is deliberately *selection-shaped*. A second family
//! lived here once — `body_is`, `cap_end_is`, `effective_spacing` and
//! twenty of their neighbors, which resolved the selected edge's
//! `GlyphConnectionConfig` so a palette entry could render itself as
//! "already active". They outlived the palette that asked the
//! question and had no caller left; #41 removed them. A surface that
//! needs "what is this edge's effective X" should read
//! `GlyphConnectionConfig::resolved_for` directly rather than
//! reinstate one predicate per config field.

use super::ConsoleContext;
use crate::application::document::SelectionState;

// ============================================================
// Selection shape
// ============================================================

pub fn always(_: &ConsoleContext) -> bool {
    true
}

/// True when the selection has a node-shape (Single, Multi,
/// Section, SectionRange, or MultiSection). Used by the
/// `border` verb — border edits collapse to per-node-id
/// fan-out via `nodes_in_selection`, so all five shapes
/// dispatch cleanly.
///
/// The `section` verb uses the stricter sibling
/// [`node_or_section_selected_single_node`] which excludes
/// `Multi(_)` because section subverbs need a single-node
/// target.
pub fn node_or_section_selected(ctx: &ConsoleContext) -> bool {
    matches!(
        &ctx.document.selection,
        SelectionState::Single(_)
            | SelectionState::Multi(_)
            | SelectionState::Section(_)
            | SelectionState::SectionRange { .. }
            | SelectionState::MultiSection(_)
    )
}

/// Stricter sibling of [`node_or_section_selected`] for the
/// `section` verb specifically — admits the same selection
/// shapes EXCEPT `Multi(_)`. Every section subverb resolves to
/// one `(node, section_idx)` target (or fans across MultiSection
/// for `move dx=/dy=` and `add` for primary-node), and there's
/// no honest "fan section verbs across multiple nodes" semantics
/// — the user has to pick which node they want first.
///
/// `border` uses `node_or_section_selected` (admits `Multi`)
/// because border edits collapse to per-node-id fan-out via
/// `nodes_in_selection`. The two predicates diverge deliberately
/// to keep the section verb's UX in sync with its runtime
/// rejection of Multi.
pub fn node_or_section_selected_single_node(ctx: &ConsoleContext) -> bool {
    matches!(
        &ctx.document.selection,
        SelectionState::Single(_)
            | SelectionState::Section(_)
            | SelectionState::SectionRange { .. }
            | SelectionState::MultiSection(_)
    )
}

pub fn edge_selected(ctx: &ConsoleContext) -> bool {
    matches!(ctx.document.selection, SelectionState::Edge(_))
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

#[cfg(test)]
mod predicate_divergence_tests {
    use super::*;
    use crate::application::document::{GraphemeRange, SectionSel, SectionSpan, SelectionState};

    fn ctx_with_selection(sel: SelectionState) -> ConsoleContext<'static> {
        // Leak a fixture document — predicate tests just need
        // `ctx.document.selection`, no real model state.
        let doc = Box::leak(Box::new(
            crate::application::document::tests_common::load_test_doc(),
        ));
        doc.selection = sel;
        ConsoleContext::from_document(doc)
    }

    /// `border`'s predicate (`node_or_section_selected`) admits
    /// `Multi(_)` because border edits collapse to per-node
    /// fan-out via `nodes_in_selection`.
    #[test]
    fn border_predicate_admits_multi_selection() {
        let ctx = ctx_with_selection(SelectionState::Multi(vec!["a".into(), "b".into()]));
        assert!(node_or_section_selected(&ctx));
    }

    /// `section`'s stricter sibling rejects `Multi(_)` so the
    /// verb hides on multi-node selections in completion + help.
    /// Runtime would reject anyway (every section subverb needs
    /// a single-node target); pinning here keeps the UX in sync.
    #[test]
    fn section_predicate_rejects_multi_selection() {
        let ctx = ctx_with_selection(SelectionState::Multi(vec!["a".into(), "b".into()]));
        assert!(!node_or_section_selected_single_node(&ctx));
    }

    /// Both predicates admit `Single`, `Section`, `SectionRange`,
    /// `MultiSection`. Pin the parity to catch a future drift
    /// where one is widened without the other.
    #[test]
    fn predicates_agree_on_single_section_sectionrange_multisection() {
        let cases = [
            SelectionState::Single("a".into()),
            SelectionState::Section(SectionSel {
                node_id: "a".into(),
                section_idx: 0,
            }),
            SelectionState::SectionRange {
                sel: SectionSel {
                    node_id: "a".into(),
                    section_idx: 0,
                },
                section_span: SectionSpan::single(0),
                grapheme_range: GraphemeRange::new(0, 1),
            },
            SelectionState::MultiSection(vec![SectionSel {
                node_id: "a".into(),
                section_idx: 0,
            }]),
        ];
        for sel in cases {
            let ctx = ctx_with_selection(sel.clone());
            assert!(node_or_section_selected(&ctx), "border should admit {:?}", sel);
            assert!(
                node_or_section_selected_single_node(&ctx),
                "section should admit {:?}",
                sel
            );
        }
    }
}
