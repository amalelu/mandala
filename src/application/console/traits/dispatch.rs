// SPDX-License-Identifier: MPL-2.0

//! `apply_kvs` + `DispatchReport` — the per-kv aggregation loop that
//! every kv-style console command (`color`, `font`, `label`, …) goes
//! through. The applier closure decides which trait method a key
//! maps to; this file only owns the fanout, the per-pair
//! aggregation, and the report formatting.

use super::outcome::Outcome;
use super::view::{selection_targets, view_for, TargetId, TargetView};
use crate::application::document::MindMapDocument;

/// Formatted summary of a command's per-kv outcome across targets,
/// used to render the scrollback line. Returns `Ok`-style text on
/// success; if at least one pair failed validation the caller turns
/// it into an `ExecResult::Err`.
pub struct DispatchReport {
    /// Count of pairs that at least one target accepted with a
    /// change. Used to pick "set" vs "unchanged" phrasing.
    pub any_applied: bool,
    /// Messages to print to scrollback, one per issue. Empty when
    /// everything applied cleanly.
    pub messages: Vec<String>,
    /// True if every pair was either Invalid or had no applicable
    /// target — `execute` then wants to turn the report into an Err.
    pub all_failed: bool,
    /// The [`Outcome::Invalid`] messages this dispatch produced, in
    /// the order the pairs were applied. Empty when nothing was
    /// rejected for being an unusable *value*.
    ///
    /// Separate from [`Self::messages`], which already carries the
    /// same text, because the two questions are different: the
    /// scrollback wants every line in one list, while a caller
    /// deciding *how loudly* to report a silent no-op needs to know
    /// which kind of failure it was. Asking that of `messages` means
    /// matching substrings, and an `Invalid` message is whatever the
    /// target wrote — there is no substring to match. That is how a
    /// keybinding carrying an unparseable value came to be the
    /// quietest failure on the parametric path: `not applicable`
    /// matched, `already set` matched, and the one that could never
    /// work matched nothing. See `log_not_applicable_if_silent`.
    pub invalid: Vec<String>,
}

/// Apply a list of kv-pairs to a TargetView list, dispatching each
/// key through the corresponding trait. `applier` tells the
/// dispatcher what trait a given key maps to and how to invoke it.
///
/// `applier` returns:
/// - `Some(Outcome)` — the key is recognized; the outcome was the
///   result of the trait call on this target
/// - `None` — the key is not recognized at all (e.g. `font bogus=1`);
///   the dispatcher reports it once (not once per target)
pub fn apply_kvs<F>(doc: &mut MindMapDocument, kvs: &[(String, String)], mut applier: F) -> DispatchReport
where
    F: FnMut(&mut TargetView, &str, &str) -> Option<Outcome>,
{
    let targets = selection_targets(&doc.selection);
    if targets.is_empty() {
        return no_target_report();
    }

    let mut any_applied = false;
    let mut messages: Vec<String> = Vec::new();
    let mut invalid: Vec<String> = Vec::new();
    let mut any_pair_succeeded = false;

    for (k, v) in kvs {
        // Aggregate this pair across every target.
        let mut tally = OutcomeTally::default();
        let mut unknown_key = false;

        for tid in &targets {
            let mut view = view_for(doc, tid);
            match applier(&mut view, k, v) {
                Some(outcome) => tally.record(outcome),
                None => {
                    unknown_key = true;
                    break;
                }
            }
        }

        if unknown_key {
            messages.push(format!("unknown key '{}'", k));
            continue;
        }
        if !tally.invalid.is_empty() {
            for m in tally.invalid {
                let line = format!("{}: {}", k, m);
                invalid.push(line.clone());
                messages.push(line);
            }
            continue;
        }
        if tally.applied > 0 {
            any_applied = true;
            any_pair_succeeded = true;
        } else if tally.unchanged > 0 {
            any_pair_succeeded = true;
            messages.push(format!("{} already {}", k, v));
        } else if tally.not_applicable == targets.len() {
            messages.push(format!(
                "{}: not applicable to {}",
                k,
                targets_kind_label(&targets),
            ));
        }
    }

    let all_failed = !any_pair_succeeded && !messages.is_empty();
    DispatchReport {
        any_applied,
        messages,
        all_failed,
        invalid,
    }
}

/// Apply a single channel-less operation to every selected target,
/// aggregating outcomes the same way [`apply_kvs`] aggregates per-pair
/// outcomes. Used by commands like `font set <name>` whose trait
/// shape is "one value, one method" — there is nothing to fan out
/// kv-style, so the kv aggregation path doesn't apply.
///
/// `op` is invoked once per [`TargetView`] in the selection;
/// returning [`Outcome::NotApplicable`] from every target surfaces a
/// "not applicable to `<kind>`" message, exactly like
/// [`apply_kvs`]. Returning [`Outcome::Invalid`] from a single target
/// surfaces that message verbatim — the caller is expected to have
/// validated the input upstream so an `Invalid` here is a
/// programmatic bug, not a user typo.
pub fn apply_to_targets<F>(doc: &mut MindMapDocument, mut op: F) -> DispatchReport
where
    F: FnMut(&mut TargetView) -> Outcome,
{
    let targets = selection_targets(&doc.selection);
    if targets.is_empty() {
        return no_target_report();
    }

    let mut tally = OutcomeTally::default();
    for tid in &targets {
        let mut view = view_for(doc, tid);
        tally.record(op(&mut view));
    }
    aggregate_single_op(tally, &targets)
}

/// Empty-selection report shared by [`apply_kvs`] and
/// [`apply_to_targets`]. Both surface the same "select a node /
/// edge / portal first" hint, so the boilerplate lives here.
fn no_target_report() -> DispatchReport {
    DispatchReport {
        any_applied: false,
        messages: vec!["no target for command (select a node, edge, or portal first)".into()],
        all_failed: true,
        // Having nothing selected is not an unusable value.
        invalid: Vec::new(),
    }
}

/// Per-target outcome tally — the structure both aggregation paths
/// fold into. Default-constructible so the kv loop can reset
/// between pairs and the channel-less loop can build a single
/// instance.
#[derive(Default)]
struct OutcomeTally {
    applied: usize,
    unchanged: usize,
    not_applicable: usize,
    invalid: Vec<String>,
}

impl OutcomeTally {
    fn record(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Applied => self.applied += 1,
            Outcome::Unchanged => self.unchanged += 1,
            Outcome::NotApplicable => self.not_applicable += 1,
            Outcome::Invalid(msg) => self.invalid.push(msg),
        }
    }
}

/// Aggregate a per-call tally for the channel-less
/// [`apply_to_targets`] shape. Sibling of the kv-shaped
/// aggregation tail inside [`apply_kvs`]; same outcome priorities
/// (Invalid > Applied > Unchanged > NotApplicable) but no kv key
/// to scope messages to.
fn aggregate_single_op(tally: OutcomeTally, targets: &[TargetId]) -> DispatchReport {
    let mut messages: Vec<String> = Vec::new();
    let mut invalid: Vec<String> = Vec::new();
    let mut any_pair_succeeded = false;
    if !tally.invalid.is_empty() {
        invalid.extend(tally.invalid.iter().cloned());
        messages.extend(tally.invalid);
    } else if tally.applied == 0 {
        if tally.unchanged > 0 {
            any_pair_succeeded = true;
            messages.push("already set".into());
        } else if tally.not_applicable == targets.len() {
            messages.push(format!("not applicable to {}", targets_kind_label(targets),));
        }
    } else {
        any_pair_succeeded = true;
    }

    let any_applied = tally.applied > 0;
    let all_failed = !any_pair_succeeded && !messages.is_empty();
    DispatchReport {
        any_applied,
        messages,
        all_failed,
        invalid,
    }
}

fn targets_kind_label(targets: &[TargetId]) -> &'static str {
    // Multi-selection is homogeneously nodes (`Multi`) or
    // homogeneously sections (`MultiSection`). Pluralize the
    // matching label when `len > 1`. Other variants are
    // single-target by construction.
    let plural = targets.len() > 1;
    match targets.first() {
        Some(TargetId::Node(_)) => {
            if plural {
                "nodes"
            } else {
                "node"
            }
        }
        Some(TargetId::Section { .. }) => {
            if plural {
                "sections"
            } else {
                "section"
            }
        }
        Some(TargetId::Edge(_)) => "edge",
        Some(TargetId::EdgeLabel(_)) => "edge label",
        Some(TargetId::PortalLabel { .. }) => "portal label",
        Some(TargetId::PortalText { .. }) => "portal text",
        None => "selection",
    }
}
