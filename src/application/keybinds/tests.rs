// SPDX-License-Identifier: MPL-2.0

//! Unit tests for keybinds — parsing, matching, default config,
//! custom-mutation binding lifecycle, JSON round-trip, and
//! contextual resolution.

use super::*;
use std::collections::HashMap;

#[test]
fn test_parse_simple_key() {
    let k = KeyBind::parse("Escape").unwrap();
    assert_eq!(k.key, "escape");
    assert!(!k.ctrl && !k.shift && !k.alt);
}

#[test]
fn test_parse_ctrl_z() {
    let k = KeyBind::parse("Ctrl+Z").unwrap();
    assert_eq!(k.key, "z");
    assert!(k.ctrl);
    assert!(!k.shift && !k.alt);
}

#[test]
fn test_parse_is_case_insensitive() {
    let k1 = KeyBind::parse("ctrl+z").unwrap();
    let k2 = KeyBind::parse("CTRL+Z").unwrap();
    let k3 = KeyBind::parse("Ctrl+Z").unwrap();
    assert_eq!(k1, k2);
    assert_eq!(k2, k3);
}

#[test]
fn test_parse_all_modifiers() {
    let k = KeyBind::parse("ctrl+shift+alt+delete").unwrap();
    assert_eq!(k.key, "delete");
    assert!(k.ctrl && k.shift && k.alt);
}

#[test]
fn test_parse_whitespace_tolerated() {
    let k = KeyBind::parse(" Ctrl + Z ").unwrap();
    assert_eq!(k.key, "z");
    assert!(k.ctrl);
}

#[test]
fn test_parse_modifier_aliases() {
    // cmd/command/meta/super all map to ctrl for cross-platform muscle memory
    assert!(KeyBind::parse("Cmd+Z").unwrap().ctrl);
    assert!(KeyBind::parse("Meta+Z").unwrap().ctrl);
    assert!(KeyBind::parse("Super+Z").unwrap().ctrl);
    // option aliases alt
    assert!(KeyBind::parse("Option+Z").unwrap().alt);
}

#[test]
fn test_parse_rejects_empty() {
    assert!(KeyBind::parse("").is_err());
    assert!(KeyBind::parse("Ctrl+").is_err());
}

#[test]
fn test_parse_rejects_multiple_keys() {
    assert!(KeyBind::parse("Z+X").is_err());
    assert!(KeyBind::parse("Ctrl+Z+X").is_err());
}

#[test]
fn test_matches_modifiers_exactly() {
    let k = KeyBind::parse("Ctrl+Z").unwrap();
    assert!(k.matches("z", true, false, false));
    // Extra shift mustn't match
    assert!(!k.matches("z", true, true, false));
    // Missing ctrl mustn't match
    assert!(!k.matches("z", false, false, false));
}

/// One row of [`test_default_config_resolves_every_documented_binding`]'s
/// table: `(context, key, ctrl, shift, alt, expected_action)`.
/// Named because the tuple is six wide and clippy's
/// `type_complexity` is right that the inline spelling is unreadable.
type BindingCase = (Option<InputContext>, &'static str, bool, bool, bool, Action);

/// Default-config bindings resolve in every context Mandala
/// honors: the bare `Document` context plus the four modal
/// contexts (`Console`, `ColorPicker`, `LabelEdit`, `TextEdit`).
/// Table-driven so a binding rename / removal triggers exactly
/// one diffable failure rather than scrolling through a wall
/// of bespoke `assert_eq!`s.
#[test]
fn test_default_config_resolves_every_documented_binding() {
    let resolved = KeybindConfig::default().resolve();

    let cases: &[BindingCase] = &[
        // Document context (the bare-context resolver).
        (None, "z", true, false, false, Action::Undo),
        (None, "p", true, false, false, Action::EnterReparentMode),
        (None, "d", true, false, false, Action::EnterConnectMode),
        (None, "delete", false, false, false, Action::DeleteSelection),
        (None, "escape", false, false, false, Action::ExitMode),
        (None, "n", true, false, false, Action::CreateOrphanNode),
        (None, "o", true, false, false, Action::OrphanSelection),
        (None, "enter", false, false, false, Action::EditSelection),
        (None, "backspace", false, false, false, Action::EditSelectionClean),
        (None, "undo", false, false, false, Action::Undo), // bare alias
        // Console.
        (
            Some(InputContext::Console),
            "escape",
            false,
            false,
            false,
            Action::ConsoleClose,
        ),
        (
            Some(InputContext::Console),
            "enter",
            false,
            false,
            false,
            Action::ConsoleSubmit,
        ),
        (
            Some(InputContext::Console),
            "tab",
            false,
            false,
            false,
            Action::ConsoleTabComplete,
        ),
        (
            Some(InputContext::Console),
            "c",
            true,
            false,
            false,
            Action::ConsoleClearLine,
        ),
        (
            Some(InputContext::Console),
            "a",
            true,
            false,
            false,
            Action::ConsoleJumpStart,
        ),
        (
            Some(InputContext::Console),
            "e",
            true,
            false,
            false,
            Action::ConsoleJumpEnd,
        ),
        (
            Some(InputContext::Console),
            "u",
            true,
            false,
            false,
            Action::ConsoleKillToStart,
        ),
        (
            Some(InputContext::Console),
            "w",
            true,
            false,
            false,
            Action::ConsoleKillWord,
        ),
        (
            Some(InputContext::Console),
            "backspace",
            false,
            false,
            false,
            Action::ConsoleDeleteBack,
        ),
        (
            Some(InputContext::Console),
            "space",
            false,
            false,
            false,
            Action::ConsoleInsertSpace,
        ),
        // ColorPicker.
        (
            Some(InputContext::ColorPicker),
            "escape",
            false,
            false,
            false,
            Action::PickerCancel,
        ),
        (
            Some(InputContext::ColorPicker),
            "enter",
            false,
            false,
            false,
            Action::PickerCommit,
        ),
        (
            Some(InputContext::ColorPicker),
            "h",
            false,
            false,
            false,
            Action::PickerNudgeHueDown,
        ),
        (
            Some(InputContext::ColorPicker),
            "h",
            false,
            true,
            false,
            Action::PickerNudgeHueUp,
        ),
        (
            Some(InputContext::ColorPicker),
            "s",
            false,
            false,
            false,
            Action::PickerNudgeSatDown,
        ),
        (
            Some(InputContext::ColorPicker),
            "v",
            false,
            false,
            false,
            Action::PickerNudgeValDown,
        ),
        // LabelEdit.
        (
            Some(InputContext::LabelEdit),
            "escape",
            false,
            false,
            false,
            Action::LabelEditCancel,
        ),
        (
            Some(InputContext::LabelEdit),
            "enter",
            false,
            false,
            false,
            Action::LabelEditCommit,
        ),
        // TextEdit.
        (
            Some(InputContext::TextEdit),
            "escape",
            false,
            false,
            false,
            Action::TextEditCancel,
        ),
    ];

    for &(ctx, key, ctrl, shift, alt, ref expected) in cases {
        let actual = match ctx {
            None => resolved.action_for(key, ctrl, shift, alt),
            Some(c) => resolved.action_for_context(c, key, ctrl, shift, alt),
        };
        let ctx_label = ctx.map_or("Document", |c| match c {
            InputContext::Console => "Console",
            InputContext::ColorPicker => "ColorPicker",
            InputContext::LabelEdit => "LabelEdit",
            InputContext::TextEdit => "TextEdit",
            InputContext::NodeEdit => "NodeEdit",
            InputContext::Document => "Document",
        });
        assert_eq!(
            actual.as_ref(),
            Some(expected),
            "{ctx_label} ctrl={ctrl} shift={shift} alt={alt} key={key:?}",
        );
    }
}

#[test]
fn test_custom_mutation_binding_resolves_when_no_built_in_action() {
    let mut bindings = HashMap::new();
    bindings.insert("Ctrl+Shift+M".into(), "my-mutation".into());
    let cfg = KeybindConfig {
        custom_mutation_bindings: bindings,
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.custom_mutation_for("m", true, true, false),
        Some("my-mutation")
    );
}

#[test]
fn test_custom_mutation_binding_loses_to_builtin_action_via_event_loop() {
    // `custom_mutation_for` is only called after `action_for`
    // returns None — a combo bound to both resolves to the
    // built-in. This test just locks the resolver shape: both
    // lookups are independent.
    let mut bindings = HashMap::new();
    bindings.insert("Ctrl+Z".into(), "collision".into());
    let cfg = KeybindConfig {
        custom_mutation_bindings: bindings,
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
    assert_eq!(
        resolved.custom_mutation_for("z", true, false, false),
        Some("collision")
    );
}

#[test]
fn test_custom_mutation_invalid_combo_is_skipped() {
    let mut bindings = HashMap::new();
    bindings.insert("Z+X".into(), "invalid".into()); // two non-modifier keys
    bindings.insert("Ctrl+M".into(), "valid".into());
    let cfg = KeybindConfig {
        custom_mutation_bindings: bindings,
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.custom_mutation_for("m", true, false, false),
        Some("valid")
    );
}

#[test]
fn test_set_custom_mutation_binding_adds_and_replaces() {
    let mut resolved = KeybindConfig::default().resolve();
    let prev = resolved
        .set_custom_mutation_binding("Ctrl+Shift+M", "first".into())
        .unwrap();
    assert!(prev.is_none());
    assert_eq!(
        resolved.custom_mutation_for("m", true, true, false),
        Some("first")
    );
    let prev = resolved
        .set_custom_mutation_binding("Ctrl+Shift+M", "second".into())
        .unwrap();
    assert_eq!(prev.as_deref(), Some("first"));
    assert_eq!(
        resolved.custom_mutation_for("m", true, true, false),
        Some("second")
    );
}

#[test]
fn test_remove_custom_mutation_binding_returns_removed_id() {
    let mut resolved = KeybindConfig::default().resolve();
    resolved
        .set_custom_mutation_binding("Ctrl+Shift+M", "id-1".into())
        .unwrap();
    let prev = resolved.remove_custom_mutation_binding("Ctrl+Shift+M").unwrap();
    assert_eq!(prev.as_deref(), Some("id-1"));
    assert_eq!(resolved.custom_mutation_for("m", true, true, false), None);
}

#[test]
fn test_keybind_string_round_trip_through_parse() {
    let cases = &["Ctrl+Z", "Ctrl+Shift+M", "Alt+F4", "Shift+Enter", "Escape"];
    for c in cases {
        let parsed = KeyBind::parse(c).unwrap();
        let rendered = parsed.to_binding_string();
        let reparsed = KeyBind::parse(&rendered).unwrap();
        assert_eq!(parsed, reparsed, "round-trip failed for '{}'", c);
    }
}

#[test]
fn test_keybind_parse_mouse_gestures() {
    let cases = &[
        ("DoubleClick", "doubleclick"),
        ("MiddleClick", "middleclick"),
        ("LeftDrag", "leftdrag"),
        ("WheelUp", "wheelup"),
        ("WheelDown", "wheeldown"),
    ];
    for (input, expected_key) in cases {
        let k = KeyBind::parse(input).unwrap();
        assert_eq!(k.key, *expected_key, "parse('{}')", input);
        assert!(!k.ctrl && !k.shift && !k.alt);
    }
}

#[test]
fn test_keybind_parse_modified_mouse_gestures() {
    let k = KeyBind::parse("Shift+DoubleClick").unwrap();
    assert_eq!(k.key, "doubleclick");
    assert!(k.shift);
    assert!(!k.ctrl && !k.alt);

    let k = KeyBind::parse("Ctrl+WheelUp").unwrap();
    assert_eq!(k.key, "wheelup");
    assert!(k.ctrl);
}

#[test]
fn test_keybind_mouse_gesture_round_trip_pascal_case() {
    let cases = &[
        "DoubleClick",
        "MiddleClick",
        "Shift+DoubleClick",
        "Ctrl+WheelUp",
        "Ctrl+Shift+LeftDrag",
        "RightClick",
        "RightDrag",
        "Ctrl+RightDrag",
    ];
    for c in cases {
        let parsed = KeyBind::parse(c).unwrap();
        let rendered = parsed.to_binding_string();
        assert_eq!(rendered, *c, "round-trip emit form for '{}'", c);
        let reparsed = KeyBind::parse(&rendered).unwrap();
        assert_eq!(parsed, reparsed);
    }
}

#[test]
fn test_gesture_key_name_matches_parser_token() {
    // Every MouseGesture's canonical name must round-trip through
    // KeyBind::parse to a binding with the matching key field.
    use strum::IntoEnumIterator;
    for g in MouseGesture::iter() {
        let name = g.key_name();
        let bind = KeyBind::parse(name).unwrap();
        assert_eq!(bind.key, name);
    }
}

// ─── WASM-compatibility classification (locks the API surface) ──

#[test]
fn test_wasm_compatibility_navigation_actions_are_compatible() {
    // Navigation / view-state Actions only touch the renderer +
    // document, both of which exist on both targets. If a contributor
    // ever flips one of these to `NativeOnly`, this test fails and
    // the WASM port loses functionality silently.
    use crate::application::keybinds::WasmCompatibility::Compatible;
    for a in [
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomReset,
        Action::ZoomFit,
        Action::PanCameraNorth,
        Action::PanCameraSouth,
        Action::PanCameraEast,
        Action::PanCameraWest,
        Action::JumpToRoot,
        Action::CenterOnSelection,
        Action::ToggleFps,
        Action::ToggleFpsDebug,
    ] {
        assert_eq!(a.wasm_compatibility(), Compatible, "{:?} should be Compatible", a);
    }
}

#[test]
fn test_wasm_compatibility_section_aabb_actions_are_compatible() {
    use crate::application::keybinds::WasmCompatibility::Compatible;
    for a in [
        Action::SetSectionOffsetDelta {
            dx: "1".into(),
            dy: "0".into(),
        },
        Action::SetSectionSizeAbs {
            w: "100".into(),
            h: "50".into(),
        },
        Action::SetSectionSizeFillParent,
        // §4.6 Action variants — pin Compatible explicitly so a
        // future contributor flipping any of them to NativeOnly
        // (e.g. for "this touches text-edit-state") trips this
        // test instead of silently breaking WASM macro fan-out.
        Action::SetSectionOffsetAbs {
            x: "10".into(),
            y: "20".into(),
        },
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
    ] {
        assert_eq!(a.wasm_compatibility(), Compatible, "{:?} should be Compatible", a);
    }
}

/// Plan §5.B6.9: pin the new no-payload border Action variants
/// as Compatible so a future contributor flipping any of them
/// to NativeOnly trips this test instead of silently breaking
/// WASM keybind reach for the cycle / toggle one-press flows.
#[test]
fn test_wasm_compatibility_border_no_payload_actions_are_compatible() {
    use crate::application::keybinds::WasmCompatibility::Compatible;
    for a in [Action::CycleBorderPreset, Action::ToggleBorderVisible] {
        assert_eq!(a.wasm_compatibility(), Compatible, "{:?} should be Compatible", a);
    }
}

#[test]
fn test_wasm_compatibility_selection_actions_are_compatible() {
    use crate::application::keybinds::WasmCompatibility::Compatible;
    for a in [
        Action::SelectAll,
        Action::DeselectAll,
        Action::InvertSelection,
        Action::SelectParent,
        Action::SelectChild,
        Action::SelectNextSibling,
        Action::SelectPrevSibling,
    ] {
        assert_eq!(a.wasm_compatibility(), Compatible, "{:?} should be Compatible", a);
    }
}

#[test]
fn test_wasm_compatibility_console_modals_are_native_only() {
    use crate::application::keybinds::WasmCompatibility::NativeOnly;
    // A representative sample — the full list lives in
    // action.rs::wasm_compatibility. The test pins the contract:
    // these Actions touch native-only `console_state`, so flipping
    // them to Compatible without porting the modal would crash WASM.
    for a in [
        Action::OpenConsole,
        Action::ConsoleClose,
        Action::ConsoleSubmit,
        Action::ConsoleHistoryUp,
        Action::ConsoleHistoryDown,
        Action::ConsoleScrollUp,
    ] {
        assert_eq!(a.wasm_compatibility(), NativeOnly, "{:?} should be NativeOnly", a);
    }
}

#[test]
fn test_wasm_compatibility_modal_actions_are_native_only() {
    use crate::application::keybinds::WasmCompatibility::NativeOnly;
    // `ExitMode` is **not** in this list: the cross-platform mode-clear
    // slice (drop `last_click`, reset `Resize` mode + rebuild) runs on
    // both targets via `dispatch_compatible`; the native-only residual
    // (Reparent/Connect overlay clear) is the fallthrough. WASM users
    // press Esc to exit Resize mode the same way native users do.
    for a in [
        Action::EnterReparentMode,
        Action::EnterConnectMode,
        Action::ReparentToTarget(None),
        Action::ConnectToTarget(None),
        Action::EnterResizeMode,
        Action::FastResizeStart,
        // EnterNodeEdit / EnterSectionEdit reach `open_text_edit`,
        // which depends on the native modal-stealer cascade
        // (`TextEditState`). Reclassification waits on Batch 4/7.
        Action::EnterNodeEdit,
        Action::EnterNodeEditClean,
        Action::EnterSectionEdit,
        Action::PickerCancel,
        Action::PickerCommit,
        Action::LabelEditCancel,
        Action::LabelEditCommit,
        Action::LabelEditOnSelection,
        Action::OpenColorPicker,
        Action::CloseColorPicker,
        Action::SaveDocument,
        Action::PanCanvas,
        Action::NewDocument,
    ] {
        assert_eq!(a.wasm_compatibility(), NativeOnly, "{:?} should be NativeOnly", a);
    }
}

/// Mixed-branch Actions (whose dispatch arm reads/writes
/// different state per branch) classify per the "ANY NativeOnly
/// branch ⇒ NativeOnly" rule. Locks the classification so a future
/// contributor can't silently downgrade the rule to "the
/// WASM-reachable branch is reachable in practice" — that's the
/// looser semantic the reviewer flagged as a forward-compat trap.
///
/// The set is not written out here: it is
/// [`MIXED_BRANCH_ACTIONS`](crate::application::keybinds::MIXED_BRANCH_ACTIONS),
/// the same constant `lift_mixed_branch_for_wasm_macro` reads. Two
/// hand-written copies drifted — this test named three members and
/// the lift named four — so there is one list now and adding a member
/// obliges both consumers.
///
/// The classification travels *with* each member because it is not
/// uniform: `ExitMode` is mixed-branch and `Compatible` (its native
/// leftover is a step, not a branch reaching native-only state).
/// Asserting a blanket `NativeOnly` is what made a shared list look
/// impossible.
#[test]
fn test_wasm_compatibility_mixed_branch_actions_are_native_only() {
    use crate::application::keybinds::MIXED_BRANCH_ACTIONS;
    for (a, expected) in MIXED_BRANCH_ACTIONS {
        assert_eq!(
            a.wasm_compatibility(),
            expected,
            "{:?} should classify {:?} under the 'ANY NativeOnly branch' rule",
            a,
            expected,
        );
    }
    // The rule has to bite somewhere: at least one member must be
    // NativeOnly, or the loop above would pass against a list that
    // had quietly become all-Compatible.
    assert!(
        MIXED_BRANCH_ACTIONS
            .iter()
            .any(|(_, c)| *c == crate::application::keybinds::WasmCompatibility::NativeOnly),
        "the mixed-branch set must contain at least one NativeOnly member",
    );
}

/// Exhaustiveness pin: every variant kind classifies cleanly under
/// the three classifier methods. `ActionKind::iter()` walks every
/// discriminant — adding a new `Action` variant extends the list
/// automatically (no hand-maintenance), and the classifier matches
/// on `ActionKind` are exhaustive (compiler-enforced) so a missing
/// arm is a build error. This test pins the *value* (every variant
/// kind returns a real `WasmCompatibility` and a `bool`, never
/// panics), the type system pins structural completeness.
#[test]
fn test_classifiers_cover_every_variant_kind() {
    use crate::application::keybinds::WasmCompatibility;
    use strum::IntoEnumIterator;
    for kind in ActionKind::iter() {
        let c = kind.wasm_compatibility();
        assert!(
            matches!(c, WasmCompatibility::Compatible | WasmCompatibility::NativeOnly),
            "{:?} returned an unexpected classification {:?}",
            kind,
            c
        );
        let _ = kind.is_destructive();
        let _ = kind.context();
    }
}

/// Lock the destructive set for the privilege gate. The
/// `ActionKind::is_destructive` match is exhaustive (compiler-
/// enforced); this test pins the *contents* so a change to which
/// variant kinds are considered destructive shows up as a diff in
/// review. Reparent/Connect `*ToTarget` are destructive (tree
/// topology mutation + undo); the `Enter*Mode` siblings stay
/// non-destructive (just app-mode toggles).
#[test]
fn test_is_destructive_destructive_set_is_pinned() {
    let destructive: &[ActionKind] = &[
        ActionKind::SaveDocument,
        ActionKind::NewDocument,
        ActionKind::DeleteSelection,
        ActionKind::OrphanSelection,
        ActionKind::CreateOrphanNode,
        ActionKind::CreateOrphanNodeAndEdit,
        ActionKind::Copy,
        ActionKind::Cut,
        ActionKind::Paste,
        ActionKind::DoubleClickActivate,
        ActionKind::EditSelection,
        ActionKind::EditSelectionClean,
        ActionKind::EnterNodeEdit,
        ActionKind::EnterNodeEditClean,
        ActionKind::EnterSectionEdit,
        ActionKind::LabelEditOnSelection,
        ActionKind::ReparentToTarget,
        ActionKind::ConnectToTarget,
        ActionKind::OpenDocument,
        ActionKind::SaveDocumentAs,
        ActionKind::NewDocumentAt,
        // FastResizeStart commits through `set_node_aabb` /
        // `set_section_aabb` on the right-button release that
        // ends the gesture — destructive per plan §6.10.
        ActionKind::FastResizeStart,
        // Plan §4.6 — section text + structural mutators are
        // destructive (rewrite text content, change the
        // sections vector length).
        ActionKind::SetSectionText,
        ActionKind::AddSection,
        ActionKind::DeleteSection,
        ActionKind::SplitSection,
    ];
    for k in destructive {
        assert!(
            k.is_destructive(),
            "{:?} expected to be destructive (privilege-gated for non-User tiers)",
            k
        );
    }
    // Inverse pin: the rest are non-destructive. Iterating
    // `ActionKind::iter()` and filtering against the destructive
    // set above is the structural completeness check.
    use std::collections::HashSet;
    use strum::IntoEnumIterator;
    let destructive_set: HashSet<ActionKind> = destructive.iter().copied().collect();
    for k in ActionKind::iter() {
        if !destructive_set.contains(&k) {
            assert!(!k.is_destructive(), "{:?} unexpectedly classified destructive", k);
        }
    }
}

#[test]
fn test_wasm_compatibility_text_edit_primitives_are_compatible() {
    // text_edit_state exists on both targets, so the cursor /
    // delete primitives all work in the browser today.
    use crate::application::keybinds::WasmCompatibility::Compatible;
    for a in [
        Action::TextEditCancel,
        Action::TextEditCommit,
        Action::TextEditCursorLeft,
        Action::TextEditCursorRight,
        Action::TextEditCursorUp,
        Action::TextEditCursorDown,
        Action::TextEditCursorHome,
        Action::TextEditCursorEnd,
        Action::TextEditCursorLeftSelect,
        Action::TextEditCursorRightSelect,
        Action::TextEditCursorUpSelect,
        Action::TextEditCursorDownSelect,
        Action::TextEditCursorHomeSelect,
        Action::TextEditCursorEndSelect,
        Action::TextEditWordLeft,
        Action::TextEditWordRight,
        Action::TextEditDeleteBack,
        Action::TextEditDeleteForward,
        Action::TextEditDeleteWordBack,
        Action::TextEditDeleteWordForward,
    ] {
        assert_eq!(a.wasm_compatibility(), Compatible, "{:?} should be Compatible", a);
    }
}

#[test]
fn test_wasm_compatibility_label_edit_primitives_are_native_only() {
    // The inline label / portal-text editors only exist on native.
    // (The node text editor is shared and tested above as
    // Compatible.) When WASM gains the inline label editor, flip
    // these to Compatible.
    use crate::application::keybinds::WasmCompatibility::NativeOnly;
    for a in [
        Action::LabelEditCursorLeft,
        Action::LabelEditCursorRight,
        Action::LabelEditCursorHome,
        Action::LabelEditCursorEnd,
        Action::LabelEditDeleteBack,
        Action::LabelEditDeleteForward,
    ] {
        assert_eq!(a.wasm_compatibility(), NativeOnly, "{:?} should be NativeOnly", a);
    }
}

// ─── Mouse-gesture default-binding regression guards ───────────
// These tests pin the user-facing contract for mouse-gesture
// defaults. A future contributor flipping a default array (or
// re-introducing the empty-canvas double-click that the user
// asked us to remove) fails one of these tests.

#[test]
fn test_double_click_activate_default_resolves_to_action() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_context(InputContext::Document, "doubleclick", false, false, false),
        Some(Action::DoubleClickActivate)
    );
}

/// `Action::SetBorderPreview` round-trips through the JSON
/// config — pre-fix the Action variant existed and was
/// dispatched but had no `KeybindConfig` field, so users could
/// not bind a key to preview-set via JSON.
#[test]
fn test_set_border_preview_keybind_round_trips_through_json() {
    use crate::application::keybinds::BorderPreviewTargetKind;
    let json = r#"{
        "set_border_preview": [
            { "combo": "Ctrl+H", "args": ["node", "preset", "heavy"] }
        ]
    }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    assert_eq!(cfg.set_border_preview.len(), 1);
    let r = cfg.resolve();
    assert_eq!(
        r.action_for_context(InputContext::Document, "h", true, false, false),
        Some(Action::SetBorderPreview {
            target_kind: BorderPreviewTargetKind::Node,
            field: "preset".into(),
            value: "heavy".into(),
        })
    );
}

/// All five `BorderPreviewTargetKind` variants round-trip
/// through the strum-derived parser.
#[test]
fn test_border_preview_target_kind_strum_round_trip() {
    use crate::application::keybinds::BorderPreviewTargetKind;
    use std::str::FromStr;
    for (s, expected) in [
        ("node", BorderPreviewTargetKind::Node),
        ("section", BorderPreviewTargetKind::Section),
        ("canvas-border", BorderPreviewTargetKind::CanvasBorder),
        ("canvas-sf", BorderPreviewTargetKind::CanvasSf),
        ("canvas-sf-focused", BorderPreviewTargetKind::CanvasSfFocused),
    ] {
        let parsed = BorderPreviewTargetKind::from_str(s).unwrap_or_else(|_| panic!("parses {}", s));
        assert_eq!(parsed, expected, "round-trip {} → variant", s);
        let back: &'static str = expected.into();
        assert_eq!(back, s, "round-trip variant → {}", s);
    }
    // Unknown tokens fail the parse — `push_parametric` warns
    // and skips on these.
    assert!(BorderPreviewTargetKind::from_str("canvas-sf-focsed").is_err());
    assert!(BorderPreviewTargetKind::from_str("nodes").is_err());
}

/// `cancel_border_preview` ships unbound by default — the
/// keybind system has no per-action active-state guard, so
/// defaulting Esc would conflict with the existing Esc-bound
/// actions in the Document context (`exit_mode` etc.). Users
/// opt in via the JSON config; the verb path
/// `border preview cancel` is the primary surface.
#[test]
fn test_cancel_border_preview_is_unbound_by_default() {
    let cfg = KeybindConfig::default();
    assert!(
        cfg.cancel_border_preview.is_empty(),
        "CancelBorderPreview must not have a default binding (would conflict with `exit_mode`)"
    );
    let r = cfg.resolve();
    // No key resolves to CancelBorderPreview in the Document
    // context with the default config.
    assert!(
        !r.has_any_binding_for(Action::CancelBorderPreview),
        "default-resolved keybinds must not include CancelBorderPreview"
    );
}

/// Users can opt in to a custom binding via the JSON config —
/// pin the round-trip path that landing
/// `cancel_border_preview` and `commit_border_preview` work.
#[test]
fn test_border_preview_keybinds_round_trip_through_json() {
    let json = r#"{
        "cancel_border_preview": ["Ctrl+Escape"],
        "commit_border_preview": ["Ctrl+Enter"]
    }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    assert_eq!(cfg.cancel_border_preview, vec!["Ctrl+Escape"]);
    assert_eq!(cfg.commit_border_preview, vec!["Ctrl+Enter"]);
    let r = cfg.resolve();
    assert_eq!(
        r.action_for_context(InputContext::Document, "escape", true, false, false),
        Some(Action::CancelBorderPreview)
    );
    assert_eq!(
        r.action_for_context(InputContext::Document, "enter", true, false, false),
        Some(Action::CommitBorderPreview)
    );
}

#[test]
fn test_create_orphan_node_and_edit_default_is_unbound() {
    // The user's primary feature request: empty-canvas double-click
    // does nothing by default. Implemented via an unbound default for
    // CreateOrphanNodeAndEdit, gated by has_any_binding_for in
    // dispatch::dispatch_action's DoubleClickActivate arm.
    let r = KeybindConfig::default().resolve();
    assert!(!r.has_any_binding_for(Action::CreateOrphanNodeAndEdit));
}

#[test]
fn test_has_any_binding_for_returns_true_when_user_opts_in() {
    let cfg = KeybindConfig {
        create_orphan_node_and_edit: vec!["DoubleClick".into()],
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert!(r.has_any_binding_for(Action::CreateOrphanNodeAndEdit));
}

#[test]
fn test_pan_canvas_default_resolves_via_middle_click_and_left_drag() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_context(InputContext::Document, "middleclick", false, false, false),
        Some(Action::PanCanvas)
    );
    assert_eq!(
        r.action_for_context(InputContext::Document, "leftdrag", false, false, false),
        Some(Action::PanCanvas)
    );
}

#[test]
fn test_zoom_in_default_resolves_to_wheelup() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_context(InputContext::Document, "wheelup", false, false, false),
        Some(Action::ZoomIn)
    );
}

#[test]
fn test_zoom_out_default_resolves_to_wheeldown() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_context(InputContext::Document, "wheeldown", false, false, false),
        Some(Action::ZoomOut)
    );
}

#[test]
fn test_action_for_gesture_falls_back_to_unmodified_binding() {
    // Modifier-fallback: Ctrl+WheelUp resolves to ZoomIn even though
    // only the bare WheelUp is bound by default. Exact-modifier
    // override still wins when the user explicitly binds the
    // modified form.
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_gesture("wheelup", true, false, false),
        Some(Action::ZoomIn),
        "Ctrl+WheelUp should fall back to bare WheelUp -> ZoomIn"
    );
    assert_eq!(
        r.action_for_gesture("middleclick", true, true, true),
        Some(Action::PanCanvas),
        "Ctrl+Shift+Alt+MiddleClick should fall back"
    );
}

/// Default `Ctrl+RightDrag` resolves to `FastResizeStart`. Pins
/// the Batch 4 gesture binding — without it, threshold-cross on
/// PendingRight would no-op silently.
#[test]
fn test_default_ctrl_right_drag_resolves_to_fast_resize_start() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_gesture("rightdrag", true, false, false),
        Some(Action::FastResizeStart),
        "Ctrl+RightDrag should resolve to FastResizeStart"
    );
}

/// Bare `RightDrag` (no Ctrl) returns `None` with the default
/// config — only `Ctrl+RightDrag` is bound. The
/// modifier-fallback mechanism flows the *other* way: a key
/// pressed with modifiers can fall back to a bare binding, but
/// a key pressed bare can't escalate to a modified binding.
/// This pins the default posture so a user pressing right-drag
/// without Ctrl doesn't accidentally trigger fast-resize.
#[test]
fn test_bare_right_drag_returns_none_with_default_config() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_gesture("rightdrag", false, false, false),
        None,
        "bare RightDrag must not resolve to anything by default; \
         the default binding is Ctrl+RightDrag and modifier-fallback \
         doesn't escalate from bare to modified"
    );
}

/// Users can opt in to bare `RightDrag` for fast-resize by
/// rebinding `fast_resize_start` to remove the Ctrl modifier.
/// Pins the user-customization path the doc-comment promises.
#[test]
fn test_user_rebind_to_bare_right_drag_works() {
    let cfg = KeybindConfig {
        fast_resize_start: vec!["RightDrag".into()],
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert_eq!(
        r.action_for_gesture("rightdrag", false, false, false),
        Some(Action::FastResizeStart),
        "user-rebind to bare RightDrag should resolve to FastResizeStart"
    );
    // Modifier fallback still works: Ctrl+RightDrag → bare RightDrag → FastResizeStart.
    assert_eq!(
        r.action_for_gesture("rightdrag", true, false, false),
        Some(Action::FastResizeStart),
        "Ctrl+RightDrag should still resolve to FastResizeStart via fallback"
    );
}

/// `LongPress` resolves to `EnterResizeMode` by default — the
/// touch peer of the keyboard's `r`. Plan §6.6 / Batch 7.
#[test]
fn test_default_long_press_resolves_to_enter_resize_mode() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_gesture("longpress", false, false, false),
        Some(Action::EnterResizeMode),
        "LongPress should be the touch peer of `r` for EnterResizeMode"
    );
}

/// `TwoFingerDrag` resolves to `FastResizeStart` by default —
/// the touch peer of `Ctrl+RightDrag`. Plan §6.6 / Batch 7.
#[test]
fn test_default_two_finger_drag_resolves_to_fast_resize_start() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_gesture("twofingerdrag", false, false, false),
        Some(Action::FastResizeStart),
        "TwoFingerDrag should be the touch peer of Ctrl+RightDrag"
    );
}

/// Default `enter_resize_mode` config carries both `r` (kbd)
/// and `LongPress` (touch). Pins the JSON-default shape so a
/// regression that drops the touch entry is caught at config-
/// resolution time.
#[test]
fn test_default_enter_resize_mode_includes_long_press() {
    let cfg = KeybindConfig::default();
    assert!(
        cfg.enter_resize_mode.iter().any(|s| s == "LongPress"),
        "default enter_resize_mode must include LongPress; got: {:?}",
        cfg.enter_resize_mode
    );
    assert!(
        cfg.enter_resize_mode.iter().any(|s| s == "r"),
        "default enter_resize_mode must still include `r`; got: {:?}",
        cfg.enter_resize_mode
    );
}

#[test]
fn test_default_fast_resize_start_includes_two_finger_drag() {
    let cfg = KeybindConfig::default();
    assert!(
        cfg.fast_resize_start.iter().any(|s| s == "TwoFingerDrag"),
        "default fast_resize_start must include TwoFingerDrag; got: {:?}",
        cfg.fast_resize_start
    );
    assert!(
        cfg.fast_resize_start.iter().any(|s| s == "Ctrl+RightDrag"),
        "default fast_resize_start must still include Ctrl+RightDrag; got: {:?}",
        cfg.fast_resize_start
    );
}

/// `RightClick` ships unbound by default. Pins the default
/// posture — users opt in via JSON config.
#[test]
fn test_right_click_is_unbound_by_default() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_gesture("rightclick", false, false, false),
        None,
        "RightClick must not have a default binding"
    );
}

#[test]
fn test_action_for_gesture_exact_modifier_match_wins_over_fallback() {
    // Clear default zoom_in (also bound to WheelUp) so the test
    // exercises only the configured bindings.
    let cfg = KeybindConfig {
        zoom_in: vec![],
        zoom_out: vec!["WheelUp".into()],        // bare WheelUp -> ZoomOut
        zoom_reset: vec!["Ctrl+WheelUp".into()], // Ctrl+WheelUp -> ZoomReset
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert_eq!(
        r.action_for_gesture("wheelup", true, false, false),
        Some(Action::ZoomReset),
        "exact Ctrl+WheelUp binding wins over the bare-WheelUp fallback"
    );
    assert_eq!(
        r.action_for_gesture("wheelup", false, false, false),
        Some(Action::ZoomOut),
        "bare wheelup honors its bare binding"
    );
}

// ─── Macro-tier resolution-order tests ─────────────────────────

#[test]
fn test_macro_for_returns_bound_id() {
    let mut bindings = HashMap::new();
    bindings.insert("Ctrl+G".to_string(), "do-stuff".to_string());
    let cfg = KeybindConfig {
        macro_bindings: bindings,
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert_eq!(r.macro_for("g", true, false, false), Some("do-stuff"));
    assert_eq!(r.macro_for("g", false, false, false), None);
}

#[test]
fn test_macro_bindings_resolve_skips_invalid_combos() {
    let mut bindings = HashMap::new();
    bindings.insert("Ctrl+G".to_string(), "valid".to_string());
    bindings.insert("Garbage++".to_string(), "would-be-orphan".to_string());
    let cfg = KeybindConfig {
        macro_bindings: bindings,
        ..KeybindConfig::default()
    };
    // Resolve survives — invalid combos log and skip; the valid one
    // still lands.
    let r = cfg.resolve();
    assert_eq!(r.macro_for("g", true, false, false), Some("valid"));
}

#[test]
fn test_action_for_gesture_returns_none_when_completely_unbound() {
    let cfg = KeybindConfig {
        zoom_in: vec![],
        zoom_out: vec![],
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert_eq!(r.action_for_gesture("wheelup", false, false, false), None);
    assert_eq!(r.action_for_gesture("wheelup", true, false, false), None);
}

#[test]
fn test_default_console_font_size_is_16() {
    let cfg = KeybindConfig::default();
    assert!(baumhard::util::geometry::almost_equal(
        cfg.console_font_size,
        16.0
    ));
}

#[test]
fn test_resolve_exposes_console_style_fields() {
    let cfg = KeybindConfig {
        console_font: "MyFont".into(),
        console_font_size: 20.0,
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert_eq!(r.console_font, "MyFont");
    assert!(baumhard::util::geometry::almost_equal(r.console_font_size, 20.0));
}

#[test]
fn test_open_console_default_bound_to_slash() {
    let cfg = KeybindConfig::default();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for("/", false, false, false),
        Some(Action::OpenConsole)
    );
}

#[test]
fn test_open_console_in_document_context() {
    // The event loop calls `action_for_context(Document, "/", …)`
    // — not the bare `action_for("/")`. Pins the resolver path the
    // event loop actually walks, guarding the `/` → console binding
    // against a regression in contextual dispatch.
    let resolved = KeybindConfig::default().resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "/", false, false, false),
        Some(Action::OpenConsole),
    );
}

#[test]
fn test_all_document_defaults_resolve_via_action_for_context() {
    // Parametric coverage for every default Document-context
    // binding under the new resolver. Catches any regression that
    // slips past the single-action tests above.
    let r = KeybindConfig::default().resolve();
    let doc = InputContext::Document;
    let cases: &[(Action, &str, bool, bool, bool)] = &[
        (Action::Undo, "z", true, false, false),
        (Action::Undo, "undo", false, false, false),
        (Action::EnterReparentMode, "p", true, false, false),
        (Action::EnterConnectMode, "d", true, false, false),
        (Action::DeleteSelection, "delete", false, false, false),
        (Action::ExitMode, "escape", false, false, false),
        (Action::CreateOrphanNode, "n", true, false, false),
        (Action::OrphanSelection, "o", true, false, false),
        (Action::EditSelection, "enter", false, false, false),
        (Action::EditSelectionClean, "backspace", false, false, false),
        (Action::OpenConsole, "/", false, false, false),
        (Action::SaveDocument, "s", true, false, false),
        (Action::Copy, "c", true, false, false),
        (Action::Copy, "copy", false, false, false),
        (Action::Paste, "v", true, false, false),
        (Action::Paste, "paste", false, false, false),
        (Action::Cut, "x", true, false, false),
        (Action::Cut, "cut", false, false, false),
    ];
    for (action, key, ctrl, shift, alt) in cases {
        assert_eq!(
            r.action_for_context(doc, key, *ctrl, *shift, *alt),
            Some(action.clone()),
            "expected {:?} for key={:?} ctrl={} shift={} alt={}",
            action,
            key,
            ctrl,
            shift,
            alt,
        );
    }
}

#[test]
fn test_save_document_default_bound_to_ctrl_s() {
    let cfg = KeybindConfig::default();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for("s", true, false, false),
        Some(Action::SaveDocument)
    );
}

#[test]
fn test_partial_json_uses_defaults_for_missing_fields() {
    // A user who only wants to rebind one action should be able to omit
    // every other field and get the defaults for them.
    let json = r#"{ "undo": ["Ctrl+Y"] }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    assert_eq!(cfg.undo, vec!["Ctrl+Y"]);
    // Other fields should still have defaults
    assert_eq!(cfg.enter_reparent_mode, vec!["Ctrl+P"]);
    assert_eq!(cfg.exit_mode, vec!["Escape"]);
}

#[test]
fn test_resolve_skips_invalid_bindings() {
    let cfg = KeybindConfig {
        undo: vec!["Ctrl+Z".into(), "Z+X".into()], // second is invalid
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    // Valid binding still works
    assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
}

#[test]
fn test_user_override_replaces_default() {
    // A user who specifies undo bindings should get only those — not
    // theirs merged with the hardcoded list. This matches common
    // config-file intuition.
    let json = r#"{ "undo": ["Ctrl+Y"] }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    let resolved = cfg.resolve();
    assert_eq!(resolved.action_for("y", true, false, false), Some(Action::Undo));
    // Original Ctrl+Z no longer bound
    assert_eq!(resolved.action_for("z", true, false, false), None);
}

#[test]
fn test_json_roundtrip() {
    let cfg = KeybindConfig::default();
    let json = serde_json::to_string(&cfg).unwrap();
    let parsed = KeybindConfig::from_json(&json).unwrap();
    let resolved = parsed.resolve();
    assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
}

#[test]
fn test_normalize_key_name() {
    assert_eq!(normalize_key_name("Escape"), "escape");
    assert_eq!(normalize_key_name("  Delete  "), "delete");
    assert_eq!(normalize_key_name("Z"), "z");
}

// ── Component-scoped actions and contextual resolution ──

#[test]
fn test_console_context_does_not_leak_document_actions() {
    let resolved = KeybindConfig::default().resolve();
    // Ctrl+Z is Undo in Document but should not resolve in Console
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "z", true, false, false),
        None,
    );
    // "/" is OpenConsole in Document but should not resolve in Console
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "/", false, false, false),
        None,
    );
}

#[test]
fn test_picker_context_falls_through_to_document() {
    let resolved = KeybindConfig::default().resolve();
    // Ctrl+Z is not a picker action, but color picker falls through
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "z", true, false, false),
        Some(Action::Undo),
    );
    // "/" opens console — should fall through from picker
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "/", false, false, false),
        Some(Action::OpenConsole),
    );
}

#[test]
fn test_picker_context_prefers_picker_action_over_document() {
    let resolved = KeybindConfig::default().resolve();
    // Escape is ExitMode at Document level but PickerCancel at picker level
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "escape", false, false, false),
        Some(Action::PickerCancel),
    );
    // Enter is EditSelection at Document level but PickerCommit at picker level
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "enter", false, false, false),
        Some(Action::PickerCommit),
    );
}

#[test]
fn test_label_edit_does_not_fall_through() {
    let resolved = KeybindConfig::default().resolve();
    // Ctrl+Z should not resolve in label edit (no fallthrough)
    assert_eq!(
        resolved.action_for_context(InputContext::LabelEdit, "z", true, false, false),
        None,
    );
}

#[test]
fn test_text_edit_does_not_fall_through() {
    let resolved = KeybindConfig::default().resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::TextEdit, "z", true, false, false),
        None,
    );
}

#[test]
fn test_document_context_matches_action_for() {
    let resolved = KeybindConfig::default().resolve();
    // Document context should match all global actions the same as action_for
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "z", true, false, false),
        resolved.action_for("z", true, false, false),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "escape", false, false, false),
        resolved.action_for("escape", false, false, false),
    );
}

#[test]
fn test_action_context_assignment() {
    assert_eq!(Action::Undo.context(), InputContext::Document);
    assert_eq!(Action::Copy.context(), InputContext::Document);
    assert_eq!(Action::ConsoleClose.context(), InputContext::Console);
    assert_eq!(Action::ConsoleClearLine.context(), InputContext::Console);
    assert_eq!(Action::PickerCancel.context(), InputContext::ColorPicker);
    assert_eq!(Action::PickerNudgeHueDown.context(), InputContext::ColorPicker);
    assert_eq!(Action::LabelEditCancel.context(), InputContext::LabelEdit);
    assert_eq!(Action::TextEditCancel.context(), InputContext::TextEdit);
    // EnterNodeEdit (and its Clean variant) lift from Document so a
    // top-level press flips the node into NodeEdit mode. EnterSectionEdit
    // sits in the NodeEdit context so binding it to Enter does not
    // shadow the same key at the Document level.
    assert_eq!(Action::EnterNodeEdit.context(), InputContext::Document);
    assert_eq!(Action::EnterNodeEditClean.context(), InputContext::Document);
    assert_eq!(Action::EnterSectionEdit.context(), InputContext::NodeEdit);
}

/// `InputContext::NodeEdit` falls through to Document so global
/// Document keybinds (Ctrl+S, Ctrl+Z, …) keep working while a
/// NodeEdit session is active. Mirrors `ColorPicker`'s
/// fallthrough discipline. A regression here would silently break
/// every Document binding inside NodeEdit mode.
#[test]
fn test_input_context_node_edit_falls_through() {
    assert!(
        InputContext::NodeEdit.falls_through(),
        "NodeEdit must fall through to Document for global keybinds"
    );
    // `Ctrl+S` is bound at Document; the cascade must surface it
    // when the user is in NodeEdit context.
    let resolved = KeybindConfig::default().resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::NodeEdit, "s", true, false, false),
        Some(Action::SaveDocument),
        "SaveDocument must reach NodeEdit context via the cascade"
    );
    assert_eq!(
        resolved.action_for_context(InputContext::NodeEdit, "z", true, false, false),
        Some(Action::Undo),
        "Undo must reach NodeEdit context via the cascade"
    );
}

#[test]
fn test_user_can_override_component_keybinds() {
    let json = r#"{ "picker_nudge_hue_down": ["j"], "picker_nudge_hue_up": ["k"] }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "j", false, false, false),
        Some(Action::PickerNudgeHueDown),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "k", false, false, false),
        Some(Action::PickerNudgeHueUp),
    );
    // Original "h" no longer bound to hue nudge
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "h", false, false, false),
        None,
    );
}

#[test]
fn test_copy_paste_cut_fall_through_to_picker() {
    let resolved = KeybindConfig::default().resolve();
    // Copy/Paste/Cut are Document-level actions that fall through
    // to the color picker context
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "c", true, false, false),
        Some(Action::Copy),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "v", true, false, false),
        Some(Action::Paste),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "x", true, false, false),
        Some(Action::Cut),
    );
}

#[test]
fn test_partial_json_preserves_component_defaults() {
    let json = r#"{ "undo": ["Ctrl+Y"] }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    // Console defaults should still be present
    assert_eq!(cfg.console_close, vec!["Escape"]);
    assert_eq!(cfg.console_clear_line, vec!["Ctrl+C"]);
    // Picker defaults should still be present
    assert_eq!(cfg.picker_nudge_hue_down, vec!["h"]);
}

#[test]
fn test_empty_binding_list_disables_action() {
    let json = r#"{ "exit_mode": [] }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "escape", false, false, false),
        None,
    );
}

#[test]
fn test_duplicate_key_in_same_context_first_wins() {
    let json = r#"{
        "console_close": ["Tab"],
        "console_tab_complete": ["Tab"]
    }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "tab", false, false, false),
        Some(Action::ConsoleClose),
    );
}

#[test]
fn test_action_for_context_document_filters_component_actions() {
    let resolved = KeybindConfig::default().resolve();
    // "tab" has no Document binding. action_for (global) returns
    // ConsoleTabComplete, but action_for_context(Document) returns None.
    assert_eq!(
        resolved.action_for("tab", false, false, false),
        Some(Action::ConsoleTabComplete),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "tab", false, false, false),
        None,
    );
}

#[test]
fn test_json_roundtrip_all_contexts() {
    let cfg = KeybindConfig::default();
    let json = serde_json::to_string(&cfg).unwrap();
    let parsed = KeybindConfig::from_json(&json).unwrap();
    let resolved = parsed.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "z", true, false, false),
        Some(Action::Undo),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "escape", false, false, false),
        Some(Action::ConsoleClose),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "h", false, false, false),
        Some(Action::PickerNudgeHueDown),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::LabelEdit, "enter", false, false, false),
        Some(Action::LabelEditCommit),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::TextEdit, "escape", false, false, false),
        Some(Action::TextEditCancel),
    );
}

// ─────────────────────────────────────────────────────────────────
// Parametric bindings (`ParametricBinding`)
// ─────────────────────────────────────────────────────────────────

#[test]
fn test_parametric_set_edge_anchor_resolves_with_two_args() {
    let cfg = KeybindConfig {
        set_edge_anchor: vec![ParametricBinding {
            combo: "Ctrl+Shift+a".into(),
            args: vec!["top".into(), "auto".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "a", true, true, false),
        Some(Action::SetEdgeAnchor {
            from: "top".into(),
            to: "auto".into(),
        }),
    );
}

#[test]
fn test_parametric_set_edge_body_glyph_resolves_with_one_arg() {
    let cfg = KeybindConfig {
        set_edge_body_glyph: vec![ParametricBinding {
            combo: "Ctrl+b".into(),
            args: vec!["dash".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "b", true, false, false),
        Some(Action::SetEdgeBodyGlyph("dash".into())),
    );
}

#[test]
fn test_parametric_wrong_arg_count_is_skipped() {
    // A 1-arg binding for a 2-arg variant — the build closure
    // returns None, the warn-log fires, no Action lands in the
    // resolved table. Crucially: not a panic, so a user-config
    // typo never crashes the app.
    //
    // The combo (Ctrl+F8) intentionally avoids the default-bound
    // chords so the assertion is about "no parametric Action got
    // built", not "the default got shadowed."
    let cfg = KeybindConfig {
        set_edge_anchor: vec![ParametricBinding {
            combo: "Ctrl+F8".into(),
            args: vec!["top".into()], // missing the `to` arg
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f8", true, false, false),
        None,
    );
}

#[test]
fn test_parametric_binding_round_trips_through_json() {
    let cfg = KeybindConfig {
        set_edge_anchor: vec![ParametricBinding {
            combo: "Ctrl+Shift+a".into(),
            args: vec!["top".into(), "auto".into()],
        }],
        set_edge_body_glyph: vec![ParametricBinding {
            combo: "Ctrl+b".into(),
            args: vec!["dash".into()],
        }],
        ..KeybindConfig::default()
    };
    let json = serde_json::to_string(&cfg).unwrap();
    let parsed = KeybindConfig::from_json(&json).unwrap();
    assert_eq!(parsed.set_edge_anchor, cfg.set_edge_anchor);
    assert_eq!(parsed.set_edge_body_glyph, cfg.set_edge_body_glyph);
}

#[test]
fn test_parametric_set_border_field_resolves_with_two_args() {
    let cfg = KeybindConfig {
        set_border_field: vec![ParametricBinding {
            combo: "Ctrl+Shift+b".into(),
            args: vec!["preset".into(), "rounded".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "b", true, true, false),
        Some(Action::SetBorderField {
            field: "preset".into(),
            value: "rounded".into(),
        }),
    );
}

#[test]
fn test_parametric_color_axes_resolve() {
    let cfg = KeybindConfig {
        set_color: vec![
            ParametricBinding {
                combo: "F1".into(),
                args: vec!["bg".into(), "#fafafa".into()],
            },
            ParametricBinding {
                combo: "F2".into(),
                args: vec!["text".into(), "accent".into()],
            },
            ParametricBinding {
                combo: "F3".into(),
                args: vec!["border".into(), "#000000".into()],
            },
        ],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f1", false, false, false),
        Some(Action::SetColor {
            axis: ColorAxis::Bg,
            value: "#fafafa".into(),
        }),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f2", false, false, false),
        Some(Action::SetColor {
            axis: ColorAxis::Text,
            value: "accent".into(),
        }),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f3", false, false, false),
        Some(Action::SetColor {
            axis: ColorAxis::Border,
            value: "#000000".into(),
        }),
    );
}

#[test]
fn test_parametric_edge_structural_resolve() {
    let cfg = KeybindConfig {
        set_edge_type: vec![ParametricBinding {
            combo: "F4".into(),
            args: vec!["cross_link".into()],
        }],
        set_edge_display_mode: vec![ParametricBinding {
            combo: "F5".into(),
            args: vec!["portal".into()],
        }],
        reset_edge: vec![ParametricBinding {
            combo: "F6".into(),
            args: vec!["style".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f4", false, false, false),
        Some(Action::SetEdgeType("cross_link".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f5", false, false, false),
        Some(Action::SetEdgeDisplayMode("portal".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f6", false, false, false),
        Some(Action::ResetEdge("style".into())),
    );
}

#[test]
fn test_parametric_font_family_size_resolve() {
    let cfg = KeybindConfig {
        set_font_family: vec![ParametricBinding {
            combo: "F7".into(),
            args: vec!["Norse".into()],
        }],
        set_font: vec![
            ParametricBinding {
                combo: "F8".into(),
                args: vec!["size".into(), "14".into()],
            },
            ParametricBinding {
                combo: "Ctrl+F8".into(),
                args: vec!["min".into(), "10".into()],
            },
            ParametricBinding {
                combo: "Shift+F8".into(),
                args: vec!["max".into(), "32".into()],
            },
        ],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f7", false, false, false),
        Some(Action::SetFontFamily("Norse".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f8", false, false, false),
        Some(Action::SetFont {
            slot: FontSlot::Size,
            value: "14".into(),
        }),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f8", true, false, false),
        Some(Action::SetFont {
            slot: FontSlot::Min,
            value: "10".into(),
        }),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f8", false, true, false),
        Some(Action::SetFont {
            slot: FontSlot::Max,
            value: "32".into(),
        }),
    );
}

#[test]
fn test_parametric_label_text_position_resolve() {
    let cfg = KeybindConfig {
        set_edge_label_text: vec![ParametricBinding {
            combo: "F9".into(),
            args: vec!["hello".into()],
        }],
        set_edge_label_position: vec![ParametricBinding {
            combo: "F10".into(),
            args: vec!["middle".into()],
        }],
        set_spacing: vec![ParametricBinding {
            combo: "F11".into(),
            args: vec!["wide".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f9", false, false, false),
        Some(Action::SetEdgeLabelText("hello".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f10", false, false, false),
        Some(Action::SetEdgeLabelPosition("middle".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f11", false, false, false),
        Some(Action::SetSpacing("wide".into())),
    );
}

#[test]
fn test_parametric_zoom_resolve_set_and_clear() {
    let cfg = KeybindConfig {
        set_zoom: vec![
            ParametricBinding {
                combo: "F12".into(),
                args: vec!["min".into(), "0.5".into()],
            },
            ParametricBinding {
                combo: "Ctrl+F12".into(),
                args: vec!["max".into(), "2.0".into()],
            },
        ],
        clear_zoom: vec![ParametricBinding {
            combo: "Shift+F12".into(),
            args: vec![],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f12", false, false, false),
        Some(Action::SetZoom {
            bound: ZoomBound::Min,
            value: "0.5".into(),
        }),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f12", true, false, false),
        Some(Action::SetZoom {
            bound: ZoomBound::Max,
            value: "2.0".into(),
        }),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f12", false, true, false),
        Some(Action::ClearZoom),
    );
}

#[test]
fn test_parametric_filesystem_variants_resolve() {
    let cfg = KeybindConfig {
        open_document: vec![ParametricBinding {
            combo: "Ctrl+F1".into(),
            args: vec!["/tmp/test.mindmap.json".into()],
        }],
        save_document_as: vec![ParametricBinding {
            combo: "Ctrl+F2".into(),
            args: vec!["/tmp/save.mindmap.json".into()],
        }],
        new_document_at: vec![ParametricBinding {
            combo: "Ctrl+F3".into(),
            args: vec!["/tmp/new.mindmap.json".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f1", true, false, false),
        Some(Action::OpenDocument("/tmp/test.mindmap.json".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f2", true, false, false),
        Some(Action::SaveDocumentAs("/tmp/save.mindmap.json".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f3", true, false, false),
        Some(Action::NewDocumentAt("/tmp/new.mindmap.json".into())),
    );
}

#[test]
fn test_parametric_clear_zoom_with_extra_args_is_skipped() {
    // ClearZoom is a unit variant — the builder closure rejects any
    // non-empty args slice. Confirms the unit-variant shape works
    // through the same `push_parametric` plumbing as payload variants.
    let cfg = KeybindConfig {
        clear_zoom: vec![ParametricBinding {
            combo: "Shift+F12".into(),
            args: vec!["unexpected".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f12", false, true, false),
        None,
    );
}

#[test]
fn test_parametric_set_edge_cap_resolves_with_two_args() {
    let cfg = KeybindConfig {
        set_edge_cap: vec![ParametricBinding {
            combo: "Ctrl+Shift+c".into(),
            args: vec!["arrow".into(), "none".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "c", true, true, false),
        Some(Action::SetEdgeCap {
            from: "arrow".into(),
            to: "none".into(),
        }),
    );
}

#[test]
fn test_parametric_binding_user_partial_config_only_overrides_listed_field() {
    // Confirm the `#[serde(default)]` shape works: a partial JSON
    // with only the parametric field set leaves every other binding
    // at its default.
    let json = r#"{
        "set_edge_body_glyph": [
            { "combo": "Ctrl+b", "args": ["dash"] }
        ]
    }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "b", true, false, false),
        Some(Action::SetEdgeBodyGlyph("dash".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "z", true, false, false),
        Some(Action::Undo),
    );
}
