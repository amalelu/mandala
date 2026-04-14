//! `open <path>` — replace the current mindmap with one loaded from disk.
//!
//! Refuses to load over a dirty document so unsaved work is never
//! silently discarded — the user has to `save` (or `save <path>`)
//! first. The dispatcher consumes `effects.replace_document`,
//! swaps the app's document, drops the cached `mindmap_tree`, and
//! clears any open modal-editor state.

use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

use super::Command;

pub const COMMAND: Command = Command {
    name: "open",
    aliases: &[],
    summary: "Open a mindmap file, replacing the current one",
    usage: "open <path>",
    tags: &["open", "load", "file"],
    applicable: always,
    complete: complete_open,
    execute: execute_open,
};

fn complete_open(_state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    Vec::new()
}

fn execute_open(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let path = match args.positional(0) {
        Some(p) => p.to_string(),
        None => return ExecResult::err("usage: open <path>"),
    };
    if eff.document.dirty {
        return ExecResult::err("unsaved changes; save before opening another map");
    }
    match MindMapDocument::load(&path) {
        Ok(doc) => {
            eff.replace_document = Some(doc);
            ExecResult::ok_msg(format!("opened {}", path))
        }
        Err(e) => ExecResult::err(e),
    }
}
