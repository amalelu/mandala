//! `save [path]` — write the current mindmap to disk.
//!
//! No args: writes to the document's bound `file_path`. Errors if no
//! path is bound (e.g. after `new` without a path).
//!
//! With a path: writes there and rebinds the document to the new
//! path, so subsequent saves (including `Ctrl+S`) target the new
//! file. Mirrors the "Save As" gesture in conventional editors —
//! the original file on disk is left untouched.

use std::path::Path;

use baumhard::mindmap::loader;

use super::Command;
use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const COMMAND: Command = Command {
    name: "save",
    aliases: &[],
    summary: "Save the current mindmap to disk",
    usage: "save [path]",
    tags: &["save", "write", "persist", "file"],
    applicable: always,
    complete: complete_save,
    execute: execute_save,
};

fn complete_save(_state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    // No completions — paths are free-form and the console doesn't
    // (yet) shell out to a filesystem walker.
    Vec::new()
}

fn execute_save(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let target_path: String = match args.positional(0) {
        Some(p) => p.to_string(),
        None => match &eff.document.file_path {
            Some(p) => p.clone(),
            None => {
                return ExecResult::err("no file path bound; use `save <path>` to choose one");
            }
        },
    };

    match loader::save_to_file(Path::new(&target_path), &eff.document.mindmap) {
        Ok(()) => {
            eff.document.file_path = Some(target_path.clone());
            eff.document.dirty = false;
            ExecResult::ok_msg(format!("saved to {}", target_path))
        }
        Err(e) => ExecResult::err(e),
    }
}
