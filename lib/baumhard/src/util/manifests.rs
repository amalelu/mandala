// SPDX-License-Identifier: MPL-2.0

//! Reader over the workspace's own `Cargo.toml` files, so a test can
//! ask "is any dependency's version written down in more than one
//! place?" and get an answer that stays true as members are added.
//!
//! The problem this exists to solve: nothing in cargo objects when
//! two members of a workspace name the same crate at two different
//! versions. It resolves both, builds both, and the only symptom is
//! a slower build and two incompatible copies of the same types.
//! That is not hypothetical here — `strum` sat at 0.27 in the app
//! crate and 0.28 in baumhard, compiling two runtime crates and two
//! proc-macro crates, until it was unified by hand. The structural
//! fix is `[workspace.dependencies]`, and the reason to test it is
//! that the fix is only as good as the next person's remembering to
//! use it: adding `strum = "0.29"` to one member re-creates the
//! split in one line.
//!
//! So the rule from the root manifest's comment is checked here
//! rather than trusted:
//!
//! - a dependency two or more members need is declared once, in
//!   `[workspace.dependencies]`, and members write
//!   `dep.workspace = true`;
//! - a dependency in that table is not given a version by any member
//!   (which would silently override it);
//! - the table carries nothing only one member uses.
//!
//! What the first two clauses do *not* reach: a declaration with no
//! version string. `path` dependencies have none by construction, and
//! neither do `git` ones — two members could pin one crate to two
//! different revisions and nothing here would report it. No member
//! uses a `git` dependency today; the day one arrives, this is where
//! that gap gets closed rather than discovered.
//!
//! **The member list is read, not restated.** [`member_manifests`]
//! parses `[workspace] members` out of the root manifest, so a fifth
//! crate is covered the day it is added — the failure mode
//! `test.sh`'s hand-written `-p` list demonstrated for as long as
//! `mandala_derive` existed.
//!
//! Test-only and native-only: it reads files off the filesystem, and
//! nothing in a shipped build has any business parsing manifests.
//!
//! ## On the parser
//!
//! It is line-oriented, not a TOML implementation, and the shape of
//! its ignorance is the whole design. A parser that quietly matched
//! nothing would turn every test below into a test that cannot fail,
//! so the rule is: **a legal shape either parses or stops the run —
//! it never deletes itself from the checked set.**
//!
//! Concretely:
//!
//! - Every spelling in use here parses; the rest stop the run. The
//!   inline `serde = { version = "1", features = [...] }`, the
//!   sub-table header `[dependencies.serde]` with its keys on the
//!   lines below, and the dotted `serde.workspace = true`. The
//!   sub-table form is what a `taplo fmt` or a long feature list
//!   turns the inline one into, and it is a one-line way to re-split
//!   a version, so it has to be seen rather than skipped. Cargo
//!   accepts one more spelling nobody here writes — the rest of the
//!   dotted family, `serde.version = "1"` and `serde.path = "../x"`,
//!   which is the sub-table written inline. That one refuses by name
//!   rather than parsing, which is the safe direction: it stops a run
//!   instead of deleting a declaration from the checked set.
//! - A leading UTF-8 BOM is stripped before anything is read. Cargo
//!   accepts a BOM, so a manifest carrying one is a manifest this
//!   reader has to see all of; left in place it hides the first
//!   section header, and every key under that header with it.
//! - A renamed dependency (`fast = { package = "rustc-hash", ... }`)
//!   is recorded under the crate it actually pulls in, so a rename
//!   cannot hide a split behind a different key.
//! - A trailing `# comment` on a declaration is stripped, quotes
//!   respected. Manifests in this workspace are heavily commented;
//!   adding one more comment must not be able to fail a test.
//! - A declaration wrapped across lines — a long `features` list, or
//!   whatever a formatter does to the 190-column `web-sys` entry — is
//!   rejoined before it is read. A line break cannot change which
//!   crate is pulled in at which version, so it may not change the
//!   answer given here.
//! - Everything left **refuses**: a section header the reader has no
//!   classification for, a table nested deeper under a dependency
//!   table, a header whose `]` never arrives, a value that is neither
//!   a version string nor an inline table. Each panics naming the
//!   file, the line, and what the reader could not do with it.
//!
//! The refusal policy is weighted by risk on purpose, because it used
//! to be weighted the other way: silence is for shapes that cannot
//! change an answer, and noise is for anything that could be a
//! dependency the rules never see.
//!
//! [`tests`] pins all of this against synthetic manifests — including
//! ones that *must* be rejected, and shapes the real manifests do not
//! yet use — before pointing the parser at the real ones. A fixture
//! made only of shapes the manifests already contain could never
//! discover a shape they could grow.

use crate::util::doc_fixtures::repo_path;
use std::collections::{BTreeMap, BTreeSet};

/// Which dependency table a declaration was found in. The string is
/// the raw section header without its brackets, so
/// `target.'cfg(unix)'.dependencies` keeps its target predicate for
/// error messages.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Table {
    /// The root manifest's `[workspace.dependencies]` — the single
    /// source of truth, not a consumer.
    Workspace,
    /// Any `[dependencies]` / `[dev-dependencies]` /
    /// `[build-dependencies]`, plain or under a `[target.'...']`.
    Member(String),
}

/// One dependency declaration as written.
#[derive(Debug, Clone)]
pub(crate) struct Dep {
    /// The key the declaration is written under — the name to the
    /// left of the `=`, or the last segment of a
    /// `[dependencies.<name>]` header. Not necessarily the crate:
    /// see [`Dep::crate_name`].
    pub(crate) name: String,
    /// The `package = "..."` override, when the declaration renames
    /// the crate it pulls in. `None` for the ordinary case where the
    /// key *is* the crate name.
    pub(crate) package: Option<String>,
    /// The table it was declared in.
    pub(crate) table: Table,
    /// The version string, when the declaration names one. `None`
    /// for `dep.workspace = true` and for path dependencies.
    pub(crate) version: Option<String>,
    /// Whether the declaration defers to `[workspace.dependencies]`.
    pub(crate) inherits_workspace: bool,
    /// Whether the declaration is an intra-workspace `path = ` dep.
    /// Those carry no version and are exempt from every rule here.
    pub(crate) is_path: bool,
}

impl Dep {
    /// The crate this declaration actually pulls in: the `package =`
    /// rename when there is one, otherwise the key.
    ///
    /// Every rule below keys on this rather than on [`Dep::name`],
    /// because the failure they exist to catch is *two versions of
    /// one crate*, and cargo's dependency-renaming syntax lets that
    /// happen under two different keys.
    pub(crate) fn crate_name(&self) -> &str {
        self.package.as_deref().unwrap_or(&self.name)
    }
}

/// The dependency declarations of one manifest.
#[derive(Debug, Clone)]
pub(crate) struct Manifest {
    /// Repo-relative path, used verbatim in assertion messages.
    pub(crate) path: String,
    /// Every declaration found, in file order.
    pub(crate) deps: Vec<Dep>,
}

impl Manifest {
    /// The declarations in this manifest's member tables — i.e.
    /// everything except a `[workspace.dependencies]` entry.
    pub(crate) fn member_deps(&self) -> impl Iterator<Item = &Dep> {
        self.deps.iter().filter(|d| !matches!(d.table, Table::Workspace))
    }

    /// The declarations in this manifest's `[workspace.dependencies]`
    /// table. Empty for everything but the root manifest.
    pub(crate) fn workspace_deps(&self) -> impl Iterator<Item = &Dep> {
        self.deps.iter().filter(|d| matches!(d.table, Table::Workspace))
    }
}

/// Every manifest in the workspace, repo-relative, root first.
///
/// Read out of `[workspace] members` rather than listed, so this
/// cannot be the thing that goes stale. The root manifest is both the
/// workspace root and a package (`mandala`), so it appears once and
/// is parsed for both roles.
pub(crate) fn member_manifests() -> Vec<String> {
    let root =
        std::fs::read_to_string(repo_path("Cargo.toml")).expect("the root Cargo.toml must be readable");

    let members = root
        .split_once("members = [")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split_once(']'))
        .map(|(list, _)| list)
        .expect("the root Cargo.toml must declare `[workspace] members = [...]`");

    let mut manifests = vec!["Cargo.toml".to_string()];
    manifests.extend(
        members
            .split(',')
            .map(|entry| entry.trim().trim_matches('"').trim())
            .filter(|entry| !entry.is_empty())
            .map(|dir| format!("{dir}/Cargo.toml")),
    );
    manifests
}

/// Parse every manifest in the workspace.
pub(crate) fn workspace_manifests() -> Vec<Manifest> {
    member_manifests()
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(repo_path(&path))
                .unwrap_or_else(|e| panic!("{path} must be readable: {e}"));
            parse(&path, &text)
        })
        .collect()
}

/// What a section header means to this reader.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Section {
    /// A dependency table proper: the lines under it are
    /// `name = <value>` declarations.
    Table(Table),
    /// A per-dependency sub-table — `[dependencies.serde_json]`,
    /// `[workspace.dependencies.serde]`,
    /// `[target.'cfg(unix)'.dependencies.foo]`. The lines under it
    /// are the *fields* of one declaration.
    Dep { table: Table, name: String },
    /// A section that cannot contain a dependency: `[package]`,
    /// `[[bench]]`, `[profile.release-lto]`, `[workspace]`. Its lines
    /// are skipped, and skipping them is safe precisely because the
    /// classification is positive rather than a fallthrough.
    Other,
}

/// Section roots that are known not to hold dependencies. The list is
/// closed on purpose: an unrecognized header panics rather than being
/// treated as harmless, because "harmless" is exactly what a missed
/// dependency table looks like from here.
///
/// `patch` and `replace` are deliberately *absent*, and
/// [`section_for`] refuses them with a message of their own that says
/// why, rather than the generic one whose advice is to extend this
/// list. Neither is a member dependency table, so none of the three
/// rules applies to them as written — but both redirect where a
/// dependency comes from, which is close enough that the first person
/// to add one should have to decide what these rules mean for it
/// rather than have this list decide silently.
const NON_DEPENDENCY_ROOTS: [&str; 12] = [
    "package",
    "lib",
    "bin",
    "bench",
    "test",
    "example",
    "features",
    "profile",
    "workspace",
    "lints",
    "badges",
    "target",
];

/// Classify a section header (already stripped of its brackets) as a
/// dependency table, or `None` if it is not one.
fn table_for(header: &str) -> Option<Table> {
    const KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

    if header == "workspace.dependencies" {
        return Some(Table::Workspace);
    }
    if KINDS.contains(&header) {
        return Some(Table::Member(header.to_string()));
    }
    // `target.'cfg(...)'.dependencies` and its dev/build siblings.
    // Matched on the suffix so any cfg predicate is accepted, but
    // still anchored on `target.` so an unrelated future section
    // ending in `dependencies` is not swept in silently.
    if header.starts_with("target.") && KINDS.iter().any(|k| header.ends_with(&format!(".{k}"))) {
        return Some(Table::Member(header.to_string()));
    }
    None
}

/// The longest prefix of `header` that is a dependency table, paired
/// with what follows it.
///
/// Split from the right so a cfg predicate containing dots
/// (`target.'cfg(target_arch = "wasm32")'.dependencies`) is not cut
/// in half before the real boundary is found.
fn dependency_prefix(header: &str) -> Option<(Table, &str)> {
    let mut cut = header.len();
    while let Some(dot) = header[..cut].rfind('.') {
        if let Some(table) = table_for(&header[..dot]) {
            return Some((table, &header[dot + 1..]));
        }
        cut = dot;
    }
    None
}

/// Classify a section header, panicking on anything this reader has
/// no positive answer for.
///
/// The panic is the point. `[dependencies.serde_json]` used to fall
/// through to "not a dependency table", which silently deleted every
/// key under it from the set the rules check — a legal, one-line way
/// to re-split a version that the enforcement could not see. Nothing
/// falls through now: a header is a dependency table, a dependency
/// sub-table, a known non-dependency section, or a test failure.
fn section_for(path: &str, header: &str) -> Section {
    if let Some(table) = table_for(header) {
        return Section::Table(table);
    }
    if let Some((table, rest)) = dependency_prefix(header) {
        let name = rest.trim().trim_matches('"');
        assert!(
            !name.is_empty() && !name.contains('.'),
            "{path}: section [{header}] nests something under a dependency table that \
             `util::manifests` cannot read as one dependency. Teach the reader this \
             shape rather than leaving the declarations under it unchecked."
        );
        return Section::Dep {
            table,
            name: name.to_string(),
        };
    }
    let root = header.split('.').next().unwrap_or(header);
    // `patch` and `replace` get their own refusal, because the generic
    // one below ends in advice — "add its root to
    // `NON_DEPENDENCY_ROOTS`" — that would resolve this case exactly
    // wrongly, in one line, and silently. Pinning a fork is a routine
    // edit; the first person to make one should read why these two are
    // held out rather than how to stop the suite complaining.
    assert!(
        root != "patch" && root != "replace",
        "{path}: section [{header}] redirects where a dependency comes from, and \
         `util::manifests` refuses it on purpose rather than classifying it. The \
         three rules are written about *which version a member asks for*; a patched \
         or replaced source keeps the asking untouched and changes what that ask \
         resolves to, so a shared `[workspace.dependencies]` entry can stay \
         letter-perfect while two members build against different code. Nobody has \
         had to decide what the rules mean for that yet, and it is a decision, not a \
         classification — so decide it here, in this module, with a test that says \
         what you decided. Do not add `{root}` to `NON_DEPENDENCY_ROOTS`: that is \
         the one-line way to make this section invisible, which is the failure this \
         reader exists to not have."
    );
    assert!(
        NON_DEPENDENCY_ROOTS.contains(&root),
        "{path}: section [{header}] is one `util::manifests` has no classification \
         for. Either it can hold dependencies — in which case the rules in this \
         module have stopped covering the workspace — or it cannot, in which case \
         add its root to `NON_DEPENDENCY_ROOTS`. Guessing is how a dependency table \
         goes unread."
    );
    Section::Other
}

/// Parse one manifest's dependency declarations.
///
/// Panics on any declaration it cannot read with confidence, naming
/// the file and the line. Refusing is the point: a parser that
/// shrugged at an unfamiliar shape would silently shrink the set
/// every test here checks.
pub(crate) fn parse(path: &str, text: &str) -> Manifest {
    let mut deps = Vec::new();
    let mut section = Section::Other;
    // The declaration a `[dependencies.<name>]` header opened, filled
    // in from the key lines below it and pushed when the section ends.
    let mut pending: Option<Dep> = None;

    for (line, raw) in logical_lines(text) {
        let (line, raw) = (line.as_str(), raw.as_str());
        if line.starts_with('[') {
            // A header that does not close is not a header, and must
            // not be classified as one. `logical_lines` joins on
            // unbalanced delimiters, so a table header split across
            // lines — or a file that ends mid-header — arrives here
            // as one long buffer starting with `[`; classifying its
            // first segment would read `[target.'cfg(any(` as the
            // known-harmless `target` root and drop the whole table
            // without a word. Illegal TOML, so cargo rejects the file
            // first, but this reader does not get to rely on that.
            assert!(
                line.ends_with(']'),
                "{path}: section header {raw:?} is never closed with `]`. \
                 `util::manifests` reads a header from one line; a header wrapped \
                 across lines, or a file that ends inside one, cannot be classified \
                 and must not be guessed at."
            );
            // `[[bench]]` and friends end a dependency table just as
            // any other header does; `section_for` classifies them as
            // `Other` and the lines after them are skipped.
            let header = line.trim_start_matches('[').trim_end_matches(']').trim();
            deps.extend(pending.take());
            section = section_for(path, header);
            if let Section::Dep { table, name } = &section {
                pending = Some(Dep {
                    name: name.clone(),
                    package: None,
                    table: table.clone(),
                    version: None,
                    inherits_workspace: false,
                    is_path: false,
                });
            }
            continue;
        }

        let (lhs, rhs) = match line.split_once('=') {
            Some(halves) => halves,
            // Only refuse inside a dependency table; a `[package]` or
            // `[features]` body may legally hold shapes with no `=`
            // on the line (an array element, a closing bracket).
            None if matches!(section, Section::Other) => continue,
            None => panic!("{path}: cannot read dependency line {raw:?}"),
        };
        let (lhs, rhs) = (lhs.trim(), rhs.trim());

        match &section {
            Section::Other => continue,
            Section::Table(table) => deps.push(parse_dep(path, raw, lhs, rhs, table.clone())),
            Section::Dep { .. } => {
                let dep = pending
                    .as_mut()
                    .expect("a `Section::Dep` header always installs a pending declaration");
                apply_sub_table_key(dep, lhs, rhs);
            }
        }
    }
    deps.extend(pending.take());

    Manifest {
        path: path.to_string(),
        deps,
    }
}

/// Fold one `key = value` line of a `[dependencies.<name>]` sub-table
/// into the declaration it describes.
///
/// Keys this reader has no rule about (`features`,
/// `default-features`, `optional`) are ignored rather than refused:
/// none of them can turn one crate into two versions, which is the
/// only thing the rules here are about.
fn apply_sub_table_key(dep: &mut Dep, key: &str, value: &str) {
    match key {
        "version" => dep.version = Some(quoted(value)),
        "workspace" => dep.inherits_workspace = value.starts_with("true"),
        "path" => dep.is_path = true,
        "package" => dep.package = Some(quoted(value)),
        _ => {}
    }
}

/// The manifest as *logical* lines: comments stripped, blank lines
/// dropped, and a value whose brackets or braces are still open at
/// end-of-line joined with the lines that close it.
///
/// Each entry is `(joined line, the first raw line it came from)`;
/// the raw line is what error messages quote, so a refusal still
/// points at a place in the file.
///
/// Joining is why a wrapped `features = [...]` list — or the whole
/// 190-column `web-sys` declaration folded across four lines by a
/// formatter — is an ordinary declaration here rather than five
/// failing tests. Nothing about a line break changes which crate is
/// pulled in at which version, so nothing about it may change the
/// answer this module gives.
///
/// A leading UTF-8 BOM is dropped first. Cargo accepts one, and left
/// in place it would make `\u{feff}[package]` fail `starts_with('[')`
/// — the first section of the file, and every key under it, read as
/// the body of no section at all.
fn logical_lines(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut buffer = String::new();
    let mut first_raw = String::new();
    let mut depth = 0;

    for raw in text.trim_start_matches('\u{feff}').lines() {
        let line = strip_trailing_comment(raw.trim());
        if line.is_empty() {
            continue;
        }
        if depth == 0 {
            buffer.clear();
            first_raw = raw.to_string();
        } else {
            buffer.push(' ');
        }
        buffer.push_str(line);
        depth += unclosed_delta(line);
        if depth <= 0 {
            depth = 0;
            out.push((buffer.clone(), first_raw.clone()));
        }
    }
    // An unbalanced tail is still handed on rather than dropped,
    // which beats a declaration disappearing because the file ended
    // mid-table. Downstream refuses it either way, but by two
    // different routes and it is worth being exact about which: a
    // tail that is a *value* (`foo = { version = "1",`) is refused by
    // `parse_dep`, which names the crate and says the inline table
    // never closes. A tail that begins with a *header* would not have
    // reached that refusal — joined, it still `starts_with('[')`, so
    // `parse` would take it for a header and `section_for` would
    // classify the pseudo-header's first segment, which for
    // `[target.'cfg(any(` is `target`: a known non-dependency root,
    // and the table under it silently gone. `parse` therefore
    // requires a header line to close with `]` before classifying it.
    if depth > 0 {
        out.push((buffer, first_raw));
    }
    out
}

/// Net openers minus closers on a line, counting `[`/`]` and `{`/`}`
/// outside quotes.
///
/// A section header nets zero (`[dependencies]`), an inline table
/// that closes on its own line nets zero, and only a value left open
/// at end-of-line nets more.
fn unclosed_delta(line: &str) -> i32 {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut depth = 0;
    for c in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, c) {
            (Some('"'), '\\') => escaped = true,
            (Some(open), _) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"') | (None, '\'') => quote = Some(c),
            (None, '[' | '{') => depth += 1,
            (None, ']' | '}') => depth -= 1,
            (None, _) => {}
        }
    }
    depth
}

/// Drop a trailing `# comment` from a manifest line, leaving `#`
/// inside a quoted string alone.
///
/// A comment on a dependency line used to take down five tests with
/// advice that could not be followed — the declaration it complained
/// was not "on one line" already was. Given how much commentary these
/// manifests carry, adding one more comment is the likeliest edit
/// they will receive, and it must not be able to fail anything.
fn strip_trailing_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (at, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, c) {
            (Some('"'), '\\') => escaped = true,
            (Some(open), _) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"') | (None, '\'') => quote = Some(c),
            (None, '#') => return line[..at].trim_end(),
            (None, _) => {}
        }
    }
    line
}

/// Read one `name = <value>` dependency line.
fn parse_dep(path: &str, raw: &str, lhs: &str, rhs: &str, table: Table) -> Dep {
    // `serde.workspace = true` — a dotted key, which is the only
    // dotted form this workspace uses. Anything else dotted is a
    // shape the rules below were not written against, so refuse it
    // rather than record it wrong.
    let (name, dotted_key) = match lhs.split_once('.') {
        Some((name, key)) => (name.trim(), Some(key.trim())),
        None => (lhs, None),
    };
    let name = name.trim_matches('"').to_string();

    if let Some(key) = dotted_key {
        assert_eq!(
            key, "workspace",
            "{path}: {raw:?} uses a dotted dependency key this reader does not \
             understand; teach `util::manifests` about it rather than leaving it \
             unchecked"
        );
        assert_eq!(rhs, "true", "{path}: {raw:?} — `{name}.workspace` must be `true`");
        return Dep {
            name,
            package: None,
            table,
            version: None,
            inherits_workspace: true,
            is_path: false,
        };
    }

    // A bare version string: `rustc-hash = "2.1.2"`.
    if let Some(version) = rhs.strip_prefix('"').and_then(|r| r.split_once('"')) {
        return Dep {
            name,
            package: None,
            table,
            version: Some(version.0.to_string()),
            inherits_workspace: false,
            is_path: false,
        };
    }

    // An inline table. Wrapping one across several lines is fine —
    // `logical_lines` has already rejoined it — so reaching the panic
    // means the value is genuinely not a shape this reader knows, and
    // the message says which half failed rather than blaming the line
    // count. Refusing, not skipping: see the module docs on why
    // silence is the dangerous answer.
    let body = rhs
        .strip_prefix('{')
        .and_then(|r| r.strip_suffix('}'))
        .unwrap_or_else(|| {
            let cause = if rhs.starts_with('{') {
                "its inline table is never closed with `}`"
            } else {
                "its value is neither a quoted version string nor an inline table opening with `{`"
            };
            panic!(
                "{path}: the declaration starting at {raw:?} could not be read: \
                 {cause}. `util::manifests` reads `{name} = \"1.2.3\"`, \
                 `{name} = {{ ... }}` (wrapped across lines or not, with or without a \
                 trailing `# comment`), `{name}.workspace = true`, and the \
                 `[{table_header}.{name}]` sub-table form. Teach it this shape rather \
                 than leaving the declaration unchecked. Read as: {rhs:?}",
                table_header = match &table {
                    Table::Workspace => "workspace.dependencies",
                    Table::Member(header) => header,
                }
            )
        });

    Dep {
        name,
        package: inline_value(body, "package").map(quoted),
        table,
        version: inline_value(body, "version").map(quoted),
        inherits_workspace: inline_value(body, "workspace").is_some_and(|v| v.starts_with("true")),
        is_path: inline_value(body, "path").is_some(),
    }
}

/// The value following `key =` inside an inline table body, or `None`
/// if the key is absent.
///
/// Matches `key` only as a whole word immediately followed by `=`, so
/// a feature string that happens to contain the key's letters — or
/// the `version` inside `version-compat` — does not match.
fn inline_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(offset) = body[from..].find(key) {
        let at = from + offset;
        let boundary = match body[..at].chars().next_back() {
            None => true,
            Some(c) => !(c.is_alphanumeric() || c == '_' || c == '-'),
        };
        let after = body[at + key.len()..].trim_start();
        if boundary {
            if let Some(value) = after.strip_prefix('=') {
                return Some(value.trim());
            }
        }
        from = at + key.len();
    }
    None
}

/// Strip the surrounding quotes from a TOML string value, basic
/// (`"1.0"`) or literal (`'1.0'`).
///
/// Both spellings mean the same version to cargo, so both must mean
/// the same string here: a sub-table's `version = '1.0'` was recorded
/// as `'1.0'`, delimiters and all. No rule misfired on it — all three
/// key on `version.is_some()` — but a version that is not the version
/// is a wrong answer waiting for the first rule that reads one.
fn quoted(value: &str) -> String {
    let value = value.trim();
    match value.chars().next() {
        Some(delimiter @ ('"' | '\'')) => value[delimiter.len_utf8()..]
            .split(delimiter)
            .next()
            .unwrap_or_default()
            .to_string(),
        _ => value.to_string(),
    }
}

/// The names declared in `[workspace.dependencies]`.
pub(crate) fn workspace_table(manifests: &[Manifest]) -> BTreeSet<String> {
    manifests
        .iter()
        .flat_map(Manifest::workspace_deps)
        .map(|d| d.crate_name().to_string())
        .collect()
}

/// For each dependency given an explicit version by more than one
/// manifest, the manifests that give it one.
///
/// This is the `strum` failure in general form: two members, two
/// version strings, no complaint from cargo. Path dependencies are
/// exempt — an intra-workspace `path = ` dep has no version to
/// diverge.
pub(crate) fn versions_declared_in_several_manifests(
    manifests: &[Manifest],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut sites: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for manifest in manifests {
        for dep in manifest.member_deps() {
            if dep.is_path || dep.version.is_none() {
                continue;
            }
            sites
                .entry(dep.crate_name().to_string())
                .or_default()
                .insert(manifest.path.clone());
        }
    }
    sites.retain(|_, where_| where_.len() > 1);
    sites
}

/// For each `[workspace.dependencies]` entry a member shadows with
/// its own version, the manifests that shadow it.
///
/// Cargo takes the member's word, so an override turns the shared
/// table into decoration — the same split as before, one indirection
/// further away from the reader.
pub(crate) fn workspace_entries_overridden(manifests: &[Manifest]) -> BTreeMap<String, BTreeSet<String>> {
    let shared = workspace_table(manifests);
    let mut overrides: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for manifest in manifests {
        for dep in manifest.member_deps() {
            if dep.version.is_some() && shared.contains(dep.crate_name()) {
                overrides
                    .entry(dep.crate_name().to_string())
                    .or_default()
                    .insert(manifest.path.clone());
            }
        }
    }
    overrides
}

/// For each `[workspace.dependencies]` entry, the manifests that
/// actually inherit it.
pub(crate) fn workspace_entry_consumers(manifests: &[Manifest]) -> BTreeMap<String, BTreeSet<String>> {
    let mut consumers: BTreeMap<String, BTreeSet<String>> = workspace_table(manifests)
        .into_iter()
        .map(|name| (name, BTreeSet::new()))
        .collect();
    for manifest in manifests {
        for dep in manifest.member_deps() {
            if !dep.inherits_workspace {
                continue;
            }
            consumers
                .entry(dep.crate_name().to_string())
                .or_default()
                .insert(manifest.path.clone());
        }
    }
    consumers
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manifest pair in the shape this workspace uses, for the
    /// parser and detector pins below. Kept synthetic so the negative
    /// controls can express failures the real manifests must not
    /// have.
    fn fixture(path: &str, body: &str) -> Manifest {
        parse(path, body)
    }

    /// The parser has to read every shape the real manifests contain,
    /// because a shape it skips is a dependency the rules below never
    /// see. Each line here appears in the workspace for real.
    #[test]
    fn test_parser_reads_the_shapes_this_workspace_uses() {
        let m = fixture(
            "fixture/Cargo.toml",
            r#"
[package]
name = "fixture"
autobenches = false

[workspace.dependencies]
strum = "0.28.0"
serde = { version = "1.0.228", features = ["derive"] }

[dependencies]
# a comment, and a blank line, both ignored

strum.workspace = true
syn = { workspace = true, features = ["full"] }
baumhard = {path="./lib/baumhard"}
wgpu = { version = "29.0", features = ["webgl"] }

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
arboard = "3.6.1"

[[bench]]
name = "test_bench"
harness = false
"#,
        );

        let by_name = |name: &str| -> Dep {
            m.deps
                .iter()
                .find(|d| d.name == name && !matches!(d.table, Table::Workspace))
                .unwrap_or_else(|| panic!("{name} must be parsed out of the member tables"))
                .clone()
        };

        assert_eq!(
            workspace_table(std::slice::from_ref(&m)),
            ["serde".to_string(), "strum".to_string()].into(),
            "`[workspace.dependencies]` entries must be recognized as the table, not \
             as member declarations"
        );

        let strum = by_name("strum");
        assert!(strum.inherits_workspace && strum.version.is_none());

        let syn = by_name("syn");
        assert!(
            syn.inherits_workspace && syn.version.is_none(),
            "`{{ workspace = true, features = [...] }}` inherits; the added features \
             are not an override"
        );

        assert!(by_name("baumhard").is_path, "a path dep has no version");
        assert_eq!(by_name("wgpu").version.as_deref(), Some("29.0"));

        let arboard = by_name("arboard");
        assert_eq!(arboard.version.as_deref(), Some("3.6.1"));
        assert_eq!(
            arboard.table,
            Table::Member("target.'cfg(not(target_arch = \"wasm32\"))'.dependencies".into()),
            "a target-gated table is still a member table"
        );

        assert!(
            !m.deps.iter().any(|d| d.name == "name" || d.name == "harness"),
            "`[[bench]]` and `[package]` keys must not be mistaken for dependencies"
        );
    }

    /// The shapes the manifests do *not* yet use but legally could.
    ///
    /// This is the half a fixture built from the current manifests
    /// cannot have: `[dependencies.<name>]` is standard cargo, is one
    /// line away from any existing declaration, and used to be
    /// invisible to this reader — a live version split written this
    /// way passed all ten tests.
    #[test]
    fn test_parser_reads_a_dependency_written_as_a_sub_table() {
        let m = fixture(
            "fixture/Cargo.toml",
            r#"
[workspace.dependencies.serde]
version = "1.0.228"
features = ["derive"]

[dependencies]
regex = "1.12.3"

[dependencies.serde_json]
version = "1.0.149"

[dependencies.serde]
workspace = true
features = [
    "rc",
]

[dependencies.baumhard]
path = "../../lib/baumhard"

[build-dependencies.ttf-parser]
version = '0.25.1'

[target.'cfg(not(target_arch = "wasm32"))'.dependencies.arboard]
version = "3.6.1"

[[bench]]
name = "test_bench"
"#,
        );

        let by_name = |name: &str| -> Dep {
            m.deps
                .iter()
                .find(|d| d.name == name && !matches!(d.table, Table::Workspace))
                .unwrap_or_else(|| panic!("{name} must be parsed out of the member tables"))
                .clone()
        };

        assert_eq!(
            workspace_table(std::slice::from_ref(&m)),
            ["serde".to_string()].into(),
            "`[workspace.dependencies.serde]` is the shared table, not a member \
             declaration"
        );
        assert_eq!(
            by_name("serde_json").version.as_deref(),
            Some("1.0.149"),
            "a sub-table's `version` key is the version, and missing it is how a \
             re-split hid from these rules"
        );
        assert_eq!(by_name("regex").version.as_deref(), Some("1.12.3"));

        let serde = by_name("serde");
        assert!(
            serde.inherits_workspace && serde.version.is_none(),
            "`workspace = true` in a sub-table inherits, and the wrapped `features` \
             list below it changes nothing"
        );
        assert!(by_name("baumhard").is_path);
        assert_eq!(
            by_name("ttf-parser").table,
            Table::Member("build-dependencies".into())
        );
        assert_eq!(
            by_name("ttf-parser").version.as_deref(),
            Some("0.25.1"),
            "a TOML literal string is the same version as a basic one, and must be \
             recorded without its delimiters"
        );
        assert_eq!(
            by_name("arboard").table,
            Table::Member("target.'cfg(not(target_arch = \"wasm32\"))'.dependencies".into()),
            "a target-gated sub-table is still a member table"
        );
        assert!(
            !m.deps.iter().any(|d| d.name == "name"),
            "`[[bench]]` keys must not be mistaken for dependencies"
        );
    }

    /// The B1 reproduction as a regression test: the exact edit that
    /// used to leave all ten tests green.
    #[test]
    fn test_detector_catches_a_split_written_as_a_sub_table() {
        let shadowed = [
            fixture(
                "Cargo.toml",
                "[workspace.dependencies]\nserde_json = \"1.0.149\"\n",
            ),
            fixture(
                "crates/maptool/Cargo.toml",
                "[dependencies]\nregex = \"1.12.3\"\n\n[dependencies.serde_json]\nversion = \"1.0.149\"\n",
            ),
        ];
        assert_eq!(
            workspace_entries_overridden(&shadowed).keys().collect::<Vec<_>>(),
            vec!["serde_json"],
            "a member that re-versions a shared entry in section syntax is the same \
             violation as one that does it inline"
        );
    }

    /// A rename is a second way to write the same crate twice without
    /// the two declarations looking alike.
    #[test]
    fn test_detector_catches_a_split_hidden_by_a_rename() {
        let split = [
            fixture("a/Cargo.toml", "[dependencies]\nstrum = \"0.27\"\n"),
            fixture(
                "b/Cargo.toml",
                "[dependencies]\nenums = { package = \"strum\", version = \"0.28.0\" }\n",
            ),
        ];
        assert_eq!(
            versions_declared_in_several_manifests(&split)
                .keys()
                .collect::<Vec<_>>(),
            vec!["strum"],
            "the split must be reported under the crate that is actually pulled in \
             twice, not under the two keys it is written behind"
        );
    }

    /// A comment on a dependency line changes nothing about any rule
    /// here, so it must change nothing about the parse either. It used
    /// to fail five tests.
    #[test]
    fn test_parser_reads_a_commented_dependency_line() {
        let m = fixture(
            "fixture/Cargo.toml",
            "[dependencies]\n\
             ordered-float = {version = \"5.3.0\", features = [\"serde\"]} # total ordering for f32 keys\n\
             rustc-hash = \"2.1.2\" # fast, non-cryptographic\n\
             strum.workspace = true # unified in #106\n\
             tag = \"1.0\" # a '#' in prose, and \"quoted\" text\n",
        );
        assert_eq!(m.deps.len(), 4, "every commented line is still a declaration");
        assert_eq!(m.deps[0].version.as_deref(), Some("5.3.0"));
        assert_eq!(m.deps[1].version.as_deref(), Some("2.1.2"));
        assert!(m.deps[2].inherits_workspace);
        assert_eq!(m.deps[3].version.as_deref(), Some("1.0"));
    }

    /// A UTF-8 BOM is something cargo accepts, so it is something this
    /// reader has to survive. Left in place, `\u{feff}[dependencies]`
    /// does not `starts_with('[')`: the header is not recognized, the
    /// section is never entered, and every declaration under it drops
    /// out of the checked set without a word — the one property this
    /// module claims it does not have. The real manifests all open
    /// with `[package]`, so today a BOM would cost nothing; that is an
    /// accident of ordering that any manifest edit can undo, which is
    /// exactly the kind of accident not worth depending on.
    #[test]
    fn test_parser_reads_a_manifest_with_a_byte_order_mark() {
        let m = fixture(
            "fixture/Cargo.toml",
            "\u{feff}[dependencies]\nfoo = \"1.0\"\nbar = \"2.0\"\n",
        );
        assert_eq!(
            m.deps.len(),
            2,
            "a BOM must not hide the first section header, or the table under it"
        );
        assert_eq!(m.deps[0].version.as_deref(), Some("1.0"));
        assert_eq!(m.deps[1].version.as_deref(), Some("2.0"));

        let shadowed = [
            fixture("Cargo.toml", "[workspace.dependencies]\nstrum = \"0.28.0\"\n"),
            fixture(
                "member/Cargo.toml",
                "\u{feff}[dependencies]\nstrum = \"0.27.0\"\n",
            ),
        ];
        assert_eq!(
            workspace_entries_overridden(&shadowed).keys().collect::<Vec<_>>(),
            vec!["strum"],
            "and a violation in a BOM-prefixed manifest is still reported"
        );
    }

    /// A header this reader cannot see the end of is a header it may
    /// not classify. `logical_lines` joins on unbalanced delimiters,
    /// so a wrapped table header arrives as one buffer beginning with
    /// `[`; its first segment here is `target`, a known non-dependency
    /// root, and taking that answer would drop the table in silence.
    /// Cargo rejects a newline inside a table header, so this is
    /// hardening rather than a live shape — but it was the last path
    /// left where an unbalanced delimiter produced silence instead of
    /// a refusal, and the module's claim is that there is no such path.
    #[test]
    #[should_panic(expected = "never closed with `]`")]
    fn test_parser_refuses_a_header_that_does_not_close() {
        parse(
            "fixture/Cargo.toml",
            "[target.'cfg(any(\n  unix,\n))'.dependencies]\nfoo = \"1.0\"\n",
        );
    }

    /// A `#` inside a quoted value is part of the value, not the
    /// start of a comment.
    #[test]
    fn test_parser_keeps_a_hash_inside_a_quoted_value() {
        let m = fixture("fixture/Cargo.toml", "[dependencies]\nfoo = \"1.0#beta\"\n");
        assert_eq!(m.deps[0].version.as_deref(), Some("1.0#beta"));
    }

    /// A section header with no classification stops the run. This is
    /// the general form of B1: the reader may not decide on its own
    /// that an unfamiliar section holds nothing worth checking.
    #[test]
    #[should_panic(expected = "no classification for")]
    fn test_parser_refuses_an_unclassified_section() {
        parse(
            "fixture/Cargo.toml",
            "[dependencies-of-tomorrow]\nstrum = \"0.27\"\n",
        );
    }

    /// ...and neither may it decide that about a table nested under a
    /// dependency table.
    #[test]
    #[should_panic(expected = "cannot read as one dependency")]
    fn test_parser_refuses_a_table_nested_under_a_dependency() {
        parse(
            "fixture/Cargo.toml",
            "[dependencies.strum.metadata]\nversion = \"0.27\"\n",
        );
    }

    /// `[patch]` and `[replace]` are refused, and *what the refusal
    /// says* is the point of pinning it: the generic unclassified-
    /// section message ends by suggesting `NON_DEPENDENCY_ROOTS`,
    /// which for these two would silence the refusal in one line and
    /// call it a fix. Pinning a fork is a routine edit, so the first
    /// person to hit this must read why rather than how.
    #[test]
    #[should_panic(expected = "Do not add `patch` to `NON_DEPENDENCY_ROOTS`")]
    fn test_parser_refuses_a_patched_source_with_its_reason() {
        parse(
            "fixture/Cargo.toml",
            "[patch.crates-io]\nstrum = { path = \"../strum\" }\n",
        );
    }

    /// A wrapped declaration is the same declaration. This is the
    /// `web-sys` line — 190 columns in the root manifest — as a
    /// formatter would leave it.
    #[test]
    fn test_parser_reads_a_wrapped_inline_table() {
        let m = fixture(
            "fixture/Cargo.toml",
            "[dependencies]\n\
             web-sys = { version = \"0.3.95\", features = [\n\
             \x20   \"Window\",\n\
             \x20   \"Document\",  # the two the renderer actually needs\n\
             ] }\n\
             wgpu = \"29.0\"\n",
        );
        assert_eq!(m.deps.len(), 2, "the wrapped entry is one declaration, not three");
        assert_eq!(m.deps[0].name, "web-sys");
        assert_eq!(m.deps[0].version.as_deref(), Some("0.3.95"));
        assert_eq!(m.deps[1].version.as_deref(), Some("29.0"));
    }

    /// Refusing beats guessing: a value this reader cannot read must
    /// stop the test run, not vanish from the set being checked — and
    /// the message must name the value rather than blame the line
    /// count, which is a cause the reader can no longer have.
    #[test]
    #[should_panic(expected = "neither a quoted version string nor an inline table")]
    fn test_parser_refuses_a_value_it_cannot_read() {
        parse("fixture/Cargo.toml", "[dependencies]\nstrum = 0.28\n");
    }

    /// The negative control for the headline rule. Without it, every
    /// assertion over the real manifests could be passing because the
    /// detector never reports anything.
    #[test]
    fn test_detector_catches_a_version_split_across_manifests() {
        let split = [
            fixture("a/Cargo.toml", "[dependencies]\nstrum = \"0.27\"\n"),
            fixture("b/Cargo.toml", "[dependencies]\nstrum = \"0.28.0\"\n"),
        ];
        let found = versions_declared_in_several_manifests(&split);
        assert_eq!(
            found.keys().collect::<Vec<_>>(),
            vec!["strum"],
            "a crate versioned in two manifests is exactly what this must report"
        );

        let unified = [
            fixture(
                "a/Cargo.toml",
                "[workspace.dependencies]\nstrum = \"0.28.0\"\n\n[dependencies]\nstrum.workspace = true\n",
            ),
            fixture("b/Cargo.toml", "[dependencies]\nstrum.workspace = true\n"),
        ];
        assert!(
            versions_declared_in_several_manifests(&unified).is_empty(),
            "the same crate inherited from the shared table is not a split"
        );
    }

    /// The other negative control: hoisting is decorative if a member
    /// may still name its own version, since cargo takes the member's.
    #[test]
    fn test_detector_catches_a_shadowed_workspace_entry() {
        let shadowed = [
            fixture(
                "a/Cargo.toml",
                "[workspace.dependencies]\nstrum = \"0.28.0\"\n\n[dependencies]\nstrum = \"0.27\"\n",
            ),
            fixture("b/Cargo.toml", "[dependencies]\nstrum.workspace = true\n"),
        ];
        assert_eq!(
            workspace_entries_overridden(&shadowed).keys().collect::<Vec<_>>(),
            vec!["strum"]
        );
    }

    /// The member list is read out of `[workspace] members`, so this
    /// pins that the read worked rather than pinning the membership
    /// itself: a fifth crate should extend the checks below on the
    /// day it is added, not fail here.
    #[test]
    fn test_workspace_member_list_is_read_from_the_root_manifest() {
        let manifests = member_manifests();
        assert_eq!(
            manifests.first().map(String::as_str),
            Some("Cargo.toml"),
            "the root manifest is a package too and must be checked as one"
        );
        for expected in [
            "lib/baumhard/Cargo.toml",
            "lib/mandala_derive/Cargo.toml",
            "crates/maptool/Cargo.toml",
        ] {
            assert!(
                manifests.iter().any(|m| m == expected),
                "`[workspace] members` must have yielded {expected}; got {manifests:?}"
            );
        }
    }

    /// Exactly how many declarations each manifest holds.
    ///
    /// An exact count rather than a floor, and per manifest rather
    /// than in total, because the failure this guards against is one
    /// declaration going *unread* — a shape the parser skips deletes
    /// itself from every rule below, and the loose `> 30` floor this
    /// replaces could not have noticed thirteen of them vanishing,
    /// let alone one. The cost is that adding or removing a
    /// dependency updates a number here, which is the intended
    /// trade: manifest edits in this workspace are rare and are
    /// exactly what these tests exist to look at.
    const DECLARATIONS_PER_MANIFEST: [(&str, usize); 4] = [
        ("Cargo.toml", 23),
        ("crates/maptool/Cargo.toml", 4),
        ("lib/baumhard/Cargo.toml", 24),
        ("lib/mandala_derive/Cargo.toml", 3),
    ];

    /// A parse that found nothing would make every rule below vacuous.
    /// Pinned against the shared table specifically, since that is the
    /// structure the rules are about.
    #[test]
    fn test_the_real_manifests_parse_into_something_worth_checking() {
        let manifests = workspace_manifests();
        let found: BTreeMap<&str, usize> = manifests
            .iter()
            .map(|m| (m.path.as_str(), m.member_deps().count()))
            .collect();
        let expected: BTreeMap<&str, usize> = DECLARATIONS_PER_MANIFEST.into_iter().collect();
        assert_eq!(
            found, expected,
            "the number of dependency declarations `util::manifests` reads out of the \
             workspace has changed. If you added or removed a dependency, update \
             `DECLARATIONS_PER_MANIFEST`. If you did not, the reader has stopped \
             understanding a manifest and every rule below is now checking a smaller \
             set than it appears to."
        );

        let shared = workspace_table(&manifests);
        assert!(
            shared.contains("strum"),
            "`strum` — the crate whose 0.27/0.28 split motivated \
             `[workspace.dependencies]` — must be declared there; found {shared:?}"
        );
    }

    /// The headline rule: one version, written once.
    #[test]
    fn test_no_dependency_version_is_written_in_two_manifests() {
        let manifests = workspace_manifests();
        let split = versions_declared_in_several_manifests(&manifests);
        assert!(
            split.is_empty(),
            "these dependencies name a version in more than one manifest, which is \
             how `strum` ended up at 0.27 and 0.28 at the same time: {split:#?}. Move \
             each to `[workspace.dependencies]` in the root Cargo.toml and write \
             `dep.workspace = true` at every use site — per-crate `features` still \
             work, they are additive on top of the shared entry."
        );
    }

    /// ...and not un-written by a member that names its own.
    #[test]
    fn test_workspace_dependencies_are_not_shadowed_by_members() {
        let manifests = workspace_manifests();
        let overridden = workspace_entries_overridden(&manifests);
        assert!(
            overridden.is_empty(),
            "these members give a version to a crate `[workspace.dependencies]` \
             already versions, which cargo resolves in the member's favor and leaves \
             the shared entry as decoration: {overridden:#?}. Write \
             `dep.workspace = true` instead; add `features` beside it if the crate \
             needs more than the shared entry declares."
        );
    }

    /// The table holds the shared set and nothing else. A single-user
    /// entry separates a version from its one use site, which is a
    /// lookup for no invariant.
    #[test]
    fn test_workspace_table_carries_only_shared_dependencies() {
        let manifests = workspace_manifests();
        let consumers = workspace_entry_consumers(&manifests);
        let shared = workspace_table(&manifests);

        let solo: BTreeMap<_, _> = consumers
            .iter()
            .filter(|(name, who)| shared.contains(*name) && who.len() < 2)
            .collect();
        assert!(
            solo.is_empty(),
            "`[workspace.dependencies]` should hold exactly the crates two or more \
             members need; these have fewer: {solo:#?}. Move each back into the one \
             manifest that uses it — or, if a second member has just stopped using \
             it, that is the change to notice here."
        );
    }

    /// Every `dep.workspace = true` names a real entry. Cargo would
    /// also refuse this, but reaching it through the reader proves the
    /// reader is seeing both halves of the relationship rather than
    /// agreeing with itself.
    #[test]
    fn test_every_workspace_inheritance_resolves() {
        let manifests = workspace_manifests();
        let shared = workspace_table(&manifests);
        let unresolved: BTreeMap<_, _> = workspace_entry_consumers(&manifests)
            .into_iter()
            .filter(|(name, _)| !shared.contains(name))
            .collect();
        assert!(
            unresolved.is_empty(),
            "these members inherit a dependency the root \
             `[workspace.dependencies]` table does not declare: {unresolved:#?}"
        );
    }
}
