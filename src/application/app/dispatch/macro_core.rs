// SPDX-License-Identifier: MPL-2.0

//! Cross-platform macro dispatch. The privilege-gating step loop
//! that drives a `Macro` (a `Vec<MacroStep>`) lives here, abstracted
//! over a [`MacroDispatchTarget`] trait so native and WASM share
//! the same body byte-for-byte. Re-implementing the loop on
//! either target is **forbidden** — the privilege gate
//! (`SourceTier::allows_action`, `allows_console_line`) is the
//! threat-model defence and must be single-sourced.
//!
//! - **Native** impl lives in
//!   `super::dispatch::NativeMacroDispatchTarget` wrapping
//!   `&mut InputHandlerContext`.
//! - **WASM** impl lives in
//!   `super::run_wasm::WasmMacroDispatchTarget` wrapping
//!   `&mut WasmInputState` (added in Track B Commit 5).
//!
//! The trait surface is intentionally small: the four operations
//! the macro body needs (Action, CustomMutation, ConsoleLine,
//! selection-resolve) plus a registry borrow for the id lookup at
//! the head of the loop.

use crate::application::keybinds::Action;
use crate::application::macros::MacroRegistry;

use super::cross_dispatch::DispatchOutcome;

/// Per-target operations the cross-platform [`dispatch_macro`]
/// step loop calls. Each implementor wraps a context type
/// (`InputHandlerContext` on native, `WasmInputState` on WASM)
/// and forwards each operation to the right platform-specific
/// helper.
///
/// **Privilege gating happens in [`dispatch_macro`], not in the
/// impl.** The impl just provides the mechanics; the policy is
/// in the loop body so it can't drift between targets.
pub(in crate::application::app) trait MacroDispatchTarget {
    /// Borrow the macro registry for the id lookup at the head of
    /// the loop.
    fn registry(&self) -> &MacroRegistry;

    /// Run a single Action step against the target's full state.
    /// Returns whether the action was recognised and dispatched
    /// (mirrors `dispatch::dispatch_action`'s `DispatchOutcome`).
    fn dispatch_action(&mut self, action: Action) -> DispatchOutcome;

    /// Apply the custom mutation identified by `id` to the node
    /// `node_id`. Looks up the mutation in the impl's document,
    /// applies it via the platform's existing keybind-trigger
    /// helper, and rebuilds the scene when the mutation lands.
    /// Warn-logs unknown ids; returns `true` only when a mutation
    /// was actually applied.
    fn apply_custom_mutation(&mut self, id: &str, node_id: &str) -> bool;

    /// Execute a free-form console line. Reaches the loop ONLY
    /// after the privilege gate (`SourceTier::allows_console_line`)
    /// approved the step — non-User tiers fail-closed-abort the
    /// macro before this method is called.
    ///
    /// Returns `true` when the line actually ran (the macro's
    /// `any_ran` flag bumps), `false` on a soft-skip path that
    /// looks like execution but didn't change document state — e.g.
    /// no document loaded on native, or the WASM no-console-runtime
    /// log path. Pinning `any_ran` to actual execution preserves
    /// pre-Track-B native semantics (a `[ConsoleLine("open foo")]`
    /// macro fired before any document loads doesn't consume the
    /// keystroke event).
    fn execute_console_line(&mut self, line: &str) -> bool;

    /// Resolve `MacroTarget::CurrentSelection` to a Single-node id.
    /// Returns `None` when the selection is multi / edge / portal
    /// / none — the step skips per `format/macros.md` § macro
    /// targets.
    fn current_selection_node_id(&self) -> Option<String>;

    /// Whether the impl's document holds a node with this id.
    /// Used to short-circuit `MacroTarget::NodeId` against typo'd
    /// ids before hitting `apply_custom_mutation`'s own warn.
    fn has_node(&self, node_id: &str) -> bool;
}

/// Run a macro by id against a [`MacroDispatchTarget`]. Returns
/// `true` iff at least one step actually executed (the same
/// semantics native callers rely on for "macro keybind was
/// handled").
///
/// **Privilege gating** runs per-step. A non-User tier's first
/// destructive Action or any ConsoleLine step **fail-closed
/// aborts** the remaining steps so a hostile pattern like
/// `[BenignAction, RejectedAction, SaveDocument]` can't smuggle
/// post-rejected destructive steps past the gate.
pub(in crate::application::app) fn dispatch_macro<T: MacroDispatchTarget>(
    macro_id: &str,
    target: &mut T,
) -> bool {
    use crate::application::macros::{MacroStep, MacroTarget};

    let (mac, source) = match target.registry().get_with_source(macro_id) {
        Some((m, s)) => (m.clone(), s),
        None => {
            log::warn!("macros: dispatch of unknown macro id '{}'", macro_id);
            return false;
        }
    };

    let mut any_ran = false;
    for step in &mac.steps {
        match step {
            MacroStep::Action { action } => {
                // Privilege gate symmetric with `ConsoleLine` below.
                // Non-User tiers cannot fire destructive / clipboard /
                // I/O Actions. Fail-closed: a rejected privileged step
                // aborts the rest of the macro so a `[DeleteSelection,
                // ConsoleLine(rejected), SaveDocument]` pattern can't
                // sneak its outer steps past the gate.
                if !source.allows_action(action) {
                    log::warn!(
                        "macro '{}' (source {:?}): Action {:?} rejected — \
                         tier may not invoke destructive / I/O Actions; \
                         aborting remaining steps",
                        macro_id,
                        source,
                        action,
                    );
                    return any_ran;
                }
                let outcome = target.dispatch_action(action.clone());
                if matches!(outcome, DispatchOutcome::Handled) {
                    any_ran = true;
                }
            }
            MacroStep::CustomMutation {
                id,
                target: macro_target,
            } => {
                let nid_opt: Option<String> = match macro_target {
                    MacroTarget::CurrentSelection => target.current_selection_node_id(),
                    MacroTarget::NodeId(s) => {
                        // Guard against typo'd or stale node ids: if
                        // the document doesn't have the named node we'd
                        // silently no-op (`apply_custom_mutation`'s
                        // snapshot loop filters missing). Surface the
                        // problem with a warn instead.
                        if target.has_node(s) {
                            Some(s.clone())
                        } else {
                            log::warn!("macro step CustomMutation: node id '{}' not found", s,);
                            continue;
                        }
                    }
                };
                let Some(nid) = nid_opt else {
                    log::debug!(
                        "macro step CustomMutation: no resolvable target; skipping id={}",
                        id,
                    );
                    continue;
                };
                if target.apply_custom_mutation(id, &nid) {
                    any_ran = true;
                }
            }
            MacroStep::ConsoleLine { line } => {
                // **Privilege gate.** `ConsoleLine` runs an arbitrary
                // console verb, including filesystem-touching ones.
                // Only `SourceTier::User` macros may carry it —
                // app-bundled, map-inline, and node-inline tiers
                // come from sources the user didn't necessarily
                // author, so they cannot do file I/O via macros.
                if !source.allows_console_line() {
                    // Fail-closed: a tier that's not allowed to run
                    // console verbs aborts the rest of the macro.
                    // `continue` would let post-gate Action steps
                    // still run, which combined with destructive
                    // Actions could leave the user in an unexpected
                    // state (e.g. `[DeleteSelection,
                    // ConsoleLine(rejected), SaveDocument]` would
                    // persist the post-delete state without consent).
                    log::warn!(
                        "macro '{}' (source {:?}): ConsoleLine step rejected — \
                         only User-tier macros may run console verbs; \
                         aborting remaining steps",
                        macro_id,
                        source,
                    );
                    return any_ran;
                }
                if target.execute_console_line(line) {
                    any_ran = true;
                }
            }
        }
    }
    any_ran
}

#[cfg(test)]
mod tests {
    //! Mock-target tests for the privilege-gating step loop.
    //! Drives `dispatch_macro` against a recording mock so the gate
    //! contract is verified at the actual loop body, not just
    //! through the per-step simulator in `macros/mod.rs`.

    use super::*;
    use crate::application::keybinds::Action;
    use crate::application::macros::{Macro, MacroRegistry, MacroStep, MacroTarget};
    use crate::application::source_tier::SourceTier;

    /// Mock target: records every method invocation in order.
    /// `Default` selection → `current_selection_node_id` returns
    /// `Some("sel".into())`; flip via the field on construction.
    struct MockTarget {
        registry: MacroRegistry,
        calls: Vec<String>,
        current_selection: Option<String>,
        known_nodes: Vec<String>,
        action_outcome: DispatchOutcome,
        custom_mutation_applied: bool,
        console_line_executed: bool,
    }

    impl MockTarget {
        fn new(registry: MacroRegistry) -> Self {
            Self {
                registry,
                calls: Vec::new(),
                current_selection: Some("sel".into()),
                known_nodes: vec!["n1".into(), "sel".into()],
                action_outcome: DispatchOutcome::Handled,
                custom_mutation_applied: true,
                console_line_executed: true,
            }
        }
    }

    impl MacroDispatchTarget for MockTarget {
        fn registry(&self) -> &MacroRegistry {
            &self.registry
        }
        fn dispatch_action(&mut self, action: Action) -> DispatchOutcome {
            self.calls.push(format!("action:{:?}", action));
            self.action_outcome
        }
        fn apply_custom_mutation(&mut self, id: &str, node_id: &str) -> bool {
            self.calls.push(format!("custom:{}@{}", id, node_id));
            self.custom_mutation_applied
        }
        fn execute_console_line(&mut self, line: &str) -> bool {
            self.calls.push(format!("console:{}", line));
            self.console_line_executed
        }
        fn current_selection_node_id(&self) -> Option<String> {
            self.current_selection.clone()
        }
        fn has_node(&self, node_id: &str) -> bool {
            self.known_nodes.iter().any(|n| n == node_id)
        }
    }

    fn registry_with(macros: Vec<(Macro, SourceTier)>) -> MacroRegistry {
        let mut r = MacroRegistry::new();
        for (m, s) in macros {
            r.insert(m, s);
        }
        r
    }

    fn macro_with_steps(id: &str, steps: Vec<MacroStep>) -> Macro {
        Macro {
            id: id.into(),
            name: String::new(),
            description: String::new(),
            steps,
        }
    }

    #[test]
    fn unknown_macro_id_returns_false() {
        let mut t = MockTarget::new(MacroRegistry::new());
        assert!(!dispatch_macro("missing", &mut t));
        assert!(t.calls.is_empty());
    }

    #[test]
    fn user_tier_macro_runs_all_step_kinds() {
        let m = macro_with_steps(
            "u1",
            vec![
                MacroStep::Action { action: Action::Undo },
                MacroStep::CustomMutation {
                    id: "mut1".into(),
                    target: MacroTarget::CurrentSelection,
                },
                MacroStep::ConsoleLine {
                    line: "fps on".into(),
                },
            ],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::User)]));
        assert!(dispatch_macro("u1", &mut t));
        assert_eq!(
            t.calls,
            vec![
                "action:Undo".to_string(),
                "custom:mut1@sel".to_string(),
                "console:fps on".to_string(),
            ],
        );
    }

    /// **Issue #31 acceptance criterion 1, at the macro loop.**
    /// `MacroStep::Action` must be able to drive every picker
    /// Action. Pre-fix the eight `Picker*` bodies lived inside the
    /// picker's key handler and `dispatch_action` had no arms for
    /// them, so a macro step reached the dispatcher and fell to
    /// `_ => Unhandled` — a silent no-op. This pins the step loop
    /// half of the contract: the gate lets them through and each
    /// one lands on `target.dispatch_action`. The arm that
    /// receives them is pinned separately by `picker_op_for`'s
    /// exhaustiveness walk (which is that arm's match guard).
    #[test]
    fn user_tier_macro_drives_every_picker_action() {
        let picker_actions = [
            Action::PickerCancel,
            Action::PickerCommit,
            Action::PickerNudgeHueDown,
            Action::PickerNudgeHueUp,
            Action::PickerNudgeSatDown,
            Action::PickerNudgeSatUp,
            Action::PickerNudgeValDown,
            Action::PickerNudgeValUp,
        ];
        let steps: Vec<MacroStep> = picker_actions
            .iter()
            .map(|a| MacroStep::Action { action: a.clone() })
            .collect();
        let m = macro_with_steps("picker", steps);
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::User)]));
        assert!(dispatch_macro("picker", &mut t));
        let expected: Vec<String> = picker_actions.iter().map(|a| format!("action:{:?}", a)).collect();
        assert_eq!(t.calls, expected, "every picker Action must reach the dispatcher");
    }

    /// The picker Actions are non-destructive, so the privilege
    /// gate lets them through from every tier — a map-authored
    /// macro can drive the wheel. Pinned value-explicitly because
    /// a future reclassification of `PickerCommit` as destructive
    /// would silently take this away, and the abort is fail-closed
    /// (the whole rest of the macro stops).
    #[test]
    fn non_user_tiers_may_drive_the_picker() {
        for tier in [SourceTier::App, SourceTier::Map, SourceTier::Inline] {
            let m = macro_with_steps(
                "p",
                vec![
                    MacroStep::Action {
                        action: Action::PickerNudgeHueUp,
                    },
                    MacroStep::Action {
                        action: Action::PickerCommit,
                    },
                ],
            );
            let mut t = MockTarget::new(registry_with(vec![(m, tier)]));
            assert!(dispatch_macro("p", &mut t), "{:?} tier", tier);
            assert_eq!(
                t.calls,
                vec![
                    "action:PickerNudgeHueUp".to_string(),
                    "action:PickerCommit".to_string(),
                ],
                "{:?} tier must reach both picker steps",
                tier
            );
        }
    }

    #[test]
    fn map_tier_console_line_fail_closed_aborts_remaining_steps() {
        // `[Undo, ConsoleLine, SaveDocument]` from Map tier:
        // Undo runs, ConsoleLine is rejected (Map tier can't run
        // console), SaveDocument MUST NOT run.
        let m = macro_with_steps(
            "m1",
            vec![
                MacroStep::Action { action: Action::Undo },
                MacroStep::ConsoleLine { line: "save".into() },
                MacroStep::Action {
                    action: Action::SaveDocument,
                },
            ],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::Map)]));
        // any_ran is true (Undo executed)
        assert!(dispatch_macro("m1", &mut t));
        // Only Undo recorded — ConsoleLine + SaveDocument both
        // skipped by the abort.
        assert_eq!(t.calls, vec!["action:Undo".to_string()]);
    }

    #[test]
    fn map_tier_destructive_action_fail_closed_aborts() {
        // Map tier carrying a destructive Action: the gate aborts
        // before the dispatch_action call lands.
        let m = macro_with_steps(
            "m2",
            vec![
                MacroStep::Action { action: Action::Undo },
                MacroStep::Action {
                    action: Action::DeleteSelection,
                },
                MacroStep::Action { action: Action::Undo },
            ],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::Map)]));
        assert!(dispatch_macro("m2", &mut t));
        // Only the first non-destructive Undo ran.
        assert_eq!(t.calls, vec!["action:Undo".to_string()]);
    }

    #[test]
    fn current_selection_none_skips_custom_mutation_step() {
        let m = macro_with_steps(
            "u3",
            vec![
                MacroStep::CustomMutation {
                    id: "mut1".into(),
                    target: MacroTarget::CurrentSelection,
                },
                MacroStep::Action { action: Action::Undo },
            ],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::User)]));
        t.current_selection = None;
        // any_ran true because the Undo Action ran
        assert!(dispatch_macro("u3", &mut t));
        // CustomMutation step skipped (no resolvable target);
        // Undo ran. Note `continue` not abort — selection-empty
        // is a soft skip.
        assert_eq!(t.calls, vec!["action:Undo".to_string()]);
    }

    #[test]
    fn missing_node_id_skips_custom_mutation_step() {
        let m = macro_with_steps(
            "u4",
            vec![
                MacroStep::CustomMutation {
                    id: "mut1".into(),
                    target: MacroTarget::NodeId("missing".into()),
                },
                MacroStep::Action { action: Action::Undo },
            ],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::User)]));
        // `known_nodes` is `["n1", "sel"]` — "missing" isn't there.
        assert!(dispatch_macro("u4", &mut t));
        // Same posture: CustomMutation soft-skipped, Undo ran.
        assert_eq!(t.calls, vec!["action:Undo".to_string()]);
    }

    #[test]
    fn empty_macro_returns_false() {
        let m = macro_with_steps("empty", vec![]);
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::User)]));
        assert!(!dispatch_macro("empty", &mut t));
        assert!(t.calls.is_empty());
    }

    #[test]
    fn unhandled_action_outcome_does_not_set_any_ran() {
        // Action steps that return `Unhandled` (e.g. dispatched in
        // a context where they don't apply) must NOT count as "ran".
        let m = macro_with_steps("u5", vec![MacroStep::Action { action: Action::Undo }]);
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::User)]));
        t.action_outcome = DispatchOutcome::Unhandled;
        // The dispatch_action call landed on the mock, but
        // any_ran stays false because Outcome wasn't Handled.
        assert!(!dispatch_macro("u5", &mut t));
        assert_eq!(t.calls, vec!["action:Undo".to_string()]);
    }

    #[test]
    fn user_tier_console_line_bumps_any_ran_when_executed() {
        // Test from parity reviewer: pin `any_ran=true` semantics
        // when ConsoleLine actually executes (the line fires).
        let m = macro_with_steps(
            "u_console",
            vec![MacroStep::ConsoleLine {
                line: "fps on".into(),
            }],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::User)]));
        t.console_line_executed = true;
        // any_ran=true because the console line "ran" (mock
        // returned true).
        assert!(dispatch_macro("u_console", &mut t));
        assert_eq!(t.calls, vec!["console:fps on".to_string()]);
    }

    #[test]
    fn user_tier_console_line_skip_does_not_bump_any_ran() {
        // Test from parity reviewer: when `execute_console_line`
        // returns false (e.g. no document loaded on native, or
        // WASM no-console-runtime soft-skip), `any_ran` stays
        // false. Pre-Track-B native preserved this through an
        // inline `if let Some(doc)` guard; the trait extraction
        // moves the guard into the impl + return-bool contract.
        let m = macro_with_steps(
            "u_console_skip",
            vec![MacroStep::ConsoleLine { line: "save".into() }],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::User)]));
        t.console_line_executed = false;
        // The mock recorded the call, but the macro returns false
        // because the only step's "execution" was a soft-skip.
        assert!(!dispatch_macro("u_console_skip", &mut t));
        assert_eq!(t.calls, vec!["console:save".to_string()]);
    }

    #[test]
    fn custom_mutation_returns_false_does_not_bump_any_ran() {
        // Test from parity reviewer: when `apply_custom_mutation`
        // returns false (e.g. unknown mutation id, or document
        // setter rejected the apply), `any_ran` stays false for
        // that step. The macro overall still returns false if no
        // other step succeeded.
        let m = macro_with_steps(
            "u_cm_failed",
            vec![MacroStep::CustomMutation {
                id: "unknown".into(),
                target: MacroTarget::CurrentSelection,
            }],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::User)]));
        t.custom_mutation_applied = false;
        assert!(!dispatch_macro("u_cm_failed", &mut t));
        // The mock recorded the call (apply was attempted) — only
        // the bool return signals the rebuild gate / any_ran flag.
        assert_eq!(t.calls, vec!["custom:unknown@sel".to_string()]);
    }

    #[test]
    fn inline_tier_destructive_action_first_step_aborts() {
        // Inline tier (highest precedence, lowest trust) — first
        // step is a destructive Action, gate aborts immediately.
        // any_ran is false because nothing ran.
        let m = macro_with_steps(
            "i1",
            vec![MacroStep::Action {
                action: Action::DeleteSelection,
            }],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::Inline)]));
        assert!(!dispatch_macro("i1", &mut t));
        assert!(t.calls.is_empty());
    }

    /// App tier (bundled with the binary, not user-authored) is
    /// gated identically to Map / Inline. Pins symmetric coverage
    /// — the Map tier was tested above; this test pins the same
    /// fail-closed posture for App so a regression that
    /// special-cased one tier without the other gets caught.
    #[test]
    fn app_tier_console_line_fail_closed_aborts_remaining_steps() {
        let m = macro_with_steps(
            "a1",
            vec![
                MacroStep::Action { action: Action::Undo },
                MacroStep::ConsoleLine {
                    line: "fps on".into(),
                },
                MacroStep::Action {
                    action: Action::SaveDocument,
                },
            ],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::App)]));
        assert!(dispatch_macro("a1", &mut t));
        assert_eq!(
            t.calls,
            vec!["action:Undo".to_string()],
            "App-tier ConsoleLine must abort remaining steps so post-gate \
             SaveDocument can't sneak through",
        );
    }

    /// App tier rejects destructive Actions. Symmetric with the
    /// Map-tier test above.
    #[test]
    fn app_tier_destructive_action_fail_closed_aborts() {
        let m = macro_with_steps(
            "a2",
            vec![
                MacroStep::Action { action: Action::Undo },
                MacroStep::Action {
                    action: Action::Cut,
                },
                MacroStep::Action { action: Action::Undo },
            ],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::App)]));
        assert!(dispatch_macro("a2", &mut t));
        assert_eq!(
            t.calls,
            vec!["action:Undo".to_string()],
            "App-tier destructive Action must abort: only the pre-gate \
             benign Undo runs",
        );
    }

    /// Inline-tier ConsoleLine is the highest-risk tier-violation
    /// path: a hostile mindmap can ship a node-inline macro that
    /// runs `save /etc/passwd` on first interaction. Fail-closed
    /// must hold; the test pins exactly that.
    #[test]
    fn inline_tier_console_line_fail_closed_aborts_remaining_steps() {
        let m = macro_with_steps(
            "i_console",
            vec![
                MacroStep::Action { action: Action::Undo },
                MacroStep::ConsoleLine {
                    line: "save /tmp/exfil.json".into(),
                },
                MacroStep::Action {
                    action: Action::NewDocument,
                },
            ],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::Inline)]));
        assert!(dispatch_macro("i_console", &mut t));
        assert_eq!(
            t.calls,
            vec!["action:Undo".to_string()],
            "Inline-tier ConsoleLine must abort: the malicious save line \
             never reaches `execute_console_line`, and the post-gate \
             NewDocument never reaches `dispatch_action`",
        );
    }
}
