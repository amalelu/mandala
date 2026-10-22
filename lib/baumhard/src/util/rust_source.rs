// SPDX-License-Identifier: MPL-2.0

//! Read this workspace's own Rust source for tests that pin a
//! *source-level* property — one no runtime assertion can see.
//!
//! Two properties are like that in practice, and both are load-bearing:
//! a statement whose only observable effect goes to a process-global
//! logger the suite does not install, and a branch that only compiles
//! on `wasm32` and so is never linked into `cargo test`
//! (`TEST_CONVENTIONS.md` §T9). A test for either has to read the
//! file.
//!
//! ## Why a module instead of `read_to_string` at the call site
//!
//! A naive `source.contains(needle)` is satisfied by **any** byte run
//! in the file, and the two cheapest places for one to hide are the
//! two a person deleting code actually writes:
//!
//! - **A comment.** Gutting a function and leaving
//!   `// this used to be log::error!("startup: {}", message);` keeps
//!   the scan green while the behavior is gone. That is not a
//!   contrived mutation — it is the ordinary shape of a removal.
//! - **The test's own body, or a later test module.** The needle a
//!   test asserts on is spelled out inside that test, so a whole-file
//!   scan of the file the test lives in matches itself. Appending a
//!   test module to a *different* file does the same to a scan of
//!   that file. This is the one a reviewer got past: the recognizer
//!   wanted `mod` as the immediate next token, so this workspace's
//!   own `#[cfg(test)] #[cfg(not(target_arch = "wasm32"))] mod tests
//!   {` idiom — seven live occurrences — was handed back as
//!   production code, and a two-line shim underneath it revived a
//!   mutation four pins had killed. What the recognizer accepts is
//!   spelled out on
//!   [`above_test_modules`](crate::util::rust_source::above_test_modules).
//!
//!   **A tree-wide sweep is not enough to hold that down, and the
//!   reason is worth stating once.** The sweep in
//!   `util::tests::rust_source_tests` reads every `.rs` file in the
//!   workspace — the count is deliberately not written down here,
//!   having gone stale twice while looking maintained — and agrees
//!   with an independently written recognizer on every cut. It
//!   still cannot see a hole, because a shim author writes a *new*
//!   shape rather than reusing one the tree already contains. Three
//!   were found that way after the sweep went green: an attribute run
//!   (`#[cfg(test)] #[allow(dead_code)]`), a raw identifier
//!   (`mod r#tests_shim {`), and one added space (`#[ cfg(test) ]`).
//!   So the recognizer is written against Rust's grammar — whitespace
//!   between every token, identifiers rather than ASCII words — and
//!   the spellings this tree does *not* write are enumerated
//!   separately, in `LEGAL_UNWRITTEN_HEADERS`, each one compiled
//!   before being listed.
//!
//! [`production_code`](crate::util::rust_source::production_code)
//! closes both: it replaces every comment with a space
//! ([`strip_comments`](crate::util::rust_source::strip_comments)) and
//! drops everything from the first test module onward
//! ([`above_test_modules`](crate::util::rust_source::above_test_modules)),
//! so what comes back is the code that ships.
//! [`braced_block_after`](crate::util::rust_source::braced_block_after)
//! narrows further, to one item's body, for the pins that are about
//! one statement.
//!
//! ## The other direction: reading the prose
//!
//! One pin wants the opposite half of the same file.
//! `crate::util::doc_commands` — unlinked here, being a test-only
//! module `cargo doc` never sees — checks that no build command this
//! repository *writes down* is a no-op, and a command written into a
//! `//` comment counts;
//! [`comment_text`](crate::util::rust_source::comment_text) is
//! [`strip_comments`](crate::util::rust_source::strip_comments)
//! inverted for it — the comments come back and the code, the
//! literals and the markers go. Sharing the walk is the point rather
//! than a convenience: a reader that decided where a comment ends by
//! pairing quotes across the file made the visibility of one comment
//! depend on the punctuation of the comments above it.
//!
//! ## What is still out of reach
//!
//! A **string literal** in production code holding the needle
//! verbatim satisfies the scan, because
//! [`strip_comments`](crate::util::rust_source::strip_comments)
//! leaves literals alone — it has to, since the needles here contain
//! string literals themselves
//! (`log::error!("startup: {}", message)`). That is the residual,
//! and it is a deliberate act rather than the ordinary shape of a
//! mistake: scoping a pin to one function body with
//! [`braced_block_after`](crate::util::rust_source::braced_block_after)
//! means hiding a needle requires planting a raw string inside the
//! very function under test.
//!
//! Native-only in effect — it reads files off the filesystem, and
//! nothing in a shipped build parses its own source. It is not
//! `#[cfg(test)]`-gated for the same reason
//! [`crate::util::doc_fixtures`] is not: the callers live in the
//! `mandala` crate, and a `cfg(test)` item in `baumhard` is invisible
//! to them.
//!
//! No criterion entry for
//! [`production_code`](crate::util::rust_source::production_code), on
//! the same precedent as [`crate::util::doc_fixtures`]: it is file
//! I/O on a cold test path, and a benchmark of it would measure the
//! page cache — and none for the tree-wide sweep in
//! `util::tests::rust_source_tests`, which is several hundred of
//! them. The five pure scanners do carry bench entries
//! (`lib/baumhard/CONVENTIONS.md` §B7):
//! [`strip_comments`](crate::util::rust_source::strip_comments),
//! [`above_test_modules`](crate::util::rust_source::above_test_modules),
//! [`braced_block_after`](crate::util::rust_source::braced_block_after),
//! [`string_literals`](crate::util::rust_source::string_literals)
//! and
//! [`statements`](crate::util::rust_source::statements).

use crate::util::doc_fixtures::repo_path;

/// The word [`above_test_modules`] scans for. Every `cfg` in the
/// file is a candidate; `attribute_opening_before` sorts the ones
/// that are attributes from the ones that are not, and `implies_test`
/// decides which of those gate on `test`.
///
/// **The word alone, not `cfg(`.** Rust's attribute grammar is a
/// token grammar, and `#[cfg (test)]` compiles exactly as
/// `#[cfg(test)]` does — `rustc` was asked. Matching the parenthesis
/// as part of the needle made a single space enough to hide a whole
/// module from this reader. The `(` is still required, found by
/// [`skip_whitespace_forward`] past the word, and that is also what
/// keeps `cfg_attr` out: the character after the word is `_`,
/// neither whitespace nor a parenthesis.
///
/// Spelled as a `concat!` so this file's own text does not contain
/// the literal it searches for:
/// `production_code("lib/baumhard/src/util/rust_source.rs")` would
/// otherwise stop to parse its own constant.
///
/// Searched **after** comments are stripped, so the many prose
/// mentions of the attribute in files like this one do not truncate
/// their own module at the first paragraph that names it. An
/// occurrence inside a *string literal* still truncates early, which
/// is the safe direction — a pin then scans less code and fails
/// rather than passing on text it should not have seen.
const CFG_WORD: &str = concat!("cf", "g");

/// What a `cfg` attribute gates: the item written after it, or
/// everything inside the module it is written in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AttributeScope {
    /// `#[cfg(…)]` — the next item.
    Item,
    /// `#![cfg(…)]` — the rest of the enclosing module. At the top
    /// of a file that is the whole file.
    Enclosing,
}

/// Read `<repo>/<relative>` and return the part of it that ships:
/// [`strip_comments`], then [`above_test_modules`].
///
/// This is the reader every source-level pin in the workspace should
/// use. The two transforms are not tidiness — see the module docs
/// for the mutations each one kills. They run in that order because
/// prose mentions `#[cfg(test)]` far more often than code declares a
/// module with it.
///
/// Line breaks survive stripping, so a line number quoted in a
/// failure message still points at the right line of the original
/// file.
///
/// Panics — never degrades — when the file cannot be read, naming
/// the path: the caller is a test whose entire purpose is to notice
/// that the file moved.
///
/// Cost: one file read plus one pass over its bytes, paid once per
/// test that pins a source property.
pub fn production_code(relative: &str) -> String {
    let path = repo_path(relative);
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
    above_test_modules(&strip_comments(&source)).to_string()
}

/// `code` up to its first test module, or all of it when there is
/// none.
///
/// A test module here is either an inline `mod name { … }` carrying a
/// `cfg` attribute that implies `test`, or — via `#![cfg(test)]` —
/// the file itself. Every part of that sentence is a hole this
/// function has fallen into, so each is spelled out.
///
/// ## Which `cfg`s count
///
/// `implies_test`, not a match on the literal `#[cfg(test)]`. This
/// workspace writes `#[cfg(all(test, not(target_arch = "wasm32")))]`
/// on six live items — `util::log`'s test module among them — and a
/// literal match sees none of them.
///
/// ## What may sit between the `cfg` and the `mod`
///
/// **Any run of further attributes.** `#[cfg(test)]
/// #[cfg(not(target_arch = "wasm32"))] mod tests {` is the house
/// idiom for a test module that cannot build for the browser, with
/// seven live occurrences, all under `src/application/app/` — the
/// subtree the startup pins read. While `mod` had to be the
/// *immediate* next token, all seven were reported as production
/// code, and appending
///
/// ```text
/// #[cfg(test)]
/// #[allow(dead_code)]
/// mod tests_shim { const SHIM: &str = "…the needle…"; }
/// ```
///
/// to any scanned file satisfied every pin built on this reader while
/// the code it pinned was gutted. One attribute line was the whole
/// difference between that and a red suite.
///
/// ## Why the `{` is load-bearing
///
/// Only an inline module hides text inside *this* file, and it is the
/// only shape that truncates. The two near-misses each cost a real
/// pin before they were handled:
///
/// - **`#[cfg(test)] use …`, or any smaller item.**
///   `src/application/renderer/mod.rs` opens with a test-only import
///   five hundred lines above the code a pin cares about.
/// - **`#[cfg(test)] mod tests_nodes;` — an *external* module.**
///   `src/application/document/mod.rs` declares fourteen of them
///   before its first real item. None of their text is in this file,
///   so there is nothing here to exclude.
///
/// Truncating at either returns a shorter string — in both those
/// cases an empty one — and an empty string satisfies every negative
/// assertion and no positive one: a scan that silently cannot fail is
/// worse than no scan.
///
/// So the anchor is `mod name {`, with the brace. A visibility in
/// front of it is accepted, since a benchmark-reusable test tree is
/// declared `pub mod tests;` (`lib/baumhard/CONVENTIONS.md` §B8).
///
/// ## The one thing that does truncate to (almost) nothing
///
/// `#![cfg(test)]` — the *inner* form, written at the top of a file
/// that is nothing but tests. **Three** files open with it, not the
/// two this paragraph used to list:
///
/// - `src/application/macros/tests.rs` and
///   `src/application/app/dispatch/cross_dispatch/selection/tests.rs`,
///   each declared by a bare `mod tests;` with no attribute on it, so
///   the inner attribute is the only gate there is;
/// - `crates/maptool/src/verify/test_helpers.rs`, which is declared
///   `#[cfg(test)] mod test_helpers;` and so is gated twice. It is
///   listed anyway: this function is handed the *file*, never its
///   declaration, so the outer gate is invisible from here and the
///   inner one is the only thing that can cut.
///
/// Cutting there really does leave nothing, and that is the honest
/// answer rather than the dangerous one: the file ships no code, so
/// reporting its test bodies as production code is the same hole the
/// rest of this function closes. It does mean a *negative* assertion
/// over such a file passes vacuously, so a caller that scans a file
/// it did not choose keeps the non-vacuity guard
/// `test_the_placard_is_not_announced_as_a_loaded_map` uses — assert
/// something you expect to find, first.
///
/// **What this deliberately does not remove:** a test `cfg` on a
/// smaller item. Those are code that exists rather than prose that
/// does not, so a needle inside one is a needle in a real function,
/// and the narrower pins built on [`braced_block_after`] do not reach
/// them anyway.
///
/// `code` should already be comment-free; a mention of the attribute
/// in prose would otherwise truncate a file at its own module docs.
///
/// Cost: one substring search per `cfg` in the file, plus a
/// balanced-paren walk of each predicate and a few bytes of lookahead
/// past the ones that imply `test`.
pub fn above_test_modules(code: &str) -> &str {
    let mut from = 0usize;
    while let Some(offset) = code[from..].find(CFG_WORD) {
        let call = from + offset;
        from = call + CFG_WORD.len();
        // The predicate's `(` may be a space away from the word, and
        // `cfg_attr` fails here rather than being special-cased.
        let open = skip_whitespace_forward(code, from);
        if code.as_bytes().get(open) != Some(&b'(') {
            continue;
        }
        let Some((at, scope)) = attribute_opening_before(code, call) else {
            continue;
        };
        let Some(close) = matching_delimiter(code, open, b'(', b')') else {
            continue;
        };
        if !implies_test(&code[open + 1..close]) {
            continue;
        }
        let Some(after) = code[close + 1..].trim_start().strip_prefix(']') else {
            continue;
        };
        match scope {
            // Everything from here down is inside the gated module.
            AttributeScope::Enclosing => return &code[..at],
            AttributeScope::Item if opens_an_inline_module(after) => return &code[..at],
            AttributeScope::Item => {}
        }
    }
    code
}

/// Where the attribute wrapping the `cfg` word at `call` begins, and
/// what it gates — or `None` when that `cfg` is not an attribute at
/// all.
///
/// The distinction is what keeps a `cfg!(test)` macro, a local
/// binding named `cfg`, or the word inside a string literal from
/// truncating a file: none of them is preceded by `#[`.
///
/// Read backwards, one token at a time, skipping whitespace between
/// every pair — `[`, then an optional `!`, then `#`. There are still
/// exactly two scopes, [`AttributeScope::Item`] for `#[` and
/// [`AttributeScope::Enclosing`] for `#![`; what changed is that they
/// are recognized token by token rather than as two fixed strings.
/// Rust puts no constraint on the whitespace between those tokens, so
/// neither does this. `#[` and `#![` are what the workspace writes,
/// but `#[ ` and `# ! [` compile identically and an earlier
/// `ends_with("#[")` saw neither — a shim under `#[ cfg(test) ]` was
/// handed back as production code, reviving a killed mutation with
/// one added space.
///
/// Cost: a bounded backwards walk over whitespace and three
/// punctuation bytes.
fn attribute_opening_before(code: &str, call: usize) -> Option<(usize, AttributeScope)> {
    let bracket = strip_char_back(code, skip_whitespace_back(code, call), '[')?;
    let before_bracket = skip_whitespace_back(code, bracket);
    let (hash_end, scope) = match strip_char_back(code, before_bracket, '!') {
        Some(bang) => (skip_whitespace_back(code, bang), AttributeScope::Enclosing),
        None => (before_bracket, AttributeScope::Item),
    };
    Some((strip_char_back(code, hash_end, '#')?, scope))
}

/// The index of the first non-whitespace byte at or after `from`.
fn skip_whitespace_forward(code: &str, from: usize) -> usize {
    code.len() - code[from..].trim_start().len()
}

/// The index just past the last non-whitespace byte before `end`.
fn skip_whitespace_back(code: &str, end: usize) -> usize {
    code[..end].trim_end().len()
}

/// The index of `wanted` when `code[..end]` ends with it, else `None`.
fn strip_char_back(code: &str, end: usize, wanted: char) -> Option<usize> {
    code[..end].strip_suffix(wanted).map(str::len)
}

/// Whether a `cfg` predicate — the text between the parentheses of
/// `#[cfg(…)]` — is true only when `test` is set.
///
/// Written as the implication rather than as a list of spellings,
/// because the list is what drifted: `#[cfg(test)]` was matched
/// literally and `#[cfg(all(test, not(target_arch = "wasm32")))]`,
/// which this workspace also writes, was not.
///
/// - `test` implies itself;
/// - `all(…)` implies `test` when **any** operand does — every
///   operand has to hold, so one of them being `test` is enough;
/// - `any(…)` implies `test` only when **every** operand does, since
///   the predicate holds as soon as one of them does. An empty
///   `any()` is `false`, which implies everything, but treating it as
///   a test gate would cut a file at a predicate that disables code
///   on all targets — so it is excluded;
/// - everything else, `not(…)` included, does not.
///
/// Erring in either direction costs something, which is why this is
/// an implication check and not a heuristic: saying "yes" too often
/// truncates a file early and makes a negative assertion pass
/// vacuously, and saying "no" too often hands a test module to a
/// caller as shipped code.
fn implies_test(predicate: &str) -> bool {
    let predicate = predicate.trim();
    if predicate == "test" {
        return true;
    }
    if let Some(operands) = combinator_operands(predicate, "all") {
        return operands.iter().any(|operand| implies_test(operand));
    }
    if let Some(operands) = combinator_operands(predicate, "any") {
        return !operands.is_empty() && operands.iter().all(|operand| implies_test(operand));
    }
    false
}

/// The comma-separated operands of `name(…)` when `predicate` is
/// exactly that call, else `None`. A trailing comma contributes no
/// operand.
fn combinator_operands<'a>(predicate: &'a str, name: &str) -> Option<Vec<&'a str>> {
    let inner = predicate.strip_prefix(name)?.trim_start();
    let close = matching_delimiter(inner, 0, b'(', b')')?;
    // `all(x) && y` is not a bare combinator; only the whole
    // predicate being the call may be split.
    if !inner[close + 1..].trim().is_empty() {
        return None;
    }
    Some(
        split_top_level(&inner[1..close])
            .into_iter()
            .filter(|operand| !operand.is_empty())
            .collect(),
    )
}

/// `list` split on the commas that are not inside a nested
/// parenthesis or a literal, each piece trimmed.
fn split_top_level(list: &str) -> Vec<&str> {
    let bytes = list.as_bytes();
    let mut pieces = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < list.len() {
        if let Some(next) = raw_string_end(list, i) {
            i = next;
            continue;
        }
        match bytes[i] {
            b'"' => {
                i = quoted_end(list, i, '"');
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                pieces.push(list[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        let ch = list[i..].chars().next().expect("in-bounds by the loop guard");
        i += ch.len_utf8();
    }
    pieces.push(list[start..].trim());
    pieces
}

/// Whether `after` — the text following a test-gating `cfg`
/// attribute — opens an *inline* module: any run of further
/// attributes, an optional visibility, `mod`, a name, then `{` rather
/// than `;`.
///
/// Whitespace is skipped between every pair of tokens, and the name
/// is a Rust identifier rather than an ASCII word — `r#` prefix
/// included. Both are the same lesson from the same direction: the
/// shapes worth handling are the ones somebody can *write*, not the
/// ones the tree happens to contain today. `mod r#tests_shim {`
/// stopped the old name scan at the `#`, left `"#tests_shim {…"`,
/// failed `starts_with('{')`, and handed the shim back as production
/// code — a killed mutation revived by two characters.
fn opens_an_inline_module(after: &str) -> bool {
    let mut rest = after.trim_start();
    // The attribute run. `#[cfg(test)] #[cfg(not(target_arch =
    // "wasm32"))] mod tests {` is the workspace's own idiom for a
    // test module the browser build cannot compile, and requiring
    // `mod` immediately is what let a shim under one of those revive
    // a killed mutation.
    while let Some(hashed) = rest.strip_prefix('#') {
        // `matching_delimiter` is told where the `[` is, so it is
        // handed a slice that begins with one.
        let from_bracket = hashed.trim_start();
        if !from_bracket.starts_with('[') {
            return false;
        }
        let Some(close) = matching_delimiter(from_bracket, 0, b'[', b']') else {
            return false;
        };
        rest = from_bracket[close + 1..].trim_start();
    }
    let rest = match strip_word(rest, "pub") {
        // `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`.
        Some(tail) => match tail.strip_prefix('(') {
            Some(paren) => match paren.find(')') {
                Some(close) => paren[close + 1..].trim_start(),
                None => return false,
            },
            None => tail,
        },
        None => rest,
    };
    let Some(named) = strip_word(rest, "mod") else {
        return false;
    };
    // Past `mod` and its name, an inline module opens a block and an
    // external one ends the statement.
    let named = named.strip_prefix("r#").unwrap_or(named);
    let end = named
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(named.len());
    named[end..].trim_start().starts_with('{')
}

/// `rest` past a leading `word` and the whitespace after it, or
/// `None` when `rest` does not begin with that whole word.
///
/// The whole word: `pubmod` and `modules` each start with one of the
/// two keywords this is asked about, and neither is that keyword.
/// Neither is valid Rust either, which is the point rather than an
/// objection — a recognizer that accepts more than the grammar does
/// cuts where the compiler sees no module, and everything below the
/// cut stops being production code as far as every pin is concerned.
/// [`opens_an_inline_module`]'s tests carry both spellings.
fn strip_word<'a>(rest: &'a str, word: &str) -> Option<&'a str> {
    let tail = rest.strip_prefix(word)?;
    let boundary = tail
        .chars()
        .next()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
    boundary.then(|| tail.trim_start())
}

/// `code` split into statements: the pieces between the `;`, `{` and
/// `}` that sit outside any `(…)` or `[…]`, each trimmed, empties
/// dropped.
///
/// The unit a caller wants when the question is "does *this* call
/// carry an `.expect(`" — and getting it wrong is not cosmetic. A
/// split on `;` alone merges every expression-bodied `fn` in a file
/// into one chunk with its neighbors, because such a function's body
/// ends in `}` and not `;`. `renderer/pipeline.rs` has two of them
/// side by side, so
///
/// ```text
/// .await.unwrap()          // in get_device
/// .await.expect("…")       // in get_adapter
/// ```
///
/// arrived as a single "statement" that contained an `.expect(`, and
/// a pin asking whether every call of `request_device` says why it
/// died answered yes about `request_adapter`'s message.
///
/// Braces *inside* parentheses are not boundaries, which is what
/// keeps a struct literal argument —
/// `request_device(&DeviceDescriptor { label: None, … })` — attached
/// to the `.expect(` that follows it. String, byte-string,
/// raw-string and character literals are skipped, so a `;` or `}` in
/// a message splits nothing.
///
/// Pass comment-free text: [`production_code`] or [`strip_comments`].
///
/// Cost: one pass over `code`, and one `Vec` of borrowed slices.
pub fn statements(code: &str) -> Vec<&str> {
    let bytes = code.as_bytes();
    let mut pieces = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < code.len() {
        if let Some(next) = raw_string_end(code, i) {
            i = next;
            continue;
        }
        match bytes[i] {
            b'"' => {
                i = quoted_end(code, i, '"');
                continue;
            }
            b'\'' => {
                if let Some(next) = char_literal_end(code, i) {
                    i = next;
                    continue;
                }
            }
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b';' | b'{' | b'}' if depth == 0 => {
                let piece = code[start..i].trim();
                if !piece.is_empty() {
                    pieces.push(piece);
                }
                start = i + 1;
            }
            _ => {}
        }
        let ch = code[i..].chars().next().expect("in-bounds by the loop guard");
        i += ch.len_utf8();
    }
    let tail = code[start..].trim();
    if !tail.is_empty() {
        pieces.push(tail);
    }
    pieces
}

/// Replace every Rust comment in `src` with a single space, leaving
/// string, byte-string, raw-string and character literals exactly as
/// they are.
///
/// Literals are preserved on purpose: the things worth pinning in
/// this workspace *are* string literals — a log line's `"<area>: "`
/// prefix, a `startup_load::adopt(` call. A stripper that blanked
/// them would leave nothing to assert on.
///
/// What it understands, because Rust source here uses all of it:
///
/// - `//` line comments, including `///` and `//!` doc comments,
///   which are comments for this purpose — a doc comment is the
///   *most* likely place for a stale needle to survive;
/// - `/* */` block comments, **nested**, which Rust allows;
/// - `"..."` and `b"..."` with backslash escapes;
/// - `r"..."`, `r#"..."#`, `br##"..."##` at any hash count, where a
///   backslash escapes nothing and only the matching `"#…#` ends it;
/// - `'x'` character literals, distinguished from a `'a` lifetime by
///   looking for the closing quote rather than by guessing.
///
/// Newlines inside a comment are emitted, so the output has the same
/// line count as the input and a byte offset still lands on the same
/// line.
///
/// Cost: one `char_indices` pass over `src` and one `String` of at
/// most its length. Never slices at a non-boundary — every cut comes
/// from `char_indices`, so a comment full of emoji is as safe as one
/// full of ASCII.
pub fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0usize;
    while i < src.len() {
        let rest = &src[i..];
        if rest.starts_with("//") {
            // Line comment: consume to the newline, which is kept.
            let end = rest.find('\n').map_or(src.len(), |n| i + n);
            out.push(' ');
            i = end;
            continue;
        }
        if rest.starts_with("/*") {
            i = skip_block_comment(src, i, &mut out);
            continue;
        }
        if let Some(next) = raw_string_end(src, i) {
            out.push_str(&src[i..next]);
            i = next;
            continue;
        }
        if bytes[i] == b'"' {
            let next = quoted_end(src, i, '"');
            out.push_str(&src[i..next]);
            i = next;
            continue;
        }
        if bytes[i] == b'\'' {
            if let Some(next) = char_literal_end(src, i) {
                out.push_str(&src[i..next]);
                i = next;
                continue;
            }
            // A lifetime (`'a`, `'static`). Ordinary code.
        }
        let ch = src[i..].chars().next().expect("in-bounds by the loop guard");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// `src` with everything that is **not** comment prose replaced by
/// spaces: the code, the string literals, and the `//` / `/*`
/// markers themselves. What a person wrote *inside* a comment stays
/// exactly where it was.
///
/// The inverse of [`strip_comments`], which keeps the code and drops
/// the prose; this keeps the prose and drops the code. Same walk, so
/// the two agree on where a comment starts and ends — including the
/// cases that make a hand-rolled reader wrong:
///
/// - a `//` or a `/*` inside a string literal opens no comment
///   (`let u = "http://x";`);
/// - a `"` inside a comment opens no literal, which is the whole
///   reason this exists rather than a
///   [`blank_string_literals`] call on raw source: pairing quotes
///   file-wide makes the visibility of a comment depend on how many
///   `"` characters the comments above it happen to contain;
/// - a block comment is nested and multi-line, and its body is prose
///   even where that prose contains `//`, `"` or `*/`-shaped text;
/// - a `'a` lifetime is not a character literal.
///
/// Every dropped byte becomes one space and every newline is kept,
/// so the result has the same **byte length** and the same line
/// numbering as `src` — a caller that reports `file:line` reports
/// the line the comment is really on.
///
/// The markers go because a caller reading the prose as text should
/// not have to strip them: `/// cargo check` yields
/// `    cargo check`, not `/// cargo check`, so a leading `///`
/// cannot be mistaken for a token of what the comment says.
///
/// Cost: one pass over `src` and one `String` of its length.
pub fn comment_text(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < src.len() {
        let rest = &src[i..];
        if rest.starts_with("//") {
            let end = rest.find('\n').map_or(src.len(), |n| i + n);
            let prose = line_comment_prose(src, i, end);
            blank_into(&src[i..prose], &mut out);
            out.push_str(&src[prose..end]);
            i = end;
            continue;
        }
        if rest.starts_with("/*") {
            i = copy_block_comment(src, i, &mut out);
            continue;
        }
        let next = if let Some(next) = raw_string_end(src, i) {
            next
        } else if bytes[i] == b'"' {
            quoted_end(src, i, '"')
        } else if bytes[i] == b'\'' {
            char_literal_end(src, i).unwrap_or_else(|| next_char_end(src, i))
        } else {
            next_char_end(src, i)
        };
        blank_into(&src[i..next], &mut out);
        i = next;
    }
    out
}

/// Byte offset just past the one character at `from`.
fn next_char_end(src: &str, from: usize) -> usize {
    let ch = src[from..].chars().next().expect("in-bounds by the caller");
    from + ch.len_utf8()
}

/// Where the prose of the line comment spanning `from..end` starts:
/// past the run of `/` that opens it and past the `!` of a `//!`
/// inner doc comment.
///
/// A `////////` rule line is all marker and no prose, which yields
/// `end` — an empty comment rather than a comment full of slashes.
fn line_comment_prose(src: &str, from: usize, end: usize) -> usize {
    let bytes = src.as_bytes();
    let mut i = from;
    while i < end && bytes[i] == b'/' {
        i += 1;
    }
    if i < end && bytes[i] == b'!' {
        i += 1;
    }
    i
}

/// Copy the body of the block comment starting at `from` into `out`,
/// blanking its `/*` and `*/` markers, and return the byte offset
/// just past it.
///
/// Nesting is Rust's, so the inner markers of `/* a /* b */ c */` go
/// too and `c` is still prose. An unterminated comment runs to the
/// end of `src`, which is the posture [`skip_block_comment`] takes
/// for the same input.
fn copy_block_comment(src: &str, from: usize, out: &mut String) -> usize {
    let mut depth = 0usize;
    let mut i = from;
    while i < src.len() {
        let rest = &src[i..];
        if rest.starts_with("/*") {
            depth += 1;
            out.push_str("  ");
            i += 2;
            continue;
        }
        if rest.starts_with("*/") {
            depth -= 1;
            out.push_str("  ");
            i += 2;
            if depth == 0 {
                return i;
            }
            continue;
        }
        let ch = rest.chars().next().expect("in-bounds by the loop guard");
        out.push(ch);
        i += ch.len_utf8();
    }
    src.len()
}

/// `src` with the *contents* of every string, byte-string and
/// raw-string literal replaced by spaces — the quotes and any `r#`
/// hashes stay where they were, and so does every newline.
///
/// The complement of [`strip_comments`], and the two answer
/// different questions. `strip_comments` asks *which of these bytes
/// did the compiler read*, which is why it keeps literals: a pin on
/// a log line's `"<area>: "` prefix has nothing to assert on
/// otherwise. This asks the narrower *which of these bytes are
/// code*, for a scan matching identifiers against source text. There
/// the answers differ: `"do_x"` written as a criterion id is the
/// name `do_x` in the file's bytes and resolves to nothing, so a
/// contract that accepts it is satisfied by a spelling rather than
/// by a call.
///
/// Every literal is blanked one space per byte, so the returned
/// string has the same **byte length** as `src` and an offset into
/// one indexes the other. That is stronger than
/// [`strip_comments`]'s line-count guarantee and is what lets a
/// caller find a span in the blanked text and then read the literal
/// it contains out of the original.
///
/// Pass comment-free text — [`production_code`] or
/// [`strip_comments`] — or a `"` in a doc comment's prose opens a
/// literal that was never one, for the same reason
/// [`string_literals`] says so.
///
/// Cost: one pass over `src` and one `String` of its length.
pub fn blank_string_literals(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < src.len() {
        if let Some((content, content_end, end)) = raw_string_at(src, i) {
            out.push_str(&src[i..content]);
            blank_into(&src[content..content_end], &mut out);
            out.push_str(&src[content_end..end]);
            i = end;
            continue;
        }
        if bytes[i] == b'"' {
            let end = quoted_end(src, i, '"');
            // `quoted_end` returns the end of `src` for an
            // unterminated literal, in which case there is no closing
            // quote to step back over.
            let content_end = if src[..end].ends_with('"') && end > i + 1 {
                end - 1
            } else {
                end
            };
            out.push('"');
            blank_into(&src[i + 1..content_end], &mut out);
            out.push_str(&src[content_end..end]);
            i = end;
            continue;
        }
        if bytes[i] == b'\'' {
            if let Some(next) = char_literal_end(src, i) {
                // A character literal holds no identifier, and
                // leaving it be keeps `'a` lifetimes untouched too.
                out.push_str(&src[i..next]);
                i = next;
                continue;
            }
        }
        let ch = src[i..].chars().next().expect("in-bounds by the loop guard");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Push one space per byte of `content`, except newlines, which are
/// pushed as themselves. Per *byte* rather than per char so the
/// output keeps the input's byte offsets: blanking `é` to one space
/// would move every offset after it by one.
fn blank_into(content: &str, out: &mut String) {
    for ch in content.chars() {
        if ch == '\n' {
            out.push('\n');
        } else {
            for _ in 0..ch.len_utf8() {
                out.push(' ');
            }
        }
    }
}

/// The contents of every string literal in `src`, in source order,
/// with the quotes and any `r#` hashes removed.
///
/// **Why a scan and not a hand-rolled `split('"')`.** The tests that
/// need this need it to *derive* a list the code owns instead of
/// restating it — the seven messages `startup_load::fetch_map_json`
/// can return, say, which no runtime assertion can reach because the
/// function only compiles for `wasm32` (`TEST_CONVENTIONS.md` §T9). A
/// restated list is a list that drifts: that one was documented as
/// five and tested as six while the code produced seven, and every
/// one of those three numbers looked checked.
///
/// Splitting on `"` gets that right only until the body grows an
/// escaped quote or a raw string. This shares
/// [`strip_comments`]'s literal walk instead, so:
///
/// - `"a \" b"` is one literal whose contents are `a \" b` —
///   *unescaped*, because the caller is matching source text against
///   source text and un-escaping would break that;
/// - `r#"a "b" c"#` is one literal, contents `a "b" c`;
/// - `b"bytes"` is a literal like any other;
/// - `'x'` is not one, and neither is the `'a` in `&'a str`;
/// - an unterminated literal yields everything to the end of `src`
///   rather than swallowing the scan.
///
/// Pass comment-free text — [`production_code`] or
/// [`strip_comments`] — or the `"` in a doc comment's prose will be
/// read as code.
///
/// Cost: one pass over `src`, and one `Vec` of borrowed slices.
pub fn string_literals(src: &str) -> Vec<&str> {
    let bytes = src.as_bytes();
    let mut found = Vec::new();
    let mut i = 0usize;
    while i < src.len() {
        if let Some((start, content_end, end)) = raw_string_at(src, i) {
            found.push(&src[start..content_end]);
            i = end;
            continue;
        }
        if bytes[i] == b'"' {
            let end = quoted_end(src, i, '"');
            // `quoted_end` returns the end of `src` for an
            // unterminated literal, in which case there is no closing
            // quote to step back over.
            let content_end = if src[..end].ends_with('"') && end > i + 1 {
                end - 1
            } else {
                end
            };
            found.push(&src[i + 1..content_end]);
            i = end;
            continue;
        }
        if bytes[i] == b'\'' {
            if let Some(next) = char_literal_end(src, i) {
                i = next;
                continue;
            }
        }
        let ch = src[i..].chars().next().expect("in-bounds by the loop guard");
        i += ch.len_utf8();
    }
    found
}

/// Consume the nested block comment starting at `from` (which points
/// at `/*`), pushing one space plus every newline it spanned into
/// `out`. Returns the byte offset just past the comment, or the end
/// of `src` for an unterminated one.
fn skip_block_comment(src: &str, from: usize, out: &mut String) -> usize {
    out.push(' ');
    let mut depth = 0usize;
    let mut i = from;
    while i < src.len() {
        let rest = &src[i..];
        if rest.starts_with("/*") {
            depth += 1;
            i += 2;
            continue;
        }
        if rest.starts_with("*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
            continue;
        }
        let ch = rest.chars().next().expect("in-bounds by the loop guard");
        if ch == '\n' {
            out.push('\n');
        }
        i += ch.len_utf8();
    }
    src.len()
}

/// Byte offset just past the `delim`-quoted literal starting at
/// `from`, honoring backslash escapes. An unterminated literal
/// yields the end of `src`.
fn quoted_end(src: &str, from: usize, delim: char) -> usize {
    let mut chars = src[from..].char_indices();
    chars.next(); // the opening delimiter
    let mut escaped = false;
    for (offset, ch) in chars {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            c if c == delim => return from + offset + ch.len_utf8(),
            _ => {}
        }
    }
    src.len()
}

/// If a raw string (`r"`, `r#"`, `br##"`, …) starts at `from`,
/// return the byte offset just past its closing `"#…#`. Otherwise
/// `None`.
fn raw_string_end(src: &str, from: usize) -> Option<usize> {
    raw_string_at(src, from).map(|(_, _, end)| end)
}

/// If a raw string starts at `from`, return where its contents begin,
/// where they end, and where the whole literal ends. Otherwise
/// `None`.
///
/// The leading `r` / `br` must not be the tail of a longer
/// identifier — `for r in ...` is not a raw string — so the byte
/// before it is checked. An unterminated literal runs to the end of
/// `src`, contents and all.
fn raw_string_at(src: &str, from: usize) -> Option<(usize, usize, usize)> {
    let bytes = src.as_bytes();
    if from > 0 {
        let prev = bytes[from - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return None;
        }
    }
    let mut i = from;
    if bytes.get(i) == Some(&b'b') {
        i += 1;
    }
    if bytes.get(i) != Some(&b'r') {
        return None;
    }
    i += 1;
    let hash_start = i;
    while bytes.get(i) == Some(&b'#') {
        i += 1;
    }
    let hashes = i - hash_start;
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    let mut terminator = String::with_capacity(hashes + 1);
    terminator.push('"');
    for _ in 0..hashes {
        terminator.push('#');
    }
    match src[i..].find(&terminator) {
        Some(at) => Some((i, i + at, i + at + terminator.len())),
        None => Some((i, src.len(), src.len())),
    }
}

/// If a character literal starts at the `'` at `from`, return the
/// byte offset just past its closing quote. `None` means the `'`
/// opens a lifetime instead.
///
/// The two are told apart by looking for the close, not by guessing
/// from the first character: `'\n'`, `'é'` and `'\u{1F600}'` are
/// literals, `'a` and `'static` are not.
fn char_literal_end(src: &str, from: usize) -> Option<usize> {
    let after = &src[from + 1..];
    let mut chars = after.char_indices();
    let (_, first) = chars.next()?;
    if first == '\'' {
        // `''` is not legal Rust; treat it as not-a-literal rather
        // than consuming a quote that may open something real.
        return None;
    }
    if first == '\\' {
        return Some(quoted_end(src, from, '\''));
    }
    match chars.next() {
        Some((offset, '\'')) => Some(from + 1 + offset + 1),
        _ => None,
    }
}

/// The `{ … }` block of the first item in `src` whose text contains
/// `header`, brace-matched, with the outer braces included.
///
/// `src` must already be comment-free — pass the output of
/// [`production_code`] or [`strip_comments`]. Braces inside string
/// and character literals are skipped, so a `"}"` in a message
/// cannot close the block early.
///
/// The point is scope: a pin that reads "this one statement is still
/// here" should fail when the statement leaves *that function*, not
/// merely when it leaves the file. `header` is matched as a
/// substring so a caller passes the signature prefix it cares about
/// (`"fn adopt("`) rather than reproducing the whole signature,
/// which would break on an unrelated parameter rename.
///
/// `None` when `header` does not occur, or when no `{` follows it —
/// both of which a caller should treat as a failure, because the
/// item it meant to pin is gone.
///
/// Cost: one pass over `src` from `header` to the matching brace.
pub fn braced_block_after<'a>(src: &'a str, header: &str) -> Option<&'a str> {
    let at = src.find(header)?;
    let open = at + src[at..].find('{')?;
    let close = matching_delimiter(src, open, b'{', b'}')?;
    Some(&src[open..=close])
}

/// Byte offset of the delimiter closing the `open` at `from`, or
/// `None` when `src[from]` is not `open` or the run never closes.
///
/// String, byte-string, raw-string and character literals are
/// skipped, so a `"}"` in a message or a `']'` in `#[doc = "]"]`
/// cannot close a run early. That is the property
/// [`braced_block_after`] is built on, and it is shared with the
/// attribute and `cfg`-predicate walks in [`above_test_modules`]
/// rather than written three times.
///
/// `open` and `close` must differ; a same-delimiter run is not a
/// nesting problem and has no business here.
///
/// `pub(crate)` for `util::bench_surface` — no intra-doc link,
/// because that module is `cfg(test)`-gated and so absent from the
/// build `cargo doc` runs — which needs the
/// same walk over `(` … `)` to find where one `bench_function` entry
/// ends and the next begins. Sharing it is the rule this module was
/// written for: a second delimiter walk would be a second chance to
/// get literals wrong.
///
/// Cost: one pass from `from` to the close.
pub(crate) fn matching_delimiter(src: &str, from: usize, open: u8, close: u8) -> Option<usize> {
    debug_assert_ne!(open, close, "a delimiter cannot close itself");
    let bytes = src.as_bytes();
    if bytes.get(from) != Some(&open) {
        return None;
    }
    let mut depth = 0usize;
    let mut i = from;
    while i < src.len() {
        if let Some(next) = raw_string_end(src, i) {
            i = next;
            continue;
        }
        let byte = bytes[i];
        if byte == b'"' {
            i = quoted_end(src, i, '"');
            continue;
        }
        if byte == b'\'' {
            if let Some(next) = char_literal_end(src, i) {
                i = next;
                continue;
            }
        }
        if byte == open {
            depth += 1;
        } else if byte == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        let ch = src[i..].chars().next().expect("in-bounds by the loop guard");
        i += ch.len_utf8();
    }
    None
}
