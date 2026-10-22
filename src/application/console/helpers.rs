// SPDX-License-Identifier: MPL-2.0

//! Shared command-execution helpers.
//!
//! The bespoke verbs (`anchor` / `cap` / `body` / `edge` / `spacing`
//! / `label`) all follow the same shape: open with a selection
//! guard, walk the kvs the grammar engine has already validated, and
//! finalize into an [`ExecResult`]. The helpers here are the shared
//! bits — the guard returns `Result<T, ExecResult>` so a verb can
//! `?` past it, and the [`ApplyTally`] aggregator collects per-kv
//! messages plus a "did anything actually apply?" bit so the closing
//! [`ApplyTally::finalize`] picks the right Ok/Err shape.
//!
//! The kv *collection* used to live here too, as
//! `collect_kvs_or_usage(args, "<hand-written usage line>")`.
//! `console::spec::kvs` does that now, from the declaration, so the
//! usage line is derived and an unread key is refused by name.
//!
//! Out of scope: the trait-dispatch path (`color`, `font`, `zoom`)
//! already routes through [`super::traits::DispatchReport`] and a
//! sibling `finalize_report`. This module exists for the bespoke
//! direct-setter verbs that don't lift to that machinery.

use baumhard::util::geometry::is_positive_finite;

use super::{ConsoleEffects, ExecResult};
use crate::application::document::EdgeRef;

/// Resolve the currently-selected edge or portal-mode edge (the
/// widened "selected edge" the bespoke verbs operate on). Returns
/// `Err(ExecResult::err("no edge selected"))` when no compatible
/// selection is active so the caller can `?` and forward the
/// error.
pub fn require_edge_or_portal(eff: &ConsoleEffects) -> Result<EdgeRef, ExecResult> {
    eff.document()
        .selection
        .selected_edge_or_portal_edge()
        .ok_or_else(|| ExecResult::err("no edge selected"))
}

/// Per-kv outcome aggregator for bespoke verbs. The pre-helper
/// shape was a hand-rolled `(messages: Vec<String>, any_applied:
/// bool)` pair plus a 6-line tail at the end of every verb that
/// folded the two into an [`ExecResult`]. This struct owns the
/// pair and exposes the same fold via [`ApplyTally::finalize`].
///
/// Why not just reuse [`super::traits::DispatchReport`]? The
/// trait-dispatch path also tracks per-target outcomes and
/// `all_failed` semantics that the bespoke verbs don't need; the
/// bespoke verbs only need messages + any-applied. Lifting them
/// onto `DispatchReport` would mean importing target-id machinery
/// they don't speak.
#[derive(Default)]
pub struct ApplyTally {
    /// Per-arg explanations the kv loop produced — both
    /// `note(false, msg)` no-op messages and `note_error(msg)`
    /// hard-error messages land here. `finalize` joins these
    /// (or emits as `Lines`) when the tally has any content.
    pub messages: Vec<String>,
    /// `true` once any kv produced a real model change. Used
    /// by `finalize` to decide between `Err(joined messages)`
    /// (nothing applied) and `Lines(messages)` (mixed
    /// messages with at least one applied).
    pub any_applied: bool,
}

impl ApplyTally {
    /// Construct an empty tally. Use as the `let mut tally =
    /// ApplyTally::new()` opener at the top of a verb's kv-loop.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one kv outcome. `applied = true` flips the
    /// `any_applied` bit; `applied = false` invokes `msg` to
    /// produce a per-arg explanation that lands in `messages`.
    /// `msg` is a thunk so the caller doesn't allocate a String
    /// on the success path.
    pub fn note<F>(&mut self, applied: bool, msg: F)
    where
        F: FnOnce() -> String,
    {
        if applied {
            self.any_applied = true;
        } else {
            self.messages.push(msg());
        }
    }

    /// Push a hard error message — used by the kv-loop branches
    /// that detected an unparseable key/value before any setter
    /// got a chance to run. These messages are emitted regardless
    /// of `any_applied` because they aren't no-ops, they're bad
    /// input.
    pub fn note_error(&mut self, message: String) {
        self.messages.push(message);
    }

    /// Fold `(messages, any_applied)` into the `ExecResult` shape
    /// every verb closed with: an all-no-op or all-error pair
    /// becomes `Err(joined)`; mixed messages with at least one
    /// applied becomes `Lines`; the clean-success path becomes
    /// `Ok("<verb> applied")`.
    pub fn finalize(self, verb: &str) -> ExecResult {
        if !self.messages.is_empty() {
            if !self.any_applied {
                return ExecResult::err(self.messages.join("; "));
            }
            return ExecResult::lines(self.messages);
        }
        ExecResult::ok_msg(format!("{} applied", verb))
    }
}

/// Parse a positive finite `f32` from a kv value string,
/// returning a uniform error message on the failure paths the
/// bespoke verbs all reproduce verbatim. `key` is the kv name
/// (e.g. `"size"`, `"max"`, `"padding"`) — appears in the error
/// so the user knows which arg was bad.
///
/// Pre-helper, three verbs (`font`, `border`, `zoom`'s f32 path)
/// each reimplemented this with byte-identical wording.
pub fn parse_finite_pt(key: &str, value: &str) -> Result<f32, String> {
    match value.parse::<f32>() {
        Ok(n) if is_positive_finite(n) => Ok(n),
        Ok(n) => Err(format!(
            "{}='{}' must be positive and finite; got {}",
            key, value, n
        )),
        Err(_) => Err(format!("{}='{}' is not a number", key, value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Positive finite values pass through; the error wording
    /// for unparseable / non-positive / non-finite is locked so
    /// the unified path doesn't drift from the pre-merge
    /// per-verb wording.
    #[test]
    fn parse_finite_pt_accepts_positive_finite() {
        assert_eq!(parse_finite_pt("size", "12.5").unwrap(), 12.5);
        assert_eq!(parse_finite_pt("min", "0.001").unwrap(), 0.001);
    }

    #[test]
    fn parse_finite_pt_rejects_zero_negative_nan_inf() {
        assert!(parse_finite_pt("size", "0").is_err());
        assert!(parse_finite_pt("size", "-1.0").is_err());
        assert!(parse_finite_pt("size", "NaN").is_err());
        assert!(parse_finite_pt("size", "inf").is_err());
    }

    #[test]
    fn parse_finite_pt_rejects_unparseable_with_named_key() {
        let err = parse_finite_pt("max", "not-a-number").unwrap_err();
        assert!(err.contains("max"), "error must name the kv key: {}", err);
        assert!(err.contains("not-a-number"));
        assert!(err.contains("is not a number"));
    }

    #[test]
    fn parse_finite_pt_rejects_negative_with_full_wording() {
        let err = parse_finite_pt("padding", "-3").unwrap_err();
        assert!(err.contains("padding"));
        assert!(err.contains("-3"));
        assert!(err.contains("must be positive and finite"));
    }

    /// `ApplyTally::note(true, msg_fn)` flips `any_applied`
    /// and does not invoke the message thunk. `note(false,
    /// msg_fn)` invokes the thunk and pushes the message.
    #[test]
    fn apply_tally_note_routes_by_applied_flag() {
        let mut tally = ApplyTally::new();
        tally.note(true, || panic!("must not run on success"));
        assert!(tally.any_applied);
        assert!(tally.messages.is_empty());

        tally.note(false, || "no-op msg".to_string());
        assert_eq!(tally.messages, vec!["no-op msg".to_string()]);
        assert!(tally.any_applied, "earlier success preserved");
    }

    /// `note_error` is unconditional — bad input lands
    /// regardless of `any_applied`.
    #[test]
    fn apply_tally_note_error_is_unconditional() {
        let mut tally = ApplyTally::new();
        tally.note_error("bad key".to_string());
        assert_eq!(tally.messages, vec!["bad key".to_string()]);
        assert!(!tally.any_applied);
    }

    /// `finalize` partitions the three output shapes:
    /// - clean success → `Ok("<verb> applied")`
    /// - mixed (any_applied + messages) → `Lines(messages)`
    /// - all-failed (messages without any_applied) →
    ///   `Err(joined messages)`
    #[test]
    fn apply_tally_finalize_clean_success_emits_verb_applied() {
        let mut tally = ApplyTally::new();
        tally.note(true, || unreachable!());
        match tally.finalize("anchor") {
            ExecResult::Ok(s) => assert_eq!(s, "anchor applied"),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn apply_tally_finalize_all_failed_emits_err_joined() {
        let mut tally = ApplyTally::new();
        tally.note_error("bad-1".to_string());
        tally.note_error("bad-2".to_string());
        match tally.finalize("anchor") {
            ExecResult::Err(s) => {
                assert!(s.contains("bad-1"));
                assert!(s.contains("bad-2"));
                assert!(s.contains("; "));
            }
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn apply_tally_finalize_mixed_emits_lines() {
        let mut tally = ApplyTally::new();
        tally.note(true, || unreachable!());
        tally.note(false, || "from already foo".to_string());
        match tally.finalize("anchor") {
            ExecResult::Lines(lines) => {
                assert_eq!(lines.len(), 1);
                assert!(lines[0].text.contains("from already foo"));
            }
            other => panic!("expected Lines, got {:?}", other),
        }
    }
}
