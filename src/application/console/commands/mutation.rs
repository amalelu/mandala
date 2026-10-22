// SPDX-License-Identifier: MPL-2.0

//! `mutation` — list, apply, and describe registered custom mutations.
//!
//! Sub-commands:
//! - `mutation list [filter]` — list mutations surfaced to the user
//!   (contexts include `map` and not just `internal`). `--all` shows
//!   every registered mutation, including internals.
//! - `mutation apply <id> [node-id]` — apply the named mutation to a
//!   single-node selection (or the given `node-id`). Refuses internal
//!   mutations and any id not in the registry.
//! - `mutation help <id>` — print the mutation's description, contexts,
//!   scope, behavior, and source layer.

use super::Command;
use crate::application::console::completion::Completion;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::{
    free, kvs, usage, Descent, Form, Grammar, Slot, Subverb, Vocabulary, Word,
};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{MindMapDocument, SelectionState};

/// The one flag in the console. It is a *slot value* rather than a
/// subverb, so `commands/mod.rs` § Casing's case-insensitive subverb
/// rule does not reach it and `mutation list --ALL` still falls to
/// the filter arm (#135). Declaring it here is what makes it
/// discoverable at all: no completer offered it before.
const ALL_FLAG: &[Word] = &[Word::new(
    "--all",
    "include internal mutations and ones that do not target the map",
)];

/// One row per registered non-internal mutation, hinted with its
/// display name.
fn mutation_id_rows(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    let partial = partial.to_ascii_lowercase();
    let mut ids: Vec<(&String, &baumhard::mindmap::custom_mutation::CustomMutation)> = ctx
        .document
        .mutation_registry
        .iter()
        .filter(|(_, cm)| !cm.is_internal())
        .filter(|(id, _)| id.to_ascii_lowercase().starts_with(&partial))
        .collect();
    ids.sort_by(|a, b| a.0.cmp(b.0));
    ids.into_iter()
        .map(|(id, cm)| Completion {
            text: id.clone(),
            display: id.clone(),
            hint: Some(cm.name.clone()),
            font_family: None,
        })
        .collect()
}

const ID_VOCAB: Vocabulary = Vocabulary::Rows {
    placeholder: "id",
    rows: mutation_id_rows,
    sentinels: &[],
};

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("list", "registry", "list the registered mutations").taking(&[Form::slots(&[
        Slot::opt(Vocabulary::Words(ALL_FLAG)),
        Slot::opt(free("filter")),
    ])]),
    Subverb::bare("apply", "registry", "run one mutation on a node")
        .taking(&[Form::slots(&[Slot::req(ID_VOCAB), Slot::opt(free("node-id"))])]),
    Subverb::bare("help", "registry", "print one mutation's description")
        .taking(&[Form::slots(&[Slot::req(ID_VOCAB)])]),
    Subverb::bare("inspect", "registry", "print one mutation's full definition")
        .taking(&[Form::slots(&[Slot::req(ID_VOCAB)])]),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "mutation",
    subverb_sets: &[SUBVERBS],
    key_sets: &[],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "mutation",
    aliases: &["mut"],
    summary: "List, apply, and inspect registered mutations",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["mut", "run", "debug"],
    execute: execute_mutation,
};

fn execute_mutation(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Sub-command names are matched case-insensitively by the
    // descent, console-wide — see `commands/mod.rs` § Casing. The
    // mutation *id* in the next slot is not: it is a registry key,
    // matched as written.
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    match descent.stop {
        Stop::Matched(subverb) => match subverb.name {
            "list" => list(&descent, args, eff),
            "apply" => apply(&descent, args, eff),
            "help" => help(&descent, args, eff),
            _ => inspect(&descent, args, eff),
        },
        Stop::Bare => ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => ExecResult::err(usage::unknown_subverb_message(
            descent.level,
            descent.typed.unwrap_or_default(),
        )),
    }
}

fn list(descent: &Descent, args: &Args, eff: &ConsoleEffects) -> ExecResult {
    // `--all` is a slot value rather than a subverb, so it stays
    // case-sensitive (#135) and sits in either of the two slots the
    // form declares.
    let slots = descent.slot_value(args);
    let mut show_all = false;
    let mut filter: Option<&str> = None;
    for i in 0..2 {
        match slots.get(i) {
            Some("--all") => show_all = true,
            Some(other) if filter.is_none() => filter = Some(other),
            Some(_) | None => {}
        }
    }

    let doc = eff.document();
    let mut rows: Vec<(&String, &baumhard::mindmap::custom_mutation::CustomMutation)> = doc
        .mutation_registry
        .iter()
        .filter(|(_, cm)| show_all || (cm.targets_map() && !cm.is_internal()))
        .filter(|(id, cm)| match filter {
            Some(f) => {
                let fl = f.to_ascii_lowercase();
                id.to_ascii_lowercase().contains(&fl) || cm.name.to_ascii_lowercase().contains(&fl)
            }
            None => true,
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));

    if rows.is_empty() {
        return ExecResult::ok_msg(match filter {
            Some(f) => format!("no mutations match '{}'", f),
            None => "no mutations registered".into(),
        });
    }

    let id_width = rows.iter().map(|(id, _)| id.len()).max().unwrap_or(0).max(8);
    let name_width = rows.iter().map(|(_, cm)| cm.name.len()).max().unwrap_or(0).max(4);

    let mut lines: Vec<String> = Vec::with_capacity(rows.len() + 1);
    let header = format!(
        "  {:<id$}  {:<name$}  {}",
        "id",
        "name",
        "description",
        id = id_width,
        name = name_width
    );
    lines.push(header);
    for (id, cm) in rows {
        let desc_first_line = cm.description.lines().next().unwrap_or("");
        lines.push(format!(
            "  {:<id$}  {:<name$}  {}",
            id,
            cm.name,
            desc_first_line,
            id = id_width,
            name = name_width
        ));
    }
    ExecResult::lines(lines)
}

fn apply(descent: &Descent, args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let slots = descent.slot_value(args);
    let id = match slots.get(0) {
        Some(s) => s.to_string(),
        None => return ExecResult::err("mutation apply needs an id (`mutation apply <id>`)"),
    };
    let explicit_node = slots.get(1).map(str::to_string);

    // Look up the mutation. `.clone()` so we don't hold a borrow while
    // mutating through `eff.document_mut()` below.
    let cm = match eff.document().mutation_registry.get(&id) {
        Some(cm) => cm.clone(),
        None => return ExecResult::err(format!("unknown mutation: {}", id)),
    };
    if cm.is_internal() {
        return ExecResult::err(format!(
            "mutation '{}' is internal and not runnable from the console",
            id
        ));
    }

    let target_id = match resolve_target_id(eff.document(), explicit_node.as_deref()) {
        Ok(t) => t,
        Err(e) => return ExecResult::err(e),
    };

    // Apply through the document's existing undo-pushing path. Both
    // tree mutations (via `apply_custom_mutation`) and
    // document-actions need to fire — a single custom mutation can
    // carry both and users expect one `mutation apply` to do both.
    //
    // The declarative (flat-apply) path needs a fresh MindMapTree;
    // the imperative handler path mutates the model directly and
    // doesn't touch the tree, so we skip the (expensive) build when
    // dispatch will go to a handler. The tree is discarded after
    // apply either way — the renderer rebuilds from the model on
    // the next frame.
    if eff.document().will_dispatch_to_handler(&cm.id) {
        eff.document_mut().apply_custom_mutation(&cm, &target_id, None);
    } else {
        let mut tree = eff.document().build_tree();
        eff.document_mut()
            .apply_custom_mutation(&cm, &target_id, Some(&mut tree));
    }
    eff.document_mut().apply_document_actions(&cm);

    ExecResult::ok_msg(format!("applied '{}' to node '{}'", id, target_id))
}

fn resolve_target_id(doc: &MindMapDocument, explicit: Option<&str>) -> Result<String, String> {
    if let Some(id) = explicit {
        if doc.mindmap.nodes.contains_key(id) {
            return Ok(id.to_string());
        }
        return Err(format!("no node with id '{}'", id));
    }
    match &doc.selection {
        SelectionState::Single(id) => Ok(id.clone()),
        // Section selection: route to the section's owning node.
        // The mutation's own `target_scope` decides whether it
        // lands on the container, the sections, or both —
        // resolving to the owning node is the right input shape
        // for `apply_custom_mutation`. Pre-fix this errored out,
        // forcing the user to deselect → re-select-as-Single
        // before `mutation apply` would work from a section
        // click.
        SelectionState::Section(s) => Ok(s.node_id.clone()),
        SelectionState::SectionRange { sel, .. } => Ok(sel.node_id.clone()),
        _ => Err(
            "mutation apply needs a single-node or section selection, or an explicit <node-id>".to_string(),
        ),
    }
}

fn help(descent: &Descent, args: &Args, eff: &ConsoleEffects) -> ExecResult {
    let id = match descent.slot_value(args).get(0) {
        Some(s) => s,
        None => return ExecResult::err("mutation help needs an id"),
    };
    let cm = match eff.document().mutation_registry.get(id) {
        Some(cm) => cm,
        None => return ExecResult::err(format!("unknown mutation: {}", id)),
    };
    let source = eff
        .document
        .mutation_sources
        .get(id)
        .map(|tier| tier.label())
        .unwrap_or("unknown");

    let mut lines = vec![
        format!("{} \u{2014} {}", cm.id, cm.name),
        format!("source: {}", source),
        format!("scope: {}", target_scope_label(&cm.target_scope)),
        format!("behavior: {}", behavior_label(&cm.behavior)),
        format!(
            "contexts: {}",
            if cm.contexts.is_empty() {
                "(none \u{2192} treated as internal)".to_string()
            } else {
                cm.contexts.join(", ")
            }
        ),
    ];
    if !cm.description.is_empty() {
        lines.push(String::new());
        for l in cm.description.lines() {
            lines.push(l.to_string());
        }
    }
    ExecResult::lines(lines)
}

/// `mutation inspect <id>` — a terser sibling to `help` aimed at
/// debugging silent-failure scenarios. Reports the layer source,
/// whether the mutation is internal, whether it has a tree
/// mutator, whether it has document actions, and whether a Rust
/// handler will intercept it on apply. Intended as the first-stop
/// command when `mutation apply` appears to do nothing.
fn inspect(descent: &Descent, args: &Args, eff: &ConsoleEffects) -> ExecResult {
    let id = match descent.slot_value(args).get(0) {
        Some(s) => s,
        None => return ExecResult::err("mutation inspect needs an id"),
    };
    let cm = match eff.document().mutation_registry.get(id) {
        Some(cm) => cm,
        None => return ExecResult::err(format!("unknown mutation: {}", id)),
    };
    let source = eff
        .document
        .mutation_sources
        .get(id)
        .map(|tier| tier.label())
        .unwrap_or("unknown");

    let visibility = if cm.is_internal() {
        "internal (hidden from `mutation list`, refused by `mutation apply`)"
    } else if cm.targets_map() {
        "user-facing (listed in `mutation list`, runnable via `mutation apply`)"
    } else {
        "user-facing (no `map.*` context tag — will not appear in default `mutation list`)"
    };

    let payload = match (cm.mutator.is_some(), cm.document_actions.is_empty()) {
        (true, false) => "tree mutator + document actions",
        (true, true) => "tree mutator only",
        (false, false) => "document actions only (no tree effect)",
        (false, true) => "NO PAYLOAD — this mutation is effectively a no-op",
    };

    let dispatch = if eff.document().will_dispatch_to_handler(id) {
        "Rust handler (imperative; mutator AST ignored)"
    } else if cm.mutator.is_some() {
        "declarative (walks the mutator AST at apply time)"
    } else if !cm.document_actions.is_empty() {
        "document-actions only"
    } else {
        "no dispatch — mutation would silently skip on apply"
    };

    let reach = cm
        .mutator
        .as_ref()
        .map(|m| format!("{:?}", baumhard::mindmap::custom_mutation::mutator_reach(m)))
        .unwrap_or_else(|| "n/a (no mutator)".to_string());

    ExecResult::lines(vec![
        format!("{} \u{2014} {}", cm.id, cm.name),
        format!("source: {}", source),
        format!("visibility: {}", visibility),
        format!("payload: {}", payload),
        format!("dispatch: {}", dispatch),
        format!("declared scope: {}", target_scope_label(&cm.target_scope)),
        format!("mutator static reach: {}", reach),
        format!("behavior: {}", behavior_label(&cm.behavior)),
    ])
}

/// Human-friendly name for a `TargetScope`, in the same casing the
/// format doc uses. `{:?}` debug formatting produced `SelfAndDescendants`
/// which reads as Rust identifier noise; this spells it out.
fn target_scope_label(s: &baumhard::mindmap::custom_mutation::TargetScope) -> &'static str {
    use baumhard::mindmap::custom_mutation::TargetScope::*;
    match s {
        SelfOnly => "self only",
        Children => "children",
        Descendants => "descendants",
        SelfAndDescendants => "self and descendants",
        Parent => "parent",
        Siblings => "siblings",
        SectionsOnly => "sections only",
    }
}

/// Human-friendly name for a `MutationBehavior`.
fn behavior_label(b: &baumhard::mindmap::custom_mutation::MutationBehavior) -> &'static str {
    use baumhard::mindmap::custom_mutation::MutationBehavior::*;
    match b {
        Persistent => "persistent (commits to model, reversible via undo)",
        Toggle => "toggle (visual only, reverses on re-trigger)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::source_tier::SourceTier;
    use baumhard::mindmap::custom_mutation::{CustomMutation, TargetScope};
    use baumhard::util::geometry::almost_equal_f64;

    /// Build a fresh doc by loading the testament map, then overwrite
    /// the registry + sources with the supplied fixtures. Routes
    /// through the process-wide cache in
    /// `document::tests_common::load_test_doc` — see that helper
    /// for the FONT_SYSTEM-lock-contention rationale.
    fn fixture_doc(reg: Vec<(&str, CustomMutation)>, sources: Vec<(&str, SourceTier)>) -> MindMapDocument {
        let mut doc = crate::application::document::tests_common::load_test_doc();
        doc.mutation_registry = reg.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        doc.mutation_sources = sources.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        doc
    }

    fn make_cm(id: &str, contexts: Vec<&str>, description: &str) -> CustomMutation {
        crate::application::document::tests_common::TestNudgeMutation::new(id, TargetScope::SelfOnly)
            .magnitude(1.0)
            .contexts(contexts.into_iter().map(String::from).collect())
            .description(description)
            .build()
    }

    use crate::application::console::tests::fixtures::{first_node_id, join_lines as joined, run};

    #[test]
    fn list_hides_internal_by_default() {
        let mut doc = fixture_doc(
            vec![
                ("public", make_cm("public", vec!["map.node"], "d")),
                ("secret", make_cm("secret", vec!["internal"], "d")),
            ],
            vec![],
        );
        match run("mutation list", &mut doc) {
            ExecResult::Lines(ls) => {
                let all = joined(&ls);
                assert!(all.contains("public"));
                assert!(!all.contains("secret"));
            }
            other => panic!("expected Lines, got {:?}", other),
        }
    }

    #[test]
    fn list_all_shows_internals() {
        let mut doc = fixture_doc(vec![("secret", make_cm("secret", vec!["internal"], "d"))], vec![]);
        match run("mutation list --all", &mut doc) {
            ExecResult::Lines(ls) => assert!(ls.iter().any(|l| l.text.contains("secret"))),
            other => panic!("expected Lines, got {:?}", other),
        }
    }

    #[test]
    fn list_filter_substring_matches_id() {
        let mut doc = fixture_doc(
            vec![
                ("grow-font", make_cm("grow-font", vec!["map.node"], "d")),
                ("shrink-font", make_cm("shrink-font", vec!["map.node"], "d")),
            ],
            vec![],
        );
        match run("mutation list grow", &mut doc) {
            ExecResult::Lines(ls) => {
                let all = joined(&ls);
                assert!(all.contains("grow-font"));
                assert!(!all.contains("shrink-font"));
            }
            other => panic!("expected Lines, got {:?}", other),
        }
    }

    #[test]
    fn apply_unknown_id_returns_err() {
        let mut doc = fixture_doc(vec![], vec![]);
        match run("mutation apply no-such-id", &mut doc) {
            ExecResult::Err(s) => assert!(s.contains("unknown mutation")),
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn apply_internal_returns_err() {
        let mut doc = fixture_doc(vec![("secret", make_cm("secret", vec!["internal"], "d"))], vec![]);
        match run("mutation apply secret", &mut doc) {
            ExecResult::Err(s) => assert!(s.contains("internal")),
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn apply_without_selection_returns_err() {
        let mut doc = fixture_doc(vec![("nudge", make_cm("nudge", vec!["map.node"], "d"))], vec![]);
        match run("mutation apply nudge", &mut doc) {
            // Wording was widened to mention "section selection"
            // when Tier-Review-Response-3 made section selections
            // a valid input — the test now matches on the
            // narrower "needs a single-node or section selection"
            // phrase shared by both branches.
            ExecResult::Err(s) => assert!(s.contains("single-node or section selection")),
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn help_unknown_returns_err() {
        let mut doc = fixture_doc(vec![], vec![]);
        match run("mutation help nope", &mut doc) {
            ExecResult::Err(s) => assert!(s.contains("unknown mutation")),
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn help_known_includes_source_and_contexts() {
        let mut doc = fixture_doc(
            vec![(
                "grow-font",
                make_cm("grow-font", vec!["map.node", "map.tree"], "The description"),
            )],
            vec![("grow-font", SourceTier::App)],
        );
        match run("mutation help grow-font", &mut doc) {
            ExecResult::Lines(ls) => {
                let all = joined(&ls);
                assert!(all.contains("grow-font"));
                assert!(all.contains("source: app"));
                assert!(all.contains("map.node, map.tree"));
                assert!(all.contains("The description"));
            }
            other => panic!("expected Lines, got {:?}", other),
        }
    }

    #[test]
    fn apply_uses_explicit_node_id_when_provided() {
        let mut doc = fixture_doc(vec![("nudge", make_cm("nudge", vec!["map.node"], "d"))], vec![]);
        // Pick the root node of testament, selection still empty.
        let node_id = first_node_id(&doc);
        let line = format!("mutation apply nudge {}", node_id);
        match run(&line, &mut doc) {
            ExecResult::Ok(s) => assert!(s.contains(&node_id)),
            other => panic!("expected Ok with node id, got {:?}", other),
        }
    }

    /// End-to-end: applying a persistent mutation through the
    /// console verb pushes an undo entry, mutates the model, and
    /// sets `dirty`. Calling `undo()` reverses the mutation and
    /// leaves the doc clean relative to the pre-apply state. §T1
    /// fundamental — every user-facing mutation path gets an undo
    /// round-trip test.
    #[test]
    fn apply_pushes_undo_mutates_model_and_round_trips() {
        let mut doc = fixture_doc(
            vec![(
                "nudge-right-5",
                make_cm("nudge-right-5", vec!["map.node"], "Nudge 5px right"),
            )],
            vec![],
        );
        let node_id = first_node_id(&doc);
        let before_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
        let before_undo_len = doc.undo_stack.len();

        let line = format!("mutation apply nudge-right-5 {}", node_id);
        match run(&line, &mut doc) {
            ExecResult::Ok(_) => {}
            other => panic!("expected Ok, got {:?}", other),
        }

        // A persistent mutation must have pushed exactly one undo
        // entry and set the dirty flag; the model must reflect the
        // nudge (our fixture CM moves x by +1.0).
        assert_eq!(doc.undo_stack.len(), before_undo_len + 1);
        assert!(doc.dirty, "dirty flag must be set after apply");
        let after_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
        // Compared in `f32`, and exactly. The forward path routes the
        // nudge through the tree, whose coordinates are `f32`, so the
        // model gets back a widened `f32`. At this fixture's x of
        // about −179 the widening that actually lands is 3.06e-7 —
        // and `f32(after) == f32(before + 1.0)` holds exactly, which
        // is what makes an exact compare the right one.
        //
        // The number that mattered is the *old* tolerance, not this
        // one: one f32 ULP at this coordinate is 2^-16 = 1.53e-5 (the
        // widening above is a fiftieth of it), so the `1e-6` that used
        // to stand here was fifteen times *below* one ULP. It could
        // not have absorbed a worst-case widening — half an ULP,
        // 7.6e-6 — and passed on the luck of the fixture rather than
        // by design. An `f64` ruler here measures the widening
        // instead of the mutation.
        assert_eq!(
            after_x as f32,
            (before_x + 1.0) as f32,
            "expected position.x to grow by 1.0 (got {} → {})",
            before_x,
            after_x
        );

        // `undo()` pops the entry and restores the pre-apply position.
        let popped = doc.undo();
        assert!(popped, "undo must report success");
        assert_eq!(doc.undo_stack.len(), before_undo_len);
        let restored_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
        // The undo *is* an `f64` restore — it writes back the
        // pre-mutation snapshot rather than re-deriving through the
        // tree — so here the tight ruler is the right one.
        assert!(
            almost_equal_f64(restored_x, before_x),
            "undo must restore the original position (got {} → {})",
            after_x,
            restored_x
        );
    }

    #[test]
    fn help_uses_human_readable_scope_and_behavior_labels() {
        let mut doc = fixture_doc(
            vec![("nudge", make_cm("nudge", vec!["map.node", "map.tree"], "d"))],
            vec![("nudge", SourceTier::App)],
        );
        match run("mutation help nudge", &mut doc) {
            ExecResult::Lines(ls) => {
                let all = joined(&ls);
                // No `{:?}` debug-format leakage.
                assert!(!all.contains("SelfOnly"), "help should not leak Rust enum names");
                assert!(
                    !all.contains("Persistent"),
                    "help should not leak Rust enum names"
                );
                // Human-readable replacements.
                assert!(all.contains("scope: self only"));
                assert!(all.contains("behavior: persistent"));
            }
            other => panic!("expected Lines, got {:?}", other),
        }
    }

    #[test]
    fn inspect_surfaces_dispatch_source_and_payload() {
        let mut doc = fixture_doc(
            vec![("nudge", make_cm("nudge", vec!["map.node"], "Nudge right"))],
            vec![("nudge", SourceTier::App)],
        );
        match run("mutation inspect nudge", &mut doc) {
            ExecResult::Lines(ls) => {
                let all = joined(&ls);
                assert!(all.contains("source: app"));
                assert!(all.contains("visibility:"));
                assert!(all.contains("payload: tree mutator only"));
                assert!(all.contains("dispatch: declarative"));
                assert!(all.contains("declared scope: self only"));
                assert!(all.contains("mutator static reach: SelfOnly"));
            }
            other => panic!("expected Lines, got {:?}", other),
        }
    }

    #[test]
    fn inspect_unknown_returns_err() {
        let mut doc = fixture_doc(vec![], vec![]);
        match run("mutation inspect nope", &mut doc) {
            ExecResult::Err(s) => assert!(s.contains("unknown mutation")),
            other => panic!("expected Err, got {:?}", other),
        }
    }

    /// Handler-id collision guard: when a user (or map, or inline)
    /// mutation takes the same `id` as a bundled handler, the
    /// registry picks the user's mutation by precedence — and
    /// dispatch must honour the user's declarative mutator rather
    /// than silently running the bundled Rust handler, which was
    /// written for the app-bundled mutation's shape. This test
    /// proves `will_dispatch_to_handler` returns `false` when the
    /// source is anything other than App, forcing the flat-apply
    /// path.
    #[test]
    fn user_override_of_bundled_id_takes_declarative_path() {
        let mut doc = crate::application::document::tests_common::load_test_doc();

        // User mutation shadowing the bundled `flower-layout` id.
        let user_cm = make_cm(
            "flower-layout",
            vec!["map.node"],
            "user-authored flower-layout override",
        );
        doc.build_mutation_registry_with_app_and_user(&[], std::slice::from_ref(&user_cm));
        // The bundled handlers registry still has `flower-layout`
        // because a real app also registers them; the test
        // simulates that by inserting directly.
        doc.mutation_handlers.insert(
            "flower-layout".to_string(),
            crate::application::document::mutations::flower_layout::apply,
        );

        assert!(
            !doc.will_dispatch_to_handler("flower-layout"),
            "user-sourced override must bypass the bundled handler"
        );

        // Now add the bundled version and rebuild — the app source
        // should win when no user shadow is present.
        let mut app_cm = user_cm.clone();
        app_cm.description = "bundled".to_string();
        doc.build_mutation_registry_with_app_and_user(&[app_cm], &[]);
        assert!(
            doc.will_dispatch_to_handler("flower-layout"),
            "app-sourced bundled mutation must dispatch to its handler"
        );
    }
}
