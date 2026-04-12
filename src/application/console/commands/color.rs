//! `color <pick|accent|edge|fg|reset>` — set the edge glyph color
//! on the selected edge. `color pick` hands off to the glyph-wheel
//! picker, dispatching on selection kind (edge or portal).
//!
//! Portal-specific color overrides live under the `portal` command —
//! this command targets edges. `color pick` is the exception and
//! routes to either `ColorTarget::Edge` or `ColorTarget::Portal`.

use super::Command;
use crate::application::color_picker::ColorTarget;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::{edge_or_portal_selected, edge_selected};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const SUBCOMMANDS: &[&str] = &["pick", "accent", "edge", "fg", "reset"];

pub const COMMAND: Command = Command {
    name: "color",
    aliases: &[],
    summary: "Set edge color, pick via glyph wheel, or reset",
    usage: "color <pick|accent|edge|fg|reset>",
    tags: &["edge", "color", "pick", "wheel", "theme", "accent"],
    applicable: edge_or_portal_selected,
    complete: complete_color,
    execute: execute_color,
};

fn complete_color(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    if state.cursor_token != 1 {
        return Vec::new();
    }
    enum_completion(SUBCOMMANDS, state.partial)
}

fn execute_color(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let sub = match args.positional(0) {
        Some(s) => s.to_ascii_lowercase(),
        None => return ExecResult::err("usage: color <pick|accent|edge|fg|reset>"),
    };
    match sub.as_str() {
        "pick" => match &eff.document.selection {
            SelectionState::Edge(er) => {
                eff.open_color_picker = Some(ColorTarget::Edge(er.clone()));
                eff.close_console = true;
                ExecResult::ok_empty()
            }
            SelectionState::Portal(pr) => {
                eff.open_color_picker = Some(ColorTarget::Portal(pr.clone()));
                eff.close_console = true;
                ExecResult::ok_empty()
            }
            _ => ExecResult::err("no edge or portal selected"),
        },
        "accent" | "edge" | "fg" | "reset" => {
            let er = match &eff.document.selection {
                SelectionState::Edge(e) if edge_selected(&ConsoleContext::from_document(eff.document)) => e.clone(),
                _ => return ExecResult::err("no edge selected"),
            };
            let color = match sub.as_str() {
                "accent" => Some("var(--accent)"),
                "edge" => Some("var(--edge)"),
                "fg" => Some("var(--fg)"),
                "reset" => None,
                _ => unreachable!(),
            };
            let changed = eff.document.set_edge_color(&er, color);
            if changed {
                ExecResult::ok_msg(format!("edge color set to {}", sub))
            } else {
                ExecResult::ok_msg(format!("edge color already {}", sub))
            }
        }
        other => ExecResult::err(format!(
            "unknown color subcommand '{}'; expected one of {}",
            other,
            SUBCOMMANDS.join("|")
        )),
    }
}
