// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::util::rust_source`], the reader every
//! source-level pin in the workspace goes through.
//!
//! These are the tests that matter most in the module, because a
//! stripper that quietly does nothing turns every pin built on it
//! into a test that cannot fail — the exact failure
//! `crate::util::manifests` warns about for its own parser. Each
//! case below therefore asserts both halves: the comment is gone
//! *and* the code around it survived.

use crate::util::rust_source::{
    above_test_modules, blank_string_literals, braced_block_after, comment_text, production_code, statements,
    string_literals, strip_comments,
};
// Only the tree-wide sweep at the bottom of this file walks the
// filesystem, and that sweep is a plain `#[test]` rather than a
// `do_*()` body — so in a build without `cfg(test)` (the benchmark
// binary, `cargo doc`) rustc has already dropped every caller these
// two imports have.
#[cfg(test)]
use crate::util::doc_fixtures::repo_path;
#[cfg(test)]
use std::path::{Path, PathBuf};

/// The attribute header spellings that sit over an inline module in
/// this workspace, and whether each one makes it a test module that
/// [`above_test_modules`] must cut.
///
/// **Enumerated from the tree, not invented.** Every `true` row is a
/// shape some file here actually writes, and the row that made this
/// list necessary is the second one:
/// `#[cfg(test)] #[cfg(not(target_arch = "wasm32"))] mod tests {`,
/// the house idiom for a test module the browser build cannot
/// compile, with seven live occurrences — five inline — all under
/// `src/application/app/`. It was silently reported as production
/// code for as long as the recognizer wanted `mod` as the immediate
/// next token, which is what let a two-line shim revive a mutation
/// four separate pins had killed.
///
/// The `false` rows are the near-misses that must survive: a `cfg`
/// that has nothing to do with `test`, and the two combinator shapes
/// where `test` appears but does not have to hold.
///
/// `test_above_test_modules_knows_every_shape_in_this_tree` is what
/// keeps the `true` rows in step with the tree as it grows; this
/// list is what states the answer for each shape in one place.
const TEST_MODULE_HEADERS: &[(&str, bool)] = &[
    // The plain form: 111 inline occurrences.
    (concat!("#[cfg(", "test)]"), true),
    // `src/application/app/text_edit/editor.rs` and six others.
    (
        concat!("#[cfg(", "test)]\n#[cfg(not(target_arch = \"wasm32\"))]"),
        true,
    ),
    // The same thing said the other way round.
    (
        concat!("#[cfg(not(target_arch = \"wasm32\"))]\n#[cfg(", "test)]"),
        true,
    ),
    // `lib/baumhard/src/util/log/mod.rs` and five others in `util/`.
    ("#[cfg(all(test, not(target_arch = \"wasm32\")))]", true),
    // A non-`cfg` attribute in between — the shim's spelling.
    (concat!("#[cfg(", "test)]\n#[allow(dead_code)]"), true),
    // …and a run of them, including one whose argument holds a `]`.
    (
        concat!("#[cfg(", "test)]\n#[allow(dead_code)]\n#[doc = \"a ] bracket\"]"),
        true,
    ),
    // `all` nests.
    ("#[cfg(all(not(miri), all(test, unix)))]", true),
    // `any` holds only when every arm does.
    ("#[cfg(any(test, feature = \"x\"))]", false),
    ("#[cfg(any(test, all(test, unix)))]", true),
    // A `cfg` with no `test` in it at all cuts nothing…
    ("#[cfg(not(target_arch = \"wasm32\"))]", false),
    ("#[cfg(feature = \"testing\")]", false),
    // …and neither does one that is true precisely when `test` is
    // *not* set.
    ("#[cfg(not(test))]", false),
];

/// Attribute headers Rust accepts that **no file here writes**, and
/// which must cut anyway.
///
/// The counterpart to [`TEST_MODULE_HEADERS`], and the reason the two
/// lists are separate rather than one. That one is enumerated from
/// the tree and is kept honest by
/// `test_above_test_modules_knows_every_shape_in_this_tree`, which
/// sweeps every `.rs` file in the workspace. How many that is stays
/// out of this sentence on purpose: it was written down twice and
/// went stale twice, and the sweep asserts a floor rather than a
/// count, so nothing here was ever going to catch the drift.
/// A tree-wide sweep can only ever find shapes
/// that exist — and every hole this recognizer has had since that
/// sweep went green was reachable only by *writing* a new one. A shim
/// is written by whoever wants a pin to stop looking; they are not
/// restricted to the spellings already in the repository, and one
/// added space was the whole difference between `#[cfg(test)]`, which
/// cut, and `#[ cfg(test) ]`, which did not.
///
/// Rust's attribute grammar is a token grammar: whitespace — spaces
/// or newlines — is allowed between every pair of `#`, `!`, `[`,
/// `cfg`, `(`, the predicate, `)` and `]`. `rustc` accepting each of
/// these is what makes them holes rather than curiosities; every row
/// below was compiled before it was written down.
const LEGAL_UNWRITTEN_HEADERS: &[&str] = &[
    concat!("#[ cfg(", "test) ]"),
    concat!("#[cfg ", "(test)]"),
    concat!("# [cfg(", "test)]"),
    // No `concat!` split needed on this row: with a newline between
    // every token there is no `#[cfg(test)]` spelling in the file for
    // the sweep above to find.
    "#\n[\ncfg\n(\ntest\n)\n]",
    concat!("#[cfg(", "test)]\n# [allow(dead_code)]"),
];

/// Every comment shape in the workspace disappears; nothing that is
/// not a comment moves.
///
/// The `(input, must_not_contain, must_contain)` triple is the whole
/// contract: a stripper that returned `String::new()` would satisfy
/// the first column and fail the second, and one that returned its
/// input unchanged would do the reverse.
pub fn do_strip_comments_removes_only_comments() {
    let cases: &[(&str, &[&str], &[&str])] = &[
        // Line comments, including both doc forms — a stale needle
        // is likeliest to survive in a doc comment.
        ("let a = 1; // log::error!\n", &["log::error!"], &["let a = 1;"]),
        ("/// log::error!\nlet a = 1;", &["log::error!"], &["let a = 1;"]),
        ("//! log::error!\nlet a = 1;", &["log::error!"], &["let a = 1;"]),
        // Block comments, including the nested form Rust allows. A
        // stripper that stopped at the first `*/` would leave
        // `keep_me */` behind.
        ("a /* log::error! */ b", &["log::error!"], &["a", "b"]),
        (
            "a /* x /* log::error! */ keep_me */ b",
            &["log::error!", "keep_me"],
            &["a", "b"],
        ),
        // A `//` inside a string is not a comment. This is the case
        // that makes the pins possible at all: the needles are
        // string literals.
        (
            r#"log::error!("startup: {}", message); // gone"#,
            &["gone"],
            [r#"log::error!("startup: {}", message);"#].as_slice(),
        ),
        (r#"let u = "http://x"; // gone"#, &["gone"], &["http://x"]),
        // Raw strings: a backslash escapes nothing, and only the
        // matching hash run closes them.
        (r##"let s = r#"a // b "# ; // gone"##, &["gone"], &["a // b"]),
        (r#"let s = r"c:\path"; // gone"#, &["gone"], &[r"c:\path"]),
        // `'` opens a lifetime far more often than a literal, and
        // mistaking one for the other swallows the rest of the file.
        (
            "fn f<'a>(x: &'a str) -> &'a str { x } // gone",
            &["gone"],
            &["fn f<'a>(x: &'a str)"],
        ),
        ("let c = '/'; let d = 1; // gone", &["gone"], &["let d = 1;"]),
        ("let c = '\\''; let d = 1; // gone", &["gone"], &["let d = 1;"]),
        // A comment is replaced, not deleted, so the tokens either
        // side cannot fuse into one.
        ("a/*x*/b", &["x"], &["a b"]),
    ];
    // The replacement is exactly one space, in both comment shapes.
    // For a block comment that is load-bearing — `a/*x*/b` must not
    // become `ab` — and for a line comment it is the documented
    // contract, which the newline would otherwise make unobservable.
    assert_eq!(strip_comments("a // b\nc"), "a  \nc");
    assert_eq!(strip_comments("a/*x*/b"), "a b");

    for (input, absent, present) in cases {
        let stripped = strip_comments(input);
        for needle in *absent {
            assert!(
                !stripped.contains(needle),
                "{needle:?} survived stripping of {input:?}: {stripped:?}"
            );
        }
        for needle in *present {
            assert!(
                stripped.contains(needle),
                "{needle:?} was destroyed by stripping of {input:?}: {stripped:?}"
            );
        }
    }
}

/// Line count is preserved, so a byte offset into the output still
/// lands on the line it came from and a failure message can quote a
/// line number that means something.
pub fn do_strip_comments_preserves_line_count() {
    for input in [
        "a\n// one\nb\n",
        "a\n/* one\n two\n three */\nb\n",
        "/// doc\n/// doc\nfn f() {}\n",
        "",
        "no trailing newline // gone",
    ] {
        assert_eq!(
            strip_comments(input).lines().count(),
            input.lines().count(),
            "line count moved for {input:?}"
        );
    }
}

/// An unterminated comment or literal must terminate the scan rather
/// than run off the end of the string — a truncated file being read
/// mid-write is the realistic way this is reached.
pub fn do_strip_comments_survives_unterminated_input() {
    for input in [
        "a /* never closed",
        "a \"never closed",
        "a r#\"never closed",
        "a '",
    ] {
        let _ = strip_comments(input);
    }
    assert!(strip_comments("keep /* gone").contains("keep"));
    assert!(!strip_comments("keep /* gone").contains("gone"));
}

/// An *inline* test module ends the production half; nothing else
/// does.
///
/// The negative cases are the ones that matter, and each cost a real
/// pin before it was handled. Truncating early returns a shorter
/// string — in both cases here, an empty one — and an empty string
/// satisfies every negative assertion and no positive one: a scan
/// that silently cannot fail.
///
/// - `src/application/renderer/mod.rs` opens with a `#[cfg(test)]
///   use …` five hundred lines above the code its pin is about.
/// - `src/application/document/mod.rs` declares fourteen
///   `#[cfg(test)] mod tests_*;` **before its first real item**.
///   None of that text is in the file, so there is nothing to cut.
///
/// The *positive* cases are enumerated from this tree rather than
/// invented — see `TEST_MODULE_HEADERS` for where each spelling
/// occurs and `test_above_test_modules_knows_every_shape_in_this_tree`
/// for the sweep that keeps that enumeration honest as the tree
/// grows.
pub fn do_above_test_modules_cuts_at_the_module_only() {
    let attribute = concat!("#[cfg(", "test)]");

    let early_use = format!("{attribute}\nuse helper::thing;\nfn ships() {{ keep(); }}\n");
    assert!(
        above_test_modules(&early_use).contains("fn ships()"),
        "a test-only import must not truncate the file"
    );

    let helper = format!("{attribute}\nfn only_in_tests() {{}}\nfn ships() {{ keep(); }}\n");
    assert!(
        above_test_modules(&helper).contains("fn ships()"),
        "a test-only helper fn must not truncate the file"
    );

    let external =
        format!("{attribute}\nmod tests_nodes;\n{attribute}\nmod tests_edges;\nfn ships() {{ keep(); }}\n");
    assert!(
        above_test_modules(&external).contains("fn ships()"),
        "an external test-module declaration holds no text in this file and must not truncate it"
    );

    // `mod` and `pub` are matched as whole words. Neither shape below
    // is valid Rust — `modules { … }` is not an item, `pubmod` is not
    // a visibility — and that is exactly why they have to be
    // rejected: over-acceptance is the dangerous direction here,
    // because a recognizer looser than the grammar cuts where the
    // compiler sees no module and hides production code from every
    // pin built on it. `strip_word`'s boundary check is the whole of
    // what holds that line, and until this ran it was untested — a
    // mutant that assumed the boundary left the suite green.
    //
    // `pubmod` rather than the `pubfn` the helper's doc names,
    // because only a near-miss that still reaches a `mod` can change
    // the answer: `pubfn tests {` is rejected one token later by the
    // real `mod` that is not there.
    for near_miss in [
        "modules { fn hidden() { needle(); } }",
        "pubmod tests { fn hidden() { needle(); } }",
    ] {
        let src = format!("fn ships() {{ keep(); }}\n{attribute}\n{near_miss}\n");
        assert!(
            above_test_modules(&src).contains("needle()"),
            "{near_miss}: a keyword was matched as a prefix rather than as a whole word, so \
             text that is not a test module truncated the file"
        );
    }

    for module in [
        "mod tests {",
        "pub mod tests {",
        "pub(crate) mod tests {",
        "pub(super) mod tests {",
        "pub(in crate::application::console) mod tests {",
        // Legal, unwritten here, and each one a shim's spelling.
        // `r#` stopped the old ASCII-word name scan dead: it read
        // `"#tests_shim {…"`, failed `starts_with('{')`, and reported
        // the module as production code.
        "mod r#tests_shim {",
        "pub mod r#tests {",
        "mod tests\n{",
        "pub (crate) mod tests {",
    ] {
        let src =
            format!("fn ships() {{ keep(); }}\n{attribute}\n{module}\n  fn hidden() {{ needle(); }}\n}}\n");
        let production = above_test_modules(&src);
        assert!(
            production.contains("fn ships()"),
            "{module}: production code was cut away"
        );
        assert!(
            !production.contains("needle()"),
            "{module}: the test module survived"
        );
    }

    // Every attribute *header* this tree writes over an inline test
    // module, positive and negative. The header is the whole risk:
    // requiring `mod` as the immediate next token silently reported
    // seven live test modules as production code.
    for (header, cuts) in TEST_MODULE_HEADERS {
        let src =
            format!("fn ships() {{ keep(); }}\n{header}\nmod tests {{\n  fn hidden() {{ needle(); }}\n}}\n");
        let production = above_test_modules(&src);
        assert!(
            production.contains("fn ships()"),
            "{header}: production code was cut away"
        );
        assert_eq!(
            !production.contains("needle()"),
            *cuts,
            "{header}: expected the module to {} cut, and it was not. Production seen: {production:?}",
            if *cuts { "be" } else { "not be" }
        );
    }

    // The spellings Rust accepts and this tree does not write. A
    // sweep over the tree cannot reach these; a shim author can.
    for header in LEGAL_UNWRITTEN_HEADERS {
        let src =
            format!("fn ships() {{ keep(); }}\n{header}\nmod tests {{\n  fn hidden() {{ needle(); }}\n}}\n");
        let production = above_test_modules(&src);
        assert!(
            production.contains("fn ships()"),
            "{header:?}: production code was cut away"
        );
        assert!(
            !production.contains("needle()"),
            "{header:?} is a legal spelling of a test gate and the module under it \
             survived as production code. Production seen: {production:?}"
        );
    }

    // The near-miss the whitespace tolerance must not swallow:
    // `cfg_attr` is a different attribute, and the `_` after the word
    // is what says so.
    let cfg_attr = format!(
        "fn ships() {{ keep(); }}\n#[{}_attr(test, allow(dead_code))]\nmod helpers {{ fn kept() {{ needle(); }} }}\n",
        "cfg"
    );
    assert!(
        above_test_modules(&cfg_attr).contains("needle()"),
        "`cfg_attr` is not a test gate and must not cut: {cfg_attr:?}"
    );

    // A `]` inside an attribute's own string argument does not end
    // the attribute, so the `mod` after it is still found.
    let quoted_bracket =
        format!("fn ships() {{ keep(); }}\n{attribute}\n#[doc = \"]\"]\nmod tests {{ fn hidden() {{ needle(); }} }}\n");
    let production = above_test_modules(&quoted_bracket);
    assert!(production.contains("fn ships()"));
    assert!(
        !production.contains("needle()"),
        "a `]` inside an attribute argument hid the module: {production:?}"
    );

    // Declarations first, then an inline module: the inline one is
    // still the cut, and the declarations above it do not move it.
    let mixed = format!(
        "{attribute}\nmod tests_nodes;\nfn ships() {{ keep(); }}\n{attribute}\nmod tests {{\n  fn hidden() {{ needle(); }}\n}}\n"
    );
    let production = above_test_modules(&mixed);
    assert!(
        production.contains("fn ships()"),
        "mixed: production code was cut away"
    );
    assert!(
        !production.contains("needle()"),
        "mixed: the inline module survived"
    );

    // No test module at all: the whole file is production.
    assert_eq!(above_test_modules("fn ships() {}"), "fn ships() {}");
}

/// The body of one item, brace-matched — and a brace inside a string
/// does not close it early. Without that, pinning a statement to a
/// function whose body formats `"{}"` would read the wrong span.
pub fn do_braced_block_after_matches_one_item() {
    let src = "fn a() { one(); }\nfn adopt(s: S) -> D { log::error!(\"x: {}\", m); d }\nfn b() { two(); }";
    let body = braced_block_after(src, "fn adopt(").expect("adopt is present");
    assert!(
        body.contains("log::error!"),
        "body missed its statement: {body:?}"
    );
    assert!(!body.contains("one()"), "body reached backwards: {body:?}");
    assert!(!body.contains("two()"), "body ran past its close: {body:?}");

    // Nested blocks close in the right order.
    let nested = "fn f() { if x { inner(); } tail(); }\nfn g() { other(); }";
    let body = braced_block_after(nested, "fn f(").expect("f is present");
    assert!(body.contains("inner();") && body.contains("tail();"));
    assert!(!body.contains("other();"), "nesting mis-counted: {body:?}");

    // A closing brace inside a literal is not a closing brace.
    let literal = r#"fn f() { let s = "}"; let c = '}'; tail(); }"#;
    let body = braced_block_after(literal, "fn f(").expect("f is present");
    assert!(
        body.contains("tail();"),
        "a literal brace closed the block: {body:?}"
    );

    assert!(braced_block_after(src, "fn absent(").is_none());
}

/// Every literal in the source comes back, with its delimiters gone
/// and its contents untouched; nothing that is not a literal does.
///
/// The consumer is a test *deriving* a list the code owns — the seven
/// messages `startup_load::fetch_map_json` can return — so both
/// directions are failures with teeth: a missed literal shortens the
/// list the caller then checks, and a spurious one adds a message the
/// code never emits.
pub fn do_string_literals_returns_every_literal() {
    let cases: &[(&str, &[&str])] = &[
        // The ordinary case, in source order.
        (
            r#"Err(format!("HTTP {} {}", a, b)).or("bare")"#,
            &["HTTP {} {}", "bare"],
        ),
        // An escaped quote does not end the literal, and the contents
        // come back exactly as written — un-escaping them would stop
        // the caller from matching source text against source text.
        (r#"let s = "a \" b"; let t = "c";"#, &[r#"a \" b"#, "c"]),
        // Raw strings: the hashes go, the inner quotes stay.
        (r####"let s = r#"a "b" c"#;"####, &[r#"a "b" c"#]),
        (r#"let s = r"c:\path";"#, &[r"c:\path"]),
        (r#"let s = b"bytes";"#, &["bytes"]),
        // A lifetime is not a literal, and neither is a `'"'` char —
        // but the quote inside that char literal must not open one.
        (
            r#"fn f<'a>(x: &'a str) { let q = '"'; let s = "kept"; }"#,
            &["kept"],
        ),
        // An empty literal is a literal.
        (r#"let s = "";"#, &[""]),
        // No literals at all.
        ("fn f() { g(); }", &[]),
        // Unterminated: everything to the end, and no panic.
        (r#"let s = "never closed"#, &["never closed"]),
    ];
    for (source, expected) in cases {
        assert_eq!(
            &string_literals(source),
            expected,
            "string_literals disagreed on {source:?}"
        );
    }
}

/// A literal's contents go, its delimiters and the code around it
/// stay, and every byte offset lands where it did.
///
/// The consumer is the §B8 bench-surface gate, which matches
/// identifiers against the bench file's source text: a criterion id
/// is a string literal, so `"do_x"` written as an id is the name
/// `do_x` in the file's bytes and resolves to nothing. Both
/// directions have teeth. Blanking too little leaves that false
/// green in place; blanking too much erases the `b.iter(do_x)` that
/// is the entry, and the gate then reports every body in the tree.
///
/// The offset assertion is not decoration: the caller finds a
/// `bench_function(` call's extent in the blanked text and reads the
/// id out of the *unblanked* text at those offsets, so a single byte
/// of drift hands it the wrong id.
pub fn do_blank_string_literals_keeps_code_and_offsets() {
    let cases: &[(&str, &str)] = &[
        // The contract, in miniature: the id is gone, the call and
        // the identifier it runs are not.
        (
            r#"c.bench_function("do_x", |b| b.iter(do_y));"#,
            r#"c.bench_function("    ", |b| b.iter(do_y));"#,
        ),
        // An escaped quote does not end the literal, so the code
        // after it is not blanked as though it were inside one.
        (r#"let s = "a \" b"; f(do_z);"#, r#"let s = "      "; f(do_z);"#),
        // Raw strings: hashes and quotes stay, contents go.
        (
            r####"let s = r#"a "b" c"#; g();"####,
            r####"let s = r#"       "#; g();"####,
        ),
        (r#"let s = b"bytes"; h();"#, r#"let s = b"     "; h();"#),
        // A `'"'` char literal must not open a string, and a `'a`
        // lifetime must not either.
        (
            r#"fn f<'a>(x: &'a str) { let q = '"'; call(do_w); }"#,
            r#"fn f<'a>(x: &'a str) { let q = '"'; call(do_w); }"#,
        ),
        // An empty literal has nothing to blank.
        (r#"let s = ""; k();"#, r#"let s = ""; k();"#),
        // Unterminated: everything to the end goes, and no panic.
        (r#"let s = "never closed"#, r#"let s = "            "#),
    ];
    for (source, expected) in cases {
        let blanked = blank_string_literals(source);
        assert_eq!(
            &blanked, expected,
            "blank_string_literals disagreed on {source:?}"
        );
        assert_eq!(
            blanked.len(),
            source.len(),
            "byte offsets moved on {source:?}: {} bytes became {}",
            source.len(),
            blanked.len()
        );
    }

    // A multi-byte char inside a literal is blanked one space per
    // *byte*, or every offset after it shifts. `é` is two bytes and
    // `→` is three.
    let wide = "let s = \"é→\"; f(do_v);";
    let blanked = blank_string_literals(wide);
    assert_eq!(blanked, "let s = \"     \"; f(do_v);");
    assert_eq!(
        blanked.len(),
        wide.len(),
        "a multi-byte literal moved the offsets"
    );

    // Newlines inside a raw string survive, so a line number in the
    // blanked text is still a line number in the original.
    let spanning = "let s = r\"one\ntwo\";\nf(do_u);";
    let blanked = blank_string_literals(spanning);
    assert_eq!(blanked.lines().count(), spanning.lines().count());
    assert_eq!(blanked.len(), spanning.len());
    assert!(
        blanked.contains("do_u"),
        "code after a spanning literal was eaten"
    );
}

/// The prose of every comment shape survives; no byte of code does;
/// and the offsets do not move.
///
/// The consumer is `crate::util::doc_commands`'s repository-wide
/// `--benches` scan, which reads what this repository's comments say
/// about how to build it. Both directions have teeth, and both were
/// reached in practice. Keeping too little is the silent one: the
/// reader this replaced blanked string literals in source that still
/// carried its comments, so a single unbalanced `"` in prose paired
/// with the next `"` below and blanked every byte between them,
/// invocation included. Keeping too much reinstates the fixture
/// problem — the commands that scan forbids are written as string
/// literals in its own tests, so a reader that returned its input
/// would fail on its own test data.
pub fn do_comment_text_keeps_comments_and_drops_code() {
    let cases: &[(&str, &str)] = &[
        // Every comment marker is blanked with its prose left in
        // place, so a `///` cannot be read as a token of what the
        // comment says.
        ("let a = 1; // note\n", "              note\n"),
        ("/// note\nlet a = 1;", "    note\n          "),
        ("//! note\nlet a = 1;", "    note\n          "),
        // A block comment is prose, nested markers and all.
        ("a /* note */ b", "     note     "),
        ("a /* x /* note */ keep */ b", "     x    note    keep     "),
        // A `//` inside a literal opens no comment. This is the case
        // that makes the scan possible: its fixtures are literals
        // holding the command it forbids.
        (r#"let u = "http://x"; // gone"#, "                       gone"),
        (
            r##"let s = r#"a // b "#; // gone"##,
            "                         gone",
        ),
        // A `"` inside a comment opens no literal — the defect this
        // reader exists to close. Under a literal-blanking pass the
        // second line vanished with the first.
        ("// a \" quote\n// note", "   a \" quote\n   note"),
        // A `'` that opens a lifetime rather than a character
        // literal must not swallow the comment after it.
        (
            "fn f<'a>(x: &'a str) {} // note",
            "                           note",
        ),
        // An unterminated block comment is prose to the end rather
        // than a scan that runs off the string.
        ("a /* note", "     note"),
    ];
    for (source, expected) in cases {
        let prose = comment_text(source);
        assert_eq!(&prose, expected, "comment_text disagreed on {source:?}");
        assert_eq!(
            prose.len(),
            source.len(),
            "byte offsets moved on {source:?}: {} bytes became {}",
            source.len(),
            prose.len()
        );
        assert_eq!(
            prose.lines().count(),
            source.lines().count(),
            "line numbering moved on {source:?}"
        );
    }

    // A multi-byte char in blanked code is one space per *byte*, as
    // in `blank_string_literals`, or every offset after it shifts.
    let wide = "let s = \"é→\"; // note";
    assert_eq!(comment_text(wide).len(), wide.len());
    // ... and one in comment prose is left alone, so what a comment
    // says is what comes back.
    assert_eq!(comment_text("// é→"), "   é→");
}

/// A statement ends at a `;`, a `{` or a `}` that is not inside
/// parentheses — and nowhere else.
///
/// The first case is the one that cost a mutant its life: two
/// expression-bodied functions in a row, which a `split(';')` hands
/// back as a single chunk carrying both of their tails.
pub fn do_statements_split_at_the_right_places() {
    // `renderer/pipeline.rs`, reduced: neither body ends in `;`, so
    // the `unwrap()` and the `expect(` must not land in one piece.
    let two_bodies = "fn a() -> D { adapter.request_device(&D { label: None }).await.unwrap() }\n\
                      fn b() -> A { instance.request_adapter(&O { p: 1 }).await.expect(\"no adapter\") }";
    let pieces = statements(two_bodies);
    let device = pieces
        .iter()
        .find(|p| p.contains("request_device"))
        .expect("the device call is a statement of its own");
    assert!(
        !device.contains("expect("),
        "two expression-bodied fns merged into one statement: {device:?}"
    );
    assert!(
        device.contains("&D { label: None }"),
        "a struct literal argument was split off its call: {device:?}"
    );

    // A `;` inside parentheses is still a boundary only at depth 0,
    // and a brace inside a literal is not a boundary at all.
    let literal = r#"fn f() { log::error!("a; b {} c", x); tail(); }"#;
    let pieces = statements(literal);
    assert!(
        pieces.contains(&r#"log::error!("a; b {} c", x)"#),
        "a `;` or `{{` inside a string literal split the statement: {pieces:?}"
    );
    assert!(pieces.contains(&"tail()"));

    // Ordinary statements, and no empty pieces from the run of
    // `}` at the end.
    assert_eq!(
        statements("fn f() { let a = 1; let b = 2; }"),
        vec!["fn f()", "let a = 1", "let b = 2"]
    );
    assert!(statements("   \n  ").is_empty());
}

/// The reader, end to end, against a file that is in the tree and
/// stays there: this module's own source.
///
/// Two properties, both of which a naive `read_to_string` fails: the
/// module's prose — which quotes the very needles the pins search
/// for — does not reach the caller, and the code does.
pub fn do_production_code_returns_code_without_prose() {
    let code = production_code("lib/baumhard/src/util/rust_source.rs");
    assert!(
        code.contains("pub fn strip_comments(src: &str) -> String"),
        "production_code dropped the code it was asked for"
    );
    assert!(
        !code.contains("A **string literal** in production code"),
        "production_code leaked its own module documentation"
    );
    // The module doc quotes this needle in prose. If prose reached
    // the caller, every pin built on this reader would be satisfiable
    // by a comment — the exact hazard the module exists to close.
    assert!(
        !code.contains(r#"log::error!("startup: {}", message)"#),
        "a needle quoted in a doc comment reached the caller"
    );
}

/// **No test module in this tree is reported as production code.**
///
/// The unit cases above are the shapes someone thought of. This is
/// the one that answers the question they cannot: *is there a shape
/// in this repository the recognizer does not know?* A shape present
/// here that [`above_test_modules`] misses is a live hole, not a
/// hypothetical — the reviewer who found the last one used the
/// workspace's own `#[cfg(test)] #[cfg(not(target_arch = "wasm32"))]
/// mod tests {` idiom, seven live occurrences, and every pin built on
/// this reader went quiet.
///
/// The property, over every `.rs` file in the workspace: **the
/// production half holds no `#[test]`.** A `#[test]` is the marker
/// only test code carries, so one that survives the cut is a test
/// module a pin would be free to read.
///
/// Two classes of file are test code *in their entirety*, so there is
/// no module inside them to cut and the whole file legitimately comes
/// back. Both are derived from the tree rather than listed, because a
/// hand-written exclusion list is exactly the thing that goes stale
/// while looking maintained:
///
/// - **Reached only under a `test` `cfg`.** `mod tests_nodes;` and
///   friends, resolved through the parent module that declares them —
///   see `reached_only_under_test_cfg`, which asks
///   [`above_test_modules`] the question rather than re-deciding it.
/// - **Reached through a module named `tests`.** Baumhard exposes its
///   test bodies as an un-gated `pub mod tests;` on purpose, so
///   `benches/test_bench.rs` can call the `do_*()` functions
///   (`lib/baumhard/CONVENTIONS.md` §B8). That carve-out is *checked*
///   rather than merely applied: it may only be what keeps a file out
///   under `lib/baumhard/`, since §B8 is a baumhard rule and an
///   un-gated `tests` module in the app crate would be a different
///   thing entirely.
///
/// **A plain `#[test]`, not a `do_*()` pair** — §B8 opt-out class 4,
/// the subject is the repository, and the one body in this module
/// that takes it. Every `do_*()` in a
/// baumhard tests tree has one `bench_function` entry, so the prefix
/// is a claim that a number for the body would mean something. This
/// one walks several hundred files per call and takes about a second
/// doing it: criterion would spend half a minute reporting the page
/// cache and the recognizer's own walk, neither of which ships. The
/// unit cases above are where the recognizer is measured; its
/// sibling [`do_production_code_returns_code_without_prose`] reads a
/// single file and is benched, because what a second call measures
/// there is the strip-and-cut over ~40 KB rather than the disk.
#[test]
fn test_above_test_modules_knows_every_shape_in_this_tree() {
    let root = repo_path(".");
    let sources = workspace_sources();
    assert!(
        sources.len() > 400,
        "the source walk found only {} files — it is not reaching the tree",
        sources.len()
    );

    let test_attribute = concat!("#[", "test]");
    let mut cut = 0usize;
    let mut all_test_code = 0usize;
    let mut leaked: Vec<String> = Vec::new();
    for path in &sources {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
        let stripped = strip_comments(&source);
        let production = above_test_modules(&stripped);
        if production.len() < stripped.len() {
            cut += 1;
        }
        let relative = path.strip_prefix(&root).unwrap_or(path).display().to_string();
        let named_tests = path.components().any(|c| c.as_os_str() == "tests");
        let gated = reached_only_under_test_cfg(path);
        if named_tests && !gated {
            assert!(
                relative.starts_with("lib/baumhard/"),
                "{relative} is kept out of this sweep only by the §B8 `pub mod tests;` \
                 carve-out, which is a baumhard rule — an un-gated `tests` module outside \
                 that crate is test code with nothing declaring it so"
            );
        }
        if named_tests || gated {
            all_test_code += 1;
            continue;
        }
        if production.contains(test_attribute) {
            leaked.push(relative);
        }
    }

    assert!(
        leaked.is_empty(),
        "these files ship production code and still hand a `#[test]` back to every pin \
         built on this reader — the recognizer does not know the shape they use:\n  {}",
        leaked.join("\n  ")
    );
    // Non-vacuity, both halves: the walk found test modules to cut,
    // and it found files that are all test code.
    assert!(cut > 100, "only {cut} files were cut — the recognizer went quiet");
    assert!(
        all_test_code > 50,
        "only {all_test_code} whole-file test sources were resolved — the parent-module walk \
         is not resolving declarations, and the sweep is passing because it excluded everything"
    );

    // The three real shapes, each named where it actually occurs, so
    // a regression says which one came back rather than only that
    // something did.
    for (relative, shape) in [
        (
            "src/application/app/startup_load.rs",
            concat!("#[cfg(", "test)] mod tests {"),
        ),
        (
            "src/application/app/text_edit/editor.rs",
            concat!(
                "#[cfg(",
                "test)] #[cfg(not(target_arch = \"wasm32\"))] mod tests {"
            ),
        ),
        (
            "lib/baumhard/src/util/log/mod.rs",
            "#[cfg(all(test, not(target_arch = \"wasm32\")))] mod tests {",
        ),
        ("src/application/macros/tests.rs", concat!("#![cfg(", "test)]")),
    ] {
        let source = std::fs::read_to_string(repo_path(relative))
            .unwrap_or_else(|e| panic!("{relative} must be readable: {e}"));
        let stripped = strip_comments(&source);
        assert!(
            stripped.contains(test_attribute),
            "{relative} no longer holds a `#[test]`, so it can no longer witness `{shape}`"
        );
        assert!(
            !above_test_modules(&stripped).contains(test_attribute),
            "`{shape}` is back: {relative} hands its test bodies to every pin that reads it"
        );
    }
}

// The filesystem walk below belongs to the tree-wide sweep alone,
// and that sweep is a plain `#[test]` — which rustc drops from any
// build without `cfg(test)`. Gating its helpers the same way keeps
// them from becoming dead code in the benchmark binary and under
// `cargo doc`, where nothing would call them.

/// Every `.rs` file under the workspace's three source roots, sorted
/// so a failure names the same file every run.
///
/// Read off the filesystem rather than off `[workspace] members`,
/// because the question is "what Rust is in this tree" and a file
/// nobody declares is exactly the kind of thing worth seeing.
#[cfg(test)]
fn workspace_sources() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in ["src", "lib", "crates"] {
        collect_rust_files(&repo_path(root), &mut found);
    }
    found.sort();
    found
}

/// Push every `.rs` file under `dir` into `out`, recursively.
/// Build output and dotted directories are skipped; nothing else is.
#[cfg(test)]
fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                collect_rust_files(&path, out);
            }
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

/// Whether `path`'s module — or any module above it — is declared
/// under a `cfg` that implies `test`, so the file is test code
/// through and through and has no inner module for
/// [`above_test_modules`] to cut.
///
/// **The recognizer answers this, rather than a second parser.** The
/// declaration is external (`mod tests_nodes;`), which
/// [`above_test_modules`] deliberately never cuts at, so the question
/// is put to it in the shape it does answer: take the parent's
/// production code, rewrite the declaration into `mod tests_nodes {
/// }`, and see whether the cut moves. Any attribute run in front of
/// the declaration is then read by exactly the code this sweep
/// exists to check — which is the point. A recognizer that stopped
/// seeing a gate would stop excluding the file and the sweep would
/// go *red*, never quiet.
///
/// The parent is searched in the two places Rust looks (`dir/mod.rs`
/// and `dir.rs`) plus the crate roots, and only files that exist are
/// followed, which is also what terminates the walk.
#[cfg(test)]
fn reached_only_under_test_cfg(path: &Path) -> bool {
    let Some((parents, name)) = parent_modules_of(path) else {
        return false;
    };
    for parent in parents {
        if parent == path || !parent.is_file() {
            continue;
        }
        if reached_only_under_test_cfg(&parent) {
            return true;
        }
        let source = std::fs::read_to_string(&parent)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", parent.display()));
        let production = above_test_modules(&strip_comments(&source)).to_string();
        let Some(at) = production.find(&format!("mod {name};")) else {
            continue;
        };
        let inlined = format!("{}mod {name} {{ }}", &production[..at]);
        if above_test_modules(&inlined).len() < inlined.len() {
            return true;
        }
    }
    false
}

/// The files that may declare `path` as a module, and the name it
/// would be declared under. `None` for a crate root, which nothing
/// declares.
#[cfg(test)]
fn parent_modules_of(path: &Path) -> Option<(Vec<PathBuf>, String)> {
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let directory = path.parent()?;
    let (name, from) = if stem == "mod" {
        // `a/b/mod.rs` is module `b`, declared one level further up.
        (
            directory.file_name()?.to_string_lossy().into_owned(),
            directory.parent()?.to_path_buf(),
        )
    } else if stem == "lib" || stem == "main" {
        return None;
    } else {
        (stem, directory.to_path_buf())
    };
    Some((
        vec![
            from.join("mod.rs"),
            from.with_extension("rs"),
            from.join("lib.rs"),
            from.join("main.rs"),
        ],
        name,
    ))
}

#[test]
fn test_strip_comments_removes_only_comments() {
    do_strip_comments_removes_only_comments();
}

#[test]
fn test_strip_comments_preserves_line_count() {
    do_strip_comments_preserves_line_count();
}

#[test]
fn test_strip_comments_survives_unterminated_input() {
    do_strip_comments_survives_unterminated_input();
}

#[test]
fn test_above_test_modules_cuts_at_the_module_only() {
    do_above_test_modules_cuts_at_the_module_only();
}

#[test]
fn test_braced_block_after_matches_one_item() {
    do_braced_block_after_matches_one_item();
}

#[test]
fn test_string_literals_returns_every_literal() {
    do_string_literals_returns_every_literal();
}

#[test]
fn test_blank_string_literals_keeps_code_and_offsets() {
    do_blank_string_literals_keeps_code_and_offsets();
}

#[test]
fn test_comment_text_keeps_comments_and_drops_code() {
    do_comment_text_keeps_comments_and_drops_code();
}

#[test]
fn test_statements_split_at_the_right_places() {
    do_statements_split_at_the_right_places();
}

#[test]
fn test_production_code_returns_code_without_prose() {
    do_production_code_returns_code_without_prose();
}
