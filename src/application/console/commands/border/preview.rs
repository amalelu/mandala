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
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::{kvs, usage, Descent};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{BorderConfigEdits, BorderEditOutcome, BorderPreviewTarget, OptionEdit};

use super::execute::{edits_has_glyph_field, stage_kv};
use super::finish::BorderEdit;

/// Entry point for the per-node `border preview …` verb.
///
/// The descent has already stepped into the `border preview` level,
/// so its `slot` is where the `commit` / `cancel` terminator sits
/// and its `level.label` is the words every message here leads
/// with. Both used to be hand-counted parameters — `subverb_pos`
/// as a literal `1` / `2` / `3` per surface, and a `verb_label`
/// string beside it.
pub(crate) fn execute_border_preview(descent: &Descent, args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    dispatch_border_preview(descent, args, eff, |sel| {
        super::nodes_in_selection(sel, "border preview").map(BorderPreviewTarget::Nodes)
    })
}

/// Shared dispatch for the four border-preview verbs. Caller
/// supplies a `target_for_verb` closure that resolves the live
/// selection into a `BorderPreviewTarget`; everything else — which
/// slot the terminator sits in, which words the messages lead with,
/// which keys the staging form reads — comes off the descent.
///
/// Generic over the four verbs so the per-verb file is the minimum
/// unique surface: one target resolver.
pub(crate) fn dispatch_border_preview<F>(
    descent: &Descent,
    args: &Args,
    eff: &mut ConsoleEffects,
    target_for_verb: F,
) -> ExecResult
where
    F: FnOnce(&crate::application::document::SelectionState) -> Result<BorderPreviewTarget, ExecResult>,
{
    let level = descent.level;
    match descent.stop {
        Stop::Matched(subverb) => {
            if let Err(msg) = kvs::read_strict(descent, args) {
                return ExecResult::err(msg);
            }
            return match subverb.name {
                "commit" => commit_border_preview_verb(eff, level.label),
                _ => cancel_border_preview_verb(eff, level.label),
            };
        }
        // Every rejection quotes what the user typed rather than the
        // normalized copy the match ran on — echoing the normalized
        // one answered `border preview NOPE` with `unknown subverb
        // 'nope'`, a word the user never wrote.
        Stop::Unknown => {
            return ExecResult::err(usage::unknown_subverb_message(
                level,
                descent.typed.unwrap_or_default(),
            ))
        }
        Stop::KvForm => return ExecResult::err(descent.quoting_hint(args)),
        Stop::Bare => {}
    }

    // Kv-form: stage edits, resolve target, set preview.
    let edits = match stage_kv_for_preview(descent, args) {
        Ok(e) => e,
        Err(err) => return ExecResult::err(err),
    };
    let target = match target_for_verb(&eff.document().selection) {
        Ok(t) => t,
        Err(e) => return e,
    };

    let bare_custom = matches!(
        edits.preset,
        OptionEdit::Set(ref s) if s.eq_ignore_ascii_case("custom")
    ) && !edits_has_glyph_field(&edits);

    let outcome: BorderEditOutcome = eff.document_mut().set_border_preview(target, edits);
    finish_preview(outcome, level.label, bare_custom)
}

/// Stage every kv the staging form reads into a fresh
/// `BorderConfigEdits`, skipping the `section=K` kv (consumed by
/// the per-section verb's target resolver, not a border field).
/// One parser for all four preview verbs. The error carries the
/// level's own label so a user running the same kv vocabulary
/// across four surfaces knows which one answered.
pub(crate) fn stage_kv_for_preview(descent: &Descent, args: &Args) -> Result<BorderConfigEdits, String> {
    let label = descent.level.label;
    let pairs = kvs::read_strict(descent, args).map_err(|e| format!("{}: {}", label, e))?;
    let mut edits = BorderConfigEdits::default();
    let mut saw_any = false;
    for pair in &pairs {
        // `section=` targets the per-section surface's resolver
        // rather than naming a border field, so the staging loop
        // skips it while the level still declares it — which is
        // what keeps it offered and documented.
        if pair.key.name == "section" {
            continue;
        }
        saw_any = true;
        if let Err(e) = stage_kv(&mut edits, pair.key.name, pair.value) {
            return Err(format!("{}: {}", label, e));
        }
    }
    if !saw_any {
        return Err(usage::no_arguments_message(descent.level));
    }
    Ok(edits)
}

/// `border preview commit` — flush the preview through the
/// matching committing setter and surface the merged outcome.
/// Returns "no preview" when no preview is active.
///
/// The fifth caller of [`BorderEdit::finish`], and the last one to
/// become one: it wrote the auto-promotion note, the `per-target`
/// scope and the single-line-vs-`Lines` collapse out by hand, so the
/// most-copied string in the family stayed written twice after the
/// commit that unified the other four. Nothing pinned it — the
/// branch needs an *active* preview, and every `EXEC_CORPUS` row runs
/// on a fresh document — which is why the oracle's `EXEC_SEQ_CORPUS`
/// exists.
pub(crate) fn commit_border_preview_verb(eff: &mut ConsoleEffects, verb_label: &'static str) -> ExecResult {
    let Some(outcome) = eff.document_mut().commit_border_preview() else {
        return ExecResult::ok_msg(format!("{}: no active preview", verb_label));
    };
    BorderEdit {
        label: verb_label,
        scope: "target",
        // A committed preview is not a "no change" outcome any more
        // than a staged one is: the write happened, and the line
        // that staged it already reported what it was.
        headline: Some(format!("{} committed", verb_label)),
        // Empty in practice — `set_border_preview` refuses a
        // violating glyph edit before it records the preview, so a
        // stored one has nothing left for the committing setter to
        // decline. Carried rather than dropped because
        // `document::nodes::border::merge_outcome` folds it here for
        // exactly this reader, and reporting success for a border the
        // model does not have is the failure that fold exists to
        // prevent.
        rejected: outcome.rejected,
        auto_promoted: outcome
            .preset_auto_promoted
            .then_some(outcome.requested_preset)
            .flatten(),
        // `preset=custom` with no glyph fields is a property of the
        // line that carried the kv, and the staging line printed the
        // hint for it. A commit has no kvs of its own.
        bare_custom: false,
    }
    .finish()
}

/// `border preview cancel` — discard the preview without writing
/// anything. Returns a quiet "no preview" line when no preview
/// was active (cancelling drift-cleared previews falls into the
/// same branch).
pub(crate) fn cancel_border_preview_verb(eff: &mut ConsoleEffects, verb_label: &'static str) -> ExecResult {
    let cleared = eff.document_mut().cancel_border_preview();
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
    // `headline` is unconditionally `Some`: a staged preview is
    // active whether or not the edit moved anything, so this verb
    // has no "no change" outcome and cannot reach that arm of the
    // shared tail. That is the one structural difference between
    // the preview family and the three committing verbs.
    BorderEdit {
        label: verb_label,
        scope: "target",
        headline: Some(format!("{} active (commit / cancel to terminate)", verb_label)),
        rejected: outcome.rejected,
        auto_promoted: outcome
            .preset_auto_promoted
            .then_some(outcome.requested_preset)
            .flatten(),
        bare_custom,
    }
    .finish()
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
