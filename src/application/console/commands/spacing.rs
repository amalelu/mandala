//! `spacing value=4.0` or `spacing value=tight` — glyph-spacing
//! setter for the selected edge. Accepts named presets
//! (tight / normal / wide) or a raw float in the preset's unit.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const PRESETS: &[(&str, f32)] = &[("tight", 0.0), ("normal", 2.0), ("wide", 6.0)];
pub const VALUE_PRESETS: &[&str] = &["tight", "normal", "wide"];
pub const KEYS: &[&str] = &["value"];

pub const COMMAND: Command = Command {
    name: "spacing",
    aliases: &[],
    summary: "Set the glyph spacing of the selected edge",
    usage: "spacing value=<tight|normal|wide | <float>>",
    tags: &["edge", "spacing", "tight", "wide"],
    applicable: edge_selected,
    complete: complete_spacing,
    execute: execute_spacing,
};

fn complete_spacing(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
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
        CompletionContext::KvValue { key } if key == "value" => {
            prefix_filter(VALUE_PRESETS, state.partial)
        }
        _ => Vec::new(),
    }
}

fn execute_spacing(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match eff.document.selection.selected_edge_or_portal_edge() {
        Some(e) => e,
        None => return ExecResult::err("no edge selected"),
    };
    let v = match args.kv("value") {
        Some(v) => v,
        None => return ExecResult::err("usage: spacing value=<tight|normal|wide | <float>>"),
    };
    let value = if let Some((_, preset)) = PRESETS.iter().find(|(n, _)| *n == v) {
        *preset
    } else {
        match v.parse::<f32>() {
            Ok(x) => x,
            Err(_) => {
                return ExecResult::err(format!(
                    "'{}' must be a preset (tight|normal|wide) or a float",
                    v
                ))
            }
        }
    };
    let changed = eff.document.set_edge_spacing(&er, value);
    if changed {
        ExecResult::ok_msg(format!("spacing set to {}", v))
    } else {
        ExecResult::ok_msg(format!("spacing already {}", v))
    }
}
