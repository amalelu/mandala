// SPDX-License-Identifier: MPL-2.0

//! `new [path]` — start a fresh, blank mindmap.
//!
//! Like `open`, refuses to discard a dirty document. Without a path
//! the new map is unbound — `save` will require an explicit path
//! before it can write. With a path, the blank map is also written
//! immediately so the binding is real on disk and `Ctrl+S` works
//! from then on.

use std::path::Path;

use baumhard::mindmap::loader;

use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{free, kvs, Bare, Form, Grammar, Slot};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

use super::Command;

pub static GRAMMAR: Grammar = Grammar {
    label: "new",
    subverb_sets: &[],
    key_sets: &[],
    bare: Some(Bare::new("file", &[Form::slots(&[Slot::opt(free("path"))])])),
};

pub const COMMAND: Command = Command {
    name: "new",
    aliases: &[],
    summary: "Start a new blank mindmap, replacing the current one",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["blank", "create", "file"],
    execute: execute_new,
};

fn execute_new(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    if eff.document().dirty {
        return ExecResult::err("unsaved changes; save before starting a new map");
    }
    let path = args.positional(0).map(|p| p.to_string());
    let doc = MindMapDocument::new_blank(path.clone());
    if let Some(ref p) = path {
        if let Err(e) = loader::save_to_file(Path::new(p), &doc.mindmap) {
            return ExecResult::err(e);
        }
    }
    eff.side_effect = Some(super::super::ConsoleSideEffect::ReplaceDocument(Box::new(doc)));
    match path {
        Some(p) => ExecResult::ok_msg(format!("new map at {}", p)),
        None => ExecResult::ok_msg("new map (no file path; use `save <path>` to bind one)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::assert_exec_err_contains;
    use crate::application::console::ConsoleSideEffect;
    use baumhard::util::test_temp::TempDir;

    /// Run `new` with `args` against `doc`, returning the result and
    /// any side effect. The effect has to be lifted out before the
    /// `ConsoleEffects` borrow of `doc` ends — same shape as
    /// `open.rs`'s `run_open`.
    fn run_new(args: &[&str], doc: &mut MindMapDocument) -> (ExecResult, Option<ConsoleSideEffect>) {
        let tokens: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut eff = ConsoleEffects::new(doc);
        let result = execute_new(&Args::new(&tokens), &mut eff);
        let side = eff.side_effect.take();
        (result, side)
    }

    /// Unwrap the replacement document a successful `new` hands the
    /// dispatcher, panicking with the offending variant otherwise.
    fn replacement(side: Option<ConsoleSideEffect>) -> MindMapDocument {
        match side {
            Some(ConsoleSideEffect::ReplaceDocument(doc)) => *doc,
            other => panic!("expected ReplaceDocument, got {:?}", other),
        }
    }

    /// A dirty document is never discarded, and the guard runs
    /// *before* the write — so a rejected `new <path>` also leaves
    /// the filesystem alone.
    ///
    /// Fails when: the `dirty` check is removed (the `Err` assertion
    /// goes), or moved below the `save_to_file` call (the
    /// `!path.exists()` assertion goes). The second half is not
    /// implied by the first: an implementation that writes the blank
    /// map and *then* refuses would satisfy every message assertion
    /// while having already clobbered the file the user named.
    ///
    /// Negative control: the same call on the same document with
    /// `dirty` cleared succeeds, which is what pins the rejection on
    /// the dirty flag rather than on the path, the arguments, or a
    /// verb that never succeeds at all.
    #[test]
    fn test_new_on_a_dirty_document_is_refused_before_anything_is_written() {
        let dir = TempDir::new("console-new-dirty");
        let path = dir.join("fresh.mindmap.json");
        let path_str = path.to_string_lossy().to_string();

        let mut doc = MindMapDocument::new_blank(None);
        doc.dirty = true;
        let (result, side) = run_new(&[&path_str], &mut doc);

        assert_exec_err_contains(result, "unsaved changes");
        assert!(side.is_none(), "a refused new must not replace the document");
        assert!(
            !path.exists(),
            "the dirty guard must run before the write; {} was created",
            path.display()
        );

        // Negative control on the same path: clear the flag the guard
        // reads and nothing else, and the identical call goes through.
        doc.dirty = false;
        let (result, side) = run_new(&[&path_str], &mut doc);
        assert!(
            matches!(result, ExecResult::Ok(_)),
            "control: a clean document must accept new, got {result:?}"
        );
        assert!(side.is_some(), "control: a clean new must replace the document");
        assert!(path.exists(), "control: a clean new must write the file");
    }

    /// `new <path>` binds the replacement to that path and makes the
    /// binding real on disk, so a later `Ctrl+S` has somewhere to go.
    ///
    /// Fails when: `save_to_file` is dropped from the path arm (the
    /// file never appears), when the replacement is built unbound
    /// (`file_path` is `None`), or when the bytes written are not a
    /// map the loader accepts (the reload errors). Reloading through
    /// `load_from_file` rather than eyeballing the JSON is what makes
    /// the third one reachable.
    #[test]
    fn test_new_with_a_path_writes_the_blank_map_and_binds_it() {
        let dir = TempDir::new("console-new-bound");
        let path = dir.join("fresh.mindmap.json");
        let path_str = path.to_string_lossy().to_string();

        let mut doc = MindMapDocument::new_blank(None);
        let (result, side) = run_new(&[&path_str], &mut doc);

        match result {
            ExecResult::Ok(msg) => assert!(
                msg.contains(&path_str),
                "the confirmation must name the path it wrote: {msg:?}"
            ),
            other => panic!("expected Ok, got {other:?}"),
        }
        let replaced = replacement(side);
        assert_eq!(replaced.file_path.as_deref(), Some(path_str.as_str()));

        let reloaded = loader::load_from_file(&path).expect("the written blank map must reload");
        assert_eq!(
            reloaded.name, replaced.mindmap.name,
            "the file on disk must be the map that was handed to the dispatcher"
        );
    }

    /// `new` with no path hands back an unbound document and says so,
    /// which is the state `save` (no args) refuses.
    ///
    /// Fails when: the replacement is bound to some default path, or
    /// when the message drops the instruction that is the user's only
    /// clue about how to bind one. Paired with the bound case above so
    /// "unbound" cannot be satisfied by a verb that never binds
    /// anything.
    #[test]
    fn test_new_without_a_path_leaves_the_replacement_unbound() {
        let mut doc = MindMapDocument::new_blank(Some("/some/where/prior.mindmap.json".to_string()));
        let (result, side) = run_new(&[], &mut doc);

        match result {
            ExecResult::Ok(msg) => assert!(
                msg.contains("save <path>"),
                "an unbound new must tell the user how to bind one: {msg:?}"
            ),
            other => panic!("expected Ok, got {other:?}"),
        }
        let replaced = replacement(side);
        assert!(
            replaced.file_path.is_none(),
            "the prior document's path must not carry over into the new map: {:?}",
            replaced.file_path
        );
    }

    /// A write that fails is reported and the current document
    /// survives — the user is left with the map they had, not with a
    /// blank one bound to a file that does not exist.
    ///
    /// Fails when: the side effect is set before the write result is
    /// checked. The input is a path inside a directory that was never
    /// created, which `write_atomic` cannot stage into.
    #[test]
    fn test_new_reports_a_failed_write_and_keeps_the_current_document() {
        let dir = TempDir::new("console-new-unwritable");
        let path = dir.join("no-such-dir").join("fresh.mindmap.json");
        let path_str = path.to_string_lossy().to_string();

        let mut doc = MindMapDocument::new_blank(None);
        let (result, side) = run_new(&[&path_str], &mut doc);

        assert_exec_err_contains(result, "failed to write");
        assert!(
            side.is_none(),
            "a new whose write failed must not replace the document"
        );
    }
}
