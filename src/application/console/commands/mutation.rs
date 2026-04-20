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
use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{MindMapDocument, SelectionState};

pub const COMMAND: Command = Command {
    name: "mutation",
    aliases: &["mut"],
    summary: "List, apply, and inspect registered mutations",
    usage: "mutation <list [--all] [filter] | apply <id> [node-id] | help <id> | inspect <id>>",
    tags: &["mut", "apply", "run", "list", "inspect", "debug"],
    applicable: always,
    complete: complete_mutation,
    execute: execute_mutation,
};

fn complete_mutation(state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
    match state.cursor_token {
        1 => {
            // Sub-command slot.
            let partial = state.partial.to_ascii_lowercase();
            ["list", "apply", "help", "inspect"]
                .iter()
                .filter(|s| s.starts_with(&partial))
                .map(|s| Completion {
                    text: s.to_string(),
                    display: s.to_string(),
                    hint: None,
                })
                .collect()
        }
        2 if matches!(
            state.tokens.get(1).map(String::as_str),
            Some("apply") | Some("help") | Some("inspect")
        ) => {
            // Mutation id slot. Show user-facing mutations by default
            // (internal ones are reachable but not completed — they
            // can still be typed by exact id in debugging sessions).
            let partial = state.partial.to_ascii_lowercase();
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
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn execute_mutation(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    match args.positional(0) {
        Some("list") => list(args, eff),
        Some("apply") => apply(args, eff),
        Some("help") => help(args, eff),
        Some("inspect") => inspect(args, eff),
        Some(other) => ExecResult::err(format!(
            "unknown mutation sub-command: {} (try list / apply / help / inspect)",
            other
        )),
        None => ExecResult::err(
            "mutation needs a sub-command (list / apply / help / inspect)",
        ),
    }
}

fn list(args: &Args, eff: &ConsoleEffects) -> ExecResult {
    // `--all` is recognized as either a positional or a bare flag; the
    // parser treats it as a positional in either case.
    let mut show_all = false;
    let mut filter: Option<&str> = None;
    for i in 1.. {
        match args.positional(i) {
            Some("--all") => show_all = true,
            Some(other) if filter.is_none() => filter = Some(other),
            Some(_) => {} // ignore extra positionals
            None => break,
        }
    }

    let doc = &eff.document;
    let mut rows: Vec<(&String, &baumhard::mindmap::custom_mutation::CustomMutation)> = doc
        .mutation_registry
        .iter()
        .filter(|(_, cm)| show_all || (cm.targets_map() && !cm.is_internal()))
        .filter(|(id, cm)| match filter {
            Some(f) => {
                let fl = f.to_ascii_lowercase();
                id.to_ascii_lowercase().contains(&fl)
                    || cm.name.to_ascii_lowercase().contains(&fl)
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
    ExecResult::Lines(lines)
}

fn apply(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let id = match args.positional(1) {
        Some(s) => s.to_string(),
        None => return ExecResult::err("mutation apply needs an id (`mutation apply <id>`)"),
    };
    let explicit_node = args.positional(2).map(str::to_string);

    // Look up the mutation. `.clone()` so we don't hold a borrow while
    // mutating `eff.document` below.
    let cm = match eff.document.mutation_registry.get(&id) {
        Some(cm) => cm.clone(),
        None => return ExecResult::err(format!("unknown mutation: {}", id)),
    };
    if cm.is_internal() {
        return ExecResult::err(format!(
            "mutation '{}' is internal and not runnable from the console",
            id
        ));
    }

    let target_id = match resolve_target_id(eff.document, explicit_node.as_deref()) {
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
    if eff.document.will_dispatch_to_handler(&cm.id) {
        eff.document.apply_custom_mutation(&cm, &target_id, None);
    } else {
        let mut tree = eff.document.build_tree();
        eff.document
            .apply_custom_mutation(&cm, &target_id, Some(&mut tree));
    }
    eff.document.apply_document_actions(&cm);

    ExecResult::ok_msg(format!("applied '{}' to node '{}'", id, target_id))
}

fn resolve_target_id(
    doc: &MindMapDocument,
    explicit: Option<&str>,
) -> Result<String, String> {
    if let Some(id) = explicit {
        if doc.mindmap.nodes.contains_key(id) {
            return Ok(id.to_string());
        }
        return Err(format!("no node with id '{}'", id));
    }
    match &doc.selection {
        SelectionState::Single(id) => Ok(id.clone()),
        _ => Err("mutation apply needs a single-node selection or an explicit <node-id>"
            .to_string()),
    }
}

fn help(args: &Args, eff: &ConsoleEffects) -> ExecResult {
    let id = match args.positional(1) {
        Some(s) => s,
        None => return ExecResult::err("mutation help needs an id"),
    };
    let cm = match eff.document.mutation_registry.get(id) {
        Some(cm) => cm,
        None => return ExecResult::err(format!("unknown mutation: {}", id)),
    };
    let source = eff
        .document
        .mutation_sources
        .get(id)
        .map(source_label)
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
    ExecResult::Lines(lines)
}

/// `mutation inspect <id>` — a terser sibling to `help` aimed at
/// debugging silent-failure scenarios. Reports the layer source,
/// whether the mutation is internal, whether it has a tree
/// mutator, whether it has document actions, and whether a Rust
/// handler will intercept it on apply. Intended as the first-stop
/// command when `mutation apply` appears to do nothing.
fn inspect(args: &Args, eff: &ConsoleEffects) -> ExecResult {
    let id = match args.positional(1) {
        Some(s) => s,
        None => return ExecResult::err("mutation inspect needs an id"),
    };
    let cm = match eff.document.mutation_registry.get(id) {
        Some(cm) => cm,
        None => return ExecResult::err(format!("unknown mutation: {}", id)),
    };
    let source = eff
        .document
        .mutation_sources
        .get(id)
        .map(source_label)
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

    let dispatch = if eff.document.will_dispatch_to_handler(id) {
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

    ExecResult::Lines(vec![
        format!("{} \u{2014} {}", cm.id, cm.name),
        format!("source: {}", source),
        format!("visibility: {}", visibility),
        format!("payload: {}", payload),
        format!("dispatch: {}", dispatch),
        format!("declared scope: {}", target_scope_label(&cm.target_scope)),
        format!("mutator static reach: {}", reach),
        format!(
            "behavior: {}",
            behavior_label(&cm.behavior)
        ),
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

fn source_label(s: &crate::application::document::mutations_loader::MutationSource) -> &'static str {
    use crate::application::document::mutations_loader::MutationSource::*;
    // `MutationSource` is `#[non_exhaustive]`; the wildcard arm is a
    // forward-compat seam for future variants (e.g. plugin sources).
    // Currently unreachable because all variants are matched.
    #[allow(unreachable_patterns)]
    match s {
        App => "app",
        User => "user",
        Map => "map",
        Inline => "inline",
        _ => "(unknown source)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::tokenize;
    use crate::application::document::mutations_loader::MutationSource;
    use baumhard::mindmap::custom_mutation::{
        CustomMutation, MutationBehavior, TargetScope,
    };

    /// Build a fresh doc by loading the testament map, then overwrite
    /// the registry + sources with the supplied fixtures. Avoids
    /// depending on private helpers in other modules.
    fn fixture_doc(
        reg: Vec<(&str, CustomMutation)>,
        sources: Vec<(&str, MutationSource)>,
    ) -> MindMapDocument {
        let path = format!(
            "{}/maps/testament.mindmap.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut doc = MindMapDocument::load(&path).expect("testament map loads");
        doc.mutation_registry = reg
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        doc.mutation_sources = sources
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        doc
    }

    fn make_cm(id: &str, contexts: Vec<&str>, description: &str) -> CustomMutation {
        use baumhard::gfx_structs::area::GlyphAreaCommand;
        use baumhard::gfx_structs::mutator::Mutation;
        CustomMutation {
            id: id.to_string(),
            name: id.to_string(),
            description: description.to_string(),
            contexts: contexts.into_iter().map(String::from).collect(),
            mutator: Some(baumhard::mindmap::custom_mutation::scope::self_only(vec![
                Mutation::area_command(GlyphAreaCommand::NudgeRight(1.0)),
            ])),
            target_scope: TargetScope::SelfOnly,
            behavior: MutationBehavior::Persistent,
            predicate: None,
            document_actions: vec![],
            timing: None,
        }
    }

    fn run(line: &str, doc: &mut MindMapDocument) -> ExecResult {
        let toks = tokenize(line);
        let mut eff = ConsoleEffects::new(doc);
        execute_mutation(&Args::new(&toks[1..]), &mut eff)
    }

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
                let all = ls.join("\n");
                assert!(all.contains("public"));
                assert!(!all.contains("secret"));
            }
            other => panic!("expected Lines, got {:?}", other),
        }
    }

    #[test]
    fn list_all_shows_internals() {
        let mut doc = fixture_doc(
            vec![("secret", make_cm("secret", vec!["internal"], "d"))],
            vec![],
        );
        match run("mutation list --all", &mut doc) {
            ExecResult::Lines(ls) => assert!(ls.iter().any(|l| l.contains("secret"))),
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
                let all = ls.join("\n");
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
        let mut doc = fixture_doc(
            vec![("secret", make_cm("secret", vec!["internal"], "d"))],
            vec![],
        );
        match run("mutation apply secret", &mut doc) {
            ExecResult::Err(s) => assert!(s.contains("internal")),
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn apply_without_selection_returns_err() {
        let mut doc = fixture_doc(
            vec![("nudge", make_cm("nudge", vec!["map.node"], "d"))],
            vec![],
        );
        match run("mutation apply nudge", &mut doc) {
            ExecResult::Err(s) => assert!(s.contains("single-node selection")),
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
            vec![("grow-font", MutationSource::App)],
        );
        match run("mutation help grow-font", &mut doc) {
            ExecResult::Lines(ls) => {
                let all = ls.join("\n");
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
        let mut doc = fixture_doc(
            vec![("nudge", make_cm("nudge", vec!["map.node"], "d"))],
            vec![],
        );
        // Pick the root node of testament, selection still empty.
        let node_id = doc.mindmap.nodes.keys().next().unwrap().clone();
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
        let node_id = doc.mindmap.nodes.keys().next().unwrap().clone();
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
        assert!(
            (after_x - before_x - 1.0).abs() < 1e-6,
            "expected position.x to grow by 1.0 (got {} → {})",
            before_x,
            after_x
        );

        // `undo()` pops the entry and restores the pre-apply position.
        let popped = doc.undo();
        assert!(popped, "undo must report success");
        assert_eq!(doc.undo_stack.len(), before_undo_len);
        let restored_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
        assert!(
            (restored_x - before_x).abs() < 1e-6,
            "undo must restore the original position (got {} → {})",
            after_x,
            restored_x
        );
    }

    #[test]
    fn help_uses_human_readable_scope_and_behavior_labels() {
        let mut doc = fixture_doc(
            vec![(
                "nudge",
                make_cm("nudge", vec!["map.node", "map.tree"], "d"),
            )],
            vec![("nudge", MutationSource::App)],
        );
        match run("mutation help nudge", &mut doc) {
            ExecResult::Lines(ls) => {
                let all = ls.join("\n");
                // No `{:?}` debug-format leakage.
                assert!(!all.contains("SelfOnly"), "help should not leak Rust enum names");
                assert!(!all.contains("Persistent"), "help should not leak Rust enum names");
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
            vec![(
                "nudge",
                make_cm("nudge", vec!["map.node"], "Nudge right"),
            )],
            vec![("nudge", MutationSource::App)],
        );
        match run("mutation inspect nudge", &mut doc) {
            ExecResult::Lines(ls) => {
                let all = ls.join("\n");
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
        let path = format!(
            "{}/maps/testament.mindmap.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut doc = MindMapDocument::load(&path).expect("testament loads");

        // User mutation shadowing the bundled `flower-layout` id.
        let user_cm = make_cm(
            "flower-layout",
            vec!["map.node"],
            "user-authored flower-layout override",
        );
        doc.build_mutation_registry_with_app_and_user(&[], &[user_cm.clone()]);
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
