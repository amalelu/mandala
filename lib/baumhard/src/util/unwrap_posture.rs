// SPDX-License-Identifier: MPL-2.0

//! CODE_CONVENTIONS §9's last sentence, held against the tree
//! instead of written down and trusted: **no shipped line in this
//! workspace calls `unwrap()`.**
//!
//! §9 spends four paragraphs on where a panic is honest — the
//! startup phases that `expect` with a reason, the map load that
//! degrades instead — and closes with "Bare `unwrap()` outside tests
//! is a bug." The difference between an `expect` and an `unwrap` is
//! the sentence a reader gets when it fires, so the rule is about
//! what a failure *says*, not about which of two functions was
//! called. Nothing enforced it: issue #42 counted twenty-six
//! offenders by hand; six weeks later nine of the twenty-six had
//! been deleted or moved by unrelated work and one had already been
//! fixed, leaving sixteen — and a scan of that same tree found
//! seventeen. The seventeenth was not something that had appeared in
//! the meantime. `mindmap/connection/mod.rs:90` had been sitting at
//! that exact line since 041ec40e (2026-04-15), eleven weeks before
//! the issue was filed; the hand count simply missed it, and the
//! shipped record here said "one was new" until a review checked.
//! So a hand list does not only drift — it can start incomplete,
//! and nothing about reading it tells you which. That is the
//! argument for a scan rather than a list.
//!
//! **What counts as shipped.** Four excisions, and each names a
//! shape of test code the previous one cannot see:
//!
//! 1. `#[cfg(test)]`-gated items, excised in place by
//!    [`crate::util::source_scan::production_source`] so a file's
//!    line numbers survive into the report.
//! 2. Files a `#[cfg(test)] mod x;` declaration names, via
//!    [`test_gated_module_files`] — the gate is in the *declaring*
//!    file and nothing in the declared one says so.
//! 3. Files that gate themselves with `#![cfg(test)]`, handled
//!    inside `production_source` for the same reason.
//! 4. Baumhard's `pub mod tests;` trees and `benches/`, which carry
//!    **no** gate at all: §T2.2 makes them `pub` on purpose so the
//!    criterion harness can import the bodies, which means they
//!    really do compile into a shipped library. They are excluded by
//!    path, and that is the one exclusion here resting on a
//!    convention rather than on an attribute.
//!
//! **`unwrap` is read as a call, not as a word.** Comments come off
//! and string literals are blanked before the scan, because this
//! module's own prose says `unwrap()` more often than the workspace's
//! code ever did, and `startup_load.rs` ships a `PANICKING` constant
//! whose *contents* are the string `".unwrap()"`. Both would
//! otherwise be offenders.
//!
//! Test-only and native-only, for the same reasons
//! [`crate::util::source_scan`] is.

use crate::util::rust_source::{blank_string_literals, strip_comments};
use crate::util::source_scan::{
    production_source, relative_to_repo, test_gated_module_files, workspace_rust_sources,
};
use std::path::PathBuf;

/// Repo-relative path fragments that mark a file as test or
/// benchmark source despite carrying no `cfg` gate.
///
/// Baumhard's tests live in `pub mod tests;` trees (§T2.2) precisely
/// so `benches/test_bench.rs` can import their bodies; both they and
/// the bench file compile into ordinary builds, so no attribute
/// excludes them and nothing but the path can. `crates/maptool/tests`
/// is cargo's own integration-test directory, which is not part of
/// any shipped target either.
const UNGATED_TEST_PATHS: &[&str] = &["/tests/", "/benches/"];

/// Every `.rs` file in the workspace that a shipped build compiles,
/// in sorted order — the four excisions the module header lists,
/// minus the one `production_source` performs on a file's text
/// rather than on the file list.
///
/// Cost: one read and one `syn` parse of every `.rs` file under
/// `lib/`, `src/` and `crates/` (the test-gate resolution).
pub(crate) fn production_rust_sources() -> Vec<PathBuf> {
    let sources = workspace_rust_sources();
    let test_only = test_gated_module_files(&sources);
    sources
        .into_iter()
        .filter(|file| !test_only.contains(file))
        .filter(|file| {
            let rel = relative_to_repo(file);
            !UNGATED_TEST_PATHS.iter().any(|dir| rel.contains(dir))
        })
        .collect()
}

/// The 1-based line numbers in `code` that call `unwrap()` with no
/// arguments, paired with the trimmed line.
///
/// Pass code that has been through [`strip_comments`] and
/// [`blank_string_literals`] — see [`bare_unwrap_sites`], which is
/// the only caller that matters and does both.
///
/// Matched as a whole identifier, which is what separates it from
/// every neighbor that is not the bug: `unwrap_or`,
/// `unwrap_or_else`, `unwrap_or_default` and `unwrap_err` all
/// continue past the `p` with an ident byte, and `expect("…")` is a
/// different name carrying the very thing §9 asks for.
///
/// Two spellings of the same panic are reported, because the
/// argument list is not the thing §9 is about. `x.unwrap()` is the
/// common one. `map(Option::unwrap)` is the other: no parentheses,
/// no receiver, and the same panic one call later — a scan keyed on
/// `unwrap()` would read it as clean, and "write it as a path" is
/// too easy an escape for a rule to leave open. The path form is
/// recognized by the `::` in front of it, so a plain `unwrap`
/// appearing as a field or a binding is not mistaken for one.
///
/// **The whitespace between the three tokens may be a newline.**
/// `o.unwrap\n()` and `o.unwrap(\n)` are both legal Rust and both
/// are the bare panic; a reader that looked only within a line
/// would call each of them clean, which is the same class of escape
/// as writing the path form and is closed the same way — the search
/// runs over the whole text and the gaps are skipped with
/// `trim_start` / `trim_end`, which do not stop at a line ending.
/// The line reported is the one the name sits on.
///
/// One false positive is left standing, in the safe direction: a
/// *declaration*, `pub fn unwrap() -> u32`, is shaped exactly like a
/// call and is reported as one. Nothing in this workspace declares
/// such a function, and the cost of the mistake is a named line a
/// reader adjudicates rather than a panic that ships.
///
/// Cost: one pass over `code`, plus one pass to collect its lines.
pub(crate) fn bare_unwrap_calls(code: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = code.lines().collect();
    let bytes = code.as_bytes();
    let mut out = Vec::new();
    // `start` only ever moves forward, so the line number is carried
    // along rather than recounted from the top of the file per hit.
    let mut line_number = 1usize;
    let mut counted_through = 0usize;
    let mut from = 0usize;
    while let Some(at) = code[from..].find("unwrap") {
        let start = from + at;
        let end = start + "unwrap".len();
        from = end;
        line_number += code[counted_through..start].matches('\n').count();
        counted_through = start;
        if start > 0 && is_ident_byte(bytes[start - 1]) {
            continue;
        }
        if end < code.len() && is_ident_byte(bytes[end]) {
            continue;
        }
        let after_name = code[end..].trim_start();
        let reported = match after_name.strip_prefix('(') {
            Some(inside) => inside.trim_start().starts_with(')'),
            None => code[..start].trim_end().ends_with("::"),
        };
        if reported {
            let source = lines.get(line_number - 1).map_or("", |line| line.trim());
            out.push((line_number, source.to_string()));
        }
    }
    out
}

/// Every bare `unwrap()` call in `files`' shipped text, rendered as
/// `"<repo-relative file>:<line>: <the line>"`.
///
/// Cost: one read and one `syn` parse per file, plus two linear
/// passes over each file's text.
pub(crate) fn bare_unwrap_sites(files: &[PathBuf]) -> Vec<String> {
    let mut found = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        // `strip_comments` keeps every newline, `blank_string_literals`
        // keeps every byte, so a line number survives both.
        let code = blank_string_literals(&strip_comments(&production_source(file, &text)));
        for (line, source) in bare_unwrap_calls(&code) {
            found.push(format!("{}:{line}: {source}", relative_to_repo(file)));
        }
    }
    found
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::doc_fixtures::repo_path;

    /// Directories the walk must show work from before its silence
    /// means anything. Same posture as `bench_surface`'s body
    /// witnesses: "no offenders" reads identically to "looked
    /// nowhere", so each crate that ships code is pinned by name and
    /// a root that moves goes red naming itself.
    const SHIPPED_ROOTS: &[&str] = &[
        "lib/baumhard/src/",
        "lib/mandala_derive/src/",
        "src/application/",
        "crates/maptool/src/",
    ];

    /// A file that is dense with `unwrap()` and is **not** shipped,
    /// used as the control that the reader fires at all. Chosen for
    /// being a `pub mod tests;` tree file — excluded by path rather
    /// than by any attribute, so pointing the reader at it needs no
    /// mutation of the code under test.
    const UNWRAP_RICH_TEST_FILE: &str = "lib/baumhard/src/gfx_structs/tests/tree_tests.rs";

    /// **No shipped line in this workspace calls `unwrap()`** —
    /// CODE_CONVENTIONS §9, enforced (issue #42).
    ///
    /// The input that makes it fail is a bare `unwrap()` written
    /// anywhere outside the four test shapes the module header
    /// lists. That is not hypothetical, and the tree to try it on is
    /// this branch's merge base. Transplanted onto `84af03ea` the
    /// test fails reporting seventeen shipped lines, from
    /// `regions.rs:293` to `connection_label.rs:201` — every site
    /// the two commits before this module fixed.
    ///
    /// The commit *immediately* before the one that added this file
    /// is the wrong tree to reach for, and this sentence used to
    /// name it. By `122ea19d` all seventeen were already fixed, so
    /// the same transplant runs green: a maintainer following the
    /// old instruction would have concluded the gate was vacuous.
    /// Both runs are the same recipe — check out the commit, restore
    /// `unwrap_posture.rs`, `util/mod.rs` and `source_scan.rs` from
    /// this branch, then `cargo test -p baumhard --lib
    /// test_no_shipped_line_calls_unwrap`.
    ///
    /// Two assertions stand in front of the offender list so its
    /// emptiness is evidence rather than silence — a file-count
    /// floor with named roots, and a control that runs the same
    /// reader over a file known to be full of `unwrap()`. Without
    /// the second, a reader that had stopped recognizing the call
    /// at all would pass this test forever.
    #[test]
    fn test_no_shipped_line_calls_unwrap() {
        let files = production_rust_sources();
        for root in SHIPPED_ROOTS {
            assert!(
                files.iter().any(|f| relative_to_repo(f).starts_with(root)),
                "the walk found no shipped source under `{root}`, so it is no longer looking \
                 there — an `unwrap()` written in it would pass in silence"
            );
        }
        // Floor well under the ~330 shipped files the workspace
        // holds today: a walker defect that dropped whole trees
        // lands here before it can hollow out the check.
        assert!(
            files.len() >= 250,
            "the walk found only {} shipped source files where roughly 330 exist; it has \
             lost most of the surface it checks",
            files.len()
        );

        // The control, on the path the assertion below exercises:
        // the same reader, over real repository bytes, must still
        // report the calls it is looking for. A `strip_comments`
        // that swallowed the file, a matcher that stopped matching,
        // or a `production_source` that blanked everything would
        // each make the offender list empty for a reason that has
        // nothing to do with the workspace being clean.
        let control = repo_path(UNWRAP_RICH_TEST_FILE);
        assert!(
            control.is_file(),
            "{UNWRAP_RICH_TEST_FILE} is the control this test rests on and it is gone; \
             point the control at another test file dense with `unwrap()`"
        );
        let control_hits = bare_unwrap_sites(&[control]);
        assert!(
            control_hits.len() > 50,
            "the reader found only {} `unwrap()` calls in {UNWRAP_RICH_TEST_FILE}, which is \
             full of them — the reader is broken, and its silence over shipped code below \
             would mean nothing",
            control_hits.len()
        );
        assert!(
            !files.iter().any(|f| relative_to_repo(f) == UNWRAP_RICH_TEST_FILE),
            "the control file is being scanned as shipped source, so it would be reported \
             as an offender and this test could never be green"
        );

        let offenders = bare_unwrap_sites(&files);
        assert!(
            offenders.is_empty(),
            "{} shipped line(s) call `unwrap()`. CODE_CONVENTIONS §9: degrade and log in an \
             interactive path, `expect(\"<the invariant relied on>\")` where the value is \
             provably there, or bind it at the guard that already proved it:\n  {}",
            offenders.len(),
            offenders.join("\n  ")
        );
    }

    /// Positive control for the matcher: both spellings of the
    /// panic are reported, and the five neighbors that are not the
    /// bug are not.
    ///
    /// `unwrap_or` and friends are the ones a substring search gets
    /// wrong; `expect` is the thing §9 asks for, so a check that
    /// flagged it would be telling authors to undo the fix; and a
    /// binding merely *named* `unwrap` is not a call at all.
    #[test]
    fn test_both_spellings_of_the_panic_are_reported_and_their_neighbors_are_not() {
        let code = "let a = x.unwrap();\n\
                    let b = y.unwrap_or(0);\n\
                    let c = y.unwrap_or_else(|| 0);\n\
                    let d = y.unwrap_or_default();\n\
                    let e = r.unwrap_err();\n\
                    let f = y.expect(\"the loader guarantees this\");\n\
                    let g = maybe.map(Option::unwrap);\n\
                    let unwrap = 3;\n";
        let reported: Vec<usize> = bare_unwrap_calls(code).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            reported,
            vec![1, 7],
            "the call and the path handed to `map` are both the bug; `unwrap_or*`, \
             `unwrap_err`, `expect` and a binding named `unwrap` are not"
        );
    }

    /// **A line break between the tokens does not make it a
    /// different call.** `o.unwrap\n()` and `o.unwrap(\n)` are both
    /// legal Rust, both are the bare panic §9 forbids, and a
    /// line-local reader called both of them clean — the input that
    /// used to make this test fail is either of the first two lines
    /// below. Left open, it was the one spelling that escaped both
    /// halves of the §9 posture at once: the scan could not see it,
    /// and until the stress-map generator grew its own
    /// `#![warn(clippy::unwrap_used)]` the lint could not reach that
    /// crate root to see it either.
    ///
    /// The last two lines are the control on the same widened path.
    /// `unwrap_or(\n0)` and `unwrap(\n1)` also span a line break, and
    /// a reader that "fixed" the split forms by skipping ahead to the
    /// next `(` — or by ignoring what sits between `(` and `)` —
    /// reports one of them and goes red here. Skipping whitespace is
    /// the whole of the widening; nothing else about the match moved.
    #[test]
    fn test_a_call_split_across_a_line_break_is_still_a_call() {
        let code = "let a = x.unwrap\n    ();\n\
                    let b = y.unwrap(\n);\n\
                    let c = z.unwrap_or(\n    0);\n\
                    let d = w.unwrap(\n    1);\n";
        let reported: Vec<(usize, String)> = bare_unwrap_calls(code);
        assert_eq!(
            reported,
            vec![
                (1, "let a = x.unwrap".to_string()),
                (3, "let b = y.unwrap(".to_string()),
            ],
            "both split spellings of the bare panic are calls, reported at the line the name \
             sits on; a split `unwrap_or` and a split `unwrap(1)` are not"
        );
    }

    /// The control that disables the mechanism on the checked path.
    ///
    /// Comment stripping and literal blanking are what stand between
    /// "the code calls `unwrap()`" and "the bytes contain
    /// `unwrap()`", and both have a live instance in this
    /// repository: this module's own header discusses the call in
    /// prose, and `src/application/app/startup_load.rs` ships a
    /// `const PANICKING` whose entries are the strings `".expect("`,
    /// `".unwrap()"` and `"panic!("`. Feed the raw text and both are
    /// offenders; feed it through the two transforms and neither is.
    #[test]
    fn test_a_mention_in_a_comment_or_a_literal_is_not_a_call() {
        let raw = "// the old shape was value.unwrap();\n\
                   const PANICKING: &[&str] = &[\".unwrap()\"];\n\
                   let live = value.unwrap();\n";
        assert_eq!(
            bare_unwrap_calls(raw).len(),
            3,
            "without the transforms all three lines look like calls, which is what makes \
             the transforms load-bearing rather than tidy"
        );
        let code = blank_string_literals(&strip_comments(raw));
        let reported: Vec<usize> = bare_unwrap_calls(&code).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            reported,
            vec![3],
            "only the third line is a call; the transforms must keep it at line 3"
        );
    }

    /// The path exclusion is the one that rests on a convention
    /// rather than on an attribute (§T2.2's `pub mod tests;` trees
    /// compile into the library), so it gets its own assertion in
    /// both directions: the trees are out, and the module beside
    /// them that is not a tests tree is in.
    #[test]
    fn test_the_ungated_tests_trees_are_excluded_and_their_neighbors_are_not() {
        let files: Vec<String> = production_rust_sources()
            .iter()
            .map(|f| relative_to_repo(f))
            .collect();
        assert!(
            !files.iter().any(|f| f.contains("/tests/")),
            "a `pub mod tests;` tree file survived the path exclusion"
        );
        assert!(
            !files.iter().any(|f| f.contains("/benches/")),
            "the criterion bench file survived the path exclusion"
        );
        assert!(
            files
                .iter()
                .any(|f| f == "lib/baumhard/src/gfx_structs/tree_walker.rs"),
            "the walker sits next to `gfx_structs/tests/` and is shipped code; an exclusion \
             that swallowed it would swallow most of the crate"
        );
    }
}
