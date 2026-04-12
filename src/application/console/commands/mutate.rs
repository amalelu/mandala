//! `mutate <list|run|bind|unbind>` — inspect and apply custom
//! mutations.
//!
//! - `mutate list` prints every mutation currently in the merged
//!   registry (user + map + inline), with source and target scope.
//! - `mutate run <id> [node_id]` applies a mutation to the current
//!   Single-selected node (or the explicit node id if supplied).
//!   Routes through `MindMapDocument::apply_custom_mutation` so undo
//!   and model sync are already handled — the command sets a
//!   deferred `RunMutationRequest` on `ConsoleEffects`, and the
//!   dispatcher in `app.rs` does the actual call (it has the tree).
//! - `mutate bind` / `mutate unbind` are stubs until commit 4.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{
    BindMutationRequest, ConsoleContext, ConsoleEffects, ExecResult, RunMutationRequest,
};
use crate::application::document::SelectionState;

pub const SUBCOMMANDS: &[&str] = &["list", "run", "bind", "unbind"];

pub const COMMAND: Command = Command {
    name: "mutate",
    aliases: &[],
    summary: "Inspect and apply custom mutations",
    usage: "mutate <list | run <id> [node_id] | bind <key> <id> | unbind <key>>",
    tags: &["mutate", "mutation", "custom", "run", "bind"],
    applicable: always,
    complete: complete_mutate,
    execute: execute_mutate,
};

fn complete_mutate(state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
    match state.cursor_token {
        1 => enum_completion(SUBCOMMANDS, state.partial),
        2 => {
            let sub = state.tokens.get(1).map(|s| s.as_str()).unwrap_or("");
            if sub.eq_ignore_ascii_case("run") {
                let ids: Vec<&str> = ctx.mutation_registry.keys().map(|s| s.as_str()).collect();
                enum_completion(&ids, state.partial)
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn execute_mutate(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let sub = match args.positional(0) {
        Some(s) => s.to_ascii_lowercase(),
        None => return ExecResult::err("usage: mutate <list|run|bind|unbind>"),
    };
    match sub.as_str() {
        "list" => execute_list(eff),
        "run" => execute_run(args, eff),
        "bind" => execute_bind(args, eff),
        "unbind" => execute_unbind(args, eff),
        other => ExecResult::err(format!("unknown mutate subcommand '{}'", other)),
    }
}

fn execute_bind(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let combo = match args.positional(1) {
        Some(k) => k.to_string(),
        None => return ExecResult::err("usage: mutate bind <key> <mutation_id>"),
    };
    let mutation_id = match args.positional(2) {
        Some(id) => id.to_string(),
        None => return ExecResult::err("usage: mutate bind <key> <mutation_id>"),
    };
    if !eff.document.mutation_registry.contains_key(&mutation_id) {
        return ExecResult::err(format!("no mutation with id '{}'", mutation_id));
    }
    eff.bind_mutation = Some(BindMutationRequest {
        combo: combo.clone(),
        mutation_id: mutation_id.clone(),
    });
    ExecResult::ok_msg(format!("bound {} → {}", combo, mutation_id))
}

fn execute_unbind(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let combo = match args.positional(1) {
        Some(k) => k.to_string(),
        None => return ExecResult::err("usage: mutate unbind <key>"),
    };
    eff.unbind_mutation = Some(combo.clone());
    ExecResult::ok_msg(format!("unbind {} requested", combo))
}

fn execute_list(eff: &mut ConsoleEffects) -> ExecResult {
    let registry = &eff.document.mutation_registry;
    if registry.is_empty() {
        return ExecResult::ok_msg("no mutations registered");
    }
    // Classify each mutation by source: inline if any node has it in
    // `inline_mutations` with the same id; else map if the map
    // carries it in `custom_mutations`; else user (defined in the
    // user-mutations file and merged at the lowest precedence).
    let mut lines: Vec<String> = Vec::with_capacity(registry.len() + 1);
    lines.push(format!(
        "{:<24}  {:<6}  {:<20}  {}",
        "id", "src", "scope", "name"
    ));
    let mut ids: Vec<&String> = registry.keys().collect();
    ids.sort();
    for id in ids {
        let m = &registry[id];
        let src = classify_source(eff.document, id);
        let scope = format!("{:?}", m.target_scope);
        lines.push(format!(
            "{:<24}  {:<6}  {:<20}  {}",
            id, src, scope, m.name
        ));
    }
    ExecResult::Lines(lines)
}

fn classify_source(
    doc: &crate::application::document::MindMapDocument,
    id: &str,
) -> &'static str {
    if doc
        .mindmap
        .nodes
        .values()
        .any(|n| n.inline_mutations.iter().any(|m| m.id == id))
    {
        "inline"
    } else if doc.mindmap.custom_mutations.iter().any(|m| m.id == id) {
        "map"
    } else {
        "user"
    }
}

fn execute_run(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let mutation_id = match args.positional(1) {
        Some(id) => id.to_string(),
        None => return ExecResult::err("usage: mutate run <id> [node_id]"),
    };
    if !eff.document.mutation_registry.contains_key(&mutation_id) {
        return ExecResult::err(format!("no mutation with id '{}'", mutation_id));
    }
    let node_id = match args.positional(2) {
        Some(id) => id.to_string(),
        None => match &eff.document.selection {
            SelectionState::Single(s) => s.clone(),
            _ => {
                return ExecResult::err(
                    "mutate run requires a single-node selection or an explicit node id",
                )
            }
        },
    };
    if !eff.document.mindmap.nodes.contains_key(&node_id) {
        return ExecResult::err(format!("no node with id '{}'", node_id));
    }
    eff.run_mutation = Some(RunMutationRequest {
        mutation_id: mutation_id.clone(),
        node_id: node_id.clone(),
    });
    ExecResult::ok_msg(format!("running mutation {} on node {}", mutation_id, node_id))
}
