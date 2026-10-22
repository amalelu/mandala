// SPDX-License-Identifier: MPL-2.0

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
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::{kvs, usage, Grammar, Subverb};
use crate::application::console::{ConsoleEffects, ExecResult};

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("on", "overlay", "show the one-line FPS snapshot"),
    Subverb::bare("off", "overlay", "hide the readout"),
    Subverb::bare("debug", "overlay", "show the rolling per-frame average"),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "fps",
    subverb_sets: &[SUBVERBS],
    key_sets: &[],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "fps",
    aliases: &[],
    summary: "Toggle the FPS overlay (on | off | debug)",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["overlay", "hud", "perf"],
    execute: execute_fps,
};

fn execute_fps(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // The descent matches subverb names case-insensitively,
    // console-wide — see `commands/mod.rs` § Casing.
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    let mode = match descent.stop {
        Stop::Matched(subverb) => match subverb.name {
            "on" => FpsDisplayMode::Snapshot,
            "off" => FpsDisplayMode::Off,
            _ => FpsDisplayMode::Debug,
        },
        Stop::Bare => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => {
            return ExecResult::err(usage::unknown_subverb_message(
                descent.level,
                descent.typed.unwrap_or_default(),
            ))
        }
    };
    eff.side_effect = Some(super::super::ConsoleSideEffect::SetFpsDisplay(mode));
    eff.close_console = true;
    ExecResult::ok_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::assert_exec_err_contains;
    use crate::application::console::ConsoleSideEffect;
    use crate::application::document::MindMapDocument;

    /// Run `fps` with `args`, returning the result alongside the two
    /// dispatcher-visible outputs: the side effect and the
    /// close-the-console flag.
    fn run_fps(args: &[&str]) -> (ExecResult, Option<ConsoleSideEffect>, bool) {
        let tokens: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut doc = MindMapDocument::new_blank(None);
        let mut eff = ConsoleEffects::new(&mut doc);
        let result = execute_fps(&Args::new(&tokens), &mut eff);
        (result, eff.side_effect.take(), eff.close_console)
    }

    /// Each subverb selects its own display mode, in any casing, and
    /// the console closes so the overlay it just enabled is visible.
    ///
    /// Fails when: two subverbs collapse onto one mode (a stub
    /// returning `Snapshot` for everything fails the `off` and
    /// `debug` rows), when the casing fold is dropped (the `ON` /
    /// `Debug` / `OFF` rows fall into the usage arm), or when
    /// `close_console` is left unset (the overlay lands behind the
    /// console it was requested from).
    ///
    /// The table is the control for its own distinctness: three rows
    /// asserting three different modes cannot all pass against a
    /// constant.
    #[test]
    fn test_fps_maps_each_subverb_to_its_display_mode() {
        let cases = [
            ("on", FpsDisplayMode::Snapshot),
            ("off", FpsDisplayMode::Off),
            ("debug", FpsDisplayMode::Debug),
            ("ON", FpsDisplayMode::Snapshot),
            ("OFF", FpsDisplayMode::Off),
            ("Debug", FpsDisplayMode::Debug),
        ];
        for (arg, expected) in cases {
            let (result, side, close) = run_fps(&[arg]);
            assert!(
                matches!(result, ExecResult::Ok(ref s) if s.is_empty()),
                "`fps {arg}` must succeed silently, got {result:?}"
            );
            match side {
                Some(ConsoleSideEffect::SetFpsDisplay(mode)) => assert_eq!(
                    mode, expected,
                    "`fps {arg}` must request {expected:?}, got {mode:?}"
                ),
                other => panic!("`fps {arg}` must request an FPS mode, got {other:?}"),
            }
            assert!(close, "`fps {arg}` must close the console");
        }
    }

    /// Anything that is not one of the three subverbs — including no
    /// subverb at all — reports the usage line, changes no mode, and
    /// leaves the console open so the user can retype.
    ///
    /// Fails when: a missing or unrecognized argument falls through
    /// to a default mode (the `side.is_none()` assertion goes), or
    /// when the error path also closes the console, hiding the
    /// message it just wrote. `"onn"` is in the table because a
    /// prefix match rather than an equality match would accept it.
    #[test]
    fn test_fps_without_a_recognized_subverb_reports_usage() {
        for args in [&[][..], &["onn"][..], &["sideways"][..], &[""][..]] {
            let (result, side, close) = run_fps(args);
            // The listing is derived from the declaration, so the
            // three words it names are the three the verb accepts.
            assert_exec_err_contains(result, "overlay: on | off | debug");
            assert!(side.is_none(), "`fps {args:?}` must not change the display mode");
            assert!(!close, "`fps {args:?}` must leave the console open");
        }
    }
}
