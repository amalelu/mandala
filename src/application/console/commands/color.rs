//! `color bg=#009c15 text=accent border=reset` — kv-form color
//! setter dispatched through the capability traits. Each key maps to
//! a trait (`bg` → HasBgColor, `text` → HasTextColor, `border` →
//! HasBorderColor). Fans out over the selection; reports per-pair
//! outcome so a pair that's not applicable to one target doesn't
//! sink the whole command.
//!
//! Axis-only positionals (`color bg`, `color text`, `color border`)
//! and the legacy `color pick` both hand off to the glyph-wheel
//! picker modal — `color bg` picks a color for that axis on the
//! current selection.

use super::Command;
use crate::application::color_picker::{ColorTarget, NodeColorAxis};
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
    summary: "Set bg/text/border color, or pick via the glyph wheel",
    usage: "color bg=<color> text=<color> border=<color>   |   color bg|text|border|pick",
    tags: &["color", "bg", "text", "border", "pick", "wheel"],
    applicable: always,
    complete: complete_color,
    execute: execute_color,
};

fn complete_color(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index } => {
            let mut out = kv_key_completions(state.partial);
            // At token 0 the bare verbs — `pick` plus the axis
            // positionals `bg` / `text` / `border` — also hand off
            // to the glyph-wheel picker. Suggest them alongside the
            // kv-key forms.
            if *index == 0 {
                out.extend(prefix_filter(&["pick", "picker"], state.partial));
            }
            // `color picker` expects `on` / `off` as the next token.
            if *index == 1
                && matches!(state.tokens.first().map(String::as_str), Some("picker"))
            {
                out.extend(prefix_filter(&["on", "off"], state.partial));
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

/// Map a bare positional verb (`pick`, `bg`, `text`, `border`) to a
/// concrete `ColorTarget` based on the current selection. Returns
/// `None` if the combination isn't applicable — e.g. `color text` on
/// a portal (portals have no text axis), or any verb with no
/// selection.
///
/// Node targets carry the axis directly. Edge / portal targets
/// collapse axis into their one color field: `bg`/`border` on an
/// edge both resolve to the edge's line color; `bg` on a portal
/// resolves to the portal's fill.
fn picker_target_for(
    verb: &str,
    selection: &SelectionState,
) -> Option<ColorTarget> {
    let axis = match verb {
        "bg" => Some(NodeColorAxis::Bg),
        "text" => Some(NodeColorAxis::Text),
        "border" => Some(NodeColorAxis::Border),
        "pick" => None, // axis-agnostic legacy flow
        _ => return None,
    };
    match selection {
        SelectionState::Single(id) => match axis {
            Some(a) => Some(ColorTarget::Node { id: id.clone(), axis: a }),
            // `color pick` on a node defaults to bg.
            None => Some(ColorTarget::Node {
                id: id.clone(),
                axis: NodeColorAxis::Bg,
            }),
        },
        SelectionState::Multi(ids) => {
            // The picker is single-target; pick the first node in
            // the multi-selection. Fanout through the picker is
            // a future addition.
            let id = ids.first()?.clone();
            Some(ColorTarget::Node {
                id,
                axis: axis.unwrap_or(NodeColorAxis::Bg),
            })
        }
        SelectionState::Edge(er) => {
            // Edge has one color; `border` maps to it, `text`
            // also currently maps to it (edge label + line share
            // `MindEdge.color`), `bg` isn't meaningful for edges
            // — reject.
            match verb {
                "bg" => None,
                _ => Some(ColorTarget::Edge(er.clone())),
            }
        }
        SelectionState::Portal(pr) => {
            // Portal has one color; `bg` and `pick` map to it,
            // `text` / `border` aren't meaningful — reject.
            match verb {
                "bg" | "pick" => Some(ColorTarget::Portal(pr.clone())),
                _ => None,
            }
        }
        SelectionState::None => None,
    }
}

fn execute_color(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Positional handoffs to the glyph-wheel picker:
    //  - `color pick` — legacy edge/portal one-axis flow
    //  - `color bg | text | border` — pick a color for that axis on
    //    the current selection (node axis for nodes, single-color
    //    target for edges/portals)
    //  - `color picker on` — open the picker as a persistent
    //    standalone palette (no target; commit applies to selection)
    //  - `color picker off` — close any open picker
    if let Some(verb) = args.positional(0) {
        if verb == "picker" {
            match args.positional(1) {
                Some("on") => {
                    eff.open_color_picker_standalone = true;
                    eff.close_console = true;
                    return ExecResult::ok_empty();
                }
                Some("off") => {
                    eff.close_color_picker = true;
                    eff.close_console = true;
                    return ExecResult::ok_empty();
                }
                _ => return ExecResult::err("usage: color picker on | color picker off"),
            }
        }
        if let Some(target) = picker_target_for(verb, &eff.document.selection) {
            eff.open_color_picker = Some(target);
            eff.close_console = true;
            return ExecResult::ok_empty();
        }
        if matches!(verb, "pick" | "bg" | "text" | "border") {
            return ExecResult::err(format!(
                "color {}: nothing to pick for this selection",
                verb
            ));
        }
    }

    let kvs: Vec<(String, String)> = args
        .kvs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if kvs.is_empty() {
        return ExecResult::err(
            "usage: color bg|text|border[=<color>]   |   color pick",
        );
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
