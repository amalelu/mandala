// SPDX-License-Identifier: MPL-2.0

//! `border preview …` — live-preview surface for node borders.
//! Three terminators:
//!
//! - `border preview <kv>=…` — stage / replace the active preview.
//! - `border preview commit`  — write through and clear preview.
//! - `border preview cancel`  — discard preview, no model write.
//!
//! Mirrored onto `section frame preview …` and
//! `canvas border preview …` / `canvas section-frame [focused]
//! preview …` via [`dispatch_border_preview`] — each verb supplies
//! its own target-resolver closure and the rest of the staging /
//! commit / cancel plumbing is shared.
//!
//! Kv vocabulary is identical to the committing
//! [`super::execute::stage_kv`] path; preview just routes to a
//! different document setter (`set_border_preview` → no model
//! write) until the user terminates with `commit` (writes
//! through) or `cancel` (discards).

use crate::application::console::parser::Args;
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{BorderConfigEdits, BorderEditOutcome, BorderPreviewTarget, OptionEdit};

use super::execute::{custom_preset_hint, edits_has_glyph_field, stage_kv};

/// Entry point for the per-node `border preview …` verb. The
/// args' positional(0) is `"preview"` (consumed by the parent
/// `border` dispatch); positional(1) is `commit` / `cancel` /
/// the first kv. Resolves the target from the live selection
/// (every selected node id), stages edits via `stage_kv_for_preview`,
/// and routes to `set_border_preview`.
pub(crate) fn execute_border_preview(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    dispatch_border_preview(
        args,
        eff,
        "border preview",
        /* subverb_pos */ 1,
        /* target_for_verb */
        |sel| super::nodes_in_selection(sel, "border preview").map(BorderPreviewTarget::Nodes),
    )
}

/// Shared dispatch for the four border-preview verbs. Caller
/// supplies a `target_for_verb` closure that resolves the live
/// selection into a `BorderPreviewTarget`, plus a `verb_label`
/// used in error / hint messages, plus the `subverb_pos` —
/// which positional index holds the `commit` / `cancel` /
/// first-kv token. `border preview …` puts it at index 1;
/// `section frame preview …` puts it at index 2; `canvas border
/// preview …` at index 2; `canvas section-frame [focused]
/// preview …` at index 2 or 3.
///
/// Generic over the four verbs so the per-verb file is the
/// minimum unique surface (target resolver + label + offset).
pub(crate) fn dispatch_border_preview<F>(
    args: &Args,
    eff: &mut ConsoleEffects,
    verb_label: &'static str,
    subverb_pos: usize,
    target_for_verb: F,
) -> ExecResult
where
    F: FnOnce(&crate::application::document::SelectionState) -> Result<BorderPreviewTarget, ExecResult>,
{
    // Subverb dispatch: `commit` / `cancel` are case-insensitive
    // terminators; everything else is the kv-form path.
    if let Some(verb) = args.positional(subverb_pos) {
        match verb.to_ascii_lowercase().as_str() {
            "commit" => return commit_border_preview_verb(eff, verb_label),
            "cancel" => return cancel_border_preview_verb(eff, verb_label),
            other if !other.contains('=') => {
                if args.kvs().next().is_some() {
                    return ExecResult::err(format!(
                        "{}: unexpected positional '{}' alongside a kv pair — \
                         did you mean to quote a multi-word value?",
                        verb_label, other
                    ));
                }
                return ExecResult::err(format!(
                    "{}: unknown subverb '{}'; use 'commit', 'cancel', or kv form",
                    verb_label, other
                ));
            }
            _ => {}
        }
    }

    // Kv-form: stage edits, resolve target, set preview.
    let edits = match stage_kv_for_preview(args, verb_label) {
        Ok(e) => e,
        Err(err) => return ExecResult::err(err),
    };
    let target = match target_for_verb(&eff.document.selection) {
        Ok(t) => t,
        Err(e) => return e,
    };

    let bare_custom = matches!(
        edits.preset,
        OptionEdit::Set(ref s) if s.eq_ignore_ascii_case("custom")
    ) && !edits_has_glyph_field(&edits);

    let outcome: BorderEditOutcome = eff.document.set_border_preview(target, edits);
    finish_preview(outcome, verb_label, bare_custom)
}

/// Stage every recognised kv on `args` into a fresh
/// `BorderConfigEdits`, skipping the `section=K` kv (consumed by
/// the per-section verb's target resolver, not a border field).
/// Mirrors the kv-staging block at `border/execute.rs:86-96` and
/// `section/frame.rs::execute_section_frame`'s loop — extracted
/// here so the four preview verbs share the same parser. Returns
/// the parser error with the verb's label prefixed (C13: tells
/// the user *which* verb they were running when the parse
/// failed; the prior shape returned the raw `stage_kv` message
/// and confused users running the same kv vocabulary across
/// four verbs).
pub(crate) fn stage_kv_for_preview(args: &Args, verb_label: &str) -> Result<BorderConfigEdits, String> {
    let mut edits = BorderConfigEdits::default();
    let mut saw_any = false;
    for (k, v) in args.kvs() {
        if k == "section" {
            continue;
        }
        saw_any = true;
        if let Err(e) = stage_kv(&mut edits, k, v) {
            return Err(format!("{}: {}", verb_label, e));
        }
    }
    if !saw_any {
        return Err(format!(
            "usage: {} <key>=<value> … | {} commit | {} cancel",
            verb_label, verb_label, verb_label,
        ));
    }
    Ok(edits)
}

/// `border preview commit` — flush the preview through the
/// matching committing setter and surface the merged outcome.
/// Returns "no preview" when no preview is active.
pub(crate) fn commit_border_preview_verb(eff: &mut ConsoleEffects, verb_label: &'static str) -> ExecResult {
    let Some(outcome) = eff.document.commit_border_preview() else {
        return ExecResult::ok_msg(format!("{}: no active preview", verb_label));
    };
    let mut lines: Vec<String> = vec![format!("{} committed", verb_label)];
    if outcome.preset_auto_promoted {
        if let Some(name) = outcome.requested_preset.as_deref() {
            lines.push(format!(
                "note: preset='{}' auto-promoted to 'custom' \
                 (a side or corner glyph was set; non-custom presets \
                 ignore the per-target glyph override)",
                name
            ));
        }
    }
    if lines.len() == 1 {
        ExecResult::ok_msg(lines.into_iter().next().expect("len==1"))
    } else {
        ExecResult::lines(lines)
    }
}

/// `border preview cancel` — discard the preview without writing
/// anything. Returns a quiet "no preview" line when no preview
/// was active (cancelling drift-cleared previews falls into the
/// same branch).
pub(crate) fn cancel_border_preview_verb(eff: &mut ConsoleEffects, verb_label: &'static str) -> ExecResult {
    let cleared = eff.document.cancel_border_preview();
    if cleared {
        ExecResult::ok_msg(format!("{} cancelled", verb_label))
    } else {
        ExecResult::ok_msg(format!("{}: no active preview", verb_label))
    }
}

/// Format the post-`set_border_preview` outcome for the verb's
/// success line. Auto-promotion notes ride alongside the success
/// message; bare `preset=custom` (no glyph fields) gets the same
/// hint the committing path emits.
fn finish_preview(outcome: BorderEditOutcome, verb_label: &'static str, bare_custom: bool) -> ExecResult {
    let mut lines: Vec<String> = vec![format!("{} active (commit / cancel to terminate)", verb_label)];
    if outcome.preset_auto_promoted {
        if let Some(name) = outcome.requested_preset.as_deref() {
            lines.push(format!(
                "note: preset='{}' auto-promoted to 'custom' \
                 (a side or corner glyph was set; non-custom presets \
                 ignore the per-target glyph override)",
                name
            ));
        }
    }
    if bare_custom {
        lines.push(custom_preset_hint(verb_label));
    }
    if lines.len() == 1 {
        ExecResult::ok_msg(lines.into_iter().next().expect("len==1"))
    } else {
        ExecResult::lines(lines)
    }
}

#[cfg(test)]
mod tests {
    use crate::application::console::tests::fixtures::{
        assert_exec_err_contains, assert_exec_ok, assert_exec_ok_strict, run,
    };
    use crate::application::console::ExecResult;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::SelectionState;

    /// Verb `border preview preset=heavy` stages the preview
    /// without writing the model — the document's `border_preview`
    /// slot becomes `Some(...)`, the model border is unchanged.
    #[test]
    fn test_border_preview_verb_routes_to_set_border_preview() {
        let mut doc = load_test_doc();
        let nid = crate::application::document::tests_common::first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(nid.clone());
        let before = doc.mindmap.nodes.get(&nid).cloned().unwrap();
        let result = run("border preview preset=heavy", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        assert!(doc.border_preview.is_some(), "preview slot populated");
        assert_eq!(
            doc.mindmap
                .nodes
                .get(&nid)
                .unwrap()
                .style
                .border
                .as_ref()
                .map(|c| c.preset.clone()),
            before.style.border.as_ref().map(|c| c.preset.clone()),
            "model border slot is unchanged after preview-set"
        );
    }

    /// `border preview commit` writes through and clears the slot.
    #[test]
    fn test_border_preview_commit_verb_routes_to_commit() {
        let mut doc = load_test_doc();
        let nid = crate::application::document::tests_common::first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(nid.clone());
        assert_exec_ok(run("border preview preset=heavy", &mut doc));
        let result = run("border preview commit", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        assert!(doc.border_preview.is_none(), "commit clears the slot");
        assert_eq!(
            doc.mindmap
                .nodes
                .get(&nid)
                .unwrap()
                .style
                .border
                .as_ref()
                .unwrap()
                .preset,
            "heavy",
            "commit wrote the staged preset through"
        );
    }

    /// `border preview cancel` clears without writing.
    #[test]
    fn test_border_preview_cancel_verb_routes_to_cancel() {
        let mut doc = load_test_doc();
        let nid = crate::application::document::tests_common::first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(nid.clone());
        let before_preset = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .style
            .border
            .as_ref()
            .map(|c| c.preset.clone());
        assert_exec_ok(run("border preview preset=heavy", &mut doc));
        // Strict-Ok: cancel always returns single-line `Ok` —
        // pin that contract so a future change that turns it
        // into a `Lines` (e.g. surfacing the pre-cancel slot)
        // trips this test.
        assert_exec_ok_strict(run("border preview cancel", &mut doc));
        assert!(doc.border_preview.is_none(), "cancel clears the slot");
        assert_eq!(
            doc.mindmap
                .nodes
                .get(&nid)
                .unwrap()
                .style
                .border
                .as_ref()
                .map(|c| c.preset.clone()),
            before_preset,
            "model unchanged after preview-then-cancel"
        );
    }

    /// `border preview` with no kvs surfaces the usage message.
    #[test]
    fn test_border_preview_no_kvs_errors_with_usage() {
        let mut doc = load_test_doc();
        let nid = crate::application::document::tests_common::first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(nid);
        assert_exec_err_contains(run("border preview", &mut doc), "usage:");
    }

    /// `border preview commit` with no preview is a quiet no-op.
    #[test]
    fn test_border_preview_commit_with_no_preview_is_quiet() {
        let mut doc = load_test_doc();
        let nid = crate::application::document::tests_common::first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(nid);
        let result = run("border preview commit", &mut doc);
        match result {
            ExecResult::Ok(s) => assert!(s.contains("no active preview")),
            other => panic!("expected Ok with no-preview message, got {:?}", other),
        }
    }

    /// C13: parser errors from the preview path are prefixed with
    /// the verb label so the user knows which surface emitted the
    /// diagnostic — confusing without it because the same kv
    /// vocabulary is shared across four verbs.
    #[test]
    fn test_border_preview_unknown_key_is_prefixed_with_verb_label() {
        let mut doc = load_test_doc();
        let nid = crate::application::document::tests_common::first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(nid);
        let result = run("border preview foo=bar", &mut doc);
        match result {
            ExecResult::Err(s) => {
                assert!(
                    s.contains("border preview"),
                    "diagnostic must include 'border preview' verb label: {}",
                    s
                );
                assert!(
                    s.contains("unknown key 'foo'") || s.contains("unknown key"),
                    "diagnostic must include the parser hint: {}",
                    s
                );
            }
            other => panic!("expected Err, got {:?}", other),
        }
    }

    /// C14: subverbs are case-insensitive — `Commit` / `CANCEL`
    /// / `Preview` route the same as their lowercase forms.
    #[test]
    fn test_border_preview_subverbs_are_case_insensitive() {
        let mut doc = load_test_doc();
        let nid = crate::application::document::tests_common::first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(nid);
        assert_exec_ok(run("BORDER preview preset=heavy", &mut doc));
        assert!(doc.border_preview.is_some(), "uppercase verb routed to set");
        assert_exec_ok(run("border PREVIEW Cancel", &mut doc));
        assert!(doc.border_preview.is_none(), "uppercase Cancel routed to cancel");
    }
}
