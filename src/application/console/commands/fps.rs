//! `fps on` / `fps off` / `fps debug` — toggle the yellow
//! screen-space FPS readout in the upper-left corner.
//!
//! - `fps on` → `FpsDisplayMode::Snapshot` — one frame's interval,
//!   re-sampled every ~200 frames. Quiet and stable for normal use.
//! - `fps debug` → `FpsDisplayMode::Debug` — rolling average over the
//!   last ~200 frames, updated every frame. Reacts live to load, for
//!   diagnosing perf regressions.
//! - `fps off` → `FpsDisplayMode::Off` — hide the readout.
//!
//! Signals the dispatcher via `ConsoleEffects::set_fps_display`; the
//! actual mode change reaches the renderer in `exec.rs` which calls
//! `Renderer::set_fps_display`.

use super::Command;
use crate::application::common::FpsDisplayMode;
use crate::application::console::completion::{
    prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const COMMAND: Command = Command {
    name: "fps",
    aliases: &[],
    summary: "Toggle the FPS overlay (on | off | debug)",
    usage: "fps on | fps off | fps debug",
    tags: &["fps", "debug", "overlay", "hud", "perf"],
    applicable: always,
    complete: complete_fps,
    execute: execute_fps,
};

fn complete_fps(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index: 0 } => {
            prefix_filter(&["on", "off", "debug"], state.partial)
        }
        _ => Vec::new(),
    }
}

fn execute_fps(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    match args.positional(0) {
        Some("on") => {
            eff.set_fps_display = Some(FpsDisplayMode::Snapshot);
            eff.close_console = true;
            ExecResult::ok_empty()
        }
        Some("off") => {
            eff.set_fps_display = Some(FpsDisplayMode::Off);
            eff.close_console = true;
            ExecResult::ok_empty()
        }
        Some("debug") => {
            eff.set_fps_display = Some(FpsDisplayMode::Debug);
            eff.close_console = true;
            ExecResult::ok_empty()
        }
        _ => ExecResult::err("usage: fps on | fps off | fps debug"),
    }
}
