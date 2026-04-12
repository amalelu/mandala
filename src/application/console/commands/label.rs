//! `label <edit|clear|position|set>` — label operations on the
//! selected edge. `label edit` hands off to the inline label editor;
//! `label set "text"` writes the label directly; `label position`
//! moves the glyph along the path.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const SUBCOMMANDS: &[&str] = &["edit", "clear", "position", "set"];
pub const POSITIONS: &[&str] = &["start", "middle", "end"];

pub const COMMAND: Command = Command {
    name: "label",
    aliases: &[],
    summary: "Edit, clear, set, or reposition the label on the selected edge",
    usage: "label <edit | clear | set \"<text>\" | position <start|middle|end>>",
    tags: &["edge", "label", "text", "position"],
    applicable: edge_selected,
    complete: complete_label,
    execute: execute_label,
};

fn complete_label(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match state.cursor_token {
        1 => enum_completion(SUBCOMMANDS, state.partial),
        2 => {
            let sub = state.tokens.get(1).map(|s| s.as_str()).unwrap_or("");
            if sub.eq_ignore_ascii_case("position") {
                enum_completion(POSITIONS, state.partial)
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn execute_label(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match &eff.document.selection {
        SelectionState::Edge(e) => e.clone(),
        _ => return ExecResult::err("no edge selected"),
    };
    let sub = match args.positional(0) {
        Some(s) => s.to_ascii_lowercase(),
        None => return ExecResult::err("usage: label <edit|clear|set|position>"),
    };
    match sub.as_str() {
        "edit" => {
            eff.open_label_edit = Some(er);
            eff.close_console = true;
            ExecResult::ok_empty()
        }
        "clear" => {
            let changed = eff.document.set_edge_label(&er, None);
            if changed {
                ExecResult::ok_msg("label cleared")
            } else {
                ExecResult::ok_msg("label already empty")
            }
        }
        "set" => {
            let text = match args.positional(1) {
                Some(t) => t.to_string(),
                None => return ExecResult::err("usage: label set \"<text>\""),
            };
            let changed = eff.document.set_edge_label(&er, Some(text));
            if changed {
                ExecResult::ok_msg("label updated")
            } else {
                ExecResult::ok_msg("label unchanged")
            }
        }
        "position" => {
            let pos_name = match args.positional(1) {
                Some(p) => p.to_ascii_lowercase(),
                None => return ExecResult::err("usage: label position <start|middle|end>"),
            };
            let t = match pos_name.as_str() {
                "start" => 0.0,
                "middle" => 0.5,
                "end" => 1.0,
                other => {
                    return ExecResult::err(format!(
                        "position '{}' must be start|middle|end",
                        other
                    ))
                }
            };
            let changed = eff.document.set_edge_label_position(&er, t);
            if changed {
                ExecResult::ok_msg(format!("label position set to {}", pos_name))
            } else {
                ExecResult::ok_msg(format!("label already at {}", pos_name))
            }
        }
        other => ExecResult::err(format!("unknown label subcommand '{}'", other)),
    }
}
