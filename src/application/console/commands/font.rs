//! `font <smaller|larger|reset>` — step the selected edge's glyph
//! font size by ±2pt or reset to the default.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const SUBCOMMANDS: &[&str] = &["smaller", "larger", "reset"];

pub const COMMAND: Command = Command {
    name: "font",
    aliases: &[],
    summary: "Step or reset the selected edge's glyph font size",
    usage: "font <smaller|larger|reset>",
    tags: &["edge", "font", "size", "smaller", "larger"],
    applicable: edge_selected,
    complete: complete_font,
    execute: execute_font,
};

fn complete_font(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    if state.cursor_token != 1 {
        return Vec::new();
    }
    enum_completion(SUBCOMMANDS, state.partial)
}

fn execute_font(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match &eff.document.selection {
        SelectionState::Edge(e) => e.clone(),
        _ => return ExecResult::err("no edge selected"),
    };
    let sub = match args.positional(0) {
        Some(s) => s.to_ascii_lowercase(),
        None => return ExecResult::err("usage: font <smaller|larger|reset>"),
    };
    let changed = match sub.as_str() {
        "smaller" => eff.document.set_edge_font_size_step(&er, -2.0),
        "larger" => eff.document.set_edge_font_size_step(&er, 2.0),
        "reset" => eff.document.reset_edge_font_size(&er),
        other => return ExecResult::err(format!("unknown font subcommand '{}'", other)),
    };
    if changed {
        ExecResult::ok_msg(format!("font {}", sub))
    } else {
        ExecResult::ok_msg(format!("font already at {}", sub))
    }
}
