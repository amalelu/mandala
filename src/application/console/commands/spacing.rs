//! `spacing <tight|normal|wide>` — set the glyph-spacing preset for
//! the selected edge's connection path.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const PRESETS: &[(&str, f32)] = &[("tight", 0.0), ("normal", 2.0), ("wide", 6.0)];
pub const NAMES: &[&str] = &["tight", "normal", "wide"];

pub const COMMAND: Command = Command {
    name: "spacing",
    aliases: &[],
    summary: "Set the glyph spacing of the selected edge",
    usage: "spacing <tight|normal|wide>",
    tags: &["edge", "spacing", "tight", "wide", "dense", "airy"],
    applicable: edge_selected,
    complete: complete_spacing,
    execute: execute_spacing,
};

fn complete_spacing(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    if state.cursor_token != 1 {
        return Vec::new();
    }
    enum_completion(NAMES, state.partial)
}

fn execute_spacing(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match &eff.document.selection {
        SelectionState::Edge(e) => e.clone(),
        _ => return ExecResult::err("no edge selected"),
    };
    let name = match args.positional(0) {
        Some(n) => n.to_ascii_lowercase(),
        None => return ExecResult::err("usage: spacing <tight|normal|wide>"),
    };
    let value = match PRESETS.iter().find(|(n, _)| *n == name) {
        Some((_, v)) => *v,
        None => {
            return ExecResult::err(format!(
                "spacing '{}' must be one of tight|normal|wide",
                name
            ))
        }
    };
    let changed = eff.document.set_edge_spacing(&er, value);
    if changed {
        ExecResult::ok_msg(format!("spacing set to {}", name))
    } else {
        ExecResult::ok_msg(format!("spacing already {}", name))
    }
}
