// SPDX-License-Identifier: MPL-2.0

//! Style-affecting apply_* helpers — border / color / font /
//! spacing edits applied to the current selection. Each delegates
//! to a `console::commands::*::apply_*_to_selection` mutation
//! core, then routes through the shared `apply_with_rebuild`
//! envelope so a `true` from the core triggers a geometry-change
//! scene rebuild.

use super::{apply_with_rebuild, RebuildContext};
use crate::application::document::MindMapDocument;

/// Set a named border field (e.g. `top` / `bottom` / `corner`)
/// to a parsed `value` across the current selection. Border-glyph
/// change → full rebuild via `apply_with_rebuild`.
pub(in crate::application::app) fn apply_set_border_field(
    field: &str,
    value: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::border::apply_border_field_to_selection(doc, field, value)
    });
}

/// `Action::CycleBorderPreset` — advance the selected node(s)'
/// border preset to the next entry in `BORDER_PRESETS`. Thin
/// wrapper over the `border preset cycle` mutation core, so the
/// keybind and the verb can never disagree about what "next"
/// means or about which preset the sample starts from.
pub(in crate::application::app) fn apply_cycle_border_preset(rc: &mut RebuildContext<'_>) {
    apply_with_rebuild(
        rc,
        crate::application::console::commands::border::cycle_border_preset_on_selection,
    );
}

/// `Action::ToggleBorderVisible` — flip `style.show_frame` per
/// selected node, each toggled independently. Thin wrapper over
/// the `border toggle` mutation core; the verb formats the same
/// report as scrollback, this arm only needs "did anything
/// change".
pub(in crate::application::app) fn apply_toggle_border_visible(rc: &mut RebuildContext<'_>) {
    apply_with_rebuild(rc, |doc| {
        match crate::application::console::commands::border::toggle_border_visible_on_selection(doc) {
            Ok(report) => report.toggled > 0,
            Err(_) => {
                log::warn!("ToggleBorderVisible: no border-applicable selection");
                false
            }
        }
    });
}

/// Stage a single-kv border preview against the live selection.
/// `target_kind` discriminates between
/// `node` / `section` / `canvas-border` / `canvas-sf` /
/// `canvas-sf-focused`. Always returns `true` for the rebuild
/// envelope (the scene needs to re-emit with the previewed
/// edits visible). Unknown `target_kind` is a no-op + warn.
pub(in crate::application::app) fn apply_set_border_preview(
    target_kind: crate::application::keybinds::BorderPreviewTargetKind,
    field: &str,
    value: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        use crate::application::console::commands::border::stage_kv;
        use crate::application::document::{BorderConfigEdits, BorderPreviewTarget};
        use crate::application::keybinds::BorderPreviewTargetKind;

        let mut edits = BorderConfigEdits::default();
        if let Err(msg) = stage_kv(&mut edits, field, value) {
            log::warn!("border: SetBorderPreview stage_kv error: {msg}");
            return false;
        }

        // Resolve `target_kind` → `BorderPreviewTarget`. The
        // selection-bound variants (`Node`, `Section`) walk the
        // live selection via the same resolvers the verb path
        // uses; canvas variants are constants. Typed-enum
        // dispatch — pre-fix this was a stringly-typed match
        // with `unknown` arms warning at runtime.
        let target = match target_kind {
            BorderPreviewTargetKind::Node => {
                let ids = match crate::application::console::commands::border::nodes_in_selection(
                    &doc.selection,
                    "border preview",
                ) {
                    Ok(ids) => ids,
                    Err(_) => return false,
                };
                BorderPreviewTarget::Nodes(ids)
            }
            BorderPreviewTargetKind::Section => {
                let pairs: Vec<(String, usize)> = match &doc.selection {
                    crate::application::document::SelectionState::Section(s) => {
                        vec![(s.node_id.clone(), s.section_idx)]
                    }
                    crate::application::document::SelectionState::SectionRange { sel, range } => {
                        let (lo, hi) = (range.0.min(range.1), range.0.max(range.1));
                        (lo..=hi).map(|i| (sel.node_id.clone(), i)).collect()
                    }
                    crate::application::document::SelectionState::MultiSection(sels) => {
                        sels.iter().map(|s| (s.node_id.clone(), s.section_idx)).collect()
                    }
                    _ => return false,
                };
                BorderPreviewTarget::Sections(pairs)
            }
            BorderPreviewTargetKind::CanvasBorder => BorderPreviewTarget::CanvasDefault,
            BorderPreviewTargetKind::CanvasSf => BorderPreviewTarget::CanvasSectionFrame,
            BorderPreviewTargetKind::CanvasSfFocused => BorderPreviewTarget::CanvasSectionFrameFocused,
        };
        let _ = doc.set_border_preview(target, edits);
        true
    });
}

/// Commit the active border preview through the matching
/// committing setter and clear the slot. No-op + no rebuild
/// when no preview is active.
pub(in crate::application::app) fn apply_commit_border_preview(rc: &mut RebuildContext<'_>) {
    apply_with_rebuild(rc, |doc| doc.commit_border_preview().is_some());
}

/// Cancel the active border preview without writing the model.
/// Returns `true` (rebuild) when a preview was actually cleared
/// — the scene needs to re-emit without the staged edits.
pub(in crate::application::app) fn apply_cancel_border_preview(rc: &mut RebuildContext<'_>) {
    apply_with_rebuild(rc, |doc| doc.cancel_border_preview());
}

/// Set a single color axis (`bg` / `frame` / `text` / `title`
/// — see [`crate::application::keybinds::ColorAxis`]) to a
/// hex / palette / `var(--name)` value across the current
/// selection. Visual change → full rebuild.
pub(in crate::application::app) fn apply_set_color_axis(
    axis: crate::application::keybinds::ColorAxis,
    value: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::color::apply_color_axis_to_selection(doc, axis.into(), value)
    });
}

/// Pin the font family on every node / edge / portal in the
/// current selection. The `family` is validated against the
/// loaded-families table by the mutation core; an unknown name
/// surfaces as a `false` from the helper and the rebuild is
/// skipped. Geometry change (per-glyph advance shifts) → full
/// rebuild.
pub(in crate::application::app) fn apply_set_font_family(family: &str, rc: &mut RebuildContext<'_>) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::font::apply_font_family_to_selection(doc, family)
    });
}

/// Set one font-size kv (`size` / `min` / `max` — see
/// [`crate::application::keybinds::FontSlot`]) across the
/// current selection. `pt` is already-parsed (the dispatcher's
/// caller is responsible for parsing the user-facing `String`
/// payload — invalid floats emit a warn-log and skip the
/// helper call entirely). Geometry change → full rebuild.
pub(in crate::application::app) fn apply_set_font_kv(
    slot: crate::application::keybinds::FontSlot,
    pt: f32,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::font::apply_font_kv_to_selection(doc, slot.into(), pt)
    });
}

/// Set per-glyph connection-path spacing (the gap between sample
/// glyphs along an edge body) across the current selection from
/// the unparsed `input` string. Mutation core handles parsing +
/// the same is_positive_finite gating every spacing-accepting
/// verb uses. Geometry change → full rebuild.
pub(in crate::application::app) fn apply_set_spacing(input: &str, rc: &mut RebuildContext<'_>) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::spacing::apply_spacing_to_selection(doc, input)
    });
}

/// Resolve the `(node_id, section_idx)` the section-targeted
/// `Action` variants apply to.
///
/// One-line wrapper over the shared cascade in
/// [`crate::application::console::commands::section::target`]
/// under its `Action` policy, which pins the two divergences from
/// the console verb path:
///
/// 1. **`Single(id)` is rejected here** even on a 1-section node,
///    where the verb path auto-resolves to `(id, 0)`. That rule
///    needs a `MindMapDocument` to count sections and this
///    dispatch surface has none in hand at resolve time; callers
///    that want the auto-resolve switch the selection to
///    `Section { node_id, section_idx: 0 }` before firing.
/// 2. **`MultiSection` naming the node more than once is
///    rejected**, mirroring the verb path's "single-target only"
///    posture for every subverb except `move dx/dy` (whose
///    fan-out lives at the verb layer in
///    `execute_move_fan_out_multisection`). Pre-fix this silently
///    collapsed to the first entry, losing the user's
///    multi-section signal entirely.
fn target_section(sel: &crate::application::document::SelectionState) -> Option<(String, usize)> {
    crate::application::console::commands::section::target::resolve_action_section_target(sel)
}

/// Nudge the selected section by `(dx, dy)` canvas units.
/// AABB rejection on overflow surfaces as a `log::warn!` and
/// no-op (the model setter returns `Err`). Mirror of
/// `section move dx=<dx> dy=<dy>` for the keybind / macro
/// path.
pub(in crate::application::app) fn apply_set_section_offset_delta(
    dx: f64,
    dy: f64,
    rc: &mut RebuildContext<'_>,
) {
    let Some((node_id, idx)) = target_section(&rc.document.selection) else {
        log::warn!("SetSectionOffsetDelta: no section selected");
        return;
    };
    let (cx, cy) = match rc
        .document
        .mindmap
        .nodes
        .get(&node_id)
        .and_then(|n| n.sections.get(idx))
        .map(|s| (s.offset.x, s.offset.y))
    {
        Some(p) => p,
        None => {
            log::warn!(
                "SetSectionOffsetDelta: section[{}] not found on node '{}'",
                idx,
                node_id
            );
            return;
        }
    };
    apply_with_rebuild(rc, |doc| {
        match doc.set_section_offset(&node_id, idx, cx + dx, cy + dy) {
            Ok(changed) => changed,
            Err(msg) => {
                log::warn!("SetSectionOffsetDelta: {}", msg);
                false
            }
        }
    });
}

/// Pin the selected section's size to `(w, h)` (or fill-parent
/// when `size = None`). Mirror of `section resize w=<w> h=<h>`
/// / `section resize fill` for the keybind / macro path.
pub(in crate::application::app) fn apply_set_section_size(
    size: Option<baumhard::mindmap::model::Size>,
    rc: &mut RebuildContext<'_>,
) {
    let Some((node_id, idx)) = target_section(&rc.document.selection) else {
        log::warn!("SetSectionSize: no section selected");
        return;
    };
    if !section_exists(&rc.document, &node_id, idx) {
        log::warn!("SetSectionSize: section[{}] not found on node '{}'", idx, node_id);
        return;
    }
    apply_with_rebuild(rc, |doc| match doc.set_section_size(&node_id, idx, size) {
        Ok(changed) => changed,
        Err(msg) => {
            log::warn!("SetSectionSize: {}", msg);
            false
        }
    });
}

/// Pin the selected section's `offset` to `(x, y)` (absolute,
/// not delta). Mirror of `section move x=<x> y=<y>` for the
/// macro path.`Action::SetSectionOffsetAbs`.
pub(in crate::application::app) fn apply_set_section_offset_abs(x: f64, y: f64, rc: &mut RebuildContext<'_>) {
    let Some((node_id, idx)) = target_section(&rc.document.selection) else {
        log::warn!("SetSectionOffsetAbs: no section selected");
        return;
    };
    let exists = rc
        .document
        .mindmap
        .nodes
        .get(&node_id)
        .map(|n| n.sections.get(idx).is_some())
        .unwrap_or(false);
    if !exists {
        log::warn!(
            "SetSectionOffsetAbs: section[{}] not found on node '{}'",
            idx,
            node_id
        );
        return;
    }
    apply_with_rebuild(rc, |doc| match doc.set_section_offset(&node_id, idx, x, y) {
        Ok(changed) => changed,
        Err(msg) => {
            log::warn!("SetSectionOffsetAbs: {}", msg);
            false
        }
    });
}

/// Replace the selected section's text. `clear_runs == true`
/// drops every prior run and lays down a single template-cloned
/// run; `false` (the default for `runs=preserve`) clips existing
/// runs to the new text length. Mirror of `section text "<text>"
/// [runs=preserve|clear]` for the macro path./// `Action::SetSectionText`. Destructive.
pub(in crate::application::app) fn apply_set_section_text(
    text: String,
    clear_runs: bool,
    rc: &mut RebuildContext<'_>,
) {
    let Some((node_id, idx)) = target_section(&rc.document.selection) else {
        log::warn!("SetSectionText: no section selected");
        return;
    };
    if !section_exists(&rc.document, &node_id, idx) {
        log::warn!("SetSectionText: section[{}] not found on node '{}'", idx, node_id);
        return;
    }
    apply_with_rebuild(rc, |doc| {
        if clear_runs {
            doc.set_section_text(&node_id, idx, text)
        } else {
            doc.set_section_text_preserving_runs(&node_id, idx, text)
        }
    });
}

fn section_exists(doc: &MindMapDocument, node_id: &str, idx: usize) -> bool {
    doc.mindmap
        .nodes
        .get(node_id)
        .map(|n| n.sections.get(idx).is_some())
        .unwrap_or(false)
}

/// Insert a new section into the selection's primary node.
/// `at = None` appends; `Some(K)` inserts at index `K` (clamped
/// to `[0, len]`). Mirror of `section add [at=<idx>]
/// [text="<text>"]` for the macro path./// `Action::AddSection`. Destructive.
pub(in crate::application::app) fn apply_add_section(
    at: Option<usize>,
    text: String,
    rc: &mut RebuildContext<'_>,
) {
    use baumhard::mindmap::model::MindSection;
    let Some(node_id) = rc.document.selection.primary_node_id().map(str::to_string) else {
        log::warn!("AddSection: no node selected (Multi/None selection or no primary)");
        return;
    };
    let section = MindSection::new_default(text, Vec::new());
    apply_with_rebuild(rc, |doc| match doc.add_section(&node_id, at, section) {
        Ok(_) => true,
        Err(msg) => {
            log::warn!("AddSection: {}", msg);
            false
        }
    });
}

/// Remove the resolved section from the selection's primary
/// node. Errors when the node has only one section (model
/// invariant: every renderable node has at least one section).
/// Mirror of `section delete [section=<idx>]` for the macro
/// path.`Action::DeleteSection`. Destructive.
pub(in crate::application::app) fn apply_delete_section(rc: &mut RebuildContext<'_>) {
    let Some((node_id, idx)) = target_section(&rc.document.selection) else {
        log::warn!("DeleteSection: no section selected");
        return;
    };
    if !section_exists(&rc.document, &node_id, idx) {
        log::warn!("DeleteSection: section[{}] not found on node '{}'", idx, node_id);
        return;
    }
    apply_with_rebuild(rc, |doc| match doc.delete_section(&node_id, idx) {
        Ok(_) => true,
        Err(msg) => {
            log::warn!("DeleteSection: {}", msg);
            false
        }
    });
}

/// Split the resolved section in two at a grapheme boundary.
/// Mirror of `section split [section=<idx>] at=<grapheme>` for
/// the macro path (`Action::SplitSection`). Destructive.
///
/// `at_grapheme` is **required**, exactly as `at=` is on the verb
/// path: the old end-of-text default silently produced an empty
/// suffix section that the user almost never wanted, and the
/// console verb was hardened against it. A macro step written as
/// `SplitSection { at_grapheme: None }` used to do precisely the
/// thing the verb rejects — the last verb↔Action body divergence.
pub(in crate::application::app) fn apply_split_section(
    at_grapheme: Option<usize>,
    rc: &mut RebuildContext<'_>,
) {
    let Some((node_id, idx, at)) = resolve_split_section_target(rc.document, at_grapheme) else {
        return;
    };
    apply_with_rebuild(rc, |doc| match doc.split_section(&node_id, idx, Some(at)) {
        Ok(_) => true,
        Err(msg) => {
            log::warn!("SplitSection: {}", msg);
            false
        }
    });
}

/// Validate a macro-path `SplitSection` against the live
/// document, returning `(node_id, section_idx, at_grapheme)` when
/// the split may proceed. Every rejection logs its reason —
/// the macro path has no scrollback, so a silent `None` would
/// leave the user with no signal at all.
///
/// Split out from [`apply_split_section`] so the gate is
/// reachable without a `Renderer` (`TEST_CONVENTIONS.md` §T8 /
/// §T9: platform-shared logic lives in functions over plain
/// values).
fn resolve_split_section_target(
    doc: &MindMapDocument,
    at_grapheme: Option<usize>,
) -> Option<(String, usize, usize)> {
    let Some(at) = at_grapheme else {
        log::warn!(
            "SplitSection: at_grapheme is required — split-at-end-of-text \
             silently creates an empty suffix section, so the console verb \
             requires `at=<grapheme>` and the macro / keybind path does too"
        );
        return None;
    };
    let Some((node_id, idx)) = target_section(&doc.selection) else {
        log::warn!("SplitSection: no section selected");
        return None;
    };
    if !section_exists(doc, &node_id, idx) {
        log::warn!("SplitSection: section[{}] not found on node '{}'", idx, node_id);
        return None;
    }
    Some((node_id, idx, at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::{SectionSel, SelectionState};

    /// `target_section` resolves `Section { node_id, section_idx }`
    /// to its inner pair.
    #[test]
    fn target_section_resolves_section_variant() {
        let sel = SelectionState::Section(SectionSel {
            node_id: "n".into(),
            section_idx: 2,
        });
        assert_eq!(target_section(&sel), Some(("n".into(), 2)));
    }

    /// `target_section` rejects `MultiSection` of length > 1
    /// with a `log::warn!`. Pre-fix it silently collapsed to
    /// the first entry, losing the user's multi-target signal
    /// — three reviewers (Architecture #1-3, API/UX B1,
    /// Correctness IMPORTANT) flagged this as the verb/Action
    /// asymmetry that loses user intent.
    #[test]
    fn target_section_rejects_multisection_of_many() {
        let sel = SelectionState::MultiSection(vec![
            SectionSel {
                node_id: "n".into(),
                section_idx: 0,
            },
            SectionSel {
                node_id: "n".into(),
                section_idx: 1,
            },
        ]);
        assert_eq!(target_section(&sel), None);
    }

    /// `MultiSection` of length 1 still resolves (no ambiguity).
    #[test]
    fn target_section_resolves_multisection_of_one() {
        let sel = SelectionState::MultiSection(vec![SectionSel {
            node_id: "n".into(),
            section_idx: 3,
        }]);
        assert_eq!(target_section(&sel), Some(("n".into(), 3)));
    }

    /// `Single(id)` is rejected — verb-path rule 3
    /// auto-resolve to `(id, 0)` lives at the verb layer (it
    /// has the document to count sections); this helper does
    /// not.
    #[test]
    fn target_section_rejects_single() {
        let sel = SelectionState::Single("n".into());
        assert_eq!(target_section(&sel), None);
    }

    /// Point the fixture document at section 0 of its first node.
    fn doc_with_section_selected() -> (MindMapDocument, String) {
        let mut doc = crate::application::document::tests_common::load_test_doc();
        let node_id = doc
            .mindmap
            .nodes
            .keys()
            .next()
            .expect("testament map has nodes")
            .clone();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: node_id.clone(),
            section_idx: 0,
        });
        (doc, node_id)
    }

    /// `Action::SplitSection { at_grapheme: None }` is refused.
    /// The console verb requires `at=` because split-at-end-of-text
    /// silently creates an empty suffix section; a macro step must
    /// not be able to do the thing the verb was hardened against.
    #[test]
    fn split_section_action_rejects_missing_at_grapheme() {
        let (doc, _id) = doc_with_section_selected();
        assert_eq!(
            resolve_split_section_target(&doc, None),
            None,
            "macro-path SplitSection must reject an absent at_grapheme"
        );
    }

    /// The same call with an explicit index resolves — the
    /// rejection is about the missing value, not about the
    /// selection.
    #[test]
    fn split_section_action_accepts_explicit_at_grapheme() {
        let (doc, id) = doc_with_section_selected();
        assert_eq!(resolve_split_section_target(&doc, Some(2)), Some((id, 0, 2)));
    }

    /// An out-of-range section index is refused before the setter
    /// runs, so a stale macro can't address a deleted section.
    #[test]
    fn split_section_action_rejects_missing_section() {
        let (mut doc, id) = doc_with_section_selected();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 99,
        });
        assert_eq!(resolve_split_section_target(&doc, Some(1)), None);
    }
}
