//! Tests for the `CustomMutation` carrier — serde roundtrip,
//! backward-compat with pre-unification JSON, and the
//! `contexts` / `is_internal` / `targets_map` helpers.

use super::*;
use crate::gfx_structs::area::GlyphAreaCommand;
use crate::gfx_structs::mutator::Mutation;

fn sample(id: &str) -> CustomMutation {
    CustomMutation {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::NudgeRight(10.0),
        )])),
        target_scope: TargetScope::SelfOnly,
        behavior: MutationBehavior::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    }
}

#[test]
fn roundtrip_preserves_all_fields() {
    let mut cm = sample("nudge");
    cm.description = "Nudge every selected node one pixel right.".into();
    cm.contexts = vec![contexts::MAP_NODE.into()];
    let json = serde_json::to_string(&cm).unwrap();
    let back: CustomMutation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, "nudge");
    assert_eq!(back.description, cm.description);
    assert_eq!(back.contexts, cm.contexts);
    assert_eq!(back.target_scope, TargetScope::SelfOnly);
}

#[test]
fn empty_description_and_contexts_omitted_from_json() {
    let cm = sample("bare");
    let json = serde_json::to_string(&cm).unwrap();
    assert!(!json.contains("\"description\""));
    assert!(!json.contains("\"contexts\""));
}

#[test]
fn legacy_json_with_mutations_and_scope_loads_into_new_shape() {
    // Pre-unification shape: `mutations` + `target_scope`, no
    // `mutator` field. The backward-compat deserializer
    // synthesizes the MutatorNode via `scope` helpers so old maps
    // load unchanged.
    let legacy = r#"{
        "id": "old",
        "name": "Old",
        "mutations": [{"AreaCommand": {"NudgeRight": 5.0}}],
        "target_scope": "Descendants"
    }"#;
    let cm: CustomMutation = serde_json::from_str(legacy).unwrap();
    assert_eq!(cm.id, "old");
    assert_eq!(cm.target_scope, TargetScope::Descendants);
    // Synthesized MutatorNode: Instruction(RepeatWhile) wrapping Macro.
    match &cm.mutator {
        Some(crate::mutator_builder::MutatorNode::Instruction { .. }) => {}
        other => panic!("expected Instruction mutator, got {:?}", other),
    }
}

#[test]
fn legacy_json_with_only_document_actions_loads_with_no_mutator() {
    // `theme_demo.mindmap.json` shape: document_actions only.
    // `mutations` absent (serde-default) + `target_scope` present.
    // The synthesized mutator is None — no tree payload.
    let legacy = r#"{
        "id": "switch-dark",
        "name": "Switch to dark theme",
        "target_scope": "SelfOnly",
        "document_actions": [{"SetThemeVariant": "dark"}]
    }"#;
    let cm: CustomMutation = serde_json::from_str(legacy).unwrap();
    assert_eq!(cm.id, "switch-dark");
    assert!(cm.mutator.is_none());
    assert_eq!(cm.document_actions.len(), 1);
}

#[test]
fn matches_context_hits_exact_and_dotted_descendants() {
    let mut cm = sample("x");
    cm.contexts = vec![contexts::MAP_NODE.into(), contexts::MAP_TREE.into()];
    assert!(cm.matches_context(contexts::MAP));
    assert!(cm.matches_context(contexts::MAP_NODE));
    assert!(cm.matches_context(contexts::MAP_TREE));
    assert!(!cm.matches_context(contexts::INTERNAL));
}

#[test]
fn is_internal_is_true_on_empty_contexts() {
    let cm = sample("empty");
    assert!(cm.is_internal());
}

#[test]
fn is_internal_is_true_when_tag_present() {
    let mut cm = sample("tagged");
    cm.contexts = vec![contexts::INTERNAL.into(), contexts::MAP_NODE.into()];
    assert!(cm.is_internal());
    // targets_map also true because MAP_NODE is under MAP.
    assert!(cm.targets_map());
}

#[test]
fn targets_map_hits_any_map_sub_namespace() {
    let mut cm = sample("m");
    cm.contexts = vec![contexts::MAP_TREE.into()];
    assert!(cm.targets_map());
    assert!(!cm.is_internal());
}

#[test]
fn trigger_binding_roundtrip() {
    let binding = TriggerBinding {
        trigger: Trigger::OnKey("r".to_string()),
        mutation_id: "highlight-red".to_string(),
        contexts: vec![PlatformContext::Desktop, PlatformContext::Web],
    };
    let json = serde_json::to_string(&binding).unwrap();
    let back: TriggerBinding = serde_json::from_str(&json).unwrap();
    assert_eq!(back.trigger, Trigger::OnKey("r".to_string()));
    assert_eq!(back.contexts.len(), 2);
}

#[test]
fn document_action_set_theme_variant_roundtrip() {
    let action = DocumentAction::SetThemeVariant("dark".to_string());
    let json = serde_json::to_string(&action).unwrap();
    let back: DocumentAction = serde_json::from_str(&json).unwrap();
    assert_eq!(back, action);
}

#[test]
fn mutation_behavior_default_is_persistent() {
    // Absent `behavior` key in source JSON deserializes to
    // `Persistent` via the struct-level serde default, so maps
    // written before the field existed still load.
    let src = serde_json::json!({
        "id": "b",
        "name": "b",
        "mutator": { "Macro": { "channel": 0, "mutations": { "Literal": [] } } },
        "target_scope": "SelfOnly"
    })
    .to_string();
    let cm: CustomMutation = serde_json::from_str(&src).unwrap();
    assert_eq!(cm.behavior, MutationBehavior::Persistent);
}

#[test]
fn all_target_scopes_serialize() {
    for scope in [
        TargetScope::SelfOnly,
        TargetScope::Children,
        TargetScope::Descendants,
        TargetScope::SelfAndDescendants,
        TargetScope::Parent,
        TargetScope::Siblings,
    ] {
        let json = serde_json::to_string(&scope).unwrap();
        let back: TargetScope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, scope);
    }
}

#[test]
fn all_triggers_serialize() {
    for trigger in [
        Trigger::OnClick,
        Trigger::OnHover,
        Trigger::OnKey("space".to_string()),
        Trigger::OnLink("action:expand".to_string()),
    ] {
        let json = serde_json::to_string(&trigger).unwrap();
        let back: Trigger = serde_json::from_str(&json).unwrap();
        assert_eq!(back, trigger);
    }
}
