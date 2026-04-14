//! Unit tests for keybinds — parsing, matching, default config,
//! custom-mutation binding lifecycle, and JSON round-trip.

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

#[test]
fn test_default_config_has_all_actions() {
    let cfg = KeybindConfig::default();
    let resolved = cfg.resolve();
    assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
    assert_eq!(resolved.action_for("p", true, false, false), Some(Action::EnterReparentMode));
    assert_eq!(resolved.action_for("d", true, false, false), Some(Action::EnterConnectMode));
    assert_eq!(resolved.action_for("delete", false, false, false), Some(Action::DeleteSelection));
    assert_eq!(resolved.action_for("escape", false, false, false), Some(Action::CancelMode));
    assert_eq!(resolved.action_for("n", true, false, false), Some(Action::CreateOrphanNode));
    assert_eq!(resolved.action_for("o", true, false, false), Some(Action::OrphanSelection));
    assert_eq!(resolved.action_for("enter", false, false, false), Some(Action::EditSelection));
    assert_eq!(resolved.action_for("backspace", false, false, false), Some(Action::EditSelectionClean));
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
    assert_eq!(resolved.custom_mutation_for("m", true, false, false), Some("valid"));
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
    assert_eq!(
        resolved.custom_mutation_for("m", true, true, false),
        None
    );
}

#[test]
fn test_keybind_string_round_trip_through_parse() {
    let cases = &[
        "Ctrl+Z",
        "Ctrl+Shift+M",
        "Alt+F4",
        "Shift+Enter",
        "Escape",
    ];
    for c in cases {
        let parsed = KeyBind::parse(c).unwrap();
        let rendered = parsed.to_binding_string();
        let reparsed = KeyBind::parse(&rendered).unwrap();
        assert_eq!(parsed, reparsed, "round-trip failed for '{}'", c);
    }
}

#[test]
fn test_default_console_font_size_is_16() {
    let cfg = KeybindConfig::default();
    assert!((cfg.console_font_size - 16.0).abs() < f32::EPSILON);
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
    assert!((r.console_font_size - 20.0).abs() < f32::EPSILON);
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
fn test_save_document_default_bound_to_ctrl_s() {
    let cfg = KeybindConfig::default();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for("s", true, false, false),
        Some(Action::SaveDocument)
    );
}

#[test]
fn test_default_config_has_undo_alias() {
    // Ctrl+Z and the bare "Undo" key should both fire undo
    let cfg = KeybindConfig::default();
    let resolved = cfg.resolve();
    assert_eq!(resolved.action_for("undo", false, false, false), Some(Action::Undo));
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
    assert_eq!(cfg.cancel_mode, vec!["Escape"]);
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
