// SPDX-License-Identifier: MPL-2.0

//! The §B8 bench-surface contract, held against the tree instead of
//! written down and trusted: **every `pub fn do_*()` in a
//! `pub mod tests;` tree has exactly one `bench_function` entry in
//! `benches/test_bench.rs`, and no two entries share an id.**
//!
//! Nothing else enforces this direction. The bench file imports the
//! test modules by glob, so a body that never gets an entry — or
//! silently loses one — upsets no compiler; `cargo check
//! --workspace --benches` only catches the *reverse* drift, an
//! entry whose body is gone. Issue #44 measured what discipline
//! alone had produced: 173 of 425 bodies unbenched, eight whole
//! modules never imported — seven of which the issue named — and
//! headers still claiming otherwise.
//!
//! The contract makes the `do_` prefix mean something checkable:
//! writing the `do_*()` / `test_*()` pair *is* putting the body on
//! the bench surface. A test with no benchmark value opts out by
//! being a plain `#[test]` fn instead — `#[test]` strips it from
//! non-test builds, which is exactly what removes it from the
//! surface this module scans. See `CONVENTIONS.md` §B8 for which
//! test shapes belong in each class.
//!
//! **A name is not a call.** The scan reads the bench file's *code*
//! — comments stripped and string literals blanked
//! ([`blank_string_literals`]) — and attributes each reference to
//! the `bench_function` call it sits inside. Both halves of that are
//! load-bearing and neither is hypothetical: a criterion id is a
//! string literal, so an id that happened to spell a body's name
//! would otherwise satisfy that body's contract while running
//! something else, and a `do_*` mentioned outside any entry is a
//! mention rather than a row.
//!
//! Built on [`crate::util::source_scan`]'s walkers so "which files
//! are a test-gated tree" is answered once, there — a
//! `#[cfg(test)] mod tests;` tree like `mindmap/tree_builder`'s
//! carries no bench obligation and is excluded by the same resolver
//! every other repo-as-subject scan uses. Test-only and
//! native-only, for the same reasons `source_scan` is.

use crate::util::doc_fixtures::repo_path;
use crate::util::rust_source::{blank_string_literals, matching_delimiter, string_literals, strip_comments};
use crate::util::source_scan::{
    is_test_gated, relative_to_repo, test_gated_module_files, workspace_rust_sources,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use syn::Item;

/// Every benchmark-reusable body the baumhard tests trees export,
/// as `(function name, repo-relative file)`: the `pub fn do_*()`
/// functions declared in files under a `tests/` directory of
/// `lib/baumhard/src` that are **not** reached through a
/// `#[cfg(test)] mod` declaration.
///
/// Cost: one read and one `syn` parse of every `.rs` file in the
/// workspace (the test-gate resolution), plus one parse per
/// tests-tree file.
pub(crate) fn benchmark_reusable_bodies() -> Vec<(String, String)> {
    let sources = workspace_rust_sources();
    let test_only = test_gated_module_files(&sources);
    let tree_files: Vec<PathBuf> = sources
        .into_iter()
        .filter(|file| {
            let rel = relative_to_repo(file);
            rel.starts_with("lib/baumhard/src/") && rel.contains("/tests/") && !test_only.contains(file)
        })
        .collect();
    collect_do_bodies(&tree_files)
}

/// The `pub fn do_*()` declarations in `files`, as `(name,
/// repo-relative file)`. Descends into ungated inline `mod` blocks;
/// skips any item behind a `cfg(test)` gate, because the bench
/// binary is not compiled under `cfg(test)` and cannot reach it.
pub(crate) fn collect_do_bodies(files: &[PathBuf]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        let parsed =
            syn::parse_file(&text).unwrap_or_else(|e| panic!("{} must parse as Rust: {e}", file.display()));
        collect_from_items(&parsed.items, &relative_to_repo(file), &mut out);
    }
    out.sort();
    out
}

fn collect_from_items(items: &[Item], rel: &str, out: &mut Vec<(String, String)>) {
    for item in items {
        match item {
            Item::Fn(fun) => {
                if is_test_gated(&fun.attrs) {
                    continue;
                }
                let name = fun.sig.ident.to_string();
                if matches!(fun.vis, syn::Visibility::Public(_)) && name.starts_with("do_") {
                    out.push((name, rel.to_string()));
                }
            }
            Item::Mod(module) => {
                if is_test_gated(&module.attrs) {
                    continue;
                }
                if let Some((_, inner)) = &module.content {
                    collect_from_items(inner, rel, out);
                }
            }
            _ => {}
        }
    }
}

/// One `c.bench_function(…)` call in `benches/test_bench.rs`: the
/// criterion id it publishes, and the `do_*()` bodies its code half
/// names.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BenchEntry {
    /// The first string literal inside the call — criterion's row id.
    pub(crate) id: String,
    /// Every `do_*` whole identifier inside the call, read from the
    /// code with literals blanked, so an id that *spells* a body
    /// name does not count as naming it.
    pub(crate) bodies: Vec<String>,
}

/// Every `bench_function` entry `bench_source` declares, in source
/// order.
///
/// Two views of the same bytes do the work. Comments come off first,
/// so a commented-out entry is not an entry. Then
/// [`blank_string_literals`] gives a code-only twin of the same byte
/// length: the call's extent and the `do_*` identifiers in it are
/// read from the twin, the id is read from the literal-bearing
/// original at the same offsets.
///
/// A `bench_function` with no string literal in it contributes no
/// entry — criterion has no such shape, and inventing an empty id
/// for one would make two of them collide.
pub(crate) fn bench_entries(bench_source: &str) -> Vec<BenchEntry> {
    const CALL: &str = "bench_function";
    let live = strip_comments(bench_source);
    let code = blank_string_literals(&live);
    debug_assert_eq!(
        code.len(),
        live.len(),
        "blanking literals must keep byte offsets, or the id and the call \
         extent are read from different places"
    );
    let mut entries = Vec::new();
    let mut from = 0usize;
    while let Some(at) = code[from..].find(CALL) {
        let start = from + at;
        let after = start + CALL.len();
        from = after;
        if start > 0 && is_ident_byte(code.as_bytes()[start - 1]) {
            continue;
        }
        let Some(open) = code[after..].find('(').map(|at| after + at) else {
            continue;
        };
        if !code[after..open].trim().is_empty() {
            continue;
        }
        let Some(close) = matching_delimiter(&code, open, b'(', b')') else {
            continue;
        };
        from = close;
        let Some(id) = string_literals(&live[open..=close])
            .first()
            .map(|id| id.to_string())
        else {
            continue;
        };
        entries.push(BenchEntry {
            id,
            bodies: do_identifiers_in(&code[open..=close]),
        });
    }
    entries
}

/// Every distinct `do_*` whole identifier in `code`, in source order.
fn do_identifiers_in(code: &str) -> Vec<String> {
    let bytes = code.as_bytes();
    let mut found: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < code.len() {
        if bytes[i] == b'd' && code[i..].starts_with("do_") && (i == 0 || !is_ident_byte(bytes[i - 1])) {
            let mut end = i + "do_".len();
            while end < code.len() && is_ident_byte(bytes[end]) {
                end += 1;
            }
            let name = code[i..end].to_string();
            if !found.contains(&name) {
                found.push(name);
            }
            i = end;
            continue;
        }
        i += 1;
    }
    found
}

/// The bodies in `bodies` that no entry in `entries` names, as
/// rendered `"<file>: <name>"` lines.
///
/// A name only counts when it appears as a whole identifier inside a
/// `bench_function` call — an entry for `do_x_and_more` says nothing
/// about `do_x`, and neither does the string `"do_x"` written as
/// some other entry's id.
pub(crate) fn missing_bench_entries(bodies: &[(String, String)], entries: &[BenchEntry]) -> Vec<String> {
    bodies
        .iter()
        .filter(|(name, _)| {
            !entries
                .iter()
                .any(|entry| entry.bodies.iter().any(|body| body == name))
        })
        .map(|(name, file)| format!("{file}: {name}"))
        .collect()
}

/// The bodies in `bodies` that more than one entry names, as
/// rendered `"<file>: <name> (<id>, <id>)"` lines.
///
/// The other half of §B8's *exactly one*. One body measured under
/// two ids is two criterion rows that move together and mean the
/// same thing, and the reader of the report has no way to tell that
/// from two rows over different code.
pub(crate) fn duplicated_bench_entries(bodies: &[(String, String)], entries: &[BenchEntry]) -> Vec<String> {
    bodies
        .iter()
        .filter_map(|(name, file)| {
            let ids: Vec<&str> = entries
                .iter()
                .filter(|entry| entry.bodies.iter().any(|body| body == name))
                .map(|entry| entry.id.as_str())
                .collect();
            (ids.len() > 1).then(|| format!("{file}: {name} ({})", ids.join(", ")))
        })
        .collect()
}

/// The ids `entries` publishes more than once, each with its count.
///
/// Criterion keys a row's stored history by its id, so two entries
/// sharing one write into the same directory: the second overwrites
/// the first's samples and the report shows one row where the file
/// declares two.
pub(crate) fn duplicate_bench_ids(entries: &[BenchEntry]) -> Vec<String> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in entries {
        *seen.entry(entry.id.as_str()).or_default() += 1;
    }
    seen.into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(id, count)| format!("{id} ({count} entries)"))
        .collect()
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_temp::TempDir;

    /// The tests-tree prefixes the scan must show work from. Same
    /// posture as `source_scan`'s citation witnesses: "nothing is
    /// missing" is exactly as loud when the walk has stopped
    /// looking, so each tree that carries bodies today is pinned by
    /// name and a root that moves goes red naming itself.
    const BODY_WITNESSES: &[&str] = &[
        "lib/baumhard/src/core/tests/",
        "lib/baumhard/src/font/tests/",
        "lib/baumhard/src/gfx_structs/tests/",
        "lib/baumhard/src/util/tests/",
    ];

    /// The bench file as the repo tests read it, panicking by name
    /// rather than returning an empty string a scan would call
    /// clean.
    fn bench_source() -> String {
        let bench_path = repo_path("lib/baumhard/benches/test_bench.rs");
        std::fs::read_to_string(&bench_path)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", bench_path.display()))
    }

    /// **Every `do_*()` body has exactly one bench entry** — §B8's
    /// checked direction, both halves. The reverse (an entry whose
    /// body is gone) fails `cargo check --workspace --benches`,
    /// which `./test.sh` runs.
    ///
    /// The failure lists every body missing its entry; the fix is
    /// one `c.bench_function("<name minus do_>", |b| b.iter(<name>))`
    /// per line, or — for a body that should not be a benchmark —
    /// folding it into its plain `#[test]` wrapper (§B8's opt-out).
    #[test]
    fn test_every_do_body_has_a_bench_entry() {
        let bodies = benchmark_reusable_bodies();
        for witness in BODY_WITNESSES {
            assert!(
                bodies.iter().any(|(_, file)| file.starts_with(witness)),
                "the scan found no do_* bodies under `{witness}`, so it is no longer \
                 looking there — a body missing its entry would pass in silence"
            );
        }
        // Floor well under the ~520 bodies the trees hold today:
        // a syn-walk defect that dropped whole files would land
        // here before it could hollow out the check. Lower it
        // consciously if the trees genuinely shrink.
        assert!(
            bodies.len() >= 400,
            "the scan found only {} do_* bodies where roughly 520 exist; the walk \
             has lost most of the surface it checks",
            bodies.len()
        );
        let entries = bench_entries(&bench_source());
        // Same posture as the body floor: an entry parser that
        // stopped finding calls would report every body missing,
        // which is loud — but one that found only a handful and
        // still matched the bodies it found is the quiet failure.
        assert!(
            entries.len() >= bodies.len(),
            "the parser found {} bench_function entries for {} do_* bodies; \
             benches/test_bench.rs cannot hold fewer entries than bodies, so \
             the parse has lost most of the file",
            entries.len(),
            bodies.len()
        );
        let missing = missing_bench_entries(&bodies, &entries);
        assert!(
            missing.is_empty(),
            "{} do_* body/bodies have no entry in benches/test_bench.rs. Add one \
             `c.bench_function` per body (id = name minus `do_`), or fold a body \
             that should not be benched into its `#[test]` wrapper — §B8 names the \
             opt-out classes:\n  {}",
            missing.len(),
            missing.join("\n  ")
        );
        let duplicated = duplicated_bench_entries(&bodies, &entries);
        assert!(
            duplicated.is_empty(),
            "{} do_* body/bodies are measured by more than one entry. §B8 is \
             exactly one: two rows over one body move together and say the same \
             thing, so drop all but one:\n  {}",
            duplicated.len(),
            duplicated.join("\n  ")
        );
    }

    /// **No two entries publish the same criterion id.** Criterion
    /// keys a row's stored history by its id, so a collision is two
    /// declarations writing one row — the second's samples replace
    /// the first's and the report is quietly a row short.
    #[test]
    fn test_every_bench_entry_id_is_unique() {
        let entries = bench_entries(&bench_source());
        assert!(
            entries.len() >= 400,
            "the parser found only {} bench_function entries where roughly 540 \
             exist; a collision among the ones it lost would not be seen",
            entries.len()
        );
        let duplicates = duplicate_bench_ids(&entries);
        assert!(
            duplicates.is_empty(),
            "{} criterion id(s) are declared by more than one entry in \
             benches/test_bench.rs; rename all but one:\n  {}",
            duplicates.len(),
            duplicates.join("\n  ")
        );
    }

    /// Positive control: a body the bench text never mentions is
    /// reported, and a body it mentions is not. Proves the repo
    /// test *can* fail before trusting its silence.
    #[test]
    fn test_a_body_without_an_entry_is_reported() {
        let bodies = vec![
            ("do_present".to_string(), "a.rs".to_string()),
            ("do_absent".to_string(), "b.rs".to_string()),
        ];
        let bench = r#"c.bench_function("present", |b| b.iter(do_present));"#;
        assert_eq!(
            missing_bench_entries(&bodies, &bench_entries(bench)),
            vec!["b.rs: do_absent"]
        );
    }

    /// The control that disables the mechanism on the checked path:
    /// comment stripping is what stands between "the entry exists"
    /// and "the entry is written somewhere in the file's bytes". A
    /// reference that survives only inside a comment must not
    /// satisfy the contract.
    #[test]
    fn test_a_commented_out_entry_does_not_satisfy_the_contract() {
        let bodies = vec![("do_present".to_string(), "a.rs".to_string())];
        let live = r#"c.bench_function("present", |b| b.iter(do_present));"#;
        let commented = r#"// c.bench_function("present", |b| b.iter(do_present));"#;
        assert!(missing_bench_entries(&bodies, &bench_entries(live)).is_empty());
        assert_eq!(
            missing_bench_entries(&bodies, &bench_entries(commented)),
            vec!["a.rs: do_present"]
        );
    }

    /// The other control on the same path, and the one comment
    /// stripping alone does not give: a criterion id is a string
    /// literal, so an entry whose *id* spells a body's name must not
    /// satisfy that body's contract while `iter`ing something else.
    /// Blanking literals is what separates the two, and the entry
    /// here is the exact shape that was a false green before it —
    /// one real body, one id that names another.
    #[test]
    fn test_a_body_named_only_by_an_entry_id_does_not_satisfy_the_contract() {
        let bodies = vec![
            ("do_named_in_a_literal".to_string(), "a.rs".to_string()),
            ("do_really_run".to_string(), "b.rs".to_string()),
        ];
        let bench = r#"c.bench_function("do_named_in_a_literal", |b| b.iter(do_really_run));"#;
        assert_eq!(
            missing_bench_entries(&bodies, &bench_entries(bench)),
            vec!["a.rs: do_named_in_a_literal"],
            "the body only the id spells must be reported, and the body the \
             entry runs must not"
        );
    }

    /// A `do_*` written outside every `bench_function` call is a
    /// mention, not a row. The entry parser is what draws that line:
    /// `missing_bench_entries` sees only the names inside a call's
    /// parentheses, so a body named by a bench-local `use` or a
    /// stray helper reference stays reported.
    #[test]
    fn test_a_reference_outside_any_entry_does_not_satisfy_the_contract() {
        let bodies = vec![("do_outside".to_string(), "a.rs".to_string())];
        let bench = "let held = do_outside;\nc.bench_function(\"other\", |b| b.iter(do_other));";
        assert_eq!(
            missing_bench_entries(&bodies, &bench_entries(bench)),
            vec!["a.rs: do_outside"]
        );
    }

    /// An entry for `do_x_and_more` says nothing about `do_x`: the
    /// reference has to be the whole identifier, or the commonest
    /// drift — renaming a body by growing its name and updating the
    /// bench file — would leave the old body's contract satisfied
    /// by the new body's entry.
    #[test]
    fn test_a_longer_identifier_does_not_satisfy_a_shorter_body() {
        let bodies = vec![("do_x".to_string(), "a.rs".to_string())];
        let bench = r#"c.bench_function("x_and_more", |b| b.iter(do_x_and_more));"#;
        assert_eq!(
            missing_bench_entries(&bodies, &bench_entries(bench)),
            vec!["a.rs: do_x"]
        );
    }

    /// The parser's own claim: one entry per `bench_function` call,
    /// its id the first literal in the call and its bodies the
    /// identifiers in the call's *code*. The fixture carries every
    /// shape the real file has — a nested `|b| b.iter(...)` closure,
    /// a multi-line call, an id that spells a body name, and a
    /// second call after a `)` the walk has to step past.
    #[test]
    fn test_the_parser_reads_one_entry_per_call() {
        let bench = "c.bench_function(\"first\", |b| b.iter(do_first));\n\
                     c.bench_function(\n    \"second\",\n    |b| b.iter(|| do_second(&fixture)),\n);\n\
                     c.bench_function(\"do_first\", |b| b.iter(do_third));\n";
        let entries = bench_entries(bench);
        assert_eq!(
            entries,
            vec![
                BenchEntry {
                    id: "first".to_string(),
                    bodies: vec!["do_first".to_string()],
                },
                BenchEntry {
                    id: "second".to_string(),
                    bodies: vec!["do_second".to_string()],
                },
                BenchEntry {
                    id: "do_first".to_string(),
                    bodies: vec!["do_third".to_string()],
                },
            ]
        );
    }

    /// Two entries running one body are reported, and the message
    /// names both ids — the reader has to know which two rows to
    /// choose between. One body under one id is not reported, or the
    /// whole tree would be.
    #[test]
    fn test_two_entries_over_one_body_are_reported() {
        let bodies = vec![
            ("do_twice".to_string(), "a.rs".to_string()),
            ("do_once".to_string(), "b.rs".to_string()),
        ];
        let bench = "c.bench_function(\"twice\", |b| b.iter(do_twice));\n\
                     c.bench_function(\"twice_again\", |b| b.iter(do_twice));\n\
                     c.bench_function(\"once\", |b| b.iter(do_once));";
        assert_eq!(
            duplicated_bench_entries(&bodies, &bench_entries(bench)),
            vec!["a.rs: do_twice (twice, twice_again)"]
        );
    }

    /// Two entries under one id are reported with the count, and
    /// distinct ids over the same body are not — that is the
    /// *other* failure, and `duplicated_bench_entries` owns it.
    #[test]
    fn test_two_entries_under_one_id_are_reported() {
        let collision = "c.bench_function(\"same\", |b| b.iter(do_one));\n\
                         c.bench_function(\"same\", |b| b.iter(do_two));";
        assert_eq!(
            duplicate_bench_ids(&bench_entries(collision)),
            vec!["same (2 entries)"]
        );
        let distinct = "c.bench_function(\"a\", |b| b.iter(do_one));\n\
                        c.bench_function(\"b\", |b| b.iter(do_one));";
        assert!(duplicate_bench_ids(&bench_entries(distinct)).is_empty());
    }

    /// The collector reads only what the bench binary can reach: a
    /// `pub fn do_*` counts, a private one does not, and anything
    /// behind a `cfg(test)` gate — including a whole inline module —
    /// is invisible to a binary compiled without `cfg(test)`.
    #[test]
    fn test_the_collector_sees_only_pub_ungated_do_fns() {
        let dir = TempDir::new("bench-surface-collect");
        std::fs::write(
            dir.join("planted.rs"),
            "pub fn do_reachable() {}\n\
             fn do_private() {}\n\
             #[cfg(test)]\n\
             pub fn do_gated() {}\n\
             #[cfg(test)]\n\
             mod inner {\n    pub fn do_inside_gate() {}\n}\n\
             mod ungated_inner {\n    pub fn do_nested_reachable() {}\n}\n\
             #[test]\n\
             fn test_do_reachable() {}\n",
        )
        .expect("planted file must be writable");
        let found = collect_do_bodies(&[dir.join("planted.rs")]);
        let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"do_reachable"), "{names:?}");
        assert!(names.contains(&"do_nested_reachable"), "{names:?}");
        assert!(!names.contains(&"do_private"), "{names:?}");
        assert!(!names.contains(&"do_gated"), "{names:?}");
        assert!(!names.contains(&"do_inside_gate"), "{names:?}");
    }
}
