//! `font size=14 [min=8] [max=128]` — atomic font + clamp setter
//! dispatched against the current selection.
//!
//! Parses all three optional kvs up front, then applies them in a
//! single atomic document call so that order-sensitive cases like
//! `font size=14 max=10` land as `size=10, max=10` (min/max write
//! first, then size clamps against the new bounds) instead of the
//! wrong `size=14, max=10`.
//!
//! Routing against the active selection:
//! - `Node`: `size` sets the node font size; `min`/`max` are
//!   NotApplicable (nodes have no screen-space clamps).
//! - `Edge`: writes `glyph_connection.{font_size_pt, min_font_size_pt,
//!   max_font_size_pt}`.
//! - `EdgeLabel`: writes `label_config.{font_size_pt, min_font_size_pt,
//!   max_font_size_pt}` so the label can be sized independently of
//!   the edge body.
//! - `PortalLabel`: writes the owning edge's `glyph_connection` —
//!   the icon inherits edge-body clamps and splitting icon clamps
//!   off from the edge is outside the user-level spec.
//! - `PortalText`: writes `PortalEndpointState.{text_font_size_pt,
//!   text_min_font_size_pt, text_max_font_size_pt}` — sibling of
//!   `EdgeLabel` for portal-mode edges.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const KEYS: &[&str] = &["size", "min", "max"];
/// Preset sizes surfaced in completion. Users can type any positive
/// float; the preset list just makes the popup useful.
pub const SIZE_PRESETS: &[&str] = &["10", "12", "14", "16", "18", "24", "32"];

pub const COMMAND: Command = Command {
    name: "font",
    aliases: &[],
    summary: "Set font size + optional min/max clamps on the selection",
    usage: "font size=<pt> [min=<pt>] [max=<pt>]",
    tags: &["font", "size", "min", "max", "clamp", "pt", "smaller", "larger"],
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
                hint: Some(match *k {
                    "size" => "target on-screen size in points",
                    "min" => "lower screen-space clamp in points",
                    "max" => "upper screen-space clamp in points",
                    _ => "points",
                }.into()),
            })
            .collect(),
        CompletionContext::KvValue { key } if KEYS.contains(&key.as_str()) => {
            prefix_filter(SIZE_PRESETS, state.partial)
        }
        _ => Vec::new(),
    }
}

/// Parse a kv value as a positive finite f32. Returns an
/// `ExecResult::Err` for non-numbers, NaN, infinity, or ≤ 0.
fn parse_pt(key: &str, value: &str) -> Result<f32, ExecResult> {
    match value.parse::<f32>() {
        Ok(pt) if pt.is_finite() && pt > 0.0 => Ok(pt),
        Ok(pt) => Err(ExecResult::err(format!(
            "{}='{}' must be positive and finite; got {}",
            key, value, pt
        ))),
        Err(_) => Err(ExecResult::err(format!(
            "{}='{}' is not a number",
            key, value
        ))),
    }
}

fn execute_font(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Parse every recognised kv up front so the atomic application
    // sees a complete Option triple. Unknown keys report an error
    // immediately — better than silently ignoring a typo.
    let mut size: Option<f32> = None;
    let mut min: Option<f32> = None;
    let mut max: Option<f32> = None;
    let mut saw_any = false;
    for (k, v) in args.kvs() {
        saw_any = true;
        match k {
            "size" => match parse_pt("size", v) {
                Ok(pt) => size = Some(pt),
                Err(e) => return e,
            },
            "min" => match parse_pt("min", v) {
                Ok(pt) => min = Some(pt),
                Err(e) => return e,
            },
            "max" => match parse_pt("max", v) {
                Ok(pt) => max = Some(pt),
                Err(e) => return e,
            },
            other => return ExecResult::err(format!("unknown key '{}'", other)),
        }
    }
    if !saw_any {
        return ExecResult::err("usage: font size=<pt> [min=<pt>] [max=<pt>]");
    }
    if size.is_none() && min.is_none() && max.is_none() {
        return ExecResult::err("font: nothing to set");
    }

    // Selection-variant dispatch. A Multi node selection fans
    // out over each node (size only; min/max are NotApplicable
    // for nodes). The edge-adjacent variants each write to
    // their own channel.
    let doc = &mut eff.document;
    match doc.selection.clone() {
        SelectionState::Single(id) => {
            node_font_outcome(doc, &id, size, min, max, 1)
        }
        SelectionState::Multi(ids) => {
            // Fanout: apply size to each node; collect a single
            // "any changed?" result. `min` / `max` are
            // NotApplicable for nodes and surface as a single
            // message rather than one per node.
            let mut changed = 0usize;
            for id in &ids {
                if let Some(pt) = size {
                    if doc.set_node_font_size(id, pt) {
                        changed += 1;
                    }
                }
            }
            let applicable_msg = (min.is_some() || max.is_some())
                .then(|| "min/max: nodes have no screen-space clamps".to_string());
            if changed == 0 && applicable_msg.is_none() {
                return ExecResult::ok_msg("font: no change");
            }
            let mut lines = Vec::new();
            if changed > 0 {
                lines.push(format!("font: applied to {} node(s)", changed));
            }
            if let Some(m) = applicable_msg {
                lines.push(m);
            }
            ExecResult::Lines(lines)
        }
        SelectionState::Edge(er) => {
            let changed = doc.set_edge_font(&er, size, min, max);
            finalize("edge", changed)
        }
        SelectionState::EdgeLabel(s) => {
            let changed = doc.set_edge_label_font(&s.edge_ref, size, min, max);
            finalize("edge label", changed)
        }
        SelectionState::PortalLabel(s) => {
            // Portal icon routes to the owning edge's
            // `glyph_connection` channel (same sink as `Edge`).
            let changed = doc.set_edge_font(&s.edge_ref(), size, min, max);
            finalize("portal label", changed)
        }
        SelectionState::PortalText(s) => {
            let changed = doc.set_portal_text_font(
                &s.edge_ref(),
                &s.endpoint_node_id,
                size,
                min,
                max,
            );
            finalize("portal text", changed)
        }
        SelectionState::None => ExecResult::err("font: no selection"),
    }
}

fn node_font_outcome(
    doc: &mut crate::application::document::MindMapDocument,
    id: &str,
    size: Option<f32>,
    min: Option<f32>,
    max: Option<f32>,
    _: i32,
) -> ExecResult {
    let mut messages = Vec::new();
    let mut any_applied = false;
    if let Some(pt) = size {
        if doc.set_node_font_size(id, pt) {
            any_applied = true;
        }
    }
    if min.is_some() || max.is_some() {
        messages.push("min/max: nodes have no screen-space clamps".to_string());
    }
    if !messages.is_empty() {
        if !any_applied {
            return ExecResult::err(messages.join("; "));
        }
        return ExecResult::Lines(messages);
    }
    if any_applied {
        ExecResult::ok_msg("font applied")
    } else {
        ExecResult::ok_msg("font: no change")
    }
}

fn finalize(kind: &str, changed: bool) -> ExecResult {
    if changed {
        ExecResult::ok_msg(format!("font applied to {}", kind))
    } else {
        ExecResult::ok_msg(format!("font: no change on {}", kind))
    }
}
