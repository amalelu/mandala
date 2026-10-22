// SPDX-License-Identifier: MPL-2.0

//! Differential oracle over the whole console grammar.
//!
//! Three views of every verb — what it accepts, what it rejects and
//! with which exact words, and what its completion popup offers —
//! recorded as data so a refactor of the machinery underneath is
//! answerable in one place. The corpus lives in
//! [`super::oracle_corpus`]; the pinned answers in
//! [`super::oracle_expected`]; the comparison is the three tests at
//! the bottom of this file.
//!
//! This exists because the console's grammar was hand-written two
//! to four times per verb (parse, complete, usage, hints) and the
//! copies drifted. The oracle is what makes "the framework changed
//! nothing a user can see" a checkable claim rather than a hope.
//!
//! # What the oracle does not see
//!
//! [`exec_signature`] renders the [`ExecResult`] and nothing else.
//! It drops the [`ConsoleEffects`] the verb filled in — the side
//! effect, the `close_console` flag, the undo entry — and it drops
//! the mutated document, which it then throws away. So a row
//! pinned as `"OK …"` says the verb accepted the line and worded
//! its acknowledgment that way; it does not say a write landed,
//! and the six rows whose verbs answer `ExecResult::ok_empty()`
//! (`color pick|bg|text|border`, `color picker on|off`) pin
//! nothing beyond `"OK "` itself — for those the side effect the
//! signature discards *is* the whole behavior.
//!
//! That is the deliberate scope: accepts / rejects with which
//! words / what the popup offers. It is not a substitute for the
//! per-verb tests that assert against the document, and a green
//! oracle is not evidence that a mutation is correct. Widening the
//! signature to cover effects and document state would make it
//! one, and is the obvious next move on this file.

use crate::application::console::completion::complete;
use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::tests_common::{load_test_doc, pinned_two_section_node};
use crate::application::document::{EdgeLabelSel, EdgeRef, MindMapDocument, SectionSel, SelectionState};

/// Selection fixtures the corpus runs each line under.
#[derive(Clone, Copy)]
pub enum Sel {
    None,
    Node,
    TwoSectionNode,
    Section,
    Multi,
    Edge,
    EdgeLabel,
}

pub fn doc_for(sel: Sel) -> MindMapDocument {
    match sel {
        Sel::None => load_test_doc(),
        Sel::Node => {
            let mut doc = load_test_doc();
            let id = doc.mindmap.nodes.keys().min().cloned().expect("nodes");
            doc.selection = SelectionState::Single(id);
            doc
        }
        Sel::TwoSectionNode => {
            let (mut doc, id) = pinned_two_section_node();
            doc.selection = SelectionState::Single(id);
            doc
        }
        Sel::Section => {
            let (mut doc, id) = pinned_two_section_node();
            doc.selection = SelectionState::Section(SectionSel {
                node_id: id,
                section_idx: 0,
            });
            doc
        }
        Sel::Multi => {
            let mut doc = load_test_doc();
            let mut ids: Vec<String> = doc.mindmap.nodes.keys().cloned().collect();
            ids.sort();
            ids.truncate(2);
            doc.selection = SelectionState::Multi(ids);
            doc
        }
        Sel::Edge => {
            let mut doc = load_test_doc();
            let e = doc.mindmap.edges.first().expect("edges").clone();
            doc.selection = SelectionState::Edge(EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type));
            doc
        }
        Sel::EdgeLabel => {
            let mut doc = load_test_doc();
            let e = doc.mindmap.edges.first().expect("edges").clone();
            doc.selection = SelectionState::EdgeLabel(EdgeLabelSel::new(EdgeRef::new(
                &e.from_id,
                &e.to_id,
                &e.edge_type,
            )));
            doc
        }
    }
}

/// Run one line and render its outcome as a single stable string.
pub fn exec_signature(line: &str, sel: Sel) -> String {
    let mut doc = doc_for(sel);
    let (cmd, tokens) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => return "PARSE-EMPTY".to_string(),
        ParseResult::Unknown(s) => return format!("PARSE-UNKNOWN {s}"),
    };
    let mut eff = ConsoleEffects::new(&mut doc);
    let res = (cmd.execute)(&Args::new(&tokens), &mut eff);
    match res {
        ExecResult::Ok(m) => format!("OK {}", escape(&m)),
        ExecResult::Lines(ls) => format!(
            "LINES {}",
            escape(&ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"))
        ),
        ExecResult::Err(m) => format!("ERR {}", escape(&m)),
    }
}

/// Run `lines` in order against **one** document and render the
/// whole interaction as a single stable string.
///
/// [`exec_signature`] throws its document away, which is what makes
/// an `EXEC_CORPUS` row independent of its neighbors — and what puts
/// every outcome that needs a *prior* line out of the oracle's reach.
/// `border preview commit` prints its auto-promotion note only when a
/// preview is active, so the note it hand-wrote for its own use went
/// unpinned through the commit that claimed to have written the
/// border family's closing move once.
pub fn exec_sequence_signature(lines: &[&str], sel: Sel) -> String {
    let mut doc = doc_for(sel);
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for line in lines {
        let (cmd, tokens) = match parse(line) {
            ParseResult::Ok { cmd, args } => (cmd, args),
            ParseResult::Empty => {
                out.push("PARSE-EMPTY".to_string());
                continue;
            }
            ParseResult::Unknown(s) => {
                out.push(format!("PARSE-UNKNOWN {s}"));
                continue;
            }
        };
        let mut eff = ConsoleEffects::new(&mut doc);
        let res = (cmd.execute)(&Args::new(&tokens), &mut eff);
        out.push(match res {
            ExecResult::Ok(m) => format!("OK {}", escape(&m)),
            ExecResult::Lines(ls) => format!(
                "LINES {}",
                escape(&ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"))
            ),
            ExecResult::Err(m) => format!("ERR {}", escape(&m)),
        });
    }
    out.join(" ;; ")
}

/// Render the completion popup for `line` (cursor at end) as a
/// single stable string.
pub fn completion_signature(line: &str, sel: Sel) -> String {
    let doc = doc_for(sel);
    let ctx = ConsoleContext::from_document(&doc);
    let out = complete(line, line.len(), &ctx);
    if out.is_empty() {
        return "-".to_string();
    }
    out.iter()
        .map(|c| {
            let mut s = c.text.clone();
            if c.display != c.text {
                s.push_str(&format!("/{}", c.display));
            }
            if let Some(h) = &c.hint {
                s.push_str(&format!(" [{}]", h));
            }
            if let Some(f) = &c.font_family {
                s.push_str(&format!(" <{}>", f));
            }
            escape(&s)
        })
        .collect::<Vec<_>>()
        .join(" ; ")
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}

/// Every corpus line's outcome still reads exactly the way it was
/// pinned. Positional: expectation N answers corpus line N.
#[test]
fn test_console_oracle_execution_outcomes_are_unchanged() {
    let corpus = super::oracle_corpus::EXEC_CORPUS;
    let expected = super::oracle_expected::EXEC_EXPECTED;
    assert_eq!(
        corpus.len(),
        expected.len(),
        "EXEC_CORPUS and EXEC_EXPECTED must stay the same length"
    );
    for (i, ((sel, line), want)) in corpus.iter().zip(expected).enumerate() {
        assert_eq!(
            &exec_signature(line, *sel),
            want,
            "oracle row {i} drifted for input {line:?}"
        );
    }
}

/// The one outcome that ends in bytes this machine chose still
/// *opens* the way it was pinned — the pinned string is a prefix
/// of the signature rather than the whole of it, and stops where
/// the console stops and `strerror_r` begins. See
/// [`super::oracle_corpus::EXEC_PREFIX_CORPUS`], which also
/// records why the three `border show` rows that used to sit
/// beside it are pinned whole instead: a prefix is the weaker
/// check, so it is spent on the row that needs it and on no
/// other.
#[test]
fn test_console_oracle_locale_bearing_readout_opens_unchanged() {
    let corpus = super::oracle_corpus::EXEC_PREFIX_CORPUS;
    let expected = super::oracle_expected::EXEC_PREFIX_EXPECTED;
    assert_eq!(corpus.len(), expected.len());
    for (i, ((sel, line), want)) in corpus.iter().zip(expected).enumerate() {
        let got = exec_signature(line, *sel);
        assert!(
            got.starts_with(want),
            "oracle prefix row {i} drifted for input {line:?}\n  want prefix: {want}\n  got:         {got}"
        );
    }
}

/// Every stateful sequence still reads exactly the way it was
/// pinned, line by line.
///
/// The rows here exist for the branches a fresh document cannot
/// reach — chiefly the border family's commit tail, which is
/// unreachable from any single line and was therefore the one copy of
/// the shared closing move that could drift unobserved.
#[test]
fn test_console_oracle_stateful_sequences_are_unchanged() {
    let corpus = super::oracle_corpus::EXEC_SEQ_CORPUS;
    let expected = super::oracle_expected::EXEC_SEQ_EXPECTED;
    assert_eq!(
        corpus.len(),
        expected.len(),
        "EXEC_SEQ_CORPUS and EXEC_SEQ_EXPECTED must stay the same length"
    );
    for (i, ((sel, lines), want)) in corpus.iter().zip(expected).enumerate() {
        assert_eq!(
            &exec_sequence_signature(lines, *sel),
            want,
            "oracle sequence row {i} drifted for input {lines:?}"
        );
    }
}

/// Every corpus line's completion popup still offers exactly the
/// rows, hints and insert texts it was pinned with.
#[test]
fn test_console_oracle_completions_are_unchanged() {
    let corpus = super::oracle_corpus::COMPLETION_CORPUS;
    let expected = super::oracle_expected::COMPLETION_EXPECTED;
    assert_eq!(
        corpus.len(),
        expected.len(),
        "COMPLETION_CORPUS and COMPLETION_EXPECTED must stay the same length"
    );
    for (i, ((sel, line), want)) in corpus.iter().zip(expected).enumerate() {
        assert_eq!(
            &completion_signature(line, *sel),
            want,
            "oracle completion row {i} drifted for input {line:?}"
        );
    }
}

/// The three vocabularies the pinned tables deliberately leave
/// out — they enumerate the *host's* installed fonts, so a
/// byte-pin would assert something about the machine rather than
/// about the console. Pinned structurally instead.
///
/// There are three slots — `font set <TAB>`, `border font <TAB>`
/// and `border font=<TAB>` — and this loop drives all three. It
/// drove one of them for a while and said "the two vocabularies",
/// so a sentinel planted in `font.rs` alone was invisible here,
/// and `font set <TAB>` was absent from `COMPLETION_CORPUS`,
/// absent from `EXEC_PREFIX_CORPUS`, and absent from the fallback
/// that exists for exactly the rows those two leave out. Then it
/// drove two while the doc claimed three, `border font=<TAB>`
/// being covered by a per-verb test rather than structurally.
///
/// There is now one body behind all three (`font.rs`'s
/// [`crate::application::console::commands::font::font_family_completions`]);
/// there were two, and the quoting assertion below is what the
/// second one lacked, so 43 of this host's 77 families tab-accepted
/// into a line `border font` refused.
#[test]
fn test_console_oracle_font_vocabularies_track_the_loaded_families() {
    let families: Vec<&str> = baumhard::font::fonts::loaded_families_iter().collect();
    assert!(
        !families.is_empty(),
        "the font system must be initialized for this fixture"
    );

    let doc = doc_for(Sel::Node);
    let ctx = ConsoleContext::from_document(&doc);
    // The border slots carry one row the `font` verb's does not:
    // the `off` sentinel that drops the override, which `font set`
    // has no equivalent of. Everything else about the three is the
    // same body.
    for (line, sentinels) in [
        ("border font ", &["off"][..]),
        ("border font=", &["off"][..]),
        ("font set ", &[][..]),
    ] {
        let all = complete(line, line.len(), &ctx);
        let (rows, extra): (Vec<_>, Vec<_>) = all.into_iter().partition(|c| c.font_family.is_some());
        assert_eq!(
            extra.iter().map(|c| c.display.as_str()).collect::<Vec<_>>(),
            sentinels,
            "`{line}<TAB>` offers exactly its non-family rows"
        );
        assert_eq!(
            rows.iter().map(|c| c.display.as_str()).collect::<Vec<_>>(),
            families,
            "`{line}<TAB>` offers one row per loaded family"
        );
        // The shaped face is the fourth channel
        // `completion_signature` renders, and the only one no
        // pinned row exercises — every corpus line whose popup
        // carries a family is a font vocabulary, and those are
        // excluded from the byte-pins for the host-dependence
        // reason above. `is_some()` alone left the channel
        // effectively unchecked: replacing every family tag with
        // one constant sentinel kept all four oracle tests green.
        // The row must name *its own* family, not merely some
        // family.
        for c in &rows {
            assert_eq!(
                c.font_family.as_deref(),
                Some(c.display.as_str()),
                "each family row of `{line}<TAB>` shapes its own label"
            );
            // Tab-accept has to produce a line the verb accepts.
            // A family whose name carries whitespace is one token
            // only when quoted, so the inserted text quotes it and
            // the displayed label does not.
            let want = if c.display.chars().any(char::is_whitespace) {
                format!("\"{}\"", c.display)
            } else {
                c.display.clone()
            };
            assert_eq!(
                c.text, want,
                "`{line}<TAB>` must insert a parseable token for {:?}",
                c.display
            );
        }
    }

    match exec_signature("font list", Sel::Node).strip_prefix("LINES ") {
        Some(body) => assert_eq!(body.split("\\n").count(), families.len()),
        other => panic!("`font list` should emit Lines, got {other:?}"),
    }
}
