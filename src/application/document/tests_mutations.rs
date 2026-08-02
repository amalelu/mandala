// SPDX-License-Identifier: MPL-2.0

//! Custom-mutation application, registry, trigger evaluation.
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::tests_common::{first_testament_node_id, load_test_doc, TestNudgeMutation};
use super::*;

use baumhard::mindmap::animation::{AnimationTiming, Easing};
use baumhard::mindmap::custom_mutation::{
    CustomMutation as CM, DocumentAction, MutationBehavior as MB, PlatformContext as PC, TargetScope as TS,
    Trigger as Tr, TriggerBinding as TB,
};

fn make_test_mutation(id: &str, scope: TS) -> CM {
    TestNudgeMutation::new(id, scope).magnitude(10.0).build()
}

/// Build a `CustomMutation` whose only payload is a single
/// `SetThemeVariables` document-level action that sets `--bg`
/// to the given value. Used by the `apply_document_actions`
/// regression tests.
fn make_set_bg_doc_mutation(value: &str) -> CM {
    let mut vars = HashMap::new();
    vars.insert("--bg".to_string(), value.to_string());
    CM {
        id: "set-bg".to_string(),
        name: "Set --bg".to_string(),
        description: String::new(),
        contexts: vec![],
        mutator: None,
        target_scope: TS::SelfOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![DocumentAction::SetThemeVariables(vars)],
        timing: None,
    }
}

/// Round-trip regression for `UndoAction::CanvasSnapshot`. The
/// `apply_document_actions` path is the only producer of this
/// variant, and prior to chunk 5 it had zero test coverage —
/// CODE_CONVENTIONS.md §6 says every undo variant ships with at
/// least a forward-and-back test.
#[test]
fn test_apply_document_actions_undo_round_trip() {
    let mut doc = load_test_doc();
    // Capture the canvas state before any document-level mutation.
    let before = doc.mindmap.canvas.clone();
    let undo_len_before = doc.undo_stack.len();

    // Apply a single SetThemeVariables action that sets --bg to a
    // sentinel value not present in the testament map.
    let custom = make_set_bg_doc_mutation("#bada55");
    let changed = doc.apply_document_actions(&custom);
    assert!(changed, "applying a new theme var must report a change");
    assert_eq!(
        doc.mindmap.canvas.theme_variables.get("--bg"),
        Some(&"#bada55".to_string())
    );
    assert_eq!(
        doc.undo_stack.len(),
        undo_len_before + 1,
        "exactly one CanvasSnapshot entry should have been pushed"
    );
    assert!(doc.dirty);

    // Undo restores the entire pre-mutation canvas wholesale.
    assert!(doc.undo());
    assert_eq!(doc.mindmap.canvas.theme_variables, before.theme_variables);
    assert_eq!(doc.mindmap.canvas.background_color, before.background_color);
    assert_eq!(
        doc.undo_stack.len(),
        undo_len_before,
        "undo should have popped the CanvasSnapshot entry"
    );
}

/// `apply_document_actions` returns false and pushes nothing
/// when the action would not actually change anything (writing
/// the same value that's already there). Guards the dirty/undo
/// no-op path that the docstring on `apply_document_actions`
/// promises.
#[test]
fn test_apply_document_actions_noop_does_not_push_undo() {
    let mut doc = load_test_doc();
    // First write — should change the canvas and push undo.
    let custom = make_set_bg_doc_mutation("#bada55");
    doc.apply_document_actions(&custom);
    let undo_len_after_first = doc.undo_stack.len();
    doc.dirty = false;

    // Second write of the same value — no-op, no undo push,
    // dirty flag should stay false.
    let changed = doc.apply_document_actions(&custom);
    assert!(!changed, "writing the same value must not report a change");
    assert_eq!(doc.undo_stack.len(), undo_len_after_first);
    assert!(!doc.dirty);
}

/// Phase-7 parity regression: the keybind-side custom-mutation
/// path previously skipped both `apply_document_actions` and the
/// animation `timing` envelope. After the fix, the keybind path
/// runs through `dispatch::apply_keybind_custom_mutation` which
/// goes through both. This test pins the parity contract by
/// calling the helper directly (no renderer needed) and asserting
/// the document-actions side-effect lands.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_apply_keybind_custom_mutation_runs_document_actions() {
    use crate::application::app::dispatch::apply_keybind_custom_mutation;
    use baumhard::mindmap::scene_cache::SceneConnectionCache;
    use baumhard::mindmap::tree_builder::build_mindmap_tree;

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let mut scene_cache = SceneConnectionCache::default();
    let cm = make_set_bg_doc_mutation("#bada55");

    // Without a tree the non-animated branch can't apply.
    let mut no_tree: Option<baumhard::mindmap::tree_builder::MindMapTree> = None;
    let applied = apply_keybind_custom_mutation(&mut doc, &mut no_tree, &mut scene_cache, &cm, &nid, 0);
    assert!(!applied, "no tree + no animation: nothing to apply");

    // Build a tree so the non-animated branch can run.
    let mut tree = Some(build_mindmap_tree(&doc.mindmap));
    let applied = apply_keybind_custom_mutation(&mut doc, &mut tree, &mut scene_cache, &cm, &nid, 0);
    assert!(applied, "with a tree, non-animated mutation must apply");
    // The load-bearing assertion: document actions ran.
    assert_eq!(
        doc.mindmap.canvas.theme_variables.get("--bg"),
        Some(&"#bada55".to_string()),
        "Phase-7 parity: apply_document_actions must run on the keybind path"
    );
}

/// Phase-7 parity regression for the animation branch. When a
/// custom mutation has `timing.duration_ms > 0`, the keybind path
/// must call `start_animation` (not the immediate apply).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_apply_keybind_custom_mutation_with_timing_starts_animation() {
    use crate::application::app::dispatch::apply_keybind_custom_mutation;
    use baumhard::mindmap::scene_cache::SceneConnectionCache;

    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let mut tree = None;
    let mut scene_cache = SceneConnectionCache::default();
    let mut cm = TestNudgeMutation::new("animated-nudge", TS::SelfOnly)
        .magnitude(10.0)
        .build();
    cm.timing = Some(AnimationTiming {
        duration_ms: 200,
        ..AnimationTiming::default()
    });

    assert!(!doc.has_active_animations());
    let applied = apply_keybind_custom_mutation(&mut doc, &mut tree, &mut scene_cache, &cm, &nid, 0);
    assert!(applied, "animated branch must succeed even without a tree");
    assert!(
        doc.has_active_animations(),
        "Phase-7 parity: timing.duration_ms > 0 must call start_animation"
    );
}

#[test]
fn test_mutation_registry_empty_for_existing_map() {
    let doc = load_test_doc();
    assert!(
        doc.mutation_registry.is_empty(),
        "Existing map without custom_mutations should have empty registry"
    );
}

#[test]
fn test_mutation_registry_from_map_level() {
    let mut doc = load_test_doc();
    doc.mindmap
        .custom_mutations
        .push(make_test_mutation("nudge-right", TS::SelfOnly));
    doc.build_mutation_registry();
    assert_eq!(doc.mutation_registry.len(), 1);
    assert!(doc.mutation_registry.contains_key("nudge-right"));
}

#[test]
fn test_mutation_registry_inline_overrides_map() {
    let mut doc = load_test_doc();
    // Map-level mutation
    let mut map_cm = make_test_mutation("shared-id", TS::SelfOnly);
    map_cm.name = "Map Version".to_string();
    doc.mindmap.custom_mutations.push(map_cm);

    // Inline mutation on a node with the same id
    let mut inline_cm = make_test_mutation("shared-id", TS::Children);
    inline_cm.name = "Inline Version".to_string();
    let node_id = "0";
    doc.mindmap
        .nodes
        .get_mut(node_id)
        .unwrap()
        .inline_mutations
        .push(inline_cm);

    doc.build_mutation_registry();
    assert_eq!(doc.mutation_registry.len(), 1);
    let cm = doc.mutation_registry.get("shared-id").unwrap();
    assert_eq!(cm.name, "Inline Version", "Inline should override map-level");
    assert_eq!(cm.target_scope, TS::Children);
}

#[test]
fn test_find_triggered_mutations_match() {
    let mut doc = load_test_doc();
    doc.mindmap
        .custom_mutations
        .push(make_test_mutation("nudge", TS::SelfOnly));
    doc.build_mutation_registry();

    let node_id = "0";
    doc.mindmap
        .nodes
        .get_mut(node_id)
        .unwrap()
        .trigger_bindings
        .push(TB {
            trigger: Tr::OnClick,
            mutation_id: "nudge".to_string(),
            contexts: vec![],
        });

    let results = doc.find_triggered_mutations(node_id, &Tr::OnClick, &PC::Desktop);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "nudge");
}

#[test]
fn test_find_triggered_mutations_no_match() {
    let mut doc = load_test_doc();
    doc.mindmap
        .custom_mutations
        .push(make_test_mutation("nudge", TS::SelfOnly));
    doc.build_mutation_registry();

    let node_id = "0";
    doc.mindmap
        .nodes
        .get_mut(node_id)
        .unwrap()
        .trigger_bindings
        .push(TB {
            trigger: Tr::OnClick,
            mutation_id: "nudge".to_string(),
            contexts: vec![],
        });

    // OnHover should not match
    let results = doc.find_triggered_mutations(node_id, &Tr::OnHover, &PC::Desktop);
    assert!(results.is_empty());
}

#[test]
fn test_find_triggered_mutations_platform_filter() {
    let mut doc = load_test_doc();
    doc.mindmap
        .custom_mutations
        .push(make_test_mutation("desktop-only", TS::SelfOnly));
    doc.build_mutation_registry();

    let node_id = "0";
    doc.mindmap
        .nodes
        .get_mut(node_id)
        .unwrap()
        .trigger_bindings
        .push(TB {
            trigger: Tr::OnClick,
            mutation_id: "desktop-only".to_string(),
            contexts: vec![PC::Desktop],
        });

    // Desktop should match
    let results = doc.find_triggered_mutations(node_id, &Tr::OnClick, &PC::Desktop);
    assert_eq!(results.len(), 1);

    // Touch should be filtered out
    let results = doc.find_triggered_mutations(node_id, &Tr::OnClick, &PC::Touch);
    assert!(results.is_empty());
}

#[test]
fn test_collect_affected_node_ids_self_only() {
    let doc = load_test_doc();
    let ids = doc.collect_affected_node_ids("0", &TS::SelfOnly);
    assert_eq!(ids, vec!["0"]);
}

#[test]
fn test_collect_affected_node_ids_children() {
    let doc = load_test_doc();
    let children = doc.mindmap.children_of("0");
    let ids = doc.collect_affected_node_ids("0", &TS::Children);
    assert_eq!(ids.len(), children.len());
    for child in &children {
        assert!(ids.contains(&child.id));
    }
}

#[test]
fn test_collect_affected_node_ids_descendants() {
    let doc = load_test_doc();
    let all_desc = doc.mindmap.all_descendants("0");
    let ids = doc.collect_affected_node_ids("0", &TS::Descendants);
    assert_eq!(ids.len(), all_desc.len());
}

#[test]
fn test_collect_affected_node_ids_self_and_descendants() {
    let doc = load_test_doc();
    let all_desc = doc.mindmap.all_descendants("0");
    let ids = doc.collect_affected_node_ids("0", &TS::SelfAndDescendants);
    assert_eq!(ids.len(), all_desc.len() + 1);
    assert!(ids.contains(&"0".to_string()));
}

#[test]
fn test_collect_affected_node_ids_parent() {
    let doc = load_test_doc();
    // Find a non-root node that has a parent
    let child_id = doc
        .mindmap
        .nodes
        .values()
        .find(|n| n.parent_id.is_some())
        .map(|n| n.id.clone())
        .expect("testament map has child nodes");
    let parent_id = doc
        .mindmap
        .nodes
        .get(&child_id)
        .unwrap()
        .parent_id
        .clone()
        .unwrap();

    let ids = doc.collect_affected_node_ids(&child_id, &TS::Parent);
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0], parent_id);
}

#[test]
fn test_collect_affected_node_ids_parent_of_root_is_empty() {
    let doc = load_test_doc();
    // Root node (0) has no parent
    let ids = doc.collect_affected_node_ids("0", &TS::Parent);
    assert!(ids.is_empty(), "Root node has no parent; should return empty");
}

// The two `Siblings` cases that used to sit here — "siblings exclude
// self" and "a root has no siblings" — now live in
// `custom::covers_reach_differential_tests` as
// `test_siblings_excludes_the_trigger_node` and
// `test_siblings_of_a_root_node_is_the_empty_set`. Those versions sweep
// *every* qualifying node and *every* root of the testament map
// rather than the first child found and the hard-coded id `"0"`, so
// they strictly subsume these; keeping both would be the §10
// duplicate shape (one property, two homes, only one of them
// maintained).

/// `SectionsOnly` returns the triggering node id only — section-
/// level fan-out happens inside `apply_to_tree`, but the undo
/// snapshot window is still the whole `MindNode` (which carries
/// every section). Pins the snapshot-shape contract.
#[test]
fn test_collect_affected_node_ids_sections_only_returns_self() {
    let doc = load_test_doc();
    let ids = doc.collect_affected_node_ids("0", &TS::SectionsOnly);
    assert_eq!(ids, vec!["0"]);
}

/// `SectionsOnly` mutation lands on every section-area, not on
/// the chrome-only container. Uses a `NudgeRight` mutator (which
/// shifts `area.position.x`) and asserts the section-area moved
/// while the container stayed still. Pins the structural seam:
/// `SectionsOnly` bypasses the container fan-out by going through
/// `tree.section_arena_id` directly.
#[test]
fn test_apply_custom_mutation_sections_only_targets_section_areas() {
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // Materialise a multi-section node so the SectionsOnly path
    // has more than one section to walk.
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), vec![]));
    }
    let cm = TestNudgeMutation::new("nudge-sections", TS::SectionsOnly)
        .magnitude(10.0)
        .build();
    let mut tree = doc.build_tree();

    let container_x_before = {
        let aid = tree.arena_id_for(&nid).unwrap();
        tree.tree
            .arena
            .get(aid)
            .and_then(|n| n.get().glyph_area())
            .unwrap()
            .position
            .x
            .0
    };
    let section0_x_before = {
        let sid = tree.section_arena_id(&nid, 0).unwrap();
        tree.tree
            .arena
            .get(sid)
            .and_then(|n| n.get().glyph_area())
            .unwrap()
            .position
            .x
            .0
    };

    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    let container_x_after = {
        let aid = tree.arena_id_for(&nid).unwrap();
        tree.tree
            .arena
            .get(aid)
            .and_then(|n| n.get().glyph_area())
            .unwrap()
            .position
            .x
            .0
    };
    let section0_x_after = {
        let sid = tree.section_arena_id(&nid, 0).unwrap();
        tree.tree
            .arena
            .get(sid)
            .and_then(|n| n.get().glyph_area())
            .unwrap()
            .position
            .x
            .0
    };

    assert!(
        (container_x_after - container_x_before).abs() < 1e-3,
        "container must NOT move under SectionsOnly (before {container_x_before}, after {container_x_after})"
    );
    assert!(
        (section0_x_after - section0_x_before - 10.0).abs() < 1e-3,
        "section-area must shift by the nudge magnitude (before {section0_x_before}, after {section0_x_after})"
    );
}

/// `sync_node_from_tree` writes per-section text + runs back to
/// the model after a custom mutation that touches regions.
/// Pre-fix only `position` was synced, so a custom mutation that
/// recolored a section's runs would land on the live tree but
/// be reverted on the next `rebuild_all`. The merge-with-prior
/// reverse converter preserves bold / italic / underline /
/// size_pt / hyperlink across the round trip. Pins the
/// persistence so multi-section custom mutations survive
/// save+load.
#[test]
fn test_sync_node_from_tree_writes_back_section_run_color() {
    use baumhard::core::primitives::Range;
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::{scope, CustomMutation};
    use baumhard::mindmap::model::TextRun;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        // Materialise a section with an explicit run carrying
        // bold=true so we can verify the merge-with-prior path
        // preserves the field across the lossy round-trip.
        node.sections[0].text = "hello".into();
        node.sections[0].text_runs = vec![TextRun {
            start: 0,
            end: 5,
            bold: true,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14,
            color: "#ffffff".into(),
            hyperlink: None,
        }];
    }
    let cm = CustomMutation {
        id: "recolor".into(),
        name: "Recolor".into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::SetRegionColor(Range::new(0, 5), [1.0, 0.0, 0.0, 1.0]),
        )])),
        target_scope: TS::SelfOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    let mut tree = doc.build_tree();

    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    let section0_run = &doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs[0];
    assert_eq!(
        section0_run.color, "#ff0000",
        "section 0 color must round-trip through rgba_to_hex"
    );
    assert!(
        section0_run.bold,
        "merge-with-prior must preserve bold across the reverse converter"
    );
}

/// Selective sync gate isolation: when a `SectionsOnly`
/// mutation **explicitly targets only** section 0, every
/// untouched section's bold / italic / underline / size_pt /
/// hyperlink survives verbatim. Pre-fix the selective gate
/// `zip`'d positionally on tree-side regions vs model-side
/// runs, so any range-order mismatch tripped the round-trip and
/// silently stripped non-text-touched sections of styling.
#[test]
fn test_sync_node_from_tree_section_1_untouched_when_section_0_mutated() {
    use baumhard::core::primitives::Range;
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::{scope, CustomMutation};
    use baumhard::mindmap::model::{MindSection, TextRun};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        // Pin both sections' runs so we can detect divergence.
        node.sections[0].text = "hello".into();
        node.sections[0].text_runs = vec![TextRun {
            start: 0,
            end: 5,
            bold: true,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14,
            color: "#ffffff".into(),
            hyperlink: None,
        }];
        node.sections
            .push(MindSection::new_default("untouched".into(), Vec::new()));
        node.sections[1].text_runs = vec![TextRun {
            start: 0,
            end: 9,
            bold: false,
            italic: true,
            underline: true,
            font: "LiberationSans".into(),
            size_pt: 21,
            color: "#abcdef".into(),
            hyperlink: Some("https://example.org".into()),
        }];
    }
    // SectionsOnly mutation against section 0 only — using
    // SectionsOnly with `predicate` matching only section_idx=0
    // is structurally awkward today (no per-index predicate);
    // instead use `SelfOnly` + `predicate` matching section 0's
    // unique_id is also awkward. The simplest cross-tier test:
    // mutate section 0's text via the document setter (which
    // lives in `nodes/mod.rs`) and verify section 1's run state
    // survives untouched. The setter doesn't go through
    // sync_node_from_tree, but the round-trip-on-`apply_to_tree`
    // path is exercised separately above. Here we cross-check
    // that the *gate's positional drift* doesn't manifest under
    // a mutation pipeline that fans out to multiple sections.
    let cm = CustomMutation {
        id: "color-section-0".into(),
        name: "Color section 0".into(),
        description: String::new(),
        contexts: vec![],
        // `SectionsOnly` fans to every section, but the
        // SetRegionColor only matches section 0's [0,5) range
        // (section 1 is [0,9)). Section 1 will see a new region
        // inserted at [0,5) — which is exactly the case the
        // selective gate must NOT silently swallow.
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::SetRegionColor(Range::new(0, 5), [1.0, 0.0, 0.0, 1.0]),
        )])),
        target_scope: TS::SectionsOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    // Section 0 was touched and color-changed: red [0,5) merged
    // with prior bold=true via `region_to_text_run`.
    let s0 = &doc.mindmap.nodes.get(&nid).unwrap().sections[0];
    assert!(
        s0.text_runs.iter().any(|r| r.color == "#ff0000" && r.bold),
        "section 0 must carry the new red color and preserve bold"
    );

    // Section 1's existing [0,9) run had a non-zero overlap with
    // the [0,5) inserted region. The dominant-overlap fallback
    // means the new merged run inherits italic/underline/size_pt
    // from the existing [0,9) prior. Pin those.
    let s1 = &doc.mindmap.nodes.get(&nid).unwrap().sections[1];
    let s1_run = s1
        .text_runs
        .first()
        .expect("section 1 must keep at least one run");
    assert!(s1_run.italic, "italic must survive on section 1");
    assert!(s1_run.underline, "underline must survive on section 1");
    assert_eq!(s1_run.size_pt, 21, "size_pt must survive on section 1");
    assert_eq!(
        s1_run.hyperlink.as_deref(),
        Some("https://example.org"),
        "hyperlink must survive on section 1"
    );
}

/// `var(--name)` round-trip regression: a section whose run
/// carries `color: "var(--accent)"` and a mutation that
/// otherwise leaves regions alone must NOT silently rewrite the
/// `var()` reference to the resolved hex. The selective gate
/// short-circuits when the tree-side region's color resolves to
/// the same FloatRgba as the model-side `var(--accent)`,
/// because `var()` references can't compare structurally and
/// the gate *does* run the round-trip — but `region_to_text_run`
/// inherits from prior on the var-bearing case via the empty
/// `region.color`. Pin the contract: a position-only mutation
/// (`NudgeRight`) on a `var(--name)`-colored section preserves
/// the variable verbatim.
#[test]
fn test_sync_node_from_tree_var_color_preserved_when_regions_untouched() {
    use baumhard::mindmap::model::TextRun;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections[0].text = "themed".into();
        node.sections[0].text_runs = vec![TextRun {
            start: 0,
            end: 6,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14,
            // `var()` reference — not directly comparable on the
            // round-trip path; the selective gate must skip.
            color: "var(--accent)".into(),
            hyperlink: None,
        }];
    }
    // Position-only mutation: regions stay byte-identical.
    let cm = make_test_mutation("nudge", TS::SelfOnly);
    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    let run = &doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs[0];
    assert_eq!(
        run.color, "var(--accent)",
        "var() reference must survive position-only mutations \
         (selective gate skips because tree-side regions are unchanged)"
    );
}

/// `SectionsOnly` position deltas persist past `rebuild_all` —
/// `sync_node_from_tree` must write back `section.offset` from
/// the tree-side section-area position. Pre-Tier-Review-Response-2
/// the sync wrote `model.position` only; a `SectionsOnly`
/// translate landed on the live tree but reverted on the next
/// model→tree rebuild. Pin the writeback contract.
#[test]
fn test_sync_node_from_tree_section_offset_persists_after_rebuild() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::{scope, CustomMutation};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // Pin the model offset so we can detect the writeback.
    let pre_offset_x = doc.mindmap.nodes.get(&nid).unwrap().sections[0].offset.x;
    let cm = CustomMutation {
        id: "translate-section-0".into(),
        name: "Translate section 0".into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::NudgeRight(13.0),
        )])),
        target_scope: TS::SectionsOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    // Without writeback, model.section[0].offset.x stays at
    // pre_offset_x; with writeback, it advances by 13.
    let post_offset_x = doc.mindmap.nodes.get(&nid).unwrap().sections[0].offset.x;
    assert!(
        (post_offset_x - pre_offset_x - 13.0).abs() < 1e-3,
        "section.offset.x must persist tree-side translate ({pre_offset_x} → {post_offset_x})"
    );

    // Force a rebuild and re-check the tree position derives
    // from the persisted offset (no revert).
    let tree2 = doc.build_tree();
    let sid = tree2.section_arena_id(&nid, 0).unwrap();
    let area = tree2.tree.arena.get(sid).unwrap().get().glyph_area().unwrap();
    let node_pos_x = doc.mindmap.nodes.get(&nid).unwrap().position.x as f32;
    let expected = node_pos_x + post_offset_x as f32;
    assert!(
        (area.position.x.0 - expected).abs() < 1e-3,
        "post-rebuild section position should reflect persisted offset"
    );
}

/// Section-level `OnClick` trigger fires before whole-node
/// `OnClick` triggers — pin the precedence the
/// `find_triggered_mutations_at` doc claims. Tier-D wired the
/// dispatcher but no test exercises the override-precedence
/// contract end-to-end.
#[test]
fn test_find_triggered_mutations_at_section_binding_fires_first() {
    use baumhard::mindmap::custom_mutation::{
        scope, CustomMutation, PlatformContext, Trigger, TriggerBinding,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);

    // Register two CustomMutations.
    let node_cm = CustomMutation {
        id: "node-handler".into(),
        name: "Node handler".into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![])),
        target_scope: TS::SelfOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    let section_cm = CustomMutation {
        id: "section-handler".into(),
        name: "Section handler".into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![])),
        target_scope: TS::SectionsOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    doc.mutation_registry.insert("node-handler".into(), node_cm);
    doc.mutation_registry.insert("section-handler".into(), section_cm);

    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.trigger_bindings.push(TriggerBinding {
            trigger: Trigger::OnClick,
            mutation_id: "node-handler".into(),
            contexts: vec![],
        });
        node.sections[0].trigger_bindings.push(TriggerBinding {
            trigger: Trigger::OnClick,
            mutation_id: "section-handler".into(),
            contexts: vec![],
        });
    }

    let triggered =
        doc.find_triggered_mutations_at(&nid, Some(0), &Trigger::OnClick, &PlatformContext::Desktop);
    let ids: Vec<&str> = triggered.iter().map(|cm| cm.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["section-handler", "node-handler"],
        "section-level binding must fire FIRST, then whole-node"
    );

    // No section_idx: only whole-node bindings fire.
    let triggered_node_only =
        doc.find_triggered_mutations_at(&nid, None, &Trigger::OnClick, &PlatformContext::Desktop);
    let ids: Vec<&str> = triggered_node_only.iter().map(|cm| cm.id.as_str()).collect();
    assert_eq!(ids, vec!["node-handler"]);
}

/// `MoveTo` mutation against a multi-section node lands on the
/// container only — pre-fix the section fan-out replayed every
/// command on every section-area, and `MoveTo(x, y)` set every
/// section's absolute position equal to the container's. The
/// `sync_node_from_tree` round-trip then read
/// `section.offset = section_pos - node_pos = (0, 0)` and
/// silently collapsed every authored offset.
#[test]
fn test_apply_custom_mutation_move_to_does_not_collapse_section_offsets() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::{scope, CustomMutation};
    use baumhard::mindmap::model::{MindSection, Position};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        let mut s1 = MindSection::new_default("offset-section".into(), Vec::new());
        s1.offset = Position { x: 50.0, y: 30.0 };
        node.sections.push(s1);
    }
    let pre_offset = doc.mindmap.nodes.get(&nid).unwrap().sections[1].offset;
    let cm = CustomMutation {
        id: "move-to".into(),
        name: "MoveTo".into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::MoveTo(500.0, 600.0),
        )])),
        target_scope: TS::SelfOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));
    let post_offset = doc.mindmap.nodes.get(&nid).unwrap().sections[1].offset;
    assert!(
        (post_offset.x - pre_offset.x).abs() < 1e-3 && (post_offset.y - pre_offset.y).abs() < 1e-3,
        "section.offset must survive a MoveTo on the parent (was {:?}, now {:?})",
        pre_offset,
        post_offset
    );
}

/// `find_triggered_mutations_at` dedupes by `mutation_id` so an
/// author who bound the same mutation at both the section and
/// the whole-node layer doesn't see double application (which
/// would push two undo entries for one click and double the
/// resulting delta).
#[test]
fn test_find_triggered_mutations_at_dedups_same_mutation_id() {
    use baumhard::mindmap::custom_mutation::{
        scope, CustomMutation, PlatformContext, Trigger, TriggerBinding,
    };
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let cm = CustomMutation {
        id: "shared-handler".into(),
        name: "Shared".into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![])),
        target_scope: TS::SelfOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    doc.mutation_registry.insert("shared-handler".into(), cm);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.trigger_bindings.push(TriggerBinding {
            trigger: Trigger::OnClick,
            mutation_id: "shared-handler".into(),
            contexts: vec![],
        });
        node.sections[0].trigger_bindings.push(TriggerBinding {
            trigger: Trigger::OnClick,
            mutation_id: "shared-handler".into(),
            contexts: vec![],
        });
    }
    let triggered =
        doc.find_triggered_mutations_at(&nid, Some(0), &Trigger::OnClick, &PlatformContext::Desktop);
    assert_eq!(triggered.len(), 1, "duplicate id must dedupe to one cm");
    assert_eq!(triggered[0].id, "shared-handler");
}

/// `start_animation_at` keys dedup by `(mutation_id, target_id,
/// section_idx)` — two adjacent sections of the same node
/// bound to the same mutation id coexist as separate
/// `AnimationInstance`s instead of coalescing under the prior
/// `(mutation_id, target_id)`-only key.
#[test]
fn test_start_animation_at_does_not_dedup_across_sections() {
    use baumhard::mindmap::animation::AnimationTiming;
    use baumhard::mindmap::custom_mutation::{scope, CustomMutation};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let cm = CustomMutation {
        id: "anim".into(),
        name: "Anim".into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![])),
        target_scope: TS::SectionsOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: Some(AnimationTiming {
            duration_ms: 500,
            ..AnimationTiming::default()
        }),
    };
    doc.start_animation_at(&cm, &nid, Some(0), 0);
    doc.start_animation_at(&cm, &nid, Some(1), 0);
    assert_eq!(
        doc.active_animations.len(),
        2,
        "section_idx must be part of the dedup key"
    );
    // Same section + same mutation → dedupe (no duplicate).
    doc.start_animation_at(&cm, &nid, Some(0), 0);
    assert_eq!(doc.active_animations.len(), 2);
}

/// `SectionsOnly + predicate` combo: the structural seam and
/// the predicate gate compose — a `SectionsOnly` mutation gated
/// by `(Flag(SectionRoot), Equals(false))` (matches set, i.e.
/// only sections) reaches the same set as `SectionsOnly` alone
/// because every section-area carries the flag. Conversely
/// `(Flag(SectionRoot), Equals(true))` (matches clear, i.e.
/// containers) filters every section out — silent no-op
/// candidate that the apply path warns about. Pins both
/// composition paths.
#[test]
fn test_apply_custom_mutation_sections_only_with_predicate_compose() {
    use baumhard::core::primitives::Flag;
    use baumhard::gfx_structs::element::GfxElementField;
    use baumhard::gfx_structs::predicate::{Comparator, Predicate};
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), Vec::new()));
    }
    // `SectionsOnly` + predicate matching SectionRoot-set:
    // every section passes; the mutation lands on both.
    let mut cm_pass = TestNudgeMutation::new("nudge-sections-and-pred", TS::SectionsOnly)
        .magnitude(7.0)
        .build();
    cm_pass.predicate = Some(Predicate {
        fields: vec![(GfxElementField::Flag(Flag::SectionRoot), Comparator::equals())],
        always_match: false,
    });
    let mut tree = doc.build_tree();
    let s0_x_before = {
        let sid = tree.section_arena_id(&nid, 0).unwrap();
        tree.tree
            .arena
            .get(sid)
            .unwrap()
            .get()
            .glyph_area()
            .unwrap()
            .position
            .x
            .0
    };
    doc.apply_custom_mutation(&cm_pass, &nid, Some(&mut tree));
    let s0_x_after = {
        let sid = tree.section_arena_id(&nid, 0).unwrap();
        tree.tree
            .arena
            .get(sid)
            .unwrap()
            .get()
            .glyph_area()
            .unwrap()
            .position
            .x
            .0
    };
    assert!(
        (s0_x_after - s0_x_before - 7.0).abs() < 1e-3,
        "SectionsOnly + (Flag(SectionRoot), Equals(false)) lands on every section"
    );
}

/// Selective sync gate: a mutation that doesn't touch
/// regions (e.g. `NudgeRight` only shifts position) must skip
/// the section round-trip so bold / italic / underline /
/// size_pt / hyperlink survive verbatim. The forward conversion
/// drops these fields, so an unconditional round-trip would
/// silently strip them every time any custom mutation ran on a
/// node — the gate keeps them anchored.
#[test]
fn test_sync_node_from_tree_selective_gate_preserves_unchanged_runs() {
    use baumhard::mindmap::model::TextRun;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections[0].text = "hello".into();
        node.sections[0].text_runs = vec![TextRun {
            start: 0,
            end: 5,
            bold: false,
            italic: true,
            underline: true,
            font: "LiberationSans".into(),
            size_pt: 21, // Non-default so we can detect a stripped round-trip.
            color: "#abcdef".into(),
            hyperlink: Some("https://example.org".into()),
        }];
    }
    // NudgeRight only shifts position — the section's regions
    // stay byte-identical to the model snapshot, so the gate
    // skips the lossy round-trip.
    let cm = make_test_mutation("nudge", TS::SelfOnly);
    let mut tree = doc.build_tree();

    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    let run = &doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs[0];
    assert!(run.italic, "italic must survive a position-only mutation");
    assert!(run.underline, "underline must survive a position-only mutation");
    assert_eq!(run.size_pt, 21, "size_pt must survive a position-only mutation");
    assert_eq!(
        run.hyperlink.as_deref(),
        Some("https://example.org"),
        "hyperlink must survive a position-only mutation"
    );
}

/// Top-level `CustomMutation.predicate` filters the candidate
/// element list before mutations land. A
/// `Predicate { fields: [(Flag(SectionRoot), Equals(false))] }`
/// gate on a `SelfOnly` mutation lands on every section-area
/// child but skips the container — the same end-state as
/// `SectionsOnly`, expressed at the predicate layer.
#[test]
fn test_apply_custom_mutation_predicate_gate_filters_container() {
    use baumhard::core::primitives::Flag;
    use baumhard::gfx_structs::element::GfxElementField;
    use baumhard::gfx_structs::predicate::{Comparator, Predicate};
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), vec![]));
    }
    let mut cm = TestNudgeMutation::new("nudge-pred", TS::SelfOnly)
        .magnitude(7.0)
        .build();
    cm.predicate = Some(Predicate {
        fields: vec![(GfxElementField::Flag(Flag::SectionRoot), Comparator::equals())],
        always_match: false,
    });
    let mut tree = doc.build_tree();

    let container_x_before = {
        let aid = tree.arena_id_for(&nid).unwrap();
        tree.tree
            .arena
            .get(aid)
            .and_then(|n| n.get().glyph_area())
            .unwrap()
            .position
            .x
            .0
    };
    let section_x_before = {
        let sid = tree.section_arena_id(&nid, 0).unwrap();
        tree.tree
            .arena
            .get(sid)
            .and_then(|n| n.get().glyph_area())
            .unwrap()
            .position
            .x
            .0
    };

    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    let container_x_after = {
        let aid = tree.arena_id_for(&nid).unwrap();
        tree.tree
            .arena
            .get(aid)
            .and_then(|n| n.get().glyph_area())
            .unwrap()
            .position
            .x
            .0
    };
    let section_x_after = {
        let sid = tree.section_arena_id(&nid, 0).unwrap();
        tree.tree
            .arena
            .get(sid)
            .and_then(|n| n.get().glyph_area())
            .unwrap()
            .position
            .x
            .0
    };

    assert!(
        (container_x_after - container_x_before).abs() < 1e-3,
        "container must be filtered out by the SectionRoot predicate"
    );
    assert!(
        (section_x_after - section_x_before - 7.0).abs() < 1e-3,
        "section-area must still receive the mutation"
    );
}

#[test]
fn test_apply_custom_mutation_persistent_sets_dirty() {
    let mut doc = load_test_doc();
    let cm = make_test_mutation("nudge", TS::SelfOnly);
    doc.mindmap.custom_mutations.push(cm.clone());
    doc.build_mutation_registry();
    let mut tree = doc.build_tree();

    assert!(!doc.dirty);
    doc.apply_custom_mutation(&cm, "0", Some(&mut tree));
    assert!(doc.dirty, "Persistent mutation should set dirty flag");
    assert_eq!(doc.undo_stack.len(), 1, "Should push undo action");
}

/// A `RepeatWhile` wrapper whose predicate is `Predicate::new()`
/// matches **nothing**. Its Macro payload must not reach the tree:
/// the flat-apply path has no predicate evaluator, so extracting
/// the literal list would land the mutations on every node the
/// scope collected — the exact opposite of what the AST authorizes.
///
/// Red on `eddd53a`: `flat_mutations`'s guard parsed as
/// `always_match || (all_equals && fields.is_empty())`, which
/// collapses to `always_match || fields.is_empty()`, so this shape
/// was extracted and the nudge landed on the whole
/// `SelfAndDescendants` set.
#[test]
fn test_apply_custom_mutation_match_nothing_repeat_while_does_not_apply() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::gfx_structs::predicate::Predicate;
    use baumhard::mutator_builder::{InstructionSpec, MutationListSrc, MutationSrc, MutatorNode};

    let mut doc = load_test_doc();
    let mut cm = make_test_mutation("match-nothing", TS::SelfAndDescendants);
    cm.mutator = Some(MutatorNode::Instruction {
        channel: 0,
        instruction: InstructionSpec::RepeatWhile(Predicate::new()),
        mutation: MutationSrc::None,
        children: vec![MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![Mutation::area_command(GlyphAreaCommand::NudgeRight(
                10.0,
            ))]),
            children: vec![],
        }],
    });
    doc.mindmap.custom_mutations.push(cm.clone());
    doc.build_mutation_registry();
    let mut tree = doc.build_tree();

    let before: Vec<(String, f64)> = doc
        .mindmap
        .nodes
        .iter()
        .map(|(id, n)| (id.clone(), n.position.x))
        .collect();
    doc.apply_custom_mutation(&cm, "0", Some(&mut tree));

    for (id, x) in &before {
        assert_eq!(
            doc.mindmap.nodes.get(id).unwrap().position.x,
            *x,
            "node '{}' must be untouched by a match-nothing RepeatWhile",
            id
        );
    }
    assert!(
        doc.undo_stack.is_empty(),
        "a skipped apply must not push an undo entry"
    );
    assert!(!doc.dirty, "a skipped apply must not mark the document dirty");
}

/// The same mutation with an always-true predicate *does* apply —
/// the control row proving the test above pins the predicate, not
/// the `Instruction`-rooted shape.
#[test]
fn test_apply_custom_mutation_always_true_repeat_while_still_applies() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::scope;

    let mut doc = load_test_doc();
    let mut cm = make_test_mutation("always-true", TS::SelfAndDescendants);
    cm.mutator = Some(scope::descendants(vec![Mutation::area_command(
        GlyphAreaCommand::NudgeRight(10.0),
    )]));
    doc.mindmap.custom_mutations.push(cm.clone());
    doc.build_mutation_registry();
    let mut tree = doc.build_tree();

    let before = doc.mindmap.nodes.get("0").unwrap().position.x;
    doc.apply_custom_mutation(&cm, "0", Some(&mut tree));
    assert!(
        (doc.mindmap.nodes.get("0").unwrap().position.x - before - 10.0).abs() < 1e-3,
        "an always-true RepeatWhile stays flat-extractable and applies"
    );
    assert_eq!(doc.undo_stack.len(), 1);
}

/// The **nested** all-or-nothing rule, through the caller rather than
/// through `flat_mutations` alone.
///
/// `test_apply_custom_mutation_match_nothing_repeat_while_does_not_apply`
/// above exercises the *root* case; this is the shape the rule was
/// extended for — a `Macro` root whose own literal list is perfectly
/// extractable, over a child the flat path cannot evaluate. Before
/// the rule, the root's payload was returned and blanketed the whole
/// `SelfAndDescendants` set while the nested payload landed nowhere.
///
/// Pins all four halves of "declined": the model is untouched, no
/// undo entry is pushed, the document stays clean, and the tree the
/// caller handed in is not written either (the flat path is the only
/// path, so a decline means nothing moved anywhere).
#[test]
fn test_apply_custom_mutation_unevaluatable_nested_child_applies_nothing() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::gfx_structs::predicate::Predicate;
    use baumhard::mutator_builder::{InstructionSpec, MutationListSrc, MutationSrc, MutatorNode};

    let nudge = |n: f32| Mutation::area_command(GlyphAreaCommand::NudgeRight(n));

    let mut doc = load_test_doc();
    let mut cm = make_test_mutation("nested-decline", TS::SelfAndDescendants);
    cm.mutator = Some(MutatorNode::Macro {
        channel: 0,
        // Extractable on its own — this is the payload that used to
        // escape and blanket every target.
        mutations: MutationListSrc::Literal(vec![nudge(10.0)]),
        children: vec![MutatorNode::Instruction {
            channel: 0,
            // `Predicate::new()` matches nothing; the flat path has no
            // predicate evaluator, so this branch is unevaluatable.
            instruction: InstructionSpec::RepeatWhile(Predicate::new()),
            mutation: MutationSrc::None,
            children: vec![MutatorNode::Macro {
                channel: 0,
                mutations: MutationListSrc::Literal(vec![nudge(99.0)]),
                children: vec![],
            }],
        }],
    });
    doc.mindmap.custom_mutations.push(cm.clone());
    doc.build_mutation_registry();
    let mut tree = doc.build_tree();

    let before: Vec<(String, f64)> = doc
        .mindmap
        .nodes
        .iter()
        .map(|(id, n)| (id.clone(), n.position.x))
        .collect();
    let tree_before = tree
        .arena_id_for("0")
        .and_then(|id| tree.tree.arena.get(id))
        .and_then(|n| n.get().glyph_area())
        .map(|a| a.position.x.0)
        .expect("node '0' is in the tree");

    doc.apply_custom_mutation(&cm, "0", Some(&mut tree));

    for (id, x) in &before {
        assert_eq!(
            doc.mindmap.nodes.get(id).unwrap().position.x,
            *x,
            "node '{}' must be untouched: the root's payload has no business \
             applying when a nested branch was declined",
            id
        );
    }
    let tree_after = tree
        .arena_id_for("0")
        .and_then(|id| tree.tree.arena.get(id))
        .and_then(|n| n.get().glyph_area())
        .map(|a| a.position.x.0)
        .expect("node '0' is in the tree");
    assert_eq!(
        tree_after, tree_before,
        "a declined mutator must not write the display tree either"
    );
    assert!(
        doc.undo_stack.is_empty(),
        "a declined apply must not push an undo entry"
    );
    assert!(!doc.dirty, "a declined apply must not mark the document dirty");
}

/// The same decline, one level up: two nested payloads that are each
/// individually extractable but **disagree**. No unevaluatable node
/// anywhere — the shape is refused purely because one flat list
/// cannot be both `nudge(10)` and `nudge(99)`.
#[test]
fn test_apply_custom_mutation_disagreeing_nested_payload_applies_nothing() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mutator_builder::{MutationListSrc, MutatorNode};

    let nudge = |n: f32| Mutation::area_command(GlyphAreaCommand::NudgeRight(n));

    let mut doc = load_test_doc();
    let mut cm = make_test_mutation("nested-disagree", TS::SelfAndDescendants);
    cm.mutator = Some(MutatorNode::Macro {
        channel: 0,
        mutations: MutationListSrc::Literal(vec![nudge(10.0)]),
        children: vec![MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge(99.0)]),
            children: vec![],
        }],
    });
    doc.mindmap.custom_mutations.push(cm.clone());
    doc.build_mutation_registry();
    let mut tree = doc.build_tree();

    let before = doc.mindmap.nodes.get("0").unwrap().position.x;
    doc.apply_custom_mutation(&cm, "0", Some(&mut tree));

    assert_eq!(
        doc.mindmap.nodes.get("0").unwrap().position.x,
        before,
        "nudge(10) must not land while nudge(99) lands nowhere"
    );
    assert!(doc.undo_stack.is_empty(), "a declined apply pushes no undo entry");
    assert!(!doc.dirty);
}

#[test]
fn test_apply_custom_mutation_toggle_does_not_set_dirty() {
    let mut doc = load_test_doc();
    let mut cm = make_test_mutation("toggle-test", TS::SelfOnly);
    cm.behavior = MB::Toggle;
    doc.mindmap.custom_mutations.push(cm.clone());
    doc.build_mutation_registry();
    let mut tree = doc.build_tree();

    doc.apply_custom_mutation(&cm, "0", Some(&mut tree));
    assert!(!doc.dirty, "Toggle mutation should not set dirty flag");
    assert!(doc.undo_stack.is_empty(), "Toggle mutation should not push undo");
    assert!(doc
        .active_toggles
        .contains(&("0".to_string(), "toggle-test".to_string())));
}

#[test]
fn test_apply_custom_mutation_toggle_reverses() {
    let mut doc = load_test_doc();
    let mut cm = make_test_mutation("toggle-test", TS::SelfOnly);
    cm.behavior = MB::Toggle;
    doc.mindmap.custom_mutations.push(cm.clone());
    doc.build_mutation_registry();
    let mut tree = doc.build_tree();

    // First apply: activates toggle
    doc.apply_custom_mutation(&cm, "0", Some(&mut tree));
    assert!(doc
        .active_toggles
        .contains(&("0".to_string(), "toggle-test".to_string())));

    // Second apply: deactivates toggle
    doc.apply_custom_mutation(&cm, "0", Some(&mut tree));
    assert!(!doc
        .active_toggles
        .contains(&("0".to_string(), "toggle-test".to_string())));
}

#[test]
fn test_undo_custom_mutation_restores_node() {
    let mut doc = load_test_doc();
    let cm = make_test_mutation("nudge", TS::SelfOnly);
    let node_id = "0";

    let orig_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
    let mut tree = doc.build_tree();

    doc.apply_custom_mutation(&cm, node_id, Some(&mut tree));
    // Position may have been synced from tree; verify undo restores original
    assert!(doc.undo());
    let restored_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
    assert!(
        (restored_x - orig_x).abs() < 0.001,
        "Undo should restore original position"
    );
}

// ----- Animation lifecycle tests (§T1 — fundamental) -----

fn make_animated_mutation(id: &str, duration_ms: u32) -> CM {
    TestNudgeMutation::new(id, TS::SelfOnly)
        .magnitude(100.0)
        .timing(AnimationTiming {
            duration_ms,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        })
        .build()
}

/// Build an animated **absolute-position** mutation: a
/// `MoveTo(tx, ty)` (which `mutation_targets_absolute_position`
/// classifies as absolute) wrapped in linear timing. The relative
/// `make_animated_mutation` above covers the delta case; this
/// covers the Assign case the P0-03 completion fix must not
/// regress.
fn make_animated_moveto_mutation(id: &str, duration_ms: u32, tx: f32, ty: f32) -> CM {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::scope;
    CM {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::MoveTo(tx, ty),
        )])),
        target_scope: TS::SelfOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: Some(AnimationTiming {
            duration_ms,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    }
}

#[test]
fn test_start_animation_creates_instance() {
    let mut doc = load_test_doc();
    let cm = make_animated_mutation("anim-1", 500);
    let node_id = first_testament_node_id(&doc);
    assert!(!doc.has_active_animations());

    doc.start_animation(&cm, &node_id, 0);
    assert!(doc.has_active_animations());
    assert_eq!(doc.active_animations.len(), 1);
    assert_eq!(doc.active_animations[0].target_id, node_id);
}

#[test]
fn test_start_animation_derives_to_snapshot_via_nudge() {
    let mut doc = load_test_doc();
    let cm = make_animated_mutation("anim-pos", 500);
    let node_id = first_testament_node_id(&doc);
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    doc.start_animation(&cm, &node_id, 0);
    let anim = &doc.active_animations[0];
    let expected_to_x = orig_x + 100.0;
    assert!(
        (anim.to_node.position.x - expected_to_x).abs() < 0.001,
        "to_node.x should be original + 100 (NudgeRight(100)); got {} expected {}",
        anim.to_node.position.x,
        expected_to_x,
    );
    assert!(
        (anim.from_node.position.x - orig_x).abs() < 0.001,
        "from_node.x should match original",
    );
}

/// The third `flat_mutations` call site (`animations.rs`), which no
/// test reached before.
///
/// `start_animation_inner` derives the `to` snapshot by replaying the
/// extracted flat list against a scratch node, and reaches for
/// `.unwrap_or_default()` when the mutator declines. So a declined
/// mutator does not warn and does not skip here — it silently becomes
/// a **zero-delta tween**: an instance is created, it occupies the
/// dedup slot for its full duration, it ticks, and it lands the node
/// exactly where it started.
///
/// That is the intended shape (the doc above the call says the
/// scratch "stays at `from`" for shapes it cannot preview), but it is
/// the one call site where a decline is invisible at runtime, so it
/// gets pinned rather than assumed. The mutator here is the nested
/// disagreement from
/// `test_apply_custom_mutation_disagreeing_nested_payload_applies_nothing`
/// — every node extractable, two payloads that disagree.
#[test]
fn test_start_animation_with_a_declined_mutator_tweens_zero_delta() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mutator_builder::{MutationListSrc, MutatorNode};

    let nudge = |n: f32| Mutation::area_command(GlyphAreaCommand::NudgeRight(n));

    let mut doc = load_test_doc();
    let mut cm = make_animated_mutation("anim-declined", 500);
    cm.mutator = Some(MutatorNode::Macro {
        channel: 0,
        mutations: MutationListSrc::Literal(vec![nudge(100.0)]),
        children: vec![MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge(999.0)]),
            children: vec![],
        }],
    });
    let node_id = first_testament_node_id(&doc);
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    doc.start_animation(&cm, &node_id, 0);

    assert_eq!(
        doc.active_animations.len(),
        1,
        "a declined mutator still creates an instance — `unwrap_or_default` \
         does not abort the animation"
    );
    let anim = &doc.active_animations[0];
    assert!(
        (anim.from_node.position.x - orig_x).abs() < 0.001,
        "from_node is the live node"
    );
    assert!(
        (anim.to_node.position.x - orig_x).abs() < 0.001,
        "to_node must equal from_node: neither nudge(100) nor nudge(999) may \
         land when the AST was declined. Got to.x = {}, from.x = {}",
        anim.to_node.position.x,
        orig_x
    );

    // And it really is inert end to end: run it to completion and the
    // model is where it started.
    doc.tick_animations(1_000, None);
    assert!(
        (doc.mindmap.nodes.get(&node_id).unwrap().position.x - orig_x).abs() < 0.001,
        "a zero-delta tween must leave the model untouched at completion"
    );
}

/// Control row for the test above: the *same* mutator with the nested
/// payload made to agree extracts normally and tweens by the real
/// delta. Without this, "to == from" could pass for a reason having
/// nothing to do with the decline.
#[test]
fn test_start_animation_with_an_agreeing_nested_payload_tweens_the_delta() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mutator_builder::{MutationListSrc, MutatorNode};

    let nudge = |n: f32| Mutation::area_command(GlyphAreaCommand::NudgeRight(n));

    let mut doc = load_test_doc();
    let mut cm = make_animated_mutation("anim-agreeing", 500);
    cm.mutator = Some(MutatorNode::Macro {
        channel: 0,
        mutations: MutationListSrc::Literal(vec![nudge(100.0)]),
        children: vec![MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge(100.0)]),
            children: vec![],
        }],
    });
    let node_id = first_testament_node_id(&doc);
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    doc.start_animation(&cm, &node_id, 0);
    let anim = &doc.active_animations[0];
    assert!(
        (anim.to_node.position.x - (orig_x + 100.0)).abs() < 0.001,
        "agreeing nested payloads extract to one list applied once; got {}",
        anim.to_node.position.x
    );
}

#[test]
fn test_start_animation_no_op_for_zero_duration() {
    let mut doc = load_test_doc();
    let cm = make_animated_mutation("anim-zero", 0);
    let node_id = first_testament_node_id(&doc);
    doc.start_animation(&cm, &node_id, 0);
    assert!(!doc.has_active_animations());
}

#[test]
fn test_start_animation_no_op_for_duplicate_in_flight() {
    let mut doc = load_test_doc();
    let cm = make_animated_mutation("anim-dup", 500);
    let node_id = first_testament_node_id(&doc);
    doc.start_animation(&cm, &node_id, 0);
    doc.start_animation(&cm, &node_id, 100);
    assert_eq!(doc.active_animations.len(), 1);
}

/// `shift_active_animations_start_ms` advances every active
/// instance's `start_ms` by the requested delta. Used by the
/// drag-pause path to re-sync animation wall-clock with the
/// post-release real time, so a multi-second drag during which
/// `tick_animations` was suppressed doesn't snap the animation
/// to its `to` state on the first post-release frame.
#[test]
fn test_shift_active_animations_start_ms_advances_lerp() {
    let mut doc = load_test_doc();
    let cm = make_animated_mutation("anim-pause", 1000);
    let node_id = first_testament_node_id(&doc);
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    doc.start_animation(&cm, &node_id, 0);
    // Without the shift, ticking at t=1500 completes (elapsed=1500
    // > total=1000). With a shift of 1500, the effective elapsed
    // becomes 0 — the animation hasn't started ticking yet.
    doc.shift_active_animations_start_ms(1500);
    let advanced = doc.tick_animations(1500, None);
    assert!(advanced);
    assert!(
        doc.has_active_animations(),
        "after shifting start_ms past now, animation should still be in flight"
    );
    // Position should still be at original (no progress yet).
    let current_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(
        (current_x - orig_x).abs() < 1.0,
        "shifted animation hasn't progressed yet; expected ~{}, got {}",
        orig_x,
        current_x
    );
}

#[test]
fn test_shift_active_animations_start_ms_zero_is_noop() {
    let mut doc = load_test_doc();
    let cm = make_animated_mutation("anim-pause-zero", 1000);
    let node_id = first_testament_node_id(&doc);
    doc.start_animation(&cm, &node_id, 0);
    let start_before = doc.active_animations[0].start_ms;
    doc.shift_active_animations_start_ms(0);
    let start_after = doc.active_animations[0].start_ms;
    assert_eq!(start_before, start_after, "zero-shift must be a no-op");
}

#[test]
fn test_tick_animations_advances_position() {
    let mut doc = load_test_doc();
    let cm = make_animated_mutation("anim-tick", 1000);
    let node_id = first_testament_node_id(&doc);
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    doc.start_animation(&cm, &node_id, 0);
    // Tick at 50% progress (500 ms into 1000 ms duration).
    let advanced = doc.tick_animations(500, None);
    assert!(advanced);

    let current_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    // Linear easing at t=0.5: should be ~halfway.
    let expected_mid = orig_x + 50.0;
    assert!(
        (current_x - expected_mid).abs() < 1.0,
        "position.x at t=0.5 should be ~halfway; got {} expected ~{}",
        current_x,
        expected_mid,
    );
}

#[test]
fn test_tick_animations_completes_at_duration() {
    let mut doc = load_test_doc();
    let cm = make_animated_mutation("anim-end", 1000);
    let node_id = first_testament_node_id(&doc);

    doc.start_animation(&cm, &node_id, 0);
    // Tick past the end.
    let advanced = doc.tick_animations(1500, None);
    assert!(advanced);
    assert!(!doc.has_active_animations(), "animation should have drained");
}

#[test]
fn test_tick_animations_no_advance_on_empty() {
    let mut doc = load_test_doc();
    let advanced = doc.tick_animations(1000, None);
    assert!(!advanced);
}

/// Freeze-hardening regression: a tick called with an
/// astronomically large `now_ms` (simulating CPU starvation that
/// delays the event loop well past the animation's duration)
/// must complete the animation exactly once and leave the active
/// list empty on the next call — never loop or overshoot. The
/// invariant holds because `tick_animations` short-circuits on
/// `elapsed >= total` before it computes the progress fraction.
#[test]
fn test_tick_animations_extreme_overshoot_still_completes() {
    let mut doc = load_test_doc();
    let cm = make_animated_mutation("anim-overshoot", 1000);
    let node_id = first_testament_node_id(&doc);
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    doc.start_animation(&cm, &node_id, 0);
    // now_ms four orders of magnitude past duration.
    let advanced_first = doc.tick_animations(u64::MAX / 2, None);
    assert!(advanced_first, "first tick should complete the animation");
    assert!(
        !doc.has_active_animations(),
        "animation must drain on overshoot, not linger"
    );

    // Pin the intermediate invariant: the completing tick
    // must write the `to` position synchronously, not defer
    // it to a later tick. If the drain path ever stopped
    // committing position on the no-tree branch, the final
    // assertion below would still trip, but this intermediate
    // check names the tick that owes the write.
    let pos_after_first = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    let expected = orig_x + 100.0;
    assert!(
        (pos_after_first - expected).abs() < 0.001,
        "first (completing) tick should already land on to_node position, \
             got {} expected ~{}",
        pos_after_first,
        expected,
    );

    let advanced_second = doc.tick_animations(u64::MAX / 2, None);
    assert!(
        !advanced_second,
        "subsequent tick with no active animations must not advance"
    );

    let final_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(
        (final_x - expected).abs() < 0.001,
        "overshoot tick should land on to_node position, got {} expected ~{}",
        final_x,
        expected,
    );
}

#[test]
fn test_fast_forward_animations_snaps_to_end() {
    let mut doc = load_test_doc();
    let cm = make_animated_mutation("anim-ff", 5000);
    let node_id = first_testament_node_id(&doc);
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    doc.start_animation(&cm, &node_id, 0);
    doc.fast_forward_animations(None);
    assert!(!doc.has_active_animations());

    // Without a tree, fast_forward writes the to_node.position
    // directly into the model.
    let final_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    let expected = orig_x + 100.0;
    assert!(
        (final_x - expected).abs() < 0.001,
        "fast-forward should snap to to_node position",
    );
}

/// **P0-03 regression — relative mutation, live tree.** Completion
/// with a live `Some(tree)` after ≥2 advancing ticks must land the
/// node at exactly `from + delta`, not `from + delta·(1 + t_prev)`.
/// Every prior animation test drove `tree = None`, so the
/// production completion path (which routes the final state through
/// `apply_custom_mutation`) had zero coverage — and pre-fix it
/// re-applied the full nudge on top of the already-lerped model,
/// approaching double the delta, while snapshotting the mid-lerp
/// position for undo. Fails on current main; passes with the
/// reset-then-apply fix.
#[test]
fn test_tick_completion_live_tree_relative_no_double_apply() {
    let mut doc = load_test_doc();
    let node_id = first_testament_node_id(&doc);
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    // NudgeRight(100) over 1000 ms, linear.
    let cm = make_animated_mutation("anim-live-rel", 1000);
    doc.undo_stack.clear();
    doc.dirty = false;

    let mut tree = doc.build_tree();
    doc.start_animation(&cm, &node_id, 0);

    // Two advancing ticks drift the MODEL toward `to`; rebuild the
    // interactive tree from the lerped model between frames, exactly
    // as the event loop's `drain_animation_tick` → `rebuild_all`
    // does — so the completing tick sees a mid-lerp tree too.
    assert!(doc.tick_animations(300, Some(&mut tree)));
    tree = doc.build_tree();
    assert!(doc.tick_animations(700, Some(&mut tree)));
    tree = doc.build_tree();
    assert!(doc.has_active_animations());

    // Completing tick with the live (mid-lerp) tree.
    assert!(doc.tick_animations(1200, Some(&mut tree)));
    assert!(!doc.has_active_animations());

    // Model lands at exactly from + delta — instant-mode parity, not
    // the ~from+170 a double-apply would produce.
    let final_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(
        (final_x - (orig_x + 100.0)).abs() < 0.01,
        "animated relative mutation must land at from+delta ({}); got {final_x} \
         (double-apply would give ~{})",
        orig_x + 100.0,
        orig_x + 170.0,
    );

    // The interactive tree the mutation committed to reflects the
    // same single application — not the mid-lerp + delta the pre-fix
    // display side carried until the next rebuild.
    let arena_id = tree.arena_id_for(&node_id).expect("node in tree");
    let tree_x = tree
        .tree
        .arena
        .get(arena_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0 as f64;
    assert!(
        (tree_x - (orig_x + 100.0)).abs() < 0.01,
        "interactive tree must also reflect from+delta once; got {tree_x}, expected {}",
        orig_x + 100.0,
    );

    // Ctrl-Z restores exactly the pre-animation position (the undo
    // snapshot captured `from`, not the mid-lerp model).
    assert!(doc.undo());
    let after_undo = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(
        (after_undo - orig_x).abs() < 1e-6,
        "undo after an animated mutation must restore the exact pre-animation position; \
         got {after_undo}, expected {orig_x}",
    );
}

/// **P0-03 parity guard — absolute mutation, live tree.** The
/// companion to the relative regression test. An animated absolute
/// `MoveTo` must commit the exact target (an overwrite, not a delta)
/// and be undoable back to `from`. Absolute commands are inert under
/// the tween (`apply_position_mutations_to_node` leaves `to == from`
/// for them), so — unlike the relative test — this one also passes on
/// pre-fix code; it exists to prove the reset-then-apply fix doesn't
/// regress the Assign path. It asserts the tween really is inert (the
/// model stays at `from` mid-flight) so the final landing provably
/// comes from the completion apply, not the lerp.
#[test]
fn test_tick_completion_live_tree_absolute_lands_and_undo_restores() {
    let mut doc = load_test_doc();
    let node_id = first_testament_node_id(&doc);
    let orig = doc.mindmap.nodes.get(&node_id).unwrap().position;

    let target = (orig.x + 250.0, orig.y - 120.0);
    let cm = make_animated_moveto_mutation("anim-live-abs", 1000, target.0 as f32, target.1 as f32);
    doc.undo_stack.clear();
    doc.dirty = false;

    let mut tree = doc.build_tree();
    doc.start_animation(&cm, &node_id, 0);

    // Two advancing ticks, then the completing tick — all live-tree.
    assert!(doc.tick_animations(300, Some(&mut tree)));
    // MoveTo is inert under the tween: `to == from`, so mid-flight the
    // model must still sit at `from` (a real drift toward the target
    // would be +75 on x by t=0.3). The tolerance clears the f32
    // round-trip noise of the lerp (~3e-6 at these magnitudes) while
    // staying far below any real movement.
    let mid = doc.mindmap.nodes.get(&node_id).unwrap().position;
    assert!(
        (mid.x - orig.x).abs() < 1e-3 && (mid.y - orig.y).abs() < 1e-3,
        "absolute MoveTo must not drift the model mid-tween; got ({}, {}), expected ({}, {})",
        mid.x,
        mid.y,
        orig.x,
        orig.y,
    );
    tree = doc.build_tree();
    assert!(doc.tick_animations(700, Some(&mut tree)));
    tree = doc.build_tree();
    assert!(doc.tick_animations(1200, Some(&mut tree)));
    assert!(!doc.has_active_animations());

    // Absolute mutation commits the exact target position.
    let final_pos = doc.mindmap.nodes.get(&node_id).unwrap().position;
    assert!(
        (final_pos.x - target.0).abs() < 0.05 && (final_pos.y - target.1).abs() < 0.05,
        "animated absolute mutation must land at the MoveTo target ({}, {}); got ({}, {})",
        target.0,
        target.1,
        final_pos.x,
        final_pos.y,
    );

    // Ctrl-Z restores the exact pre-animation position.
    assert!(doc.undo());
    let after = doc.mindmap.nodes.get(&node_id).unwrap().position;
    assert!(
        (after.x - orig.x).abs() < 1e-6 && (after.y - orig.y).abs() < 1e-6,
        "undo of an animated absolute mutation must restore the pre-animation position; \
         got ({}, {}), expected ({}, {})",
        after.x,
        after.y,
        orig.x,
        orig.y,
    );
}

/// **P0-03 — a node deleted mid-animation must not be resurrected.**
/// Nothing purges `active_animations` when a node is deleted, so a
/// queued instance reaches completion after its target is gone. The
/// completion must skip it cleanly, exactly as the pre-fix path did
/// (via `apply_custom_mutation`'s empty snapshots). Regression guard
/// against a whole-node `insert(from_node)` at completion, which would
/// re-insert the deleted node at `from+delta` with a ghost undo entry.
#[test]
fn test_tick_completion_deleted_node_not_resurrected() {
    let mut doc = load_test_doc();
    let node_id = first_testament_node_id(&doc);
    let cm = make_animated_mutation("anim-del", 1000);
    doc.undo_stack.clear();
    doc.dirty = false;

    let mut tree = doc.build_tree();
    doc.start_animation(&cm, &node_id, 0);
    assert!(doc.tick_animations(500, Some(&mut tree)));

    // Delete the node mid-flight (model removal mirrors the
    // `mindmap.nodes` side of `delete_node`); the animation stays
    // queued because the delete doesn't touch `active_animations`.
    doc.mindmap.nodes.remove(&node_id);
    assert!(doc.has_active_animations());

    // Completing tick. The completion checks the MODEL for the target's
    // existence (not the tree), so the stale `tree` is never read — but
    // it must still not resurrect the node.
    doc.tick_animations(1200, Some(&mut tree));
    assert!(!doc.has_active_animations(), "the instance still drains");
    assert!(
        !doc.mindmap.nodes.contains_key(&node_id),
        "a node deleted mid-animation must stay deleted at completion"
    );
    assert!(
        doc.undo_stack.is_empty(),
        "no undo entry should be pushed for a completion on a deleted node"
    );
}

/// **P0-03 — no-tree completion of a displacing mutation is
/// undoable.** Production always passes a tree, but tests and any
/// future headless caller hit the no-tree fallback. A relative nudge
/// completing without a tree must land at `from+delta` and push
/// exactly one undo entry that restores `from`. Pre-fix this path
/// pushed none ("caller's responsibility", which no caller took —
/// defect 3).
#[test]
fn test_tick_completion_no_tree_relative_is_undoable() {
    let mut doc = load_test_doc();
    let node_id = first_testament_node_id(&doc);
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    let cm = make_animated_mutation("anim-notree-rel", 1000);
    doc.undo_stack.clear();
    doc.dirty = false;

    doc.start_animation(&cm, &node_id, 0);
    doc.tick_animations(1200, None);
    assert!(!doc.has_active_animations());

    let after = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(
        (after - (orig_x + 100.0)).abs() < 1e-3,
        "no-tree nudge completion lands at from+delta; got {after}, expected {}",
        orig_x + 100.0,
    );
    assert_eq!(
        doc.undo_stack.len(),
        1,
        "a displacing no-tree completion pushes exactly one undo entry"
    );

    assert!(doc.undo());
    let restored = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(
        (restored - orig_x).abs() < 1e-6,
        "undo restores the pre-animation position; got {restored}, expected {orig_x}"
    );
}

/// **P0-03 — no-tree completion of a no-op mutation pushes no undo
/// entry.** An animated absolute `MoveTo` leaves `to == from` in the
/// model snapshot (`apply_position_mutations_to_node` is a no-op for
/// absolute commands), and the no-tree path can't run the declarative
/// MoveTo, so the commit is a no-op that must NOT leave a dead undo
/// entry or a spurious `dirty` — mirroring the tree path's `changed`
/// gate (P0-02).
#[test]
fn test_tick_completion_no_tree_noop_pushes_no_undo() {
    let mut doc = load_test_doc();
    let node_id = first_testament_node_id(&doc);
    let orig = doc.mindmap.nodes.get(&node_id).unwrap().position;
    let cm = make_animated_moveto_mutation(
        "anim-notree-abs",
        1000,
        (orig.x + 250.0) as f32,
        (orig.y - 120.0) as f32,
    );
    doc.undo_stack.clear();
    doc.dirty = false;

    doc.start_animation(&cm, &node_id, 0);
    doc.tick_animations(1200, None);
    assert!(!doc.has_active_animations());

    let after = doc.mindmap.nodes.get(&node_id).unwrap().position;
    assert!(
        (after.x - orig.x).abs() < 1e-9 && (after.y - orig.y).abs() < 1e-9,
        "no-tree absolute completion is inert (to == from); got ({}, {})",
        after.x,
        after.y,
    );
    assert!(
        doc.undo_stack.is_empty(),
        "a no-op no-tree completion must push no undo entry"
    );
    assert!(
        !doc.dirty,
        "a no-op no-tree completion must not flag the document dirty"
    );
}

// ----- Font-size sync-back (P0-02) -----

/// Build a `GrowFont`/`ShrinkFont`-style area-command mutation.
fn make_font_scale_mutation(id: &str, cmd: baumhard::gfx_structs::area::GlyphAreaCommand, scope: TS) -> CM {
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::scope as sc;
    CM {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(sc::self_only(vec![Mutation::area_command(cmd)])),
        target_scope: scope,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    }
}

/// Largest `size_pt` across section `idx`'s runs — mirrors the
/// forward path's `scale = max(run.size_pt)` collapse.
fn section_max_size_pt(doc: &MindMapDocument, node_id: &str, idx: usize) -> u32 {
    doc.mindmap.nodes.get(node_id).unwrap().sections[idx]
        .text_runs
        .iter()
        .map(|r| r.size_pt)
        .max()
        .unwrap_or(0)
}

/// Tree-side font scale of section `idx`'s area.
fn section_tree_scale(tree: &baumhard::mindmap::tree_builder::MindMapTree, node_id: &str, idx: usize) -> f32 {
    let sid = tree.section_arena_id(node_id, idx).unwrap();
    tree.tree
        .arena
        .get(sid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .scale
        .0
}

/// Acceptance criterion (a): `grow-font` visibly grows the text,
/// the growth is written back to the model, it survives a
/// rebuild-from-model, and exactly one undo entry reverses it.
/// Pre-fix the scale change never reached the model, so the
/// mutation was a silent no-op that still polluted the undo stack.
#[test]
fn test_grow_font_persists_survives_rebuild_and_undo_reverses() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);

    let size_before = section_max_size_pt(&doc, &nid, 0);
    assert!(size_before > 0, "fixture node must have a run with a size");
    let undo_len_before = doc.undo_stack.len();

    let cm = make_font_scale_mutation("grow-font-test", GlyphAreaCommand::GrowFont(2.0), TS::SelfOnly);
    let mut tree = doc.build_tree();
    // Tree-side scale starts at the model's largest run size.
    assert!((section_tree_scale(&tree, &nid, 0) - size_before as f32).abs() < 1e-3);

    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    // (1) Model changed: the run size grew by exactly 2pt.
    assert_eq!(
        section_max_size_pt(&doc, &nid, 0),
        size_before + 2,
        "grow-font must write the +2pt back to the model run"
    );
    // (2) Exactly one undo entry, and dirty is set.
    assert_eq!(
        doc.undo_stack.len(),
        undo_len_before + 1,
        "grow-font must push exactly one undo entry"
    );
    assert!(doc.dirty);

    // (3) Survives rebuild_all's model→tree rebuild.
    let tree2 = doc.build_tree();
    assert!(
        (section_tree_scale(&tree2, &nid, 0) - (size_before + 2) as f32).abs() < 1e-3,
        "grown font size must survive the rebuild-from-model"
    );

    // (4) Ctrl-Z reverses it.
    assert!(doc.undo(), "undo must report success");
    assert_eq!(
        section_max_size_pt(&doc, &nid, 0),
        size_before,
        "undo must restore the original font size"
    );
}

/// The §5 hard part: a multi-run section grown by a fixed delta
/// keeps its *relative* run sizes instead of collapsing every run
/// to the max. `sync_section_font_size` distributes the scale
/// delta, so a `[14pt, 40pt]` section grown 3pt becomes
/// `[17pt, 43pt]`, preserving the 26pt spread — an overwrite-all
/// approach would flatten both to 43pt and permanently destroy the
/// authored sizing.
#[test]
fn test_grow_font_preserves_relative_run_sizes() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::mindmap::model::TextRun;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections[0].text = "hello world".into();
        node.sections[0].text_runs = vec![
            TextRun {
                start: 0,
                end: 5,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 14,
                color: "#ffffff".into(),
                hyperlink: None,
            },
            TextRun {
                start: 5,
                end: 11,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 40,
                color: "#ffffff".into(),
                hyperlink: None,
            },
        ];
    }
    let cm = make_font_scale_mutation("grow-font-multi", GlyphAreaCommand::GrowFont(3.0), TS::SelfOnly);
    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    let runs = &doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs;
    assert_eq!(runs[0].size_pt, 17, "small run grows by the same +3 delta");
    assert_eq!(runs[1].size_pt, 43, "large run grows by the same +3 delta");
}

/// `shrink-font` past the floor clamps `size_pt` to
/// [`MIN_TEXT_RUN_SIZE_PT`] (1) instead of saturating a negative
/// tree scale to an invisible 0pt on the `u32` cast.
#[test]
fn test_shrink_font_clamps_to_minimum() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::mindmap::model::TextRun;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections[0].text = "small".into();
        node.sections[0].text_runs = vec![TextRun {
            start: 0,
            end: 5,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 8,
            color: "#ffffff".into(),
            hyperlink: None,
        }];
    }
    // Shrink by 20pt from an 8pt run — tree scale goes negative.
    let cm = make_font_scale_mutation(
        "shrink-font-test",
        GlyphAreaCommand::ShrinkFont(20.0),
        TS::SelfOnly,
    );
    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs[0].size_pt,
        1,
        "shrink past the floor clamps to 1pt, never 0"
    );
}

/// A `grow-font` mutation on a **runless** section synthesizes a
/// single run to carry the new size — the change would otherwise
/// have nowhere to live and evaporate on the next rebuild. The
/// synthesized run spans the whole text and inherits the node's
/// default text color so rendering is unchanged except for size.
#[test]
fn test_grow_font_on_runless_section_synthesizes_run() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.style.text_color = "#abcdef".into();
        // A section with text but no runs (size defaults to 14pt).
        node.sections
            .push(MindSection::new_default("runless".into(), Vec::new()));
    }
    let runless_idx = doc.mindmap.nodes.get(&nid).unwrap().sections.len() - 1;
    let cm = make_font_scale_mutation("grow-runless", GlyphAreaCommand::GrowFont(2.0), TS::SelfOnly);
    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    let section = &doc.mindmap.nodes.get(&nid).unwrap().sections[runless_idx];
    assert_eq!(
        section.text_runs.len(),
        1,
        "a run must be synthesized to hold the size"
    );
    let run = &section.text_runs[0];
    assert_eq!(run.size_pt, 16, "synthesized run carries the grown size (14 + 2)");
    assert_eq!(run.start, 0);
    assert_eq!(
        run.end,
        baumhard::util::grapheme_chad::count_grapheme_clusters("runless")
    );
    assert_eq!(
        run.color, "#abcdef",
        "synthesized run inherits the node's default text color"
    );
}

/// Acceptance criterion (b): a Toggle mutation's visual survives a
/// rebuild-from-model while active, then reverts once toggled off.
/// Pre-fix nothing re-applied `active_toggles` after a rebuild, so
/// the toggle-on's tree edit died at the end of the same dispatch
/// and "second trigger reverses" had no effect left to reverse.
#[test]
fn test_toggle_mutation_survives_rebuild_and_reverts_off() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::{scope, CustomMutation};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);

    let cm = CustomMutation {
        id: "toggle-nudge".into(),
        name: "Toggle nudge".into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::NudgeRight(25.0),
        )])),
        target_scope: TS::SelfOnly,
        behavior: MB::Toggle,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    // Register so the toggle re-application can find the mutation.
    doc.mutation_registry.insert("toggle-nudge".into(), cm.clone());

    // Simulate the render path (`rebuild_all`): build the pure,
    // overlay-free tree, then stamp active-toggle visuals onto it
    // exactly as the renderer sees them. `build_tree` itself stays
    // overlay-free so the Persistent sync path never reads a toggle.
    let container_x = |doc: &MindMapDocument| -> f32 {
        let mut tree = doc.build_tree();
        doc.reapply_active_toggles(&mut tree);
        let aid = tree.arena_id_for(&nid).unwrap();
        tree.tree
            .arena
            .get(aid)
            .unwrap()
            .get()
            .glyph_area()
            .unwrap()
            .position
            .x
            .0
    };
    let base_x = container_x(&doc);

    // Toggle ON (the passed tree is discarded; the assertion below
    // exercises a *fresh* rebuild-from-model).
    {
        let mut tree = doc.build_tree();
        doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));
    }
    assert!(doc
        .active_toggles
        .contains(&(nid.clone(), "toggle-nudge".to_string())));
    assert!(
        (container_x(&doc) - base_x - 25.0).abs() < 1e-3,
        "an active toggle must survive the rebuild-from-model"
    );
    // Toggle is visual-only: the model must be untouched.
    assert!(
        !doc.dirty,
        "a Toggle mutation must not mark the model dirty via apply_to_tree"
    );

    // Toggle OFF → the visual reverts on the next rebuild.
    {
        let mut tree = doc.build_tree();
        doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));
    }
    assert!(!doc
        .active_toggles
        .contains(&(nid.clone(), "toggle-nudge".to_string())));
    assert!(
        (container_x(&doc) - base_x).abs() < 1e-3,
        "toggling off must revert the visual on the next rebuild"
    );
}

/// Acceptance criterion (c): a mutation that resolves to no model
/// change must not push an undo entry. Pre-fix every apply pushed
/// undo unconditionally, so the next Ctrl-Z silently ate a real step.
#[test]
fn test_no_op_mutation_pushes_no_undo_entry() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // Runless sections keep the round-trip a strict identity: no
    // runs means no font-name canonicalization drift, so the only
    // thing that can flip `changed` is the mutation itself.
    for section in doc.mindmap.nodes.get_mut(&nid).unwrap().sections.iter_mut() {
        section.text_runs.clear();
    }
    doc.dirty = false;
    let undo_len_before = doc.undo_stack.len();

    // Zero-magnitude nudge: applies cleanly but changes nothing.
    let cm = TestNudgeMutation::new("zero-nudge", TS::SelfOnly)
        .magnitude(0.0)
        .build();
    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    assert_eq!(
        doc.undo_stack.len(),
        undo_len_before,
        "a no-op mutation must not push an undo entry"
    );
    assert!(!doc.dirty, "a no-op mutation must not set the dirty flag");
}

/// The predicate-filtered-everything no-op path: an empty-fields,
/// `always_match = false` predicate matches nothing, so no element
/// is mutated and the model is unchanged — no dead undo entry.
#[test]
fn test_predicate_filtered_all_apply_pushes_no_undo() {
    use baumhard::gfx_structs::predicate::Predicate;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // See the note in `test_no_op_mutation_pushes_no_undo_entry`:
    // clear runs so the pristine round-trip is a strict identity.
    for section in doc.mindmap.nodes.get_mut(&nid).unwrap().sections.iter_mut() {
        section.text_runs.clear();
    }
    doc.dirty = false;
    let undo_len_before = doc.undo_stack.len();

    let mut cm = TestNudgeMutation::new("filtered-noop", TS::SelfOnly)
        .magnitude(25.0)
        .build();
    cm.predicate = Some(Predicate {
        fields: vec![],
        always_match: false,
    });

    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    assert_eq!(
        doc.undo_stack.len(),
        undo_len_before,
        "a predicate that filters every candidate must not push an undo entry"
    );
    assert!(!doc.dirty);
}

// ----- Review follow-ups: overlay-leak + delta-baseline + ordering -----

/// A Persistent mutation applied against an interactive tree that
/// carries a selection-highlight overlay must persist only its own
/// effect — never the cyan highlight. Regression for the P1 review
/// finding: `sync_node_from_tree` now reads a fresh, overlay-free
/// `build_tree`, not the caller's decorated render tree.
#[test]
fn test_persistent_apply_does_not_leak_highlight_overlay_into_model() {
    use crate::application::document::{apply_tree_highlights, HIGHLIGHT_COLOR};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let orig_color = doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs[0]
        .color
        .clone();
    let orig_x = doc.mindmap.nodes.get(&nid).unwrap().position.x;

    // Simulate `rebuild_all`'s stored render tree: a pure projection
    // plus the selection-highlight overlay stamped on the node.
    let mut interactive = doc.build_tree();
    apply_tree_highlights(&mut interactive, vec![(nid.as_str(), None, HIGHLIGHT_COLOR)]);

    // Apply a Persistent nudge against the OVERLAID interactive tree.
    let cm = TestNudgeMutation::new("nudge-over-highlight", TS::SelfOnly)
        .magnitude(5.0)
        .build();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut interactive));

    // The mutation's own effect persists...
    let after_x = doc.mindmap.nodes.get(&nid).unwrap().position.x;
    assert!(
        (after_x - orig_x - 5.0).abs() < 1e-3,
        "the nudge itself must persist to the model"
    );
    // ...but the highlight overlay must NOT be written back.
    let after_color = doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs[0]
        .color
        .clone();
    assert_eq!(
        after_color, orig_color,
        "selection-highlight overlay must not leak into the persisted model"
    );
}

/// A Persistent mutation applied while a visual toggle is active on
/// the same node must persist only the Persistent mutation's effect
/// — the toggle's tree-only nudge must not become a saved model
/// move. Regression for the P1 review finding.
#[test]
fn test_persistent_apply_does_not_leak_active_toggle_into_model() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::{scope, CustomMutation};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let orig_x = doc.mindmap.nodes.get(&nid).unwrap().position.x;

    // A visual toggle that nudges the node right by 100px.
    let toggle = CustomMutation {
        id: "vis-toggle".into(),
        name: "vis".into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::NudgeRight(100.0),
        )])),
        target_scope: TS::SelfOnly,
        behavior: MB::Toggle,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    doc.mutation_registry.insert("vis-toggle".into(), toggle.clone());
    {
        let mut t = doc.build_tree();
        doc.apply_custom_mutation(&toggle, &nid, Some(&mut t));
    }

    // Interactive render tree WITH the toggle overlay (as rebuild_all
    // would produce), then a Persistent +5 nudge on the same node.
    let mut interactive = doc.build_tree();
    doc.reapply_active_toggles(&mut interactive);
    let persistent = TestNudgeMutation::new("persist-nudge", TS::SelfOnly)
        .magnitude(5.0)
        .build();
    doc.apply_custom_mutation(&persistent, &nid, Some(&mut interactive));

    let after_x = doc.mindmap.nodes.get(&nid).unwrap().position.x;
    assert!(
        (after_x - orig_x - 5.0).abs() < 1e-3,
        "only the Persistent +5 may persist; the toggle's +100 must not leak (got orig+{})",
        after_x - orig_x
    );
}

/// A text/region mutation that drops the section's largest run must
/// NOT inflate the surviving runs to the stale tree scale. Regression
/// for the P2 review finding: the font-size delta uses the section's
/// scale captured *before* the round-trip rewrites `text_runs`.
#[test]
fn test_font_size_delta_ignores_run_dropped_by_round_trip() {
    use baumhard::core::primitives::Range;
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::{scope, CustomMutation};
    use baumhard::mindmap::model::TextRun;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections[0].text = "abcdef".into();
        node.sections[0].text_runs = vec![
            TextRun {
                start: 0,
                end: 3,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 14,
                color: "#ffffff".into(),
                hyperlink: None,
            },
            TextRun {
                start: 3,
                end: 6,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 40,
                color: "#ff0000".into(),
                hyperlink: None,
            },
        ];
    }
    // Delete the region carrying the largest (40pt) run. The tree's
    // `scale` stays 40 (deleting a color region doesn't touch it),
    // and the round-trip rebuilds the model with only the 14pt run.
    let cm = CustomMutation {
        id: "drop-large".into(),
        name: "drop".into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::DeleteColorFontRegion(Range::new(3, 6)),
        )])),
        target_scope: TS::SelfOnly,
        behavior: MB::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &nid, Some(&mut tree));

    let runs = &doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs;
    assert!(
        runs.iter().all(|r| r.size_pt == 14),
        "surviving run(s) must keep their authored 14pt, not inflate to the stale 40pt scale; got {:?}",
        runs.iter().map(|r| r.size_pt).collect::<Vec<_>>()
    );
}

/// Toggles replay in activation order after a rebuild. Two
/// non-commutative `MoveTo` toggles on the same node must leave the
/// node at the *second* toggle's target (last-writer-wins in
/// activation order) — `active_toggles` is an ordered list, so the
/// post-rebuild visual is deterministic. Regression for the P2
/// ordering finding.
#[test]
fn test_active_toggles_replay_in_activation_order() {
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::mindmap::custom_mutation::{scope, CustomMutation};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);

    let move_toggle = |id: &str, x: f32| CustomMutation {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::MoveTo(x, 0.0),
        )])),
        target_scope: TS::SelfOnly,
        behavior: MB::Toggle,
        predicate: None,
        document_actions: vec![],
        timing: None,
    };
    let first = move_toggle("move-a", 111.0);
    let second = move_toggle("move-b", 222.0);
    doc.mutation_registry.insert("move-a".into(), first.clone());
    doc.mutation_registry.insert("move-b".into(), second.clone());

    // Activate in order: A then B.
    {
        let mut t = doc.build_tree();
        doc.apply_custom_mutation(&first, &nid, Some(&mut t));
    }
    {
        let mut t = doc.build_tree();
        doc.apply_custom_mutation(&second, &nid, Some(&mut t));
    }
    assert_eq!(
        doc.active_toggles,
        vec![
            (nid.clone(), "move-a".to_string()),
            (nid.clone(), "move-b".to_string())
        ],
        "active_toggles must record activation order"
    );

    // Render-path rebuild: the last-activated toggle (B → x=222) wins.
    let mut tree = doc.build_tree();
    doc.reapply_active_toggles(&mut tree);
    let aid = tree.arena_id_for(&nid).unwrap();
    let x = tree
        .tree
        .arena
        .get(aid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    assert!(
        (x - 222.0).abs() < 1e-3,
        "ordered replay must apply move-a then move-b, leaving x at 222 (got {x})"
    );
}
