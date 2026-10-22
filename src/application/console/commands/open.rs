// SPDX-License-Identifier: MPL-2.0

//! `open <path>` — replace the current mindmap with one loaded from disk.
//!
//! Refuses to load over a dirty document so unsaved work is never
//! silently discarded — the user has to `save` (or `save <path>`)
//! first. The dispatcher consumes `effects.replace_document`,
//! swaps the app's document, drops the cached `mindmap_tree`, and
//! clears any open modal-editor state.

use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{free, kvs, usage, Bare, Form, Grammar, Slot};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

use super::Command;

pub static GRAMMAR: Grammar = Grammar {
    label: "open",
    subverb_sets: &[],
    key_sets: &[],
    bare: Some(Bare::new("file", &[Form::slots(&[Slot::req(free("path"))])])),
};

pub const COMMAND: Command = Command {
    name: "open",
    aliases: &[],
    summary: "Open a mindmap file, replacing the current one",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["load", "file"],
    execute: execute_open,
};

fn execute_open(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    let path = match args.positional(0) {
        Some(p) => p.to_string(),
        None => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
    };
    if eff.document().dirty {
        return ExecResult::err("unsaved changes; save before opening another map");
    }
    match MindMapDocument::load(&path) {
        Ok(doc) => {
            eff.side_effect = Some(super::super::ConsoleSideEffect::ReplaceDocument(Box::new(doc)));
            ExecResult::ok_msg(format!("opened {}", path))
        }
        // The loader's message verbatim, prefixed with the path it
        // was asked for. `MindMapDocument::load` reports nothing on
        // its own — every caller is a different surface, and this
        // one is the console overlay.
        Err(e) => ExecResult::err(format!("open {}: {}", path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::Args;
    use crate::application::console::tests::fixtures::assert_exec_err_contains;
    use crate::application::console::ConsoleSideEffect;
    use crate::application::document::tests_common::{load_test_doc, test_map_path};
    use baumhard::util::test_temp::TempDir;

    /// Run `open <path>` against `doc`, returning the result and any
    /// side effect. The effect has to be lifted out before the
    /// `ConsoleEffects` borrow of `doc` ends.
    fn run_open(path: &str, doc: &mut MindMapDocument) -> (ExecResult, Option<ConsoleSideEffect>) {
        let tokens = vec![path.to_string()];
        let mut eff = ConsoleEffects::new(doc);
        let result = execute_open(&Args::new(&tokens), &mut eff);
        let side = eff.side_effect.take();
        (result, side)
    }

    /// A rejected `open` puts the loader's own message in the console
    /// overlay and leaves the current document alone.
    ///
    /// This surface was already correct before the startup placard
    /// landed — #107 named it as the thing startup should have been
    /// doing. It was also unpinned, and reporting has since moved out
    /// of `MindMapDocument::load` and into the callers, so it is
    /// pinned here: a caller that forgot to report would now be
    /// silent rather than double-reporting.
    #[test]
    fn test_open_of_a_rejected_map_reports_the_loader_message() {
        let dir = TempDir::new("console-open-rejected");
        let path = dir.join("broken.mindmap.json");
        std::fs::write(&path, "{ this is not json").expect("seed the broken map");
        let path_str = path.to_string_lossy().to_string();

        let mut doc = load_test_doc();
        let name_before = doc.mindmap.name.clone();
        let (result, side) = run_open(&path_str, &mut doc);

        // The path the user typed, and the loader's own words.
        assert_exec_err_contains(result, "Failed to parse mindmap JSON");
        let (result, _) = run_open(&path_str, &mut doc);
        assert_exec_err_contains(result, &path_str);

        assert!(side.is_none(), "a rejected open must not replace the document");
        assert_eq!(doc.mindmap.name, name_before);
    }

    /// The dirty guard is the only thing between a typo'd `open` and
    /// losing unsaved work, and it is checked before the loader runs:
    /// a dirty document is refused even when the path names a map
    /// that would have loaded cleanly.
    ///
    /// Fails when: the `dirty` check is removed, or moved below the
    /// `MindMapDocument::load` call — the second shape still refuses,
    /// but only for paths that happen to parse, so a broken map would
    /// report the loader's message instead of the unsaved-work one.
    /// Using the testament map as the argument is what makes that
    /// distinction reachable.
    ///
    /// Negative control: clearing `dirty` on the same document and
    /// repeating the same call succeeds, which pins the rejection on
    /// the flag rather than on the path or the document.
    #[test]
    fn test_open_on_a_dirty_document_is_refused_before_the_load() {
        let path = test_map_path().to_string_lossy().to_string();
        let mut doc = load_test_doc();
        doc.dirty = true;
        let name_before = doc.mindmap.name.clone();

        let (result, side) = run_open(&path, &mut doc);
        assert_exec_err_contains(result, "unsaved changes");
        assert!(side.is_none(), "a refused open must not replace the document");
        assert_eq!(doc.mindmap.name, name_before);

        doc.dirty = false;
        let (result, side) = run_open(&path, &mut doc);
        assert!(
            matches!(result, ExecResult::Ok(_)),
            "control: a clean document must accept the same open, got {result:?}"
        );
        assert!(side.is_some(), "control: a clean open must replace the document");
    }

    /// The happy path still swaps the document in. Pairs with the
    /// rejection test so "reports the error" cannot be satisfied by
    /// a verb that never succeeds.
    #[test]
    fn test_open_of_a_valid_map_replaces_the_document() {
        let path = test_map_path().to_string_lossy().to_string();
        let mut doc = MindMapDocument::new_blank(None);
        let (result, side) = run_open(&path, &mut doc);

        assert!(
            matches!(result, ExecResult::Ok(_)),
            "the testament map must open: {result:?}"
        );
        assert!(
            matches!(side, Some(ConsoleSideEffect::ReplaceDocument(_))),
            "a successful open must hand the dispatcher a replacement document"
        );
    }
}
