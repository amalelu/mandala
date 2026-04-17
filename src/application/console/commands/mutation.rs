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
    summary: "List and apply registered mutations",
    usage: "mutation <list [--all] [filter] | apply <id> [node-id] | help <id>>",
    tags: &["mut", "apply", "run", "list"],
    applicable: always,
    complete: complete_mutation,
    execute: execute_mutation,
};

fn complete_mutation(state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
    match state.cursor_token {
        1 => {
            // Sub-command slot.
            let partial = state.partial.to_ascii_lowercase();
            ["list", "apply", "help"]
                .iter()
                .filter(|s| s.starts_with(&partial))
                .map(|s| Completion {
                    text: s.to_string(),
                    display: s.to_string(),
                    hint: None,
                })
                .collect()
        }
        2 if matches!(state.tokens.get(1).map(String::as_str), Some("apply") | Some("help")) => {
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
        Some(other) => ExecResult::err(format!(
            "unknown mutation sub-command: {} (try list / apply / help)",
            other
        )),
        None => ExecResult::err("mutation needs a sub-command (list / apply / help)"),
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
    // The tree apply needs a fresh MindMapTree; we build one, hand it
    // to `apply_custom_mutation`, discard it afterwards (the renderer
    // rebuilds from the model on the next frame).
    let mut tree = eff.document.build_tree();
    eff.document.apply_custom_mutation(&cm, &target_id, &mut tree);
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
        format!("{} — {}", cm.id, cm.name),
        format!("source: {}", source),
        format!("scope: {:?}", cm.target_scope),
        format!("behavior: {:?}", cm.behavior),
        format!(
            "contexts: {}",
            if cm.contexts.is_empty() {
                "(none → treated as internal)".to_string()
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

fn source_label(s: &crate::application::document::animations::MutationSource) -> &'static str {
    use crate::application::document::animations::MutationSource::*;
    match s {
        App => "app",
        User => "user",
        Map => "map",
        Inline => "inline",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::tokenize;
    use crate::application::document::animations::MutationSource;
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
}
