//! `portal <create|delete>` plus `portal glyph=star` /
//! `portal color=accent` — portal lifecycle + style.
//!
//! `create` / `delete` are positional verbs. Glyph and color are
//! kv-form; color goes through the `HasBgColor` trait on the portal
//! target (portals have no border vs fill distinction — their single
//! color IS the fill), and glyph stays a portal-only concept.

use super::color::finalize_report;
use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::traits::{
    apply_kvs, ColorValue, HasBgColor, Outcome,
};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const VERBS: &[&str] = &["create", "delete"];
pub const KEYS: &[&str] = &["glyph", "color"];

/// Portal glyph presets: name → unicode.
pub const GLYPHS: &[(&str, &str)] = &[
    ("hexagon", "\u{2B21}"),
    ("diamond", "\u{25C6}"),
    ("star", "\u{2726}"),
    ("circle", "\u{25C9}"),
];

pub const COLOR_PRESETS: &[&str] = &["accent", "edge", "fg", "reset"];

pub const COMMAND: Command = Command {
    name: "portal",
    aliases: &[],
    summary: "Create, delete, or restyle portals",
    usage: "portal create   |   portal delete   |   portal glyph=<name> color=<color>",
    tags: &["portal", "create", "delete", "glyph", "color", "link"],
    applicable: always,
    complete: complete_portal,
    execute: execute_portal,
};

fn complete_portal(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index: 0 } => {
            let mut out = prefix_filter(VERBS, state.partial);
            for k in KEYS {
                if k.starts_with(state.partial) {
                    out.push(Completion {
                        text: format!("{}=", k),
                        display: format!("{}=", k),
                        hint: None,
                    });
                }
            }
            out
        }
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
            let names: Vec<&str> = GLYPHS.iter().map(|(n, _)| *n).collect();
            prefix_filter(&names, state.partial)
        }
        CompletionContext::KvValue { key } if key == "color" => {
            prefix_filter(COLOR_PRESETS, state.partial)
        }
        _ => Vec::new(),
    }
}

fn execute_portal(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Positional verbs first.
    match args.positional(0) {
        Some("create") => {
            let (a, b) = match &eff.document.selection {
                SelectionState::Multi(ids) if ids.len() == 2 => (ids[0].clone(), ids[1].clone()),
                _ => {
                    return ExecResult::err(
                        "portal create requires exactly two nodes selected",
                    )
                }
            };
            return match eff.document.apply_create_portal(&a, &b) {
                Ok(pref) => {
                    eff.document.selection = SelectionState::Portal(pref);
                    ExecResult::ok_msg("portal created")
                }
                Err(e) => ExecResult::err(format!("portal create failed: {}", e)),
            };
        }
        Some("delete") => {
            let pref = match &eff.document.selection {
                SelectionState::Portal(p) => p.clone(),
                _ => return ExecResult::err("no portal selected"),
            };
            return if eff.document.apply_delete_portal(&pref).is_some() {
                eff.document.selection = SelectionState::None;
                ExecResult::ok_msg("portal deleted")
            } else {
                ExecResult::err("portal not found")
            };
        }
        Some(other) => {
            return ExecResult::err(format!(
                "unknown portal verb '{}'; use create / delete or glyph=... color=...",
                other
            ))
        }
        None => {}
    }

    let kvs: Vec<(String, String)> = args
        .kvs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if kvs.is_empty() {
        return ExecResult::err(
            "usage: portal create | delete | glyph=<name> | color=<color>",
        );
    }

    // Glyph is portal-only; color routes through the trait layer
    // so `color=accent` is consistent with the generic `color`
    // command.
    let glyph_kv = kvs.iter().find(|(k, _)| k == "glyph").cloned();
    let color_kvs: Vec<(String, String)> = kvs
        .iter()
        .filter(|(k, _)| k == "color")
        .cloned()
        .collect();
    let unknown_kvs: Vec<(String, String)> = kvs
        .iter()
        .filter(|(k, _)| k != "glyph" && k != "color")
        .cloned()
        .collect();

    let mut messages = Vec::new();
    let mut any_applied = false;

    if let Some((_, value)) = glyph_kv {
        let pref = match &eff.document.selection {
            SelectionState::Portal(p) => p.clone(),
            _ => return ExecResult::err("no portal selected"),
        };
        match GLYPHS.iter().find(|(n, _)| *n == value) {
            Some((_, g)) => {
                let changed = eff.document.set_portal_glyph(&pref, g);
                if changed {
                    any_applied = true;
                } else {
                    messages.push(format!("glyph already {}", value));
                }
            }
            None => {
                messages.push(format!(
                    "glyph '{}' must be one of hexagon|diamond|star|circle",
                    value
                ));
            }
        }
    }

    if !color_kvs.is_empty() {
        let report = apply_kvs(eff.document, &color_kvs, |view, key, value| match key {
            "color" => match ColorValue::parse(value) {
                Ok(c) => Some(view.set_bg_color(c)),
                Err(msg) => Some(Outcome::Invalid(msg)),
            },
            _ => None,
        });
        any_applied |= report.any_applied;
        // If the trait layer considered every target invalid and the
        // earlier glyph didn't apply either, surface an Err directly.
        if report.all_failed && !any_applied {
            return finalize_report(report, "portal");
        }
        messages.extend(report.messages);
    }

    for (k, _) in unknown_kvs {
        messages.push(format!("unknown key '{}'", k));
    }

    if !messages.is_empty() {
        if !any_applied {
            return ExecResult::err(messages.join("; "));
        }
        return ExecResult::Lines(messages);
    }
    ExecResult::ok_msg("portal applied")
}
