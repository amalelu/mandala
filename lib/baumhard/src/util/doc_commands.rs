// SPDX-License-Identifier: MPL-2.0

//! The `cargo` commands this repository writes down, held against the
//! workspace they claim to drive.
//!
//! A command written into a doc is a claim — *run this and it does
//! what the sentence around it says* — and nothing here checked
//! those claims. Two had been false since they were written, and
//! #148 found both.
//!
//! `CLAUDE.md`'s "Common tasks" and `TEST_CONVENTIONS.md` §T11 both
//! told a reader to narrow a mandala test run with `-p mandala`
//! plus `--lib`, which exits 101 with `no library targets found in
//! package 'mandala'` — that member is a binary crate. The flag is
//! correct on its neighbor, since baumhard *is* a library, and had
//! been copied across.
//!
//! The other was quieter and cost more. Five documents, `CLAUDE.md`
//! §7 and `AGENTS.md` among them, published a `--benches` check as
//! the proof that a bench target still compiles — the only proof an
//! agent is allowed, since running the benchmarks is forbidden — and
//! wrote it with no package selection. Cargo then takes the root
//! package alone, which declares `autobenches = false` and owns no
//! bench target, so it compiles nothing, prints "Finished" and exits
//! 0 over a bench file full of type errors. A gate that cannot fail
//! reads exactly like a gate that passes.
//!
//! Both defects are mechanical, so both are checked mechanically
//! against the real manifests:
//!
//! - every `-p <name>` in an audited section names a workspace
//!   member;
//! - every `--lib` sits on a member that actually has a library
//!   target;
//! - every `--benches` sits on a selection that owns a bench target.
//!
//! **The third question is asked repository-wide** — every Markdown
//! file, every Rust comment, the shell scripts and the CI workflows
//! — while the first two are asked of two named sections. The difference is not an
//! oversight in either direction. A `--lib` is a mistake *inside* a
//! passage about narrowing a test run, and [`AUDITED_SECTIONS`] is
//! where those passages live; a `--benches` with no selection is a
//! no-op wherever it is written, including in `test.sh`, which is
//! where it is actually run rather than merely described.
//!
//! **What this reads.** Every way those two sections can publish a
//! command: an inline code span opened with any number of backticks,
//! and every line of a fenced block — backtick-fenced or
//! tilde-fenced, tagged or not. The tagged fence is called out
//! because it was the one gap that made the reader *inconsistent*
//! rather than narrow — the tag glued itself to the front of the
//! command and the `cargo` filter then dropped it, so a fence opened
//! with a `bash` tag hid a command that the same fence untagged did
//! not.
//!
//! **What this does not reach**, said plainly so the silence is not
//! read as coverage. It runs nothing: that a documented command is
//! well-formed against the manifests is a weaker claim than that it
//! succeeds, and the strong one belongs to a person at a terminal.
//! It judges only spans carrying an explicit `-p` / `--package`,
//! because a command without one selects whatever package the
//! working directory holds and a code span does not carry a working
//! directory. It reads a command that is *marked up* as one: a
//! `cargo` invocation typed into running prose with no backticks
//! around it is prose to this reader, as is one hidden inside a
//! quoted string. Inside a fence it takes one line as one command,
//! so a shell continuation (`\` at end of line) reads as two. And
//! it reads two named sections rather than every Markdown file, for
//! the reason [`crate::util::doc_fixtures`] exists at all: a
//! whole-file scan stays green after the thing it claims to pin has
//! moved somewhere else entirely.
//!
//! The repository-wide half has its own edges. It reads text rather
//! than markup, so a command in running prose is judged too — but
//! only where the tokens in front of the flag *are* an invocation:
//! `cargo`, a subcommand, then nothing but flags and their values. A
//! `--benches` in a sentence about the flag is prose, not a command,
//! and is passed over. That is a deliberate hole with a floor under
//! it: `assert_bench_scan_is_looking` requires the invocation to
//! still be found in both documents that state the rule and in the
//! script that runs it, so the shape cannot quietly empty out.
//!
//! In a Rust file it reads the comments — `//`, `///`, `//!` and
//! `/* … */` alike, by way of
//! [`comment_text`](crate::util::rust_source::comment_text) — and
//! nothing else, so a string literal holding the bare command is not
//! a claim about how to build this repository. That reader is the
//! whole of the exemption: a hand-rolled one that blanked literals
//! in source that still had its comments would let an unbalanced
//! `"` in prose hide the invocation underneath it.
//!
//! One edge remains, and it is in the loud direction: the reader
//! judges a command by what it says, not by where it would run, so
//! an invocation that a preceding `cd` into a member's directory
//! would make correct is reported anyway. No such shape is written
//! here, and reporting one that is fine costs a sentence in a
//! message, while passing one that is not costs the rule — the
//! direction a checker of something nobody may test by running it
//! has to fail in.
//!
//! Test-only and native-only, for the same reasons
//! `crate::util::manifests` — whose member list this reuses rather
//! than restating — is.

use crate::util::doc_fixtures::{repo_path, section_text};
use crate::util::manifests::member_manifests;
use crate::util::rust_source::comment_text;
use crate::util::source_scan;
use std::collections::BTreeMap;
use std::path::Path;

/// The doc sections that publish runnable commands, as
/// `(repo-relative file, exact heading line)`.
///
/// Adding a section here is how a new set of published commands
/// joins the checked set. [`section_text`] panics when a heading is
/// gone, so a renamed section stops the run rather than quietly
/// removing itself.
pub(crate) const AUDITED_SECTIONS: [(&str, &str); 2] = [
    ("CLAUDE.md", "## Common tasks"),
    ("TEST_CONVENTIONS.md", "## §T11 Running the suite"),
];

/// One `cargo` invocation quoted in an audited doc section.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DocCommand {
    /// Repo-relative file the span was read from, for the message.
    pub(crate) doc: String,
    /// The span, whitespace-normalized — code spans wrap across
    /// source lines and a reader should see one line.
    pub(crate) command: String,
    /// The argument of `-p` / `--package`, when the span carries
    /// one. `None` means the selection depends on the working
    /// directory, which this module deliberately does not guess.
    pub(crate) package: Option<String>,
    /// Whether cargo's own arguments — those before a bare `--` —
    /// include `--lib`.
    pub(crate) wants_lib: bool,
}

/// Every `cargo` command quoted in [`AUDITED_SECTIONS`], parsed.
///
/// Cost: one read of each audited doc plus a linear scan of the
/// named section. Paid once per test that asks.
pub(crate) fn documented_cargo_commands() -> Vec<DocCommand> {
    let mut out = Vec::new();
    for (doc, heading) in AUDITED_SECTIONS {
        for command in cargo_spans(&section_text(&repo_path(doc), heading), doc) {
            let package = package_selected(&command, doc);
            let wants_lib = cargo_arguments(&command).any(|token| token == "--lib");
            out.push(DocCommand {
                doc: doc.to_string(),
                command,
                package,
                wants_lib,
            });
        }
    }
    out
}

/// The code in `text` that holds a `cargo` invocation,
/// whitespace-normalized: every inline code span, plus every line of
/// every fenced block, in document order.
///
/// A fenced block is taken line by line and its opening marker —
/// info string and all — is dropped. Reading the fence body as prose
/// instead is what used to make a language-tagged fence invisible:
/// the tag ended up glued to the front of the command, and the
/// `cargo` filter below dropped the result. A doc that publishes a
/// command in a fence is publishing a command, and a tag is a
/// syntax-highlighting hint, not a claim that the line is unrunnable.
///
/// An opening fence that is never closed, and a code span whose
/// backtick run is never matched by a run of the same length, both
/// panic naming `doc`: either one swallows the rest of the section
/// into a single span, turning a section full of commands into a
/// section holding none. That posture is `crate::util::manifests`':
/// a shape the reader cannot handle stops the run instead of
/// deleting itself from the checked set.
pub(crate) fn cargo_spans(text: &str, doc: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut prose = String::new();
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        match (fence, fence_open(trimmed)) {
            (Some(marker), _) if is_fence_close(trimmed, marker) => fence = None,
            (Some(_), _) => spans.push(normalize(line)),
            (None, Some(marker)) => {
                spans.extend(inline_spans(&prose, doc));
                prose.clear();
                fence = Some(marker);
            }
            (None, None) => {
                prose.push('\n');
                prose.push_str(line);
            }
        }
    }
    assert!(
        fence.is_none(),
        "{doc}: a fenced code block is opened and never closed, so everything below \
         it reads as code and this reader can no longer tell a published command \
         from prose"
    );
    spans.extend(inline_spans(&prose, doc));
    spans.retain(|span| span == "cargo" || span.starts_with("cargo "));
    spans
}

/// The marker `trimmed` opens a fenced block with, or `None` when it
/// opens none: a run of three or more backticks, or of three or more
/// tildes.
///
/// CommonMark gives both, and this repository writes only the
/// backtick form — which is exactly why the tilde form is read too. A
/// reader that knew one of the two spellings would go quiet on a
/// command published in the other, and go quiet in the shape this
/// module exists to remove: silently, on a document that looks
/// checked.
fn fence_open(trimmed: &str) -> Option<(char, usize)> {
    let opener = trimmed.chars().next()?;
    if opener != '`' && opener != '~' {
        return None;
    }
    let run = trimmed.chars().take_while(|c| *c == opener).count();
    (run >= 3).then_some((opener, run))
}

/// Whether `trimmed` closes a fence opened with `marker` — a run of
/// at least as many of the same character as opened it, and nothing
/// else on the line, which is what CommonMark asks of a closing
/// fence.
fn is_fence_close(trimmed: &str, (opener, opened): (char, usize)) -> bool {
    let run = trimmed.chars().take_while(|c| *c == opener).count();
    run >= opened && trimmed.trim_end().len() == run
}

/// `text` with every run of whitespace — line breaks included —
/// collapsed to one space. A code span wraps across source lines and
/// a reader should see one line.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The inline code spans in `text`, normalized, in document order.
///
/// A span opens on a run of backticks and closes on the next run of
/// exactly the same length, so the two-backtick form Markdown offers
/// for a command that itself contains a backtick is read rather than
/// silently skipped. An opener with no matching closer panics: it is
/// far more often a typo that hid a command than a literal backtick
/// somebody meant.
fn inline_spans(text: &str, doc: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let open = backtick_run(bytes, at);
        let Some(close) = closing_run(bytes, at + open, open) else {
            panic!(
                "{doc}: a code span opened with {open} backtick(s) is never closed by a run \
                 of {open} — its backticks do not pair up, so this reader would swallow the \
                 rest of the section into one span. It opens at: {:?}",
                normalize(&text[at..bytes.len().min(at + 60)])
            );
        };
        out.push(normalize(&text[at + open..close]));
        at = close + open;
    }
    out
}

/// The length of the run of backticks starting at `at`.
fn backtick_run(bytes: &[u8], at: usize) -> usize {
    bytes[at..].iter().take_while(|byte| **byte == b'`').count()
}

/// The byte offset of the next run of exactly `open` backticks at or
/// after `from`, or `None` when the text holds no such run.
fn closing_run(bytes: &[u8], from: usize, open: usize) -> Option<usize> {
    let mut at = from;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let run = backtick_run(bytes, at);
        if run == open {
            return Some(at);
        }
        at += run;
    }
    None
}

/// Cargo's own arguments in `command` — every token up to a bare
/// `--`, which hands the rest to the built binary or the invoked
/// tool. `cargo run -- --lib` passes `--lib` to mandala; it does not
/// ask cargo for a library target.
pub(crate) fn cargo_arguments(command: &str) -> impl Iterator<Item = &str> {
    command.split(' ').take_while(|token| *token != "--")
}

/// The package `command` selects with `-p` / `--package`, or `None`
/// when it names none.
///
/// A selection flag with nothing after it panics naming `doc`: the
/// span is not a command anyone can run, which is the class of
/// defect this module exists for rather than an input to tolerate.
pub(crate) fn package_selected(command: &str, doc: &str) -> Option<String> {
    let mut tokens = cargo_arguments(command);
    while let Some(token) = tokens.next() {
        if let Some(name) = token.strip_prefix("--package=") {
            return Some(name.to_string());
        }
        if token == "-p" || token == "--package" {
            return Some(
                tokens
                    .next()
                    .unwrap_or_else(|| panic!("{doc}: `{command}` ends on {token} with no package after it"))
                    .to_string(),
            );
        }
    }
    None
}

/// Every workspace member, mapped to whether it has a library
/// target.
///
/// Read from the manifests [`member_manifests`] resolves out of
/// `[workspace] members`, so a fifth crate joins the checked set the
/// day it is added. A member has a library target when its manifest
/// declares `[lib]` or cargo would auto-discover `src/lib.rs` beside
/// it — the two ways one comes to exist.
///
/// Cost: one read of each member manifest plus one `stat` per
/// member.
pub(crate) fn library_targets() -> BTreeMap<String, bool> {
    member_targets(has_lib_target)
}

/// Every workspace member, mapped to whether it has a **bench**
/// target — the question `--benches` turns on.
///
/// Same reader, same cost, different predicate: a member owns a
/// bench target when its manifest declares `[[bench]]`, or when
/// nothing switched `autobenches` off and there is a `benches/*.rs`
/// beside it for cargo to discover. Both members that set
/// `autobenches = false` here mean it: the root package owns no
/// bench at all, and baumhard declares its one explicitly because a
/// criterion bench needs `harness = false`.
pub(crate) fn bench_targets() -> BTreeMap<String, bool> {
    member_targets(has_bench_target)
}

/// Every workspace member, mapped by `has_target`.
fn member_targets(has_target: impl Fn(&str, &str) -> bool) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    for manifest in member_manifests() {
        let text = std::fs::read_to_string(repo_path(&manifest))
            .unwrap_or_else(|e| panic!("{manifest} must be readable: {e}"));
        out.insert(package_name(&text, &manifest), has_target(&manifest, &text));
    }
    out
}

/// The `[package] name` of a manifest, section-scoped.
///
/// Section-scoped because `name` is not unique in a manifest:
/// `lib/baumhard/Cargo.toml` carries a second one under `[[bench]]`,
/// and a whole-file search for the first `name = ` would answer
/// correctly there only by accident of ordering.
fn package_name(text: &str, manifest: &str) -> String {
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("name") else {
            continue;
        };
        let Some((_, value)) = rest.split_once('=') else {
            continue;
        };
        let name = value.trim().trim_matches('"').trim();
        assert!(
            !name.is_empty() && name != "true",
            "{manifest}: `[package] name` reads {name:?} — an inherited or empty name \
             is a shape this reader cannot resolve to a package"
        );
        return name.to_string();
    }
    panic!("{manifest}: no `[package] name` — the doc scan cannot name what it checks")
}

/// Whether the member at `manifest` has a library target: a declared
/// `[lib]` table, or a `src/lib.rs` cargo would auto-discover.
fn has_lib_target(manifest: &str, text: &str) -> bool {
    if text.lines().any(|line| line.trim() == "[lib]") {
        return true;
    }
    member_dir(manifest).join("src").join("lib.rs").is_file()
}

/// Whether the member at `manifest` has a bench target: a declared
/// `[[bench]]` table, or — when `autobenches` has not switched
/// discovery off — a `benches/*.rs` cargo would find.
fn has_bench_target(manifest: &str, text: &str) -> bool {
    let mut declared = false;
    let mut autobenches = true;
    for line in text.lines() {
        let trimmed = line.trim();
        declared |= trimmed == "[[bench]]";
        if let Some(value) = trimmed.strip_prefix("autobenches") {
            autobenches = value.trim_start().strip_prefix('=').map(str::trim) != Some("false");
        }
    }
    if declared {
        return true;
    }
    autobenches
        && std::fs::read_dir(member_dir(manifest).join("benches")).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                entry.path().extension().is_some_and(|ext| ext == "rs") && entry.path().is_file()
            })
        })
}

/// The directory the member at `manifest` lives in.
fn member_dir(manifest: &str) -> std::path::PathBuf {
    let dir = Path::new(manifest).parent().unwrap_or_else(|| Path::new(""));
    repo_path(&dir.to_string_lossy())
}

/// Every documented command that asks cargo for a library target of
/// a package that has none, rendered `"<doc>: <command>"`.
///
/// A `--lib` on a package outside `targets` is *not* reported here —
/// [`unknown_package_selections`] owns that failure, and reporting
/// it twice would say one defect in two voices.
pub(crate) fn lib_flag_violations(commands: &[DocCommand], targets: &BTreeMap<String, bool>) -> Vec<String> {
    commands
        .iter()
        .filter(|command| command.wants_lib)
        .filter(|command| {
            command
                .package
                .as_ref()
                .and_then(|name| targets.get(name))
                .is_some_and(|has_lib| !has_lib)
        })
        .map(|command| format!("{}: `{}`", command.doc, command.command))
        .collect()
}

/// Every documented `-p <name>` naming something that is not a
/// workspace member, rendered `"<doc>: <command>"`.
pub(crate) fn unknown_package_selections(
    commands: &[DocCommand],
    targets: &BTreeMap<String, bool>,
) -> Vec<String> {
    commands
        .iter()
        .filter(|command| {
            command
                .package
                .as_ref()
                .is_some_and(|name| !targets.contains_key(name))
        })
        .map(|command| format!("{}: `{}`", command.doc, command.command))
        .collect()
}

/// How a file's lines turn into text a command can be read out of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextKind {
    /// Markdown and shell: every line is text as written. A command
    /// in a `#` comment and one the shell actually runs are both
    /// just text here, which is the point — `test.sh` is where the
    /// rule is kept or lost.
    Prose,
    /// Rust: the file is reduced to its comment prose first
    /// ([`comment_text`]), so what is read is what a person wrote in
    /// a `//`, `///`, `//!` or `/* … */` comment and nothing else. A
    /// line of code carries no prose, which makes it a block
    /// boundary, so a `cargo` in one doc comment cannot be read as
    /// the head of a `--benches` far below it.
    RustComment,
}

/// One `cargo … --benches` written somewhere in this repository.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BenchCheck {
    /// `"<repo-relative file>:<line>"` of the flag, for the message.
    pub(crate) site: String,
    /// The invocation the flag sits in, stripped of the markup around
    /// it and whitespace-normalized.
    pub(crate) invocation: String,
}

/// How far back from a `--benches` this reader looks for the `cargo`
/// that opens its invocation. Every site in this repository is within
/// four tokens; the slack is for a longer invocation, not for a
/// paragraph that happens to mention cargo earlier.
const CARGO_LOOKBACK: usize = 12;

/// How far past a `--benches` the trailing run of flags may reach, so
/// that a selection written *after* the flag is still read while an
/// unrelated later command is not.
const TRAILING_FLAGS: usize = 6;

/// Every `--benches` this repository writes: in its Markdown, in its
/// Rust comments, and in the shell scripts and CI workflows that run
/// the command.
///
/// Deliberately wider than [`documented_cargo_commands`], which reads
/// two named sections. The `--lib` defect was a mistake inside a
/// section about narrowing a test run; this one is a rule the
/// repository states in five documents and *executes* in `test.sh`,
/// and the bare form is a no-op wherever it is written — so it is
/// judged wherever it is written.
///
/// Cost: one read of every `.md`, `.rs`, `.sh` and workflow file in
/// the workspace. Paid once per test that asks.
pub(crate) fn bench_checks() -> Vec<BenchCheck> {
    let prose = source_scan::workspace_markdown_docs()
        .into_iter()
        .chain(source_scan::workspace_shell_scripts())
        .chain(source_scan::workspace_ci_workflows())
        .map(|file| (file, TextKind::Prose));
    let rust = source_scan::workspace_rust_sources()
        .into_iter()
        .map(|file| (file, TextKind::RustComment));
    let mut out = Vec::new();
    for (file, kind) in prose.chain(rust) {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        out.extend(bench_checks_in(
            &text,
            kind,
            &source_scan::relative_to_repo(&file),
        ));
    }
    out
}

/// Every `cargo … --benches` in `text`, with `file` naming the
/// source for the message.
///
/// A Rust file is reduced to its comment prose first
/// ([`comment_text`], which keeps byte offsets and line breaks), so
/// the scan reads what the comments say and nothing else — this
/// module's own fixtures are string literals holding the very
/// command it forbids.
///
/// That reduction is a real comment reader rather than a
/// literal-blanking pass over raw source, and the difference is not
/// academic. Blanking literals in a file that still has its comments
/// pairs the `"` characters *in the comments* with the ones in the
/// code, so an unbalanced quote in prose — a typo, or a sentence
/// that opens a quotation and closes it with a curly `”` — opens a
/// literal that was never one and blanks live comment text until the
/// next `"`. A `--benches` inside that run disappears, silently, and
/// whether it does depends on nothing more than the parity of the
/// quotes above it. That is what
/// [`blank_string_literals`](crate::util::rust_source::blank_string_literals)'s
/// own documentation means by "pass comment-free text".
///
/// A line with no prose on it ends the block, in both kinds: a blank
/// line in Markdown, a line of code in Rust.
pub(crate) fn bench_checks_in(text: &str, kind: TextKind, file: &str) -> Vec<BenchCheck> {
    let prose;
    let text = match kind {
        TextKind::RustComment => {
            prose = comment_text(text);
            prose.as_str()
        }
        TextKind::Prose => text,
    };
    let mut out = Vec::new();
    let mut block: Vec<(String, usize)> = Vec::new();
    for (offset, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            out.extend(bench_checks_in_block(&block, file));
            block.clear();
        } else {
            block.extend(tokens(line).map(|token| (token, offset + 1)));
        }
    }
    out.extend(bench_checks_in_block(&block, file));
    out
}

/// `text` split into tokens, with the markup a command is wrapped in
/// — code ticks, emphasis, quotes, line continuations — and the
/// punctuation a sentence ends it with taken off.
fn tokens(text: &str) -> impl Iterator<Item = String> {
    text.replace(['`', '*', '"', '\'', '\\'], " ")
        .split_whitespace()
        .map(|token| {
            token
                .trim_start_matches(['(', '[', '{'])
                .trim_end_matches([')', ']', '}', '.', ',', ';', ':', '!', '?'])
                .to_string()
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .into_iter()
}

/// Every `cargo … --benches` in one block of tokens.
fn bench_checks_in_block(block: &[(String, usize)], file: &str) -> Vec<BenchCheck> {
    block
        .iter()
        .enumerate()
        .filter(|(_, (token, _))| token == "--benches")
        .filter_map(|(at, (_, line))| {
            Some(BenchCheck {
                site: format!("{file}:{line}"),
                invocation: invocation_around(block, at)?,
            })
        })
        .collect()
}

/// The invocation the `--benches` at `at` belongs to: from the
/// nearest `cargo` within [`CARGO_LOOKBACK`] through the run of flags
/// that follows, so a selection written on either side of the flag is
/// read.
///
/// `None` when the tokens in front of the flag are not an invocation
/// — a subcommand and then nothing but flags and their values. That
/// is the difference between a command a reader can run and a
/// sentence about one, and this module's own prose is full of the
/// second kind.
fn invocation_around(block: &[(String, usize)], at: usize) -> Option<String> {
    let start = block[..at].iter().rposition(|(token, _)| token == "cargo")?;
    if at - start > CARGO_LOOKBACK {
        return None;
    }
    let subcommand = &block.get(start + 1)?.0;
    if subcommand.starts_with('-')
        || !subcommand
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    for i in start + 2..at {
        let token = &block[i].0;
        let previous = &block[i - 1].0;
        let is_value = previous.starts_with('-') && !previous.contains('=');
        if !token.starts_with('-') && !is_value {
            return None;
        }
    }
    let mut end = at + 1;
    while end < block.len() && end - at <= TRAILING_FLAGS && block[end].0.starts_with('-') {
        // A flag written without `=` may take the next token as its
        // value, exactly as the walk in front of the flag reads one.
        // Naming `-p` and `--package` alone instead is what made
        // `--jobs 4 --workspace` a violation: the walk stopped at
        // the `4` and never reached the selection behind it.
        let takes_value = !block[end].0.contains('=');
        end += 1;
        if takes_value && end < block.len() && !block[end].0.starts_with('-') {
            end += 1;
        }
    }
    Some(
        block[start..end]
            .iter()
            .map(|(token, _)| token.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Every `--benches` written here that compiles no bench target,
/// rendered `"<file>:<line>: <what is wrong>"`.
///
/// `--workspace` (or its `--all` alias) passes because the workspace
/// owns a bench target; the caller asserts that separately, since a
/// workspace that owned none would make every one of these pass for
/// the wrong reason.
pub(crate) fn bench_selection_violations(
    checks: &[BenchCheck],
    targets: &BTreeMap<String, bool>,
) -> Vec<String> {
    checks
        .iter()
        .filter_map(|check| {
            let invocation = check.invocation.as_str();
            if cargo_arguments(invocation).any(|token| token == "--workspace" || token == "--all") {
                return None;
            }
            match package_selected(invocation, &check.site) {
                Some(name) if targets.get(&name) == Some(&true) => None,
                Some(name) => Some(format!(
                    "{}: `{invocation}` — `{name}` owns no bench target, so this compiles none",
                    check.site
                )),
                None => Some(format!(
                    "{}: `{invocation}` — no package selection, so cargo takes the root \
                     package alone, which owns no bench target",
                    check.site
                )),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a command the way [`documented_cargo_commands`] does,
    /// so a control exercises the same parse the repo test does
    /// rather than a hand-set struct that cannot disagree with it.
    fn parse(span: &str) -> DocCommand {
        let spans = cargo_spans(&format!("prose `{span}` prose"), "fixture.md");
        assert_eq!(spans.len(), 1, "the fixture must yield exactly one span");
        let command = spans.into_iter().next().unwrap_or_default();
        let package = package_selected(&command, "fixture.md");
        let wants_lib = cargo_arguments(&command).any(|token| token == "--lib");
        DocCommand {
            doc: "fixture.md".to_string(),
            command,
            package,
            wants_lib,
        }
    }

    fn workspace_fixture() -> BTreeMap<String, bool> {
        BTreeMap::from([("alib".to_string(), true), ("abin".to_string(), false)])
    }

    /// The same shape as [`workspace_fixture`] for the other
    /// question: one member owning a bench target, one owning none.
    fn bench_fixture() -> BTreeMap<String, bool> {
        BTreeMap::from([("hasbench".to_string(), true), ("nobench".to_string(), false)])
    }

    /// **No documented command asks for a library target of a
    /// package that has none** — #148's defect, which lived in two
    /// documents at once because the flag was copied off a neighbor
    /// that legitimately carries it.
    ///
    /// The fix for a failure is to drop `--lib`: on a binary crate
    /// it narrows nothing, it exits 101 before a single test runs.
    #[test]
    fn test_no_documented_command_targets_a_lib_the_package_lacks() {
        let commands = documented_cargo_commands();
        let targets = library_targets();
        assert_scan_is_looking(&commands, &targets);
        let violations = lib_flag_violations(&commands, &targets);
        assert!(
            violations.is_empty(),
            "{} documented command(s) ask cargo for a library target of a package \
             that has none. Drop `--lib` — on a binary crate it does not narrow the \
             run, it fails it:\n  {}",
            violations.len(),
            violations.join("\n  ")
        );
    }

    /// **No `--benches` written in this repository compiles zero
    /// bench targets** — #148's other defect, and the one that cost
    /// something. The bare form — `cargo check` with no `--workspace`
    /// — had been the project's stated proof that a bench target
    /// still builds, in `CLAUDE.md` §7, in `AGENTS.md` and in three
    /// more documents, and it exits 0 on a bench file that does not
    /// compile: the root package is the default selection, it
    /// declares `autobenches = false`, and it owns no bench target,
    /// so cargo checks nothing and says "Finished".
    ///
    /// Note what this test's own prose cannot do, and neither can any
    /// other prose here: write the bad command out to name it. There
    /// is no opt-out marker, deliberately — a quoted counter-example
    /// is indistinguishable from an instruction to the next person
    /// who copies a line out of a document.
    ///
    /// The fix for a failure is to name a selection that owns a bench
    /// — `--workspace` is the one this repository uses everywhere,
    /// `test.sh` included.
    #[test]
    fn test_no_bench_check_selects_a_package_that_owns_no_bench() {
        let checks = bench_checks();
        let targets = bench_targets();
        assert_bench_scan_is_looking(&checks, &targets);
        let violations = bench_selection_violations(&checks, &targets);
        assert!(
            violations.is_empty(),
            "{} `--benches` invocation(s) select no package that owns a bench target, so \
             they compile nothing and pass. Say `--workspace`:\n  {}",
            violations.len(),
            violations.join("\n  ")
        );
    }

    /// The preconditions the check above rests on. A repository that
    /// stopped writing the command anywhere, a member reader that
    /// answers the same way for everyone, a walk that no longer
    /// reaches the two documents stating the rule or the script
    /// running it — each is a way to report "nothing is wrong" while
    /// having stopped looking.
    fn assert_bench_scan_is_looking(checks: &[BenchCheck], targets: &BTreeMap<String, bool>) {
        for file in ["CLAUDE.md", "AGENTS.md", "test.sh"] {
            assert!(
                checks
                    .iter()
                    .any(|check| check.site.starts_with(&format!("{file}:"))),
                "{file} writes no `--benches` any more. It is one of the three places this \
                 rule lives — the two documents that state it and the script that runs it — \
                 so either the walk stopped reaching it or the gate itself is gone"
            );
        }
        assert!(
            checks.len() >= 6,
            "the walk found {} `--benches` site(s) across the whole repository; it published \
             more than that, so it is no longer reading what it claims to check",
            checks.len()
        );
        assert!(
            targets.values().any(|has_bench| *has_bench),
            "no workspace member owns a bench target, so `--workspace --benches` would \
             compile nothing either and every check here passes for the wrong reason: \
             {targets:?}"
        );
        assert!(
            targets.values().any(|has_bench| !*has_bench),
            "the member reader classified every one of {} member(s) the same way, so it \
             cannot tell a bench owner from a member that has none: {targets:?}",
            targets.len()
        );
    }

    /// **Every documented `-p <name>` names a workspace member.**
    /// The sibling failure to the one above and the reason
    /// [`lib_flag_violations`] stays quiet about an unknown package:
    /// a doc that outlived a crate rename is a different repair than
    /// a doc that asked for the wrong target.
    #[test]
    fn test_every_documented_package_selection_names_a_workspace_member() {
        let commands = documented_cargo_commands();
        let targets = library_targets();
        assert_scan_is_looking(&commands, &targets);
        let unknown = unknown_package_selections(&commands, &targets);
        assert!(
            unknown.is_empty(),
            "{} documented command(s) select a package this workspace does not have. \
             The members are {:?}:\n  {}",
            unknown.len(),
            targets.keys().collect::<Vec<_>>(),
            unknown.join("\n  ")
        );
    }

    /// The preconditions both repo tests rest on. Each one is a way
    /// the checks above could report "nothing is wrong" while having
    /// stopped looking — a section rewritten to quote no commands,
    /// a member reader that answers the same way for everyone, a
    /// `--lib` that no longer appears anywhere to be judged.
    fn assert_scan_is_looking(commands: &[DocCommand], targets: &BTreeMap<String, bool>) {
        for (doc, heading) in AUDITED_SECTIONS {
            let found = commands.iter().filter(|command| command.doc == doc).count();
            assert!(
                found >= 3,
                "{doc}'s {heading:?} yields {found} cargo command(s); it published \
                 several, so the scan is no longer reading what it claims to check"
            );
        }
        assert!(
            commands.iter().any(|command| command.wants_lib),
            "no documented command carries `--lib` any more, so the check that a \
             `--lib` names a library target has nothing left to judge"
        );
        assert!(
            targets.values().any(|has_lib| *has_lib) && targets.values().any(|has_lib| !has_lib),
            "the member reader classified every one of {} member(s) the same way, so \
             it cannot tell a library from a binary: {targets:?}",
            targets.len()
        );
    }

    /// Positive control: the judgment fires on a binary crate and
    /// stays quiet on a library one. Without this, a
    /// [`lib_flag_violations`] that returned an empty vector
    /// unconditionally would read exactly like a clean repository.
    #[test]
    fn test_a_lib_flag_on_a_binary_crate_is_reported() {
        let targets = workspace_fixture();
        let offending = vec![parse("cargo test -p abin --lib pattern")];
        assert_eq!(
            lib_flag_violations(&offending, &targets),
            vec!["fixture.md: `cargo test -p abin --lib pattern`"]
        );
        let fine = vec![
            parse("cargo test -p alib --lib pattern"),
            parse("cargo test -p abin pattern"),
        ];
        assert!(lib_flag_violations(&fine, &targets).is_empty());
    }

    /// The control that disables the mechanism on the exercised
    /// path: truncating at the bare `--` is what separates cargo's
    /// arguments from the invoked program's. Read the whole span and
    /// `cargo run -p abin -- --lib` becomes a false report against a
    /// crate that is behaving correctly.
    #[test]
    fn test_a_flag_after_the_argument_separator_is_not_cargos() {
        let after_separator = parse("cargo run -p abin -- --lib map.json");
        assert!(!after_separator.wants_lib);
        assert!(lib_flag_violations(&[after_separator], &workspace_fixture()).is_empty());
        assert!(parse("cargo test -p abin --lib").wants_lib);
    }

    /// A package selection is read from cargo's flags in either
    /// spelling, and only from them.
    #[test]
    fn test_package_selection_is_read_in_every_spelling_cargo_accepts() {
        assert_eq!(parse("cargo test -p alib").package.as_deref(), Some("alib"));
        assert_eq!(
            parse("cargo test --package alib").package.as_deref(),
            Some("alib")
        );
        assert_eq!(
            parse("cargo test --package=alib").package.as_deref(),
            Some("alib")
        );
        assert_eq!(parse("cargo test --workspace").package, None);
        assert_eq!(parse("cargo run -- -p alib").package, None);
    }

    /// Only `cargo` spans are collected, and a span that wraps
    /// across source lines — which the audited sections do write —
    /// is normalized to one line rather than dropped.
    #[test]
    fn test_the_span_reader_collects_cargo_invocations_and_nothing_else() {
        let text = "run `./test.sh --lint`, or `cargo test\n  -p alib --lib name`, not `--lib`";
        assert_eq!(
            cargo_spans(text, "fixture.md"),
            vec!["cargo test -p alib --lib name".to_string()]
        );
    }

    /// A backtick run with no matching run stops the scan. The rest
    /// of the section would otherwise read as one enormous span,
    /// turning a section full of commands into a section holding
    /// none — the silent shape this module refuses.
    #[test]
    #[should_panic(expected = "do not pair up")]
    fn test_an_unmatched_backtick_run_stops_the_scan() {
        cargo_spans("a `cargo test -p alib` and a stray ` tick", "fixture.md");
    }

    /// A command published in a fenced block is published, whatever
    /// the fence is spelled with. The tagged fence is the case that
    /// used to disappear: the info string was read as the first word
    /// of the command, and the `cargo` filter dropped what came out.
    /// The tilde fence is CommonMark's other spelling, unwritten
    /// here — and a reader that knew only the spelling in front of it
    /// would go quiet on the day somebody used the other one.
    #[test]
    fn test_a_fenced_block_publishes_commands_however_it_is_fenced() {
        let expected = vec!["cargo test -p abin --lib pattern".to_string()];
        for fence in ["```bash", "```", "~~~bash", "~~~", "````", "~~~~"] {
            let close = fence.trim_end_matches(|c: char| c.is_ascii_alphabetic());
            let text = format!("prose\n\n{fence}\ncargo test -p abin --lib pattern\n{close}\n\nmore");
            assert_eq!(
                cargo_spans(&text, "fixture.md"),
                expected,
                "a fence opened with {fence:?} hid the command inside it"
            );
        }
        // A tilde fence is closed by tildes, so a backtick run inside
        // one is content — content this reader must still read.
        let mixed = "~~~\n```\ncargo test -p abin --lib pattern\n~~~";
        assert_eq!(cargo_spans(mixed, "fixture.md"), expected);
    }

    /// Spans and fences are read in document order, and a fence body
    /// is read as code rather than as prose to scan for backticks —
    /// so a `#` comment or an unbalanced tick inside one cannot
    /// derail the reader.
    #[test]
    fn test_fenced_and_inline_commands_are_read_in_document_order() {
        let text = "`cargo test -p alib` then\n\n```sh\n# a lone ` tick in a comment\ncargo build -p abin\n```\n\nand `cargo doc -p alib`";
        assert_eq!(
            cargo_spans(text, "fixture.md"),
            vec![
                "cargo test -p alib".to_string(),
                "cargo build -p abin".to_string(),
                "cargo doc -p alib".to_string(),
            ]
        );
    }

    /// The two-backtick span Markdown offers for a command that
    /// itself contains a backtick is a span like any other. Read as
    /// a split on single ticks it yielded nothing at all.
    #[test]
    fn test_a_span_delimited_by_two_backticks_is_read() {
        assert_eq!(
            cargo_spans("write ``cargo test -p abin --lib `x` `` here", "fixture.md"),
            vec!["cargo test -p abin --lib `x`".to_string()]
        );
    }

    /// An unclosed fence stops the run: everything below it reads as
    /// code, so the reader can no longer tell a published command
    /// from the prose around it.
    #[test]
    #[should_panic(expected = "opened and never closed")]
    fn test_an_unclosed_fence_stops_the_scan() {
        cargo_spans("prose\n\n```bash\ncargo test -p alib\n\nmore prose", "fixture.md");
    }

    /// Positive control for the bench judgment: it fires on each way
    /// an invocation can select nothing that owns a bench, and stays
    /// quiet on each way one can select something that does. Without
    /// this, a `bench_selection_violations` that returned an empty
    /// vector unconditionally would read exactly like a clean
    /// repository — which is what the repository had instead of this
    /// check for the whole life of the rule.
    #[test]
    fn test_a_bench_check_that_compiles_no_bench_target_is_reported() {
        let targets = bench_fixture();
        let bare = bench_checks_in("run cargo check --benches here", TextKind::Prose, "f.md");
        assert_eq!(
            bench_selection_violations(&bare, &targets),
            vec![
                "f.md:1: `cargo check --benches` — no package selection, so cargo takes the \
                 root package alone, which owns no bench target"
                    .to_string()
            ]
        );
        let wrong_member = bench_checks_in("cargo check -p nobench --benches", TextKind::Prose, "f.md");
        assert_eq!(
            bench_selection_violations(&wrong_member, &targets),
            vec![
                "f.md:1: `cargo check -p nobench --benches` — `nobench` owns no bench target, \
                 so this compiles none"
                    .to_string()
            ]
        );
        // `--jobs 4` is a flag and its value, not the end of the
        // command: a walk that stopped at the `4` never reached the
        // `--workspace` behind it and reported a correct invocation.
        let jobs = bench_checks_in("cargo check --benches --jobs 4", TextKind::Prose, "f.md");
        assert_eq!(
            bench_selection_violations(&jobs, &targets).len(),
            1,
            "a flag's value does not make a selection: {jobs:?}"
        );
        for clean in [
            "cargo check --workspace --benches",
            "cargo check --benches --workspace",
            "cargo check -p hasbench --benches",
            "cargo check --benches --jobs 4 --workspace",
            "cargo check --benches --jobs 4 -p hasbench",
        ] {
            let checks = bench_checks_in(clean, TextKind::Prose, "f.md");
            assert_eq!(checks.len(), 1, "{clean} must yield one site");
            assert!(
                bench_selection_violations(&checks, &targets).is_empty(),
                "{clean} selects a bench owner"
            );
        }
    }

    /// A sentence about the flag is not a command, and neither is a
    /// sentence that happens to say "cargo" a few words earlier.
    /// Both shapes are in this module's own prose, and reading them
    /// as commands is how a checker of documented commands ends up
    /// forbidding documentation.
    #[test]
    fn test_a_benches_written_into_a_sentence_is_not_a_command() {
        for prose in [
            "the --benches flag",
            "cargo in one paragraph cannot be read as the head of a --benches",
            "run cargo check, then talk about --benches",
        ] {
            assert!(
                bench_checks_in(prose, TextKind::Prose, "f.md").is_empty(),
                "{prose:?} is prose, not an invocation"
            );
        }
    }

    /// The markup a command is written in does not hide it: code
    /// ticks, a bold run, a sentence's comma, a wrap across lines,
    /// and the `//!` of a Rust doc comment all come off. Every one of
    /// those shapes is in this repository today — `AGENTS.md` and
    /// `bench_surface.rs` both wrap the invocation mid-command.
    #[test]
    fn test_the_bench_scan_reads_through_markup_and_line_wraps() {
        let wrapped_doc = "prose `cargo check\n  --workspace --benches`, which runs nothing";
        let checks = bench_checks_in(wrapped_doc, TextKind::Prose, "f.md");
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].invocation, "cargo check --workspace --benches");
        assert_eq!(checks[0].site, "f.md:2", "the site is where the flag is");
        let wrapped_comment = "//! **`cargo check\n//! --workspace --benches`** proves it.\nfn f() {}";
        assert_eq!(
            bench_checks_in(wrapped_comment, TextKind::RustComment, "f.rs")[0].invocation,
            "cargo check --workspace --benches"
        );
    }

    /// What the scan reads in Rust, pinned so the silence is not read
    /// as coverage: comments, and not the code around them. A string
    /// literal is not a claim about how to build this repository —
    /// and the fixtures in this very module are literals holding the
    /// command it forbids, so a reader that took them at face value
    /// would fail on its own test data.
    #[test]
    fn test_the_bench_scan_reads_rust_comments_and_not_rust_code() {
        for code in [
            "let command = \"cargo check --benches\";",
            "assert_eq!(read(\"// cargo check --benches\"), 1);",
            "let s = r#\"// cargo check --benches\"#;",
        ] {
            assert!(
                bench_checks_in(code, TextKind::RustComment, "f.rs").is_empty(),
                "{code:?} is code, not a comment"
            );
        }
        for comment in [
            "// cargo check --benches",
            "/// cargo check --benches",
            "//! cargo check --benches",
            "/* cargo check --benches */",
            "let x = 1; // cargo check --benches",
            "/*\n * cargo check --benches\n */",
        ] {
            assert_eq!(
                bench_checks_in(comment, TextKind::RustComment, "f.rs").len(),
                1,
                "{comment:?} is a comment holding a command"
            );
        }
    }

    /// A `"` in comment prose does not hide the comment underneath
    /// it. This is the reader's whole reason for existing: blanking
    /// string literals in source that still carries its comments
    /// pairs a typo's quote with the next `"` anywhere below, blanks
    /// every byte between them, and takes a real invocation with it —
    /// silently, and decided by nothing but quote parity.
    ///
    /// The fixture is the one a reviewer planted in
    /// `bench_surface.rs`: three ordinary comment lines, one of them
    /// carrying an unbalanced quote, with the bare command below.
    /// Under the literal-blanking reader the scan found nothing;
    /// closing that one quote made it find the command again.
    #[test]
    fn test_an_unbalanced_quote_in_a_comment_hides_nothing_below_it() {
        let planted = "// The gate prints \"Finished and exits 0.\n\
                       // Prove the bench target still builds with:\n\
                       // cargo check --benches\n";
        let checks = bench_checks_in(planted, TextKind::RustComment, "f.rs");
        assert_eq!(checks.len(), 1, "the invocation is on line 3: {checks:?}");
        assert_eq!(checks[0].site, "f.rs:3");
        assert_eq!(checks[0].invocation, "cargo check --benches");
        // The same text with the quote closed must read identically:
        // a reader whose answer depends on quote parity is the defect.
        let balanced = planted.replace("\"Finished", "\"Finished\"");
        assert_eq!(
            bench_checks_in(&balanced, TextKind::RustComment, "f.rs"),
            checks,
            "the quote's parity changed the reading"
        );
    }

    /// A block ends at a blank line in prose and at any non-comment
    /// line in Rust, so a `cargo` in one paragraph cannot be read as
    /// the head of a `--benches` in the next.
    #[test]
    fn test_a_block_boundary_keeps_two_commands_apart() {
        let split = "cargo check --workspace\n\nand then --benches";
        assert!(
            bench_checks_in(split, TextKind::Prose, "f.md").is_empty(),
            "the `cargo` is in the block above"
        );
        let across_code = "// cargo check --workspace\nfn f() {}\n// --benches";
        assert!(bench_checks_in(across_code, TextKind::RustComment, "f.rs").is_empty());
    }

    /// The manifests answer both ways for the real workspace, named
    /// member by member: the reader that says "owns a bench" for
    /// everything and the reader that says it for nothing both pass
    /// every other assertion here.
    #[test]
    fn test_the_member_reader_tells_this_workspace_bench_owners_from_the_rest() {
        let targets = bench_targets();
        assert_eq!(
            targets.get("baumhard"),
            Some(&true),
            "baumhard declares `[[bench]]`"
        );
        assert_eq!(
            targets.get("mandala"),
            Some(&false),
            "the root package sets `autobenches = false` and owns no bench — the whole \
             reason the bare form is a no-op"
        );
        assert_eq!(targets.get("maptool"), Some(&false), "maptool has no benches");
        assert_eq!(
            targets.get("mandala_derive"),
            Some(&false),
            "mandala_derive has no benches"
        );
    }

    /// The manifests answer both ways for the real workspace, named
    /// member by member: the reader that says "library" for
    /// everything and the reader that says "binary" for everything
    /// both pass every other assertion here.
    #[test]
    fn test_the_member_reader_tells_this_workspace_libraries_from_binaries() {
        let targets = library_targets();
        assert_eq!(targets.get("baumhard"), Some(&true), "baumhard is a library");
        assert_eq!(
            targets.get("mandala_derive"),
            Some(&true),
            "mandala_derive declares `[lib]`"
        );
        assert_eq!(targets.get("mandala"), Some(&false), "mandala is a binary crate");
        assert_eq!(targets.get("maptool"), Some(&false), "maptool is a binary crate");
    }
}
