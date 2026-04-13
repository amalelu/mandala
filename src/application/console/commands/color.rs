//! `color bg=#009c15 text=accent border=reset` — kv-form color
//! setter dispatched through the capability traits. Each key maps to
//! a trait (`bg` → HasBgColor, `text` → HasTextColor, `border` →
//! HasBorderColor). Fans out over the selection; reports per-pair
//! outcome so a pair that's not applicable to one target doesn't
//! sink the whole command.
//!
//! Also supports `color pick` as a positional — hands off to the
//! glyph-wheel picker modal for edge / portal targets.

use super::Command;
use crate::application::color_picker::ColorTarget;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::traits::{
    apply_kvs, ColorValue, HasBgColor, HasBorderColor, HasTextColor, Outcome,
};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const KEYS: &[&str] = &["bg", "text", "border"];
pub const VALUE_PRESETS: &[&str] = &["accent", "edge", "fg", "reset"];

pub const COMMAND: Command = Command {
    name: "color",
    aliases: &[],
    summary: "Set bg/text/border color, or `pick` via the glyph wheel",
    usage: "color bg=<color> text=<color> border=<color>   |   color pick",
    tags: &["color", "bg", "text", "border", "pick", "wheel"],
    applicable: always,
    complete: complete_color,
    execute: execute_color,
};

fn complete_color(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index } => {
            let mut out = kv_key_completions(state.partial);
            // `pick` is positional-only at slot 0 (the glyph-wheel
            // handoff). Don't show it mid-command — it's a verb, not
            // an arg.
            if *index == 0 {
                out.extend(prefix_filter(&["pick"], state.partial));
            }
            out
        }
        CompletionContext::KvValue { key } if KEYS.iter().any(|k| k == key) => {
            prefix_filter(VALUE_PRESETS, state.partial)
        }
        _ => Vec::new(),
    }
}

fn kv_key_completions(partial: &str) -> Vec<Completion> {
    KEYS.iter()
        .filter(|k| k.starts_with(partial))
        .map(|k| Completion {
            text: format!("{}=", k),
            display: format!("{}=", k),
            hint: Some(kv_hint(k).to_string()),
        })
        .collect()
}

fn kv_hint(key: &str) -> &'static str {
    match key {
        "bg" => "fill / background color",
        "text" => "text / label color",
        "border" => "frame / line color",
        _ => "",
    }
}

fn execute_color(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // `color pick` positional handoff — lands in the glyph-wheel
    // modal, not in the trait dispatcher. No target fanout; just one
    // edge or one portal.
    if matches!(args.positional(0), Some("pick")) {
        return match &eff.document.selection {
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
            _ => ExecResult::err("color pick needs an edge or portal selected"),
        };
    }

    let kvs: Vec<(String, String)> = args
        .kvs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if kvs.is_empty() {
        return ExecResult::err("usage: color bg=<color> text=<color> border=<color>");
    }

    let report = apply_kvs(eff.document, &kvs, |view, key, value| {
        let color = match ColorValue::parse(value) {
            Ok(c) => c,
            Err(msg) => return Some(Outcome::Invalid(msg)),
        };
        match key {
            "bg" => Some(view.set_bg_color(color)),
            "text" => Some(view.set_text_color(color)),
            "border" => Some(view.set_border_color(color)),
            _ => None,
        }
    });

    finalize_report(report, "color")
}

/// Common report-to-ExecResult conversion used by every
/// trait-dispatched command.
pub(super) fn finalize_report(
    report: crate::application::console::traits::DispatchReport,
    verb: &str,
) -> ExecResult {
    if report.all_failed {
        return ExecResult::err(report.messages.join("; "));
    }
    if !report.messages.is_empty() {
        return ExecResult::Lines(report.messages);
    }
    if report.any_applied {
        ExecResult::ok_msg(format!("{} applied", verb))
    } else {
        ExecResult::ok_empty()
    }
}
