// SPDX-License-Identifier: MPL-2.0

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
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{free, kvs, Bare, Form, Grammar, Slot};
use crate::application::console::{ConsoleEffects, ExecResult};

/// The path slot offers no rows: paths are free-form and the console
/// does not (yet) shell out to a filesystem walker. Declaring it is
/// still what puts `save [<path>]` in `help save`.
pub static GRAMMAR: Grammar = Grammar {
    label: "save",
    subverb_sets: &[],
    key_sets: &[],
    bare: Some(Bare::new("file", &[Form::slots(&[Slot::opt(free("path"))])])),
};

pub const COMMAND: Command = Command {
    name: "save",
    aliases: &[],
    summary: "Save the current mindmap to disk",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["write", "persist", "file"],
    execute: execute_save,
};

fn execute_save(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    let target_path: String = match args.positional(0) {
        Some(p) => p.to_string(),
        None => match &eff.document().file_path {
            Some(p) => p.clone(),
            None => {
                return ExecResult::err("no file path bound; use `save <path>` to choose one");
            }
        },
    };

    match loader::save_to_file(Path::new(&target_path), &eff.document().mindmap) {
        Ok(()) => {
            eff.document_mut().file_path = Some(target_path.clone());
            eff.document_mut().dirty = false;
            ExecResult::ok_msg(format!("saved to {}", target_path))
        }
        Err(e) => ExecResult::err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::assert_exec_err_contains;
    use crate::application::document::MindMapDocument;
    use baumhard::util::test_temp::TempDir;

    /// Run `save` with `args` against `doc`. `save` produces no side
    /// effect, so only the result comes back — the interesting
    /// outcomes are on `doc` and on disk.
    fn run_save(args: &[&str], doc: &mut MindMapDocument) -> ExecResult {
        let tokens: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut eff = ConsoleEffects::new(doc);
        execute_save(&Args::new(&tokens), &mut eff)
    }

    /// A document with no bound path is told so rather than guessing
    /// a filename, and stays dirty — the work is still unwritten.
    ///
    /// Fails when: the no-path arm invents a default (nothing is
    /// reported and the flag clears), or when `dirty` is cleared
    /// before the write is known to have happened.
    ///
    /// Negative control: binding a real path to the *same* document
    /// and repeating the *same* argument-free call succeeds, which
    /// pins the rejection on the missing binding rather than on the
    /// document, the map, or a verb that never succeeds.
    #[test]
    fn test_save_without_a_bound_path_reports_it_and_stays_dirty() {
        let mut doc = MindMapDocument::new_blank(None);
        doc.dirty = true;

        assert_exec_err_contains(run_save(&[], &mut doc), "no file path bound");
        assert!(doc.dirty, "a save that never wrote must leave the document dirty");

        let dir = TempDir::new("console-save-unbound");
        let path = dir.join("bound.mindmap.json");
        doc.file_path = Some(path.to_string_lossy().to_string());
        assert!(
            matches!(run_save(&[], &mut doc), ExecResult::Ok(_)),
            "control: a bound document must accept an argument-free save"
        );
        assert!(path.exists(), "control: the bound save must write the file");
    }

    /// `save <path>` is Save As: it writes there, rebinds the
    /// document, clears the dirty flag, and leaves the file the
    /// document *was* bound to exactly as it found it.
    ///
    /// Fails when: the argument is ignored and the bound path is
    /// written instead (the original's bytes change and the new file
    /// never appears), when the rebind is dropped (`file_path` still
    /// names the original), or when `dirty` survives a successful
    /// write. The pre-assertions on `dirty` and on the original's
    /// bytes are what keep the two post-assertions from being
    /// satisfied by a document that was already clean and an original
    /// that was never written.
    #[test]
    fn test_save_to_a_new_path_rebinds_and_leaves_the_original_untouched() {
        let dir = TempDir::new("console-save-as");
        let original = dir.join("original.mindmap.json");
        let target = dir.join("target.mindmap.json");

        let mut doc = MindMapDocument::new_blank(Some(original.to_string_lossy().to_string()));
        assert!(matches!(run_save(&[], &mut doc), ExecResult::Ok(_)));
        let original_bytes = std::fs::read(&original).expect("the original must exist to be compared");

        // Diverge the in-memory map from what is on disk, so "the
        // original is untouched" has an observable value to hold.
        doc.mindmap.name = "renamed after the first save".to_string();
        doc.dirty = true;

        let target_str = target.to_string_lossy().to_string();
        assert!(matches!(run_save(&[&target_str], &mut doc), ExecResult::Ok(_)));

        assert_eq!(
            doc.file_path.as_deref(),
            Some(target_str.as_str()),
            "save <path> must rebind the document to the path it wrote"
        );
        assert!(!doc.dirty, "a successful save must clear the dirty flag");
        assert_eq!(
            std::fs::read(&original).expect("the original must survive a Save As"),
            original_bytes,
            "save <path> must not write through to the previously bound file"
        );
        let written = loader::load_from_file(&target).expect("the Save As target must reload");
        assert_eq!(written.name, "renamed after the first save");
    }

    /// A failed write is reported, and the document keeps both its
    /// binding and its dirty flag — the user's next `Ctrl+S` still
    /// targets the file they chose, and still has work to write.
    ///
    /// Fails when: `file_path` / `dirty` are written before the
    /// `save_to_file` result is matched. The input is a bound path
    /// inside a directory that was never created.
    #[test]
    fn test_save_that_cannot_write_keeps_the_binding_and_the_dirty_flag() {
        let dir = TempDir::new("console-save-unwritable");
        let bound = dir.join("no-such-dir").join("map.mindmap.json");
        let bound_str = bound.to_string_lossy().to_string();

        let mut doc = MindMapDocument::new_blank(Some(bound_str.clone()));
        doc.dirty = true;

        assert_exec_err_contains(run_save(&[], &mut doc), "failed to write");
        assert_eq!(doc.file_path.as_deref(), Some(bound_str.as_str()));
        assert!(doc.dirty, "a failed save must leave the document dirty");
    }
}
