// SPDX-License-Identifier: MPL-2.0

//! Unit tests for the [`crate::application::macros`] registry +
//! step-iteration privilege gate. Exercises:
//!
//! - `MacroRegistry` insert / get / replace / clear-tier semantics.
//! - The shadow-stack reveal property: clearing a higher tier
//!   exposes the lower-tier slot underneath instead of leaving
//!   the id permanently displaced.
//! - `SourceTier::allows_action` / `allows_console_line` —
//!   the privilege gate the dispatcher reads before running
//!   each step. Includes the fail-closed contract: a rejected
//!   step aborts the rest of the macro.
//! - JSON serde round-trips for `MacroStep` (the load-bearing
//!   on-disk shape `~/.config/mandala/macros.json` uses).

#![cfg(test)]

use super::*;

#[test]
fn macro_registry_insert_and_get() {
    let mut reg = MacroRegistry::new();
    let m = Macro {
        id: "test".into(),
        name: "Test".into(),
        description: String::new(),
        steps: vec![MacroStep::Action { action: Action::Undo }],
    };
    assert!(reg.is_empty());
    reg.insert(m.clone(), SourceTier::User);
    assert_eq!(reg.len(), 1);
    assert!(reg.contains("test"));
    let got = reg.get("test").unwrap();
    assert_eq!(got.id, "test");
    assert_eq!(got.steps.len(), 1);
    let (_, src) = reg.get_with_source("test").unwrap();
    assert_eq!(src, SourceTier::User);
}

#[test]
fn macro_registry_insert_replaces_by_id() {
    let mut reg = MacroRegistry::new();
    let m1 = Macro {
        id: "x".into(),
        name: "first".into(),
        description: String::new(),
        steps: vec![],
    };
    let m2 = Macro {
        id: "x".into(),
        name: "second".into(),
        description: String::new(),
        steps: vec![MacroStep::ConsoleLine {
            line: "border on".into(),
        }],
    };
    reg.insert(m1, SourceTier::User);
    let prev = reg.insert(m2, SourceTier::User);
    assert!(prev.is_some());
    assert_eq!(reg.get("x").unwrap().name, "second");
}

#[test]
fn macro_source_console_line_gating() {
    assert!(SourceTier::User.allows_console_line());
    assert!(!SourceTier::App.allows_console_line());
    assert!(!SourceTier::Map.allows_console_line());
    assert!(!SourceTier::Inline.allows_console_line());
}

#[test]
fn macro_source_allows_action_gates_destructive_actions_for_non_user() {
    // User passes everything.
    for a in [
        Action::SaveDocument,
        Action::DeleteSelection,
        Action::Cut,
        Action::Paste,
        Action::Copy,
        Action::OrphanSelection,
        Action::CreateOrphanNode,
        Action::CreateOrphanNodeAndEdit,
        Action::NewDocument,
        // Parametric filesystem variants — User tier passes
        // them; non-User tiers reject (asserted in the next
        // block).
        Action::OpenDocument("/tmp/x.mindmap.json".into()),
        Action::SaveDocumentAs("/tmp/x.mindmap.json".into()),
        Action::NewDocumentAt("/tmp/x.mindmap.json".into()),
        // §4.6 destructive section variants — User tier passes;
        // non-User tier rejects (next block).
        Action::SetSectionText {
            text: "x".into(),
            runs_mode: "clear".into(),
        },
        Action::AddSection {
            at: "".into(),
            text: "x".into(),
        },
        Action::DeleteSection,
        Action::SplitSection {
            at_grapheme: "".into(),
        },
        Action::FastResizeStart,
    ] {
        assert!(
            SourceTier::User.allows_action(&a),
            "User tier should allow {:?}",
            a
        );
    }
    // Non-User tiers reject all of the above.
    for tier in [SourceTier::App, SourceTier::Map, SourceTier::Inline] {
        for a in [
            Action::SaveDocument,
            Action::DeleteSelection,
            Action::Cut,
            Action::Paste,
            Action::Copy,
            Action::OrphanSelection,
            Action::CreateOrphanNode,
            Action::CreateOrphanNodeAndEdit,
            // Mixed-branch destructive Actions — their dispatch
            // arms reach editor modals that mutate model state
            // on commit.
            Action::DoubleClickActivate,
            Action::EditSelection,
            Action::EditSelectionClean,
            // Same dispatch surface (`open_single_line_edit`)
            // as `EditSelection` — a
            // hostile macro firing this with an edge label /
            // portal selection forces the user into an inline
            // editor over selected content.
            Action::LabelEditOnSelection,
            Action::NewDocument,
            // Parametric filesystem variants — denylisted for
            // non-User tiers. A hostile inline macro must not be
            // able to overwrite arbitrary files, replace the
            // active document with attacker content, or write
            // out a blank doc on top of an existing path.
            Action::OpenDocument("/tmp/x.mindmap.json".into()),
            Action::SaveDocumentAs("/tmp/x.mindmap.json".into()),
            Action::NewDocumentAt("/tmp/x.mindmap.json".into()),
            // §4.6 destructive section variants — pin the gate
            // value-explicitly per Test Quality #2 + Security
            // M-2. Structural exhaustiveness via `ActionKind::iter()`
            // covers them transitively, but a load-bearing
            // explicit pin catches a future refactor that
            // bypassed `is_destructive()` for any reason.
            Action::SetSectionText {
                text: "x".into(),
                runs_mode: "clear".into(),
            },
            Action::AddSection {
                at: "".into(),
                text: "x".into(),
            },
            Action::DeleteSection,
            Action::SplitSection {
                at_grapheme: "".into(),
            },
            // Batch 4 destructive-on-commit variant — pin
            // explicitly so the `set_node_aabb` / `set_section_aabb`
            // commit on right-button release stays gated.
            Action::FastResizeStart,
        ] {
            assert!(!tier.allows_action(&a), "{:?} tier should reject {:?}", tier, a);
        }
    }
}

#[test]
fn macro_source_allows_action_passes_navigation_for_non_user() {
    // Non-User tiers may still invoke navigation / view-state
    // Actions — they don't touch the filesystem or destroy data.
    for tier in [SourceTier::App, SourceTier::Map, SourceTier::Inline] {
        for a in [
            Action::ZoomIn,
            Action::ZoomOut,
            Action::ZoomReset,
            Action::ZoomFit,
            Action::SelectAll,
            Action::DeselectAll,
            Action::CenterOnSelection,
            Action::JumpToRoot,
            Action::Undo,
        ] {
            assert!(
                tier.allows_action(&a),
                "{:?} tier should allow non-destructive {:?}",
                tier,
                a
            );
        }
    }
}

/// Pure-logic test for the fail-closed step-iteration contract
/// in `dispatch_macro`: walking a macro's steps stops at the
/// first rejected step. The dispatcher itself is renderer-
/// touching and can't run under unit tests
/// (TEST_CONVENTIONS §T8); this test simulates the per-step
/// policy decision so the load-bearing security claim
/// ("fail-closed: a rejected privileged step aborts the rest
/// of the macro") has a runtime regression check.
fn step_allowed(step: &MacroStep, src: SourceTier) -> bool {
    match step {
        MacroStep::Action { action } => src.allows_action(action),
        MacroStep::ConsoleLine { .. } => src.allows_console_line(),
        // CustomMutation has no per-step privilege gate today
        // (registry presence is the implicit gate). Treat as
        // always-allowed for the iteration test.
        MacroStep::CustomMutation { .. } => true,
    }
}

fn run_until_rejected(steps: &[MacroStep], src: SourceTier) -> &[MacroStep] {
    let mut executed = 0;
    for s in steps {
        if !step_allowed(s, src) {
            break;
        }
        executed += 1;
    }
    &steps[..executed]
}

#[test]
fn dispatch_macro_fail_closed_aborts_on_first_reject() {
    // Map-tier macro: Undo (allowed) → ConsoleLine (rejected) →
    // SaveDocument (would also reject). Iteration must stop at
    // ConsoleLine; SaveDocument never gets a chance to run even
    // though it would have been rejected on its own.
    let steps = vec![
        MacroStep::Action { action: Action::Undo },
        MacroStep::ConsoleLine {
            line: "save /tmp/evil".into(),
        },
        MacroStep::Action {
            action: Action::SaveDocument,
        },
    ];
    let executed = run_until_rejected(&steps, SourceTier::Map);
    assert_eq!(executed.len(), 1);
    assert!(matches!(
        &executed[0],
        MacroStep::Action { action } if matches!(action, Action::Undo)
    ));
}

#[test]
fn dispatch_macro_fail_closed_first_step_rejected() {
    // First step rejected → no steps execute.
    let steps = vec![
        MacroStep::Action {
            action: Action::SaveDocument,
        },
        MacroStep::Action { action: Action::Undo },
    ];
    let executed = run_until_rejected(&steps, SourceTier::Map);
    assert!(executed.is_empty());
}

#[test]
fn dispatch_macro_user_tier_passes_destructive_steps() {
    // User-authored macros pass everything — User is trusted by
    // construction (the user wrote the file).
    let steps = vec![
        MacroStep::Action {
            action: Action::SaveDocument,
        },
        MacroStep::ConsoleLine { line: "save".into() },
        MacroStep::Action {
            action: Action::DeleteSelection,
        },
    ];
    let executed = run_until_rejected(&steps, SourceTier::User);
    assert_eq!(executed.len(), 3);
}

#[test]
fn dispatch_macro_inline_tier_rejects_label_edit_on_selection() {
    // Regression test for the LabelEditOnSelection denylist gap
    // closed in batch 1: Inline-tier macro with this Action gets
    // rejected on the first step.
    let steps = vec![MacroStep::Action {
        action: Action::LabelEditOnSelection,
    }];
    let executed = run_until_rejected(&steps, SourceTier::Inline);
    assert!(executed.is_empty());
}

#[test]
fn macro_step_serde_round_trip() {
    let steps = vec![
        MacroStep::Action { action: Action::Undo },
        MacroStep::ConsoleLine {
            line: "fps on".into(),
        },
        MacroStep::CustomMutation {
            id: "nudge-right".into(),
            target: MacroTarget::CurrentSelection,
        },
    ];
    let json = serde_json::to_string(&steps).unwrap();
    let parsed: Vec<MacroStep> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.len(), 3);
    assert!(matches!(parsed[0], MacroStep::Action { action: Action::Undo }));
    if let MacroStep::ConsoleLine { line } = &parsed[1] {
        assert_eq!(line, "fps on");
    } else {
        panic!("step 1 should be ConsoleLine");
    }
}

/// Locks the on-disk JSON shape for every `MacroStep` variant —
/// hand-authored macro files in `~/.config/mandala/macros.json`
/// use these keys; a serde-derive rearrangement would silently
/// break user configs. Covers Action, ConsoleLine, and the two
/// CustomMutation target shapes (default-omitted CurrentSelection
/// and explicit `{node_id: "..."}`).
#[test]
fn macro_step_json_shape_round_trip() {
    let action = MacroStep::Action { action: Action::Undo };
    assert_eq!(
        serde_json::to_string(&action).unwrap(),
        r#"{"kind":"Action","action":"Undo"}"#,
    );

    let console = MacroStep::ConsoleLine { line: "save".into() };
    assert_eq!(
        serde_json::to_string(&console).unwrap(),
        r#"{"kind":"ConsoleLine","line":"save"}"#,
    );

    let omitted: MacroStep = serde_json::from_str(r#"{"kind":"CustomMutation","id":"x"}"#).unwrap();
    let MacroStep::CustomMutation { id, target } = omitted else {
        panic!("expected CustomMutation");
    };
    assert_eq!(id, "x");
    assert!(matches!(target, MacroTarget::CurrentSelection));

    let node_target: MacroStep =
        serde_json::from_str(r#"{"kind":"CustomMutation","id":"x","target":{"node_id":"abc"}}"#).unwrap();
    let MacroStep::CustomMutation {
        target: MacroTarget::NodeId(s),
        ..
    } = node_target
    else {
        panic!("expected NodeId target");
    };
    assert_eq!(s, "abc");
}

/// Inline-tier macros override every lower tier on id
/// collision. Locks the precedence contract for the highest-
/// precedence tier — the property load-bearing for the
/// privilege gate, since Inline is the most-likely-untrusted
/// Inline tier wins on lookup when multiple tiers hold an
/// entry for the same id. Critical for the privilege gate —
/// Inline-tier macros are denylisted from `ConsoleLine` and
/// destructive Actions, so a hostile mindmap shadowing a
/// User-tier macro must be picked up at the Inline tier on
/// lookup, not the lower User tier.
#[test]
fn macro_registry_inline_overrides_user_and_map_by_id() {
    let mut reg = MacroRegistry::new();
    let mk = |name: &str| Macro {
        id: "shared".into(),
        name: name.into(),
        description: "".into(),
        steps: vec![],
    };
    // Order matches run_native_init::build: App → User →
    // Map → Inline.
    reg.insert(mk("app"), SourceTier::App);
    reg.insert(mk("user"), SourceTier::User);
    reg.insert(mk("map"), SourceTier::Map);
    // Inserting Inline does NOT displace Map; both coexist
    // under the shadow-stack design. `insert` returns the
    // previous slot at the SAME tier (None here — this is
    // the first Inline write).
    let prev = reg.insert(mk("inline"), SourceTier::Inline);
    assert!(
        prev.is_none(),
        "no prior Inline-tier entry; Map slot should not have been touched"
    );
    // Lookup walks high-to-low and finds Inline first.
    let (m, src) = reg.get_with_source("shared").unwrap();
    assert_eq!(m.name, "inline");
    assert_eq!(src, SourceTier::Inline);
}

/// Higher-tier macros SHADOW lower-tier ones with the same id —
/// they don't displace. Clearing the higher tier reveals the
/// lower-tier entry underneath. The reveal property fixes the
/// "displacement is permanent within the session" issue
/// reviewers flagged.
#[test]
fn macro_registry_user_overrides_app_by_id() {
    let mut reg = MacroRegistry::new();
    let app_macro = Macro {
        id: "shared-id".into(),
        name: "App version".into(),
        description: String::new(),
        steps: vec![],
    };
    let user_macro = Macro {
        id: "shared-id".into(),
        name: "User version".into(),
        description: String::new(),
        steps: vec![MacroStep::Action { action: Action::Undo }],
    };
    // Insert order matches run_native_init::build: App first.
    reg.insert(app_macro, SourceTier::App);
    // First write to the User tier — no prior User entry to
    // return.
    let prev = reg.insert(user_macro, SourceTier::User);
    assert!(prev.is_none(), "no prior User-tier entry");
    // Lookup walks high-to-low → User wins.
    let (m, src) = reg.get_with_source("shared-id").unwrap();
    assert_eq!(m.name, "User version");
    assert_eq!(src, SourceTier::User);
    // Critical for the privilege gate: the tier upgrade is
    // honored (User can run ConsoleLine / privileged Actions
    // even if an App macro had the same id first).
    assert!(src.allows_console_line());

    // Reveal property: clearing User exposes the App entry
    // underneath, so a higher-tier load is non-destructive
    // to the lower tier.
    reg.clear_tier(SourceTier::User);
    let (m, src) = reg
        .get_with_source("shared-id")
        .expect("App entry must re-emerge");
    assert_eq!(m.name, "App version");
    assert_eq!(src, SourceTier::App);
    assert!(!src.allows_console_line(), "App tier rejects ConsoleLine");
}

/// Sentinel for `SourceTier::COUNT` ↔ variant-count drift. If a
/// future contributor adds a fifth `SourceTier` variant without
/// extending `SourceTier::ALL`, `SourceTier::index()` would return
/// an out-of-bounds index for the new variant and every lookup
/// involving it would panic in `dispatch_macro`. Inserting at every
/// tier the ladder declares and asserting the count catches the
/// omission at test time rather than at runtime.
#[test]
fn macro_registry_every_tier_holds_an_entry() {
    let mut reg = MacroRegistry::new();
    let make = |id: &str| Macro {
        id: id.into(),
        name: String::new(),
        description: String::new(),
        steps: vec![],
    };
    for (i, src) in SourceTier::ALL.iter().enumerate() {
        reg.insert(make(&format!("id-{}", i)), *src);
        assert!(
            reg.get_with_source(&format!("id-{}", i)).is_some(),
            "tier {:?} (SourceTier::COUNT={}) failed to store an entry — \
             likely a slot-array / SourceTier drift",
            src,
            SourceTier::COUNT,
        );
    }
    assert_eq!(reg.len(), SourceTier::COUNT);
}

/// `clear_tier` removes only entries with the matching source —
/// other tiers stay in place. Critical for the document-replace
/// path: opening a different document must wipe Map-tier macros
/// from the previous document while keeping App / User intact.
#[test]
fn macro_registry_clear_tier_only_drops_matching() {
    let mut reg = MacroRegistry::new();
    reg.insert(
        Macro {
            id: "app".into(),
            name: "".into(),
            description: "".into(),
            steps: vec![],
        },
        SourceTier::App,
    );
    reg.insert(
        Macro {
            id: "user".into(),
            name: "".into(),
            description: "".into(),
            steps: vec![],
        },
        SourceTier::User,
    );
    reg.insert(
        Macro {
            id: "map".into(),
            name: "".into(),
            description: "".into(),
            steps: vec![],
        },
        SourceTier::Map,
    );
    assert_eq!(reg.len(), 3);
    reg.clear_tier(SourceTier::Map);
    assert_eq!(reg.len(), 2);
    assert!(reg.contains("app"));
    assert!(reg.contains("user"));
    assert!(!reg.contains("map"));
}

/// Shadow-stack reveal property — the central new behavior
/// of the per-tier slot design. Clearing a higher tier reveals
/// the lower-tier entry underneath instead of leaving the id
/// permanently displaced.
#[test]
fn clear_higher_tier_reveals_lower_tier() {
    let mut reg = MacroRegistry::new();
    let mk = |name: &str| Macro {
        id: "x".into(),
        name: name.into(),
        description: String::new(),
        steps: vec![],
    };
    reg.insert(mk("user"), SourceTier::User);
    reg.insert(mk("map"), SourceTier::Map);

    // Map shadows User on lookup.
    let (m, src) = reg.get_with_source("x").unwrap();
    assert_eq!(m.name, "map");
    assert_eq!(src, SourceTier::Map);

    // Clearing Map reveals User — does not leave "x" gone.
    reg.clear_tier(SourceTier::Map);
    let (m, src) = reg
        .get_with_source("x")
        .expect("User entry should re-emerge after Map clear");
    assert_eq!(m.name, "user");
    assert_eq!(src, SourceTier::User);
}

/// `clear_tier(X)` only zeroes the slot for tier `X` on each
/// id. The other tiers' slots survive in place. (The
/// `macro_registry_clear_tier_only_drops_matching` test
/// above covers the case where each id has only one slot
/// occupied; this test covers the case where ids have
/// multiple tiers stacked.)
#[test]
fn clear_tier_removes_only_matching_slot() {
    let mut reg = MacroRegistry::new();
    let mk = |id: &str, name: &str| Macro {
        id: id.into(),
        name: name.into(),
        description: String::new(),
        steps: vec![],
    };
    // Stack two tiers on "x" and one tier each on "y" / "z".
    reg.insert(mk("x", "x.app"), SourceTier::App);
    reg.insert(mk("x", "x.user"), SourceTier::User);
    reg.insert(mk("y", "y.user"), SourceTier::User);
    reg.insert(mk("z", "z.map"), SourceTier::Map);

    reg.clear_tier(SourceTier::User);

    // "x" still has its App entry — id survives, just at a
    // lower tier.
    let (m, src) = reg.get_with_source("x").unwrap();
    assert_eq!(m.name, "x.app");
    assert_eq!(src, SourceTier::App);
    // "y" had only User → id evicted entirely.
    assert!(!reg.contains("y"));
    // "z" had only Map → unaffected.
    let (m, src) = reg.get_with_source("z").unwrap();
    assert_eq!(m.name, "z.map");
    assert_eq!(src, SourceTier::Map);
}

/// Document-replace cycle: a Map-tier macro shadows a
/// User-tier macro in document A; opening document B (which
/// has no `mindmap.macros`) clears the Map tier and the
/// User-tier macro re-emerges. Locks the load-bearing
/// user-visible behavior change shadow-stacking enables.
#[test]
fn cross_tier_within_session_round_trip() {
    let mut reg = MacroRegistry::new();
    let mk = |source_label: &str| Macro {
        id: "save-and-quit".into(),
        name: source_label.into(),
        description: String::new(),
        steps: vec![],
    };
    // Startup: User tier loaded.
    reg.insert(mk("user-macro"), SourceTier::User);
    // Open document A: Map tier shadows User.
    reg.insert(mk("docA-macro"), SourceTier::Map);
    assert_eq!(reg.get_with_source("save-and-quit").unwrap().0.name, "docA-macro");

    // Open document B: replace path runs `clear_tier(Map)`
    // first, then `extend_with_tier(empty, Map)`.
    reg.clear_tier(SourceTier::Map);
    reg.extend_with_tier(Vec::<Macro>::new(), SourceTier::Map);

    // User entry re-emerges.
    let (m, src) = reg.get_with_source("save-and-quit").expect("User entry restored");
    assert_eq!(m.name, "user-macro");
    assert_eq!(src, SourceTier::User);
}

/// Within-tier id collisions follow last-writer-wins —
/// documented at `format/macros.md` "Within-tier and cross-
/// tier collision semantics". A Map-tier `mindmap.macros`
/// array with two entries both `id: "x"` keeps only the
/// second.
#[test]
fn macro_registry_extend_with_tier_within_tier_collision_is_last_writer() {
    let mut reg = MacroRegistry::new();
    let macros = vec![
        Macro {
            id: "x".into(),
            name: "first".into(),
            description: "".into(),
            steps: vec![],
        },
        Macro {
            id: "x".into(),
            name: "second".into(),
            description: "".into(),
            steps: vec![],
        },
    ];
    reg.extend_with_tier(macros, SourceTier::Map);
    assert_eq!(reg.len(), 1);
    let (m, _src) = reg.get_with_source("x").unwrap();
    assert_eq!(m.name, "second", "later writer wins within a tier");
}

/// `extend_with_tier` is the bulk-insert form used by the
/// document-load path. Combined with `clear_tier`, it gives a
/// "wipe and reinstall this tier" idiom.
#[test]
fn macro_registry_extend_with_tier_inserts_at_correct_source() {
    let mut reg = MacroRegistry::new();
    let macros = vec![
        Macro {
            id: "a".into(),
            name: "".into(),
            description: "".into(),
            steps: vec![],
        },
        Macro {
            id: "b".into(),
            name: "".into(),
            description: "".into(),
            steps: vec![],
        },
    ];
    reg.extend_with_tier(macros, SourceTier::Map);
    assert_eq!(reg.len(), 2);
    let (_, src_a) = reg.get_with_source("a").unwrap();
    let (_, src_b) = reg.get_with_source("b").unwrap();
    assert_eq!(src_a, SourceTier::Map);
    assert_eq!(src_b, SourceTier::Map);
    // Both tagged Map — should reject ConsoleLine and destructive
    // Actions (the privilege gate).
    assert!(!src_a.allows_console_line());
    assert!(!src_a.allows_action(&Action::SaveDocument));
}

/// `MacroRegistry::get_with_source` returns the loader-pinned
/// tier alongside the macro. This is the load-bearing accessor
/// the dispatcher uses to gate `ConsoleLine` and privileged
/// `Action` steps. Without it, a future caller that uses bare
/// `get` would silently bypass the gate.
#[test]
fn macro_registry_get_with_source_returns_pinned_tier() {
    let mut reg = MacroRegistry::new();
    let map_macro = Macro {
        id: "hostile".into(),
        name: "From a hostile mindmap".into(),
        description: String::new(),
        steps: vec![MacroStep::ConsoleLine {
            line: "save /tmp/evil".into(),
        }],
    };
    reg.insert(map_macro, SourceTier::Map);
    let (m, src) = reg.get_with_source("hostile").unwrap();
    assert_eq!(src, SourceTier::Map);
    // The dispatcher looks at this exact pair to decide gating.
    assert!(!src.allows_console_line());
    assert!(matches!(&m.steps[0], MacroStep::ConsoleLine { .. }));
}
