//! `new [path]` — start a fresh, blank mindmap.
//!
//! Like `open`, refuses to discard a dirty document. Without a path
//! the new map is unbound — `save` will require an explicit path
//! before it can write. With a path, the blank map is also written
//! immediately so the binding is real on disk and `Ctrl+S` works
//! from then on.

use std::path::Path;

use baumhard::mindmap::loader;

use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

use super::Command;

pub const COMMAND: Command = Command {
    name: "new",
    aliases: &[],
    summary: "Start a new blank mindmap, replacing the current one",
    usage: "new [path]",
    tags: &["new", "blank", "create", "file"],
    applicable: always,
    complete: complete_new,
    execute: execute_new,
};

fn complete_new(_state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    Vec::new()
}

fn execute_new(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    if eff.document.dirty {
        return ExecResult::err("unsaved changes; save before starting a new map");
    }
    let path = args.positional(0).map(|p| p.to_string());
    let doc = MindMapDocument::new_blank(path.clone());
    if let Some(ref p) = path {
        if let Err(e) = loader::save_to_file(Path::new(p), &doc.mindmap) {
            return ExecResult::err(e);
        }
    }
    eff.replace_document = Some(doc);
    match path {
        Some(p) => ExecResult::ok_msg(format!("new map at {}", p)),
        None => ExecResult::ok_msg("new map (no file path; use `save <path>` to bind one)"),
    }
}
