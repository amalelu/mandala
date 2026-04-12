//! `body <dot|dash|double|wave|chain>` — set the body glyph of the
//! selected edge to a named preset.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::{body_is, edge_selected};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

/// Body-glyph presets. Kept as `(name, glyph)` pairs so the command
/// table stays one source of truth for both completion and exec.
pub const PRESETS: &[(&str, &str)] = &[
    ("dot", "\u{00B7}"),    // ·
    ("dash", "\u{2500}"),   // ─
    ("double", "\u{2550}"), // ═
    ("wave", "\u{223C}"),   // ∼
    ("chain", "\u{22EF}"),  // ⋯
];

pub const COMMAND: Command = Command {
    name: "body",
    aliases: &[],
    summary: "Set the body glyph of the selected edge",
    usage: "body <dot|dash|double|wave|chain>",
    tags: &["edge", "body", "glyph", "style", "connection"],
    applicable: edge_selected,
    complete: complete_body,
    execute: execute_body,
};

fn complete_body(state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
    if state.cursor_token != 1 {
        return Vec::new();
    }
    // Hide already-set values from completion (completion may
    // optionally skip already-set values — a small kindness).
    let names: Vec<&'static str> = PRESETS
        .iter()
        .filter(|(_, glyph)| !body_is(ctx, glyph))
        .map(|(n, _)| *n)
        .collect();
    enum_completion(&names, state.partial)
}

fn execute_body(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match &eff.document.selection {
        SelectionState::Edge(e) => e.clone(),
        _ => return ExecResult::err("no edge selected"),
    };
    let name = match args.positional(0) {
        Some(n) => n.to_ascii_lowercase(),
        None => return ExecResult::err("usage: body <dot|dash|double|wave|chain>"),
    };
    let glyph = match PRESETS.iter().find(|(n, _)| *n == name) {
        Some((_, g)) => *g,
        None => {
            return ExecResult::err(format!(
                "unknown body '{}'; expected one of dot|dash|double|wave|chain",
                name
            ))
        }
    };
    let changed = eff.document.set_edge_body_glyph(&er, glyph);
    if changed {
        ExecResult::ok_msg(format!("body set to {}", name))
    } else {
        ExecResult::ok_msg(format!("body already {}", name))
    }
}
