//! `portal <create|delete|glyph|color>` — portal creation, deletion,
//! and style. Applicability is broad (always `true`) so `portal
//! create` is discoverable even without a selection; each subcommand
//! validates its own prerequisites.

use super::Command;
use crate::application::console::completion::{enum_completion, Completion, CompletionState};
use crate::application::console::constants::{
    PORTAL_DEFAULT_COLOR, VAR_ACCENT, VAR_EDGE, VAR_FG,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

pub const SUBCOMMANDS: &[&str] = &["create", "delete", "glyph", "color"];

/// Portal glyph presets: name → unicode.
pub const GLYPHS: &[(&str, &str)] = &[
    ("hexagon", "\u{2B21}"),
    ("diamond", "\u{25C6}"),
    ("star", "\u{2726}"),
    ("circle", "\u{25C9}"),
];

/// Portal color presets. `reset` maps to the default hex.
pub const COLORS: &[(&str, &str)] = &[
    ("accent", VAR_ACCENT),
    ("edge", VAR_EDGE),
    ("fg", VAR_FG),
    ("reset", PORTAL_DEFAULT_COLOR),
];

pub const COMMAND: Command = Command {
    name: "portal",
    aliases: &[],
    summary: "Create, delete, or restyle portals",
    usage: "portal <create | delete | glyph <name> | color <name>>",
    tags: &["portal", "create", "delete", "glyph", "color", "link"],
    applicable: always,
    complete: complete_portal,
    execute: execute_portal,
};

fn complete_portal(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match state.cursor_token {
        1 => enum_completion(SUBCOMMANDS, state.partial),
        2 => {
            let sub = state.tokens.get(1).map(|s| s.as_str()).unwrap_or("");
            if sub.eq_ignore_ascii_case("glyph") {
                let names: Vec<&str> = GLYPHS.iter().map(|(n, _)| *n).collect();
                enum_completion(&names, state.partial)
            } else if sub.eq_ignore_ascii_case("color") {
                let names: Vec<&str> = COLORS.iter().map(|(n, _)| *n).collect();
                enum_completion(&names, state.partial)
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn execute_portal(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let sub = match args.positional(0) {
        Some(s) => s.to_ascii_lowercase(),
        None => return ExecResult::err("usage: portal <create|delete|glyph|color>"),
    };
    match sub.as_str() {
        "create" => {
            let (a, b) = match &eff.document.selection {
                SelectionState::Multi(ids) if ids.len() == 2 => (ids[0].clone(), ids[1].clone()),
                _ => {
                    return ExecResult::err(
                        "portal create requires exactly two nodes selected",
                    )
                }
            };
            match eff.document.apply_create_portal(&a, &b) {
                Ok(pref) => {
                    eff.document.selection = SelectionState::Portal(pref);
                    ExecResult::ok_msg("portal created")
                }
                Err(e) => ExecResult::err(format!("portal create failed: {}", e)),
            }
        }
        "delete" => {
            let pref = match &eff.document.selection {
                SelectionState::Portal(p) => p.clone(),
                _ => return ExecResult::err("no portal selected"),
            };
            if eff.document.apply_delete_portal(&pref).is_some() {
                eff.document.selection = SelectionState::None;
                ExecResult::ok_msg("portal deleted")
            } else {
                ExecResult::err("portal not found")
            }
        }
        "glyph" => {
            let pref = match &eff.document.selection {
                SelectionState::Portal(p) => p.clone(),
                _ => return ExecResult::err("no portal selected"),
            };
            let name = match args.positional(1) {
                Some(n) => n.to_ascii_lowercase(),
                None => return ExecResult::err("usage: portal glyph <name>"),
            };
            let glyph = match GLYPHS.iter().find(|(n, _)| *n == name) {
                Some((_, g)) => *g,
                None => {
                    return ExecResult::err(format!(
                        "portal glyph '{}' must be one of hexagon|diamond|star|circle",
                        name
                    ))
                }
            };
            let changed = eff.document.set_portal_glyph(&pref, glyph);
            if changed {
                ExecResult::ok_msg(format!("portal glyph set to {}", name))
            } else {
                ExecResult::ok_msg(format!("portal glyph already {}", name))
            }
        }
        "color" => {
            let pref = match &eff.document.selection {
                SelectionState::Portal(p) => p.clone(),
                _ => return ExecResult::err("no portal selected"),
            };
            let name = match args.positional(1) {
                Some(n) => n.to_ascii_lowercase(),
                None => return ExecResult::err("usage: portal color <name>"),
            };
            let color = match COLORS.iter().find(|(n, _)| *n == name) {
                Some((_, c)) => *c,
                None => {
                    return ExecResult::err(format!(
                        "portal color '{}' must be one of accent|edge|fg|reset",
                        name
                    ))
                }
            };
            let changed = eff.document.set_portal_color(&pref, color);
            if changed {
                ExecResult::ok_msg(format!("portal color set to {}", name))
            } else {
                ExecResult::ok_msg(format!("portal color already {}", name))
            }
        }
        other => ExecResult::err(format!("unknown portal subcommand '{}'", other)),
    }
}
