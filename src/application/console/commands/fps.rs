//! `fps on` / `fps off` — toggle the yellow screen-space FPS readout
//! in the upper-left corner. Signals the dispatcher via
//! `ConsoleEffects::set_fps_display`; the actual toggle reaches the
//! renderer in `exec.rs` which calls `Renderer::set_fps_display`.

use super::Command;
use crate::application::console::completion::{
    prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const COMMAND: Command = Command {
    name: "fps",
    aliases: &[],
    summary: "Toggle the FPS overlay",
    usage: "fps on | fps off",
    tags: &["fps", "debug", "overlay", "hud"],
    applicable: always,
    complete: complete_fps,
    execute: execute_fps,
};

fn complete_fps(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index: 0 } => {
            prefix_filter(&["on", "off"], state.partial)
        }
        _ => Vec::new(),
    }
}

fn execute_fps(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    match args.positional(0) {
        Some("on") => {
            eff.set_fps_display = Some(true);
            eff.close_console = true;
            ExecResult::ok_empty()
        }
        Some("off") => {
            eff.set_fps_display = Some(false);
            eff.close_console = true;
            ExecResult::ok_empty()
        }
        _ => ExecResult::err("usage: fps on | fps off"),
    }
}
