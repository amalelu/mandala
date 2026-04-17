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
pub fn apply_kvs<F>(
    doc: &mut MindMapDocument,
    kvs: &[(String, String)],
    mut applier: F,
) -> DispatchReport
where
    F: FnMut(&mut TargetView, &str, &str) -> Option<Outcome>,
{
    let targets = selection_targets(&doc.selection);
    if targets.is_empty() {
        return DispatchReport {
            any_applied: false,
            messages: vec!["no target for command (select a node, edge, or portal first)".into()],
            all_failed: true,
        };
    }

    let mut any_applied = false;
    let mut messages: Vec<String> = Vec::new();
    let mut any_pair_succeeded = false;

    for (k, v) in kvs {
        // Aggregate this pair across every target.
        let mut applied_count = 0usize;
        let mut unchanged_count = 0usize;
        let mut na_count = 0usize;
        let mut invalid_msgs: Vec<String> = Vec::new();
        let mut unknown_key = false;

        for tid in &targets {
            let mut view = view_for(doc, tid);
            match applier(&mut view, k, v) {
                Some(Outcome::Applied) => {
                    applied_count += 1;
                }
                Some(Outcome::Unchanged) => {
                    unchanged_count += 1;
                }
                Some(Outcome::NotApplicable) => {
                    na_count += 1;
                }
                Some(Outcome::Invalid(msg)) => {
                    invalid_msgs.push(msg);
                }
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
        if !invalid_msgs.is_empty() {
            for m in invalid_msgs {
                messages.push(format!("{}: {}", k, m));
            }
            continue;
        }
        if applied_count > 0 {
            any_applied = true;
            any_pair_succeeded = true;
        } else if unchanged_count > 0 {
            any_pair_succeeded = true;
            messages.push(format!("{} already {}", k, v));
        } else if na_count == targets.len() {
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
    }
}

fn targets_kind_label(targets: &[TargetId]) -> &'static str {
    // Multi-selection is homogeneously nodes today; other combos
    // are single-target. Pick the obvious label.
    match targets.first() {
        Some(TargetId::Node(_)) => {
            if targets.len() > 1 {
                "nodes"
            } else {
                "node"
            }
        }
        Some(TargetId::Edge(_)) => "edge",
        Some(TargetId::PortalLabel { .. }) => "portal label",
        None => "selection",
    }
}
