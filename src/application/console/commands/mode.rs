// SPDX-License-Identifier: MPL-2.0

//! `mode` — query and change the active high-level interaction mode.
//!
//! Verbs:
//! - `mode default` — exit any active mode and return to `Default`.
//!   Equivalent to pressing the `ExitMode` keybind (Esc by default).
//! - `mode resize` — enter Resize mode targeting the current selection.
//!   Equivalent to `Action::EnterResizeMode` (`r` by default).
//! - `mode node-edit` — enter NodeEdit mode targeting the selection's
//!   primary node. Mirrors `Action::EnterNodeEdit` but does **not**
//!   trigger the single-section short-circuit (the verb path is
//!   "set mode" only — for the legacy "Enter on a node opens the
//!   editor" UX, use the keybind).
//!
//! `mode section-edit` is intentionally absent: the user-named
//! verb for "open the section editor" is `section edit` (with
//! `[section=<idx>]`), not `mode section-edit` — the bus
//! carries `OpenSectionEdit { node_id, section_idx }` and the
//! section verb is the only producer. Authors who think in
//! "modes" should reach for `section edit` to drop into the
//! editor; `mode default` exits any mode including the editor
//! via the post-rebuild close-target logic.
//!
//! `mode show` is intentionally absent until a later batch plumbs
//! the active mode through `ConsoleEffects` (same §5 reasoning —
//! no surface, no verb). Reparent / Connect transitions remain
//! keybind-only (`Ctrl+P` / `Ctrl+D`).
//!
//! See `SECTIONS_BORDERS_RESIZE_PLAN.md` §3.9 for the full target
//! grammar.

use super::Command;
#[cfg(test)]
use crate::application::app::ResizeTarget;
use crate::application::app::{resolve_resize_target, InteractionMode, ResizeTargetError};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::{kvs, usage, Grammar, Subverb};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ExecResult};
use crate::application::document::SelectionState;

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("default", "modes", "leave the current mode and return to Default"),
    Subverb::bare("resize", "modes", "enter Resize on the current selection"),
    Subverb::bare(
        "node-edit",
        "modes",
        "enter NodeEdit on the selection's owning node",
    ),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "mode",
    subverb_sets: &[SUBVERBS],
    key_sets: &[],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "mode",
    aliases: &[],
    summary: "Change the active interaction mode (default / resize / node-edit)",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["interaction", "exit", "edit"],
    execute: execute_mode,
};

fn execute_mode(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Subverb names are matched case-insensitively by the descent,
    // console-wide (`commands/mod.rs` § Casing); the unknown-subverb
    // message still echoes what the user typed.
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    let name = match descent.stop {
        Stop::Matched(subverb) => subverb.name,
        Stop::Bare => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => {
            return ExecResult::err(usage::unknown_subverb_message(
                descent.level,
                descent.typed.unwrap_or_default(),
            ))
        }
    };
    match name {
        "default" => {
            eff.side_effect = Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Default));
            eff.close_console = true;
            ExecResult::ok_msg("mode: returning to Default")
        }
        "resize" => match resolve_resize_target(&eff.document().selection, &eff.document().mindmap) {
            Ok(target) => {
                eff.side_effect = Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Resize {
                    target,
                }));
                eff.close_console = true;
                ExecResult::ok_msg("entering resize mode")
            }
            Err(e) => ExecResult::err(format_resize_error(&e)),
        },
        _ => match resolve_node_edit_target(&eff.document().selection) {
            Ok(node_id) => {
                eff.side_effect = Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::NodeEdit {
                    node_id: node_id.clone(),
                }));
                eff.close_console = true;
                ExecResult::ok_msg(format!("entering node-edit mode on '{}'", node_id))
            }
            Err(msg) => ExecResult::err(msg),
        },
    }
}

/// Resolve the selection's primary node id for `mode node-edit`.
/// Mirrors [`SelectionState::primary_node_id`]'s semantics — Single,
/// Section, and SectionRange all yield the owning node id; Multi /
/// MultiSection / Edge / None error with verb-specific messaging.
fn resolve_node_edit_target(selection: &SelectionState) -> Result<String, String> {
    match selection.primary_node_id() {
        Some(id) => Ok(id.to_string()),
        None => match selection {
            SelectionState::None => Err("mode node-edit: no selection; click a node first".into()),
            SelectionState::Multi(_) | SelectionState::MultiSection(_) => {
                Err("mode node-edit: multi-target selection — single-target only".into())
            }
            _ => Err("mode node-edit: selection is not a node — try clicking on a node body".into()),
        },
    }
}

/// Format a [`ResizeTargetError`] (returned by the shared
/// [`crate::application::app::resolve_resize_target`] resolver) as a
/// user-facing console message prefixed with the verb name. The
/// underlying error type is the same one the dispatcher arm
/// (`apply_enter_resize_mode`) consumes, with `log::warn!` lines
/// rather than console output — keeping the resolver one source of
/// truth while letting each consumer phrase its own user surface.
fn format_resize_error(e: &ResizeTargetError) -> String {
    match e {
        ResizeTargetError::NoSelection => "mode resize: no selection; click a node or section first".into(),
        ResizeTargetError::MultiTarget => "mode resize: multi-target selection — single-target only".into(),
        ResizeTargetError::SectionFillParent { node_id, section_idx } => format!(
            "mode resize: section {}[{}] fills its parent — no AABB to stretch. \
             Pin a size first via `section resize w=<w> h=<h>`",
            node_id, section_idx,
        ),
        ResizeTargetError::EdgeOrPortal => {
            "mode resize: edge / label / portal selection — not resizable".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::parse;
    use crate::application::console::parser::ParseResult;
    use crate::application::console::Args;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::{
        EdgeLabelSel, EdgeRef, GraphemeRange, PortalLabelSel, SectionSel, SectionSpan, SelectionState,
    };
    use baumhard::mindmap::scene_cache::EdgeKey;

    /// Parse `line` and run the `mode` verb body against `doc`,
    /// returning the result and the side-effect that the dispatcher
    /// would consume. Internal callers pass the doc separately
    /// rather than borrowing it inside the helper because
    /// `ConsoleEffects` borrows the doc for its lifetime; capturing
    /// the side effect requires extracting it before the borrow
    /// ends.
    fn run_mode(
        line: &str,
        doc: &mut crate::application::document::MindMapDocument,
    ) -> (ExecResult, Option<ConsoleSideEffect>, bool) {
        let tokens = match parse(line) {
            ParseResult::Ok { cmd: _, args } => args,
            ParseResult::Empty => panic!("empty input: {:?}", line),
            ParseResult::Unknown(s) => panic!("unknown command '{}' in {:?}", s, line),
        };
        let mut eff = ConsoleEffects::new(doc);
        let result = execute_mode(&Args::new(&tokens), &mut eff);
        let close = eff.close_console;
        let side = eff.side_effect.take();
        (result, side, close)
    }

    fn assert_err_contains(r: &ExecResult, needle: &str) {
        match r {
            ExecResult::Err(msg) => assert!(msg.contains(needle), "msg = {}", msg),
            _ => panic!("expected Err containing {:?}, got non-Err", needle),
        }
    }

    #[test]
    fn test_mode_default_emits_set_default_side_effect() {
        let mut doc = load_test_doc();
        let (result, side, close) = run_mode("mode default", &mut doc);
        assert!(matches!(result, ExecResult::Ok { .. }));
        assert!(matches!(
            side,
            Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Default))
        ));
        assert!(close);
    }

    #[test]
    fn test_mode_resize_with_single_node_selection_targets_node() {
        let mut doc = load_test_doc();
        let id = doc
            .mindmap
            .nodes
            .keys()
            .next()
            .expect("test doc has nodes")
            .clone();
        doc.selection = SelectionState::Single(id.clone());
        let (result, side, close) = run_mode("mode resize", &mut doc);
        assert!(matches!(result, ExecResult::Ok { .. }));
        // Successful Resize-mode entry closes the console so the
        // anchors are visible without the user closing it manually.
        assert!(close, "successful mode resize must set close_console=true");
        match side {
            Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Resize {
                target: ResizeTarget::Node(target_id),
            })) => assert_eq!(target_id, id),
            _ => panic!("expected SetInteractionMode(Resize::Node)"),
        }
    }

    /// `SectionRange` shares the resolver path with `Section` —
    /// pin that the verb routes both to the same Section target.
    #[test]
    fn test_mode_resize_with_section_range_selection_targets_section() {
        use crate::application::document::tests_common::pinned_two_section_node;
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::SectionRange {
            sel: SectionSel {
                node_id: id.clone(),
                section_idx: 1,
            },
            section_span: SectionSpan::single(1),
            grapheme_range: GraphemeRange::new(0, 1),
        };
        let (result, side, _close) = run_mode("mode resize", &mut doc);
        assert!(matches!(result, ExecResult::Ok { .. }));
        match side {
            Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Resize {
                target:
                    ResizeTarget::Section {
                        node_id: tid,
                        section_idx,
                    },
            })) => {
                assert_eq!(tid, id);
                assert_eq!(section_idx, 1);
            }
            _ => panic!("expected Resize::Section for SectionRange selection"),
        }
    }

    #[test]
    fn test_mode_resize_with_no_selection_errors() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::None;
        let (result, side, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "no selection");
        // No mode flip on error.
        assert!(side.is_none());
    }

    #[test]
    fn test_mode_resize_with_multi_selection_errors() {
        let mut doc = load_test_doc();
        let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
        assert_eq!(ids.len(), 2, "test doc must have at least 2 nodes");
        doc.selection = SelectionState::Multi(ids);
        let (result, _, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "single-target only");
    }

    /// MultiSection (≥ 2 sections selected) — Resize is single-target.
    #[test]
    fn test_mode_resize_with_multi_section_selection_errors() {
        use crate::application::document::tests_common::pinned_two_section_node;
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::MultiSection(vec![
            SectionSel {
                node_id: id.clone(),
                section_idx: 0,
            },
            SectionSel {
                node_id: id,
                section_idx: 1,
            },
        ]);
        let (result, _, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "single-target only");
    }

    #[test]
    fn test_mode_resize_with_fill_parent_section_errors() {
        let mut doc = load_test_doc();
        let nid = doc.mindmap.nodes.keys().next().expect("nodes").clone();
        // Force section 0 to fill-parent (size = None).
        if let Some(node) = doc.mindmap.nodes.get_mut(&nid) {
            if let Some(s) = node.sections.first_mut() {
                s.size = None;
            }
        }
        doc.selection = SelectionState::Section(SectionSel {
            node_id: nid,
            section_idx: 0,
        });
        let (result, _, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "fills its parent");
    }

    /// Edge selection — not a resizable surface.
    #[test]
    fn test_mode_resize_with_edge_selection_errors() {
        let mut doc = load_test_doc();
        let er = doc
            .mindmap
            .edges
            .first()
            .map(|e| EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type))
            .expect("test doc has edges");
        doc.selection = SelectionState::Edge(er);
        let (result, _, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "edge / label / portal");
    }

    /// EdgeLabel selection — same not-resizable arm as `Edge`.
    #[test]
    fn test_mode_resize_with_edge_label_selection_errors() {
        let mut doc = load_test_doc();
        let er = doc
            .mindmap
            .edges
            .first()
            .map(|e| EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type))
            .expect("test doc has edges");
        doc.selection = SelectionState::EdgeLabel(EdgeLabelSel::new(er));
        let (result, _, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "edge / label / portal");
    }

    /// PortalLabel selection — same not-resizable arm. The resolver
    /// doesn't read the model for this arm so a synthetic key
    /// matching no real edge is fine.
    #[test]
    fn test_mode_resize_with_portal_label_selection_errors() {
        let mut doc = load_test_doc();
        let sel = PortalLabelSel {
            edge_key: EdgeKey::new("0", "1", "cross_link"),
            endpoint_node_id: "1".into(),
        };
        doc.selection = SelectionState::PortalLabel(sel);
        let (result, _, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "edge / label / portal");
    }

    /// PortalText selection — same not-resizable arm.
    #[test]
    fn test_mode_resize_with_portal_text_selection_errors() {
        let mut doc = load_test_doc();
        let sel = PortalLabelSel {
            edge_key: EdgeKey::new("0", "1", "cross_link"),
            endpoint_node_id: "1".into(),
        };
        doc.selection = SelectionState::PortalText(sel);
        let (result, _, _) = run_mode("mode resize", &mut doc);
        assert_err_contains(&result, "edge / label / portal");
    }

    #[test]
    fn test_mode_unknown_subverb_errors() {
        let mut doc = load_test_doc();
        let (result, _, _) = run_mode("mode wibble", &mut doc);
        assert_err_contains(&result, "unknown subverb");
        if let ExecResult::Err(msg) = &result {
            assert!(msg.contains("'wibble'"), "msg = {}", msg);
        }
    }

    #[test]
    fn test_mode_no_subverb_emits_usage() {
        let mut doc = load_test_doc();
        let (result, _, _) = run_mode("mode", &mut doc);
        assert_err_contains(&result, "usage:");
    }

    /// `mode node-edit` with a Single selection emits a NodeEdit
    /// `SetInteractionMode` side effect carrying the selected node's id.
    #[test]
    fn test_mode_node_edit_with_single_node_emits_node_edit_side_effect() {
        let mut doc = load_test_doc();
        let id = doc
            .mindmap
            .nodes
            .keys()
            .next()
            .expect("test doc has nodes")
            .clone();
        doc.selection = SelectionState::Single(id.clone());
        let (result, side, close) = run_mode("mode node-edit", &mut doc);
        assert!(matches!(result, ExecResult::Ok { .. }));
        assert!(close, "successful mode node-edit must close the console");
        match side {
            Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::NodeEdit { node_id })) => {
                assert_eq!(node_id, id);
            }
            _ => panic!("expected SetInteractionMode(NodeEdit)"),
        }
    }

    /// `mode node-edit` with a Section selection routes to the
    /// owning node — the selection's `primary_node_id` is the
    /// canonical resolution.
    #[test]
    fn test_mode_node_edit_with_section_routes_to_owner_node() {
        use crate::application::document::tests_common::pinned_two_section_node;
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel::new(&id, 1));
        let (result, side, _close) = run_mode("mode node-edit", &mut doc);
        assert!(matches!(result, ExecResult::Ok { .. }));
        match side {
            Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::NodeEdit { node_id })) => {
                assert_eq!(node_id, id);
            }
            _ => panic!("expected SetInteractionMode(NodeEdit) on Section selection"),
        }
    }

    #[test]
    fn test_mode_node_edit_with_no_selection_errors() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::None;
        let (result, side, _) = run_mode("mode node-edit", &mut doc);
        assert_err_contains(&result, "no selection");
        assert!(side.is_none());
    }

    #[test]
    fn test_mode_node_edit_with_multi_selection_errors() {
        let mut doc = load_test_doc();
        let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
        assert_eq!(ids.len(), 2);
        doc.selection = SelectionState::Multi(ids);
        let (result, _, _) = run_mode("mode node-edit", &mut doc);
        assert_err_contains(&result, "single-target only");
    }

    #[test]
    fn test_mode_node_edit_with_edge_selection_errors() {
        let mut doc = load_test_doc();
        let er = doc
            .mindmap
            .edges
            .first()
            .map(|e| EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type))
            .expect("test doc has edges");
        doc.selection = SelectionState::Edge(er);
        let (result, _, _) = run_mode("mode node-edit", &mut doc);
        assert_err_contains(&result, "not a node");
    }
}
