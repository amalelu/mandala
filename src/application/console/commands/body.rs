//! `body glyph=dash` — set the body glyph of the selected edge.
//! Edge-specific; the concept doesn't generalize beyond edges.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
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

pub const KEYS: &[&str] = &["glyph"];

pub const COMMAND: Command = Command {
    name: "body",
    aliases: &[],
    summary: "Set the body glyph of the selected edge",
    usage: "body glyph=<dot|dash|double|wave|chain>",
    tags: &["edge", "body", "glyph", "style"],
    applicable: edge_selected,
    complete: complete_body,
    execute: execute_body,
};

fn complete_body(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { .. } => KEYS
            .iter()
            .filter(|k| k.starts_with(state.partial))
            .map(|k| Completion {
                text: format!("{}=", k),
                display: format!("{}=", k),
                hint: None,
            })
            .collect(),
        CompletionContext::KvValue { key } if key == "glyph" => {
            let names: Vec<&str> = PRESETS.iter().map(|(n, _)| *n).collect();
            prefix_filter(&names, state.partial)
        }
        _ => Vec::new(),
    }
}

fn execute_body(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Accept a portal-label selection too: the same `body` glyph
    // drives the portal marker symbol, so `body glyph=…` on a
    // portal label retargets the owning edge.
    let er = match eff.document.selection.selected_edge_or_portal_edge() {
        Some(e) => e,
        None => return ExecResult::err("no edge selected"),
    };
    let name = match args.kv("glyph") {
        Some(n) => n.to_ascii_lowercase(),
        None => return ExecResult::err("usage: body glyph=<dot|dash|double|wave|chain>"),
    };
    let glyph = match PRESETS.iter().find(|(n, _)| *n == name) {
        Some((_, g)) => *g,
        None => {
            return ExecResult::err(format!(
                "glyph '{}' must be one of dot|dash|double|wave|chain",
                name
            ))
        }
    };
    let changed = eff.document.set_edge_body_glyph(&er, glyph);
    if changed {
        ExecResult::ok_msg(format!("body glyph set to {}", name))
    } else {
        ExecResult::ok_msg(format!("body glyph already {}", name))
    }
}
