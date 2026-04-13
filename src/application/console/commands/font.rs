//! `font size=14` — kv-form font setter dispatched through
//! `HasFontSize`. Fans out over the selection so multi-selected
//! nodes all change at once.

use super::color::finalize_report;
use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::traits::{apply_kvs, HasFontSize, Outcome};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const KEYS: &[&str] = &["size"];
/// Preset sizes surfaced in completion. Users can type any positive
/// float; the preset list just makes the popup useful.
pub const SIZE_PRESETS: &[&str] = &["10", "12", "14", "16", "18", "24", "32"];

pub const COMMAND: Command = Command {
    name: "font",
    aliases: &[],
    summary: "Set font size on the selected component(s)",
    usage: "font size=<points>",
    tags: &["font", "size", "pt", "smaller", "larger"],
    applicable: always,
    complete: complete_font,
    execute: execute_font,
};

fn complete_font(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { .. } => KEYS
            .iter()
            .filter(|k| k.starts_with(state.partial))
            .map(|k| Completion {
                text: format!("{}=", k),
                display: format!("{}=", k),
                hint: Some("size in points".into()),
            })
            .collect(),
        CompletionContext::KvValue { key } if key == "size" => {
            prefix_filter(SIZE_PRESETS, state.partial)
        }
        _ => Vec::new(),
    }
}

fn execute_font(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let kvs: Vec<(String, String)> = args
        .kvs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if kvs.is_empty() {
        return ExecResult::err("usage: font size=<points>");
    }

    let report = apply_kvs(eff.document, &kvs, |view, key, value| match key {
        "size" => match value.parse::<f32>() {
            Ok(pt) => Some(view.set_font_size(pt)),
            Err(_) => Some(Outcome::Invalid(format!("'{}' is not a number", value))),
        },
        _ => None,
    });

    finalize_report(report, "font")
}
