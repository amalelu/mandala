// SPDX-License-Identifier: MPL-2.0

//! Registry-wide invariants over every declared [`Grammar`], and
//! over the registry's *use* of the engine that reads them.
//!
//! These are the checks that replace the per-verb assertions #128
//! had to write twice and called "a per-verb copy of a check, i.e.
//! more of the same disease". They walk the whole registry, every
//! nesting level, and hold the declaration against itself in both
//! directions — a form naming a key its level does not declare, and
//! a key no form prints. The second is the one that would have
//! caught `font range=`, `color range=` and `color section=` on the
//! day each was written: each was parseable, named in the verb's own
//! rejection, and absent from the list `help` printed.
//!
//! Holding the declaration against itself is not enough on its own,
//! because a verb can decline to ask. `label` printed a
//! hand-written usage literal naming two of the four keys it
//! declares, so its popup, its `help` page and its kv rejection all
//! said four while the verb itself said two — and every invariant
//! here stayed green, because every one of them read the table
//! rather than the verb.
//! [`test_no_console_verb_hand_writes_its_own_usage_line`] is the
//! direction that catches that: it reads the sources under
//! `commands/` instead of the registry.

use super::{Grammar, Subverb};
use crate::application::console::commands::{Command, COMMANDS};

/// Every level reachable from `root`, outermost first. Depth is
/// bounded by the declaration (four, at
/// `canvas section-frame focused preview`), so the recursion cannot
/// run away — a cycle would need a `Grammar` to name itself as its
/// own descendant, which `&'static` initialization order forbids.
fn levels(root: &'static Grammar, out: &mut Vec<&'static Grammar>) {
    out.push(root);
    for subverb in root.subverbs() {
        if let Some(child) = subverb.child {
            levels(child, out);
        }
    }
}

/// Every grammar level in the registry, paired with the command that
/// roots it — the one that owns its `help` page and its completion
/// entry point, whatever depth the level sits at.
fn all_levels_rooted() -> Vec<(&'static Command, &'static Grammar)> {
    let mut out = Vec::new();
    for cmd in COMMANDS {
        let mut mine = Vec::new();
        levels(cmd.grammar, &mut mine);
        out.extend(mine.into_iter().map(|g| (cmd, g)));
    }
    out
}

/// [`all_levels_rooted`], keyed by the rooting verb's name, so a
/// failure names something a reader can find.
fn all_levels() -> Vec<(&'static str, &'static Grammar)> {
    all_levels_rooted()
        .into_iter()
        .map(|(cmd, g)| (cmd.name, g))
        .collect()
}

/// Every key a form names is a key its level declares.
///
/// The failing input is a `Form` whose `required` or `optional`
/// carries a name absent from the level's `key_sets` — the shape a
/// rename produces, and the one that makes `help` print
/// `bogus=<value>` for a key the parse loop then rejects.
#[test]
fn test_every_form_key_is_declared_by_its_level() {
    for (verb, grammar) in all_levels() {
        let forms = grammar
            .subverbs()
            .flat_map(|s| s.forms.iter())
            .chain(grammar.bare.iter().flat_map(|b| b.forms.iter()));
        for form in forms {
            for name in form.names() {
                assert!(
                    grammar.key(name).is_some(),
                    "{verb}: level `{}` has a form naming key '{name}', which the level does not declare",
                    grammar.label
                );
            }
        }
    }
}

/// Every key a level declares is printed by at least one of its
/// forms.
///
/// This is the direction that catches the defect this epic exists
/// for. A key reachable by the parse loop and named by no form is a
/// key `help <verb>` never mentions and the popup offers only by
/// accident — `font range=` shipped that way, and so did
/// `color range=` after it.
#[test]
fn test_every_declared_key_is_printed_by_some_form() {
    for (verb, grammar) in all_levels() {
        for key in grammar.keys() {
            let printed = grammar
                .subverbs()
                .flat_map(|s| s.forms.iter())
                .chain(grammar.bare.iter().flat_map(|b| b.forms.iter()))
                .any(|form| form.names().any(|n| n == key.name));
            assert!(
                printed,
                "{verb}: level `{}` declares key '{}' that no form prints — \
                 it would be parseable and undocumented",
                grammar.label, key.name
            );
        }
    }
}

/// A subverb either descends into a child level or declares shapes
/// of its own, never both.
///
/// [`super::descent`] relies on it: a subverb with a child is a step
/// deeper and a subverb with forms is the end of the walk, so one
/// carrying both would have its slots and keys silently unreachable.
#[test]
fn test_no_subverb_both_descends_and_declares_forms() {
    for (verb, grammar) in all_levels() {
        for subverb in grammar.subverbs() {
            assert!(
                subverb.child.is_none() || subverb.forms.is_empty(),
                "{verb}: `{} {}` declares both a child level and shapes of its own",
                grammar.label,
                subverb.name
            );
        }
    }
}

/// A child level's label extends its parent's, so every message a
/// nested level prints is copy-pasteable at the depth it printed
/// from.
#[test]
fn test_every_child_label_extends_its_parent() {
    for (verb, grammar) in all_levels() {
        for subverb in grammar.subverbs() {
            let Some(child) = subverb.child else { continue };
            let want = format!("{} {}", grammar.label, subverb.name);
            assert_eq!(
                child.label, want,
                "{verb}: child level of `{}` labels itself '{}' rather than '{want}'",
                grammar.label, child.label
            );
        }
    }
}

/// Names are unique within a level, in both vocabularies.
///
/// A duplicate subverb name makes the second declaration
/// unreachable (`Grammar::subverb` takes the first match) while
/// still printing a usage line and a popup row for it; a duplicate
/// key name does the same to the value vocabulary.
#[test]
fn test_names_are_unique_within_a_level() {
    for (verb, grammar) in all_levels() {
        let mut seen: Vec<&'static str> = Vec::new();
        for subverb in grammar.subverbs() {
            assert!(
                !seen.contains(&subverb.name),
                "{verb}: level `{}` declares subverb '{}' twice",
                grammar.label,
                subverb.name
            );
            seen.push(subverb.name);
        }
        let mut seen: Vec<&'static str> = Vec::new();
        for key in grammar.keys() {
            assert!(
                !seen.contains(&key.name),
                "{verb}: level `{}` declares key '{}' twice",
                grammar.label,
                key.name
            );
            seen.push(key.name);
        }
    }
}

/// Every usage line a level publishes leads with that level's own
/// label, so a form lifted out of `help` and pasted back into the
/// console reaches the verb it documents.
#[test]
fn test_every_usage_form_leads_with_its_verb() {
    for cmd in COMMANDS {
        for form in super::usage::forms(cmd.grammar) {
            assert!(
                form.starts_with(cmd.name),
                "{}: usage form '{form}' does not lead with the verb name",
                cmd.name
            );
        }
    }
}

/// Every key **every form at every level** names is offered by the
/// popup at that form's own slot, and published by `help <verb>`.
///
/// This is the acceptance criterion of the whole engine stated as a
/// test: adding a kv key is one table row, and parse, complete, help
/// and hint follow. The `help` half is a mirror — `key_lines` derives
/// from the same declaration — so what it can catch is narrow. The
/// *popup* half is not: it builds the line the user would type, hands
/// it to the real completion engine, and so runs the descent, the
/// positional-vs-kv gate, the form narrowing and the readable-key
/// resolution.
///
/// It used to iterate `COMMANDS` and take `cmd.grammar.bare`: 14 of
/// the registry's 29 levels, and none of the 20 subverbs that read
/// keys — while calling itself the acceptance criterion of the whole
/// engine. The control that showed the gap: making
/// `complete::form_rows` withhold its `key_rows` at every level whose
/// label contains a space — `border preview`, `canvas border`,
/// `canvas section-frame [focused]`, `section frame` — left all ten
/// invariants in this file green, and only the pinned corpus noticed.
///
/// Reaching a nested level is what the level's `label` is for: it is
/// exactly the words that lead a line there
/// (`test_every_child_label_extends_its_parent` holds that), so
/// `canvas section-frame focused ` is constructible from the
/// declaration alone. A form's *required* slots are filled from their
/// own vocabularies first, because the engine withholds a form's keys
/// until they sit behind the cursor — `border palette off <TAB>`
/// offers `field=` where `border palette <TAB>` correctly does not.
/// Filling them from the slot's own vocabulary is also what keeps the
/// form eligible under [`super::Form::admits_prefix`], so this walk
/// asks each shape for its own keys rather than for the subverb's
/// union.
#[test]
fn test_every_form_key_reaches_the_popup_and_help() {
    let doc = crate::application::document::tests_common::load_test_doc();
    let ctx = crate::application::console::ConsoleContext::from_document(&doc);
    let mut probes = 0usize;
    let mut keys_seen = 0usize;
    for (cmd, grammar) in all_levels_rooted() {
        let published = cmd.key_lines();
        let mut check = |head: String, form: &super::Form| {
            probes += 1;
            let mut line = head;
            // Fill up to and including the last required slot; the
            // cursor then sits one slot past it, which is where the
            // form's keys come on offer.
            let fill_upto = form
                .slots
                .iter()
                .rposition(|slot| !slot.optional)
                .map_or(0, |i| i + 1);
            for slot in form.slots.iter().take(fill_upto) {
                line.push(' ');
                line.push_str(&slot_filler(&slot.vocab));
            }
            line.push(' ');
            let offered: Vec<String> =
                crate::application::console::completion::complete(&line, line.len(), &ctx)
                    .into_iter()
                    .map(|c| c.text)
                    .collect();
            for name in form.names() {
                keys_seen += 1;
                assert!(
                    offered.iter().any(|t| t == &format!("{}=", name)),
                    "{}: `{line}<TAB>` must offer '{name}='; got {offered:?}",
                    cmd.name
                );
                assert!(
                    published.iter().any(|l| l.starts_with(&format!("{}=", name))),
                    "{}: `help {}` must publish '{name}='; got {published:?}",
                    cmd.name,
                    cmd.name
                );
            }
        };
        if let Some(bare) = &grammar.bare {
            for form in bare.forms {
                check(grammar.label.to_string(), form);
            }
        }
        for subverb in grammar.subverbs() {
            for form in subverb.forms {
                check(format!("{} {}", grammar.label, subverb.name), form);
            }
        }
    }
    // The registry is 29 levels and 20 key-bearing subverbs; the
    // floors are below both so a declaration change does not fail the
    // run, and far enough above the 14 bare forms this walk used to
    // see that shrinking back to them would.
    assert!(
        probes >= 40 && keys_seen >= 150,
        "the form walk reached {probes} forms and {keys_seen} keys — the walk, not the \
         registry, is what broke"
    );
}

/// A single token a slot with this vocabulary would accept, for
/// building a line that reaches the slot after it.
///
/// Drawn from the vocabulary itself where there is one to draw from,
/// so the filled prefix is a line a user could really have typed and
/// [`super::Form::admits_prefix`] keeps the form it belongs to.
fn slot_filler(vocab: &super::Vocabulary) -> String {
    let words: &[super::Word] = match vocab {
        super::Vocabulary::Words(words) | super::Vocabulary::FreeWords { words, .. } => words,
        super::Vocabulary::Rows { sentinels, .. } => sentinels,
        super::Vocabulary::Free { .. } => &[],
    };
    // `x` for an open vocabulary with no sentinels — a font family,
    // a file path. One token, which is all the slot behind it needs.
    words
        .first()
        .map_or_else(|| "x".to_string(), |w| w.name.to_string())
}

/// A subverb that reads no keys refuses one by name rather than
/// dropping it.
///
/// The failing input is any subverb whose form list grows a key it
/// does not mean to read, or a `kvs::read` that stops asking per
/// *form*. Before the engine, four border surfaces staged
/// `border preset heavy color=#fff`'s preset and discarded the
/// color without a word.
#[test]
fn test_a_subverb_that_reads_no_keys_refuses_one_by_name() {
    use crate::application::console::parser::Args;
    use crate::application::console::spec::descent::descend_at;
    for (verb, grammar) in all_levels() {
        let Some(key) = grammar.keys().next() else { continue };
        for subverb in grammar.subverbs() {
            if subverb.child.is_some() || !subverb.readable_keys_for(&[]).is_empty() {
                continue;
            }
            // Enter this level directly so the probe is one
            // subverb plus one kv, whatever depth the level sits
            // at on a real line.
            let tokens = vec![subverb.name.to_string(), format!("{}=x", key.name)];
            let args = Args::new(&tokens);
            let descent = descend_at(grammar, &tokens, 0);
            let err = super::kvs::read(&descent, &args).err().unwrap_or_else(|| {
                panic!(
                    "{verb}: `{} {} {}=x` must be refused",
                    grammar.label, subverb.name, key.name
                )
            });
            assert!(
                err.contains(key.name),
                "{verb}: the refusal must name the key: {err}"
            );
            assert!(
                err.contains(subverb.name),
                "{verb}: the refusal must name the form: {err}"
            );
        }
    }
}

/// The gate is only ever declared on subverbs that have something
/// to be confused with — a level with no kv keys at all can never
/// put its subverb slot in kv form, so a gate there would be inert
/// and misleading.
#[test]
fn test_a_gated_subverb_sits_at_a_level_that_has_keys() {
    for (verb, grammar) in all_levels() {
        let has_keys = grammar.keys().next().is_some();
        for subverb in grammar.subverbs().filter(|s: &&Subverb| s.gated) {
            assert!(
                has_keys,
                "{verb}: `{} {}` is gated on the positional-vs-kv discriminator \
                 at a level that declares no keys, so the gate can never fire",
                grammar.label, subverb.name
            );
        }
    }
}

/// The stray-positional rejection opens a second sentence only when
/// it has one to open, and that sentence stands on its own words.
///
/// Four levels, four shapes: keys and `preview`, keys alone,
/// `preview` alone, neither. Fails when: the two suggestion clauses
/// are appended one after the other rather than composed — the
/// no-suggestion row then ends `'x'..` (which is what `open` really
/// printed) and the preview-only row opens on `or`.
///
/// Three of the four arms are reachable from the real registry, and
/// `EXEC_CORPUS` pins one line through each: `border padding 12 50`
/// and `canvas border padding 12 50` through `(composed, staged)`,
/// `anchor sideways` through `(composed, bare)`, and
/// `open a.mindmap.json b.mindmap.json` through `(neither)`. The
/// levels differ, so the bytes do — what those rows hold is the arm,
/// which is why a reword that satisfied this test by editing one
/// `match` arm would still move a pinned row.
///
/// The fourth arm — `preview` beside a bare form that reads no keys
/// — is declared by no level today, so the synthetic `PREVIEW_ONLY`
/// grammar below is the only thing that pins its wording. That is
/// the arm this test exists for; the other three it merely seconds.
#[test]
fn test_the_extra_positional_sentence_is_built_from_what_the_level_has() {
    use super::usage::extra_positional_message;
    use super::{Bare, Form, Key, Vocabulary};

    static KEYS: &[Key] = &[Key::new("pad", "padding", Vocabulary::Free { placeholder: "n" })];
    static COMPOSED: &[Form] = &[Form::opt(&["pad"])];
    static SLOT_ONLY: &[Form] = &[Form::slots(&[])];
    static PREVIEW: &[Subverb] = &[Subverb::bare("preview", "staged", "stage the edit")];

    static BOTH: Grammar = Grammar {
        label: "both",
        subverb_sets: &[PREVIEW],
        key_sets: &[KEYS],
        bare: Some(Bare::new("composed", COMPOSED)),
    };
    static KEYS_ONLY: Grammar = Grammar {
        label: "keys-only",
        subverb_sets: &[],
        key_sets: &[KEYS],
        bare: Some(Bare::new("composed", COMPOSED)),
    };
    static PREVIEW_ONLY: Grammar = Grammar {
        label: "preview-only",
        subverb_sets: &[PREVIEW],
        key_sets: &[],
        bare: Some(Bare::new("slots", SLOT_ONLY)),
    };
    static NEITHER: Grammar = Grammar {
        label: "neither",
        subverb_sets: &[],
        key_sets: &[],
        bare: Some(Bare::new("slots", SLOT_ONLY)),
    };

    assert_eq!(
        extra_positional_message(&BOTH, "both pad", "50"),
        "both pad: unexpected extra positional '50'. Compose multiple edits via \
         the kv form (`both <key>=<value> …`) or stage with `both preview …`."
    );
    assert_eq!(
        extra_positional_message(&KEYS_ONLY, "keys-only", "50"),
        "keys-only: unexpected extra positional '50'. Compose multiple edits via \
         the kv form (`keys-only <key>=<value> …`)."
    );
    assert_eq!(
        extra_positional_message(&PREVIEW_ONLY, "preview-only", "50"),
        "preview-only: unexpected extra positional '50'. Stage with `preview-only preview …`."
    );
    assert_eq!(
        extra_positional_message(&NEITHER, "neither", "50"),
        "neither: unexpected extra positional '50'."
    );

    // Hold the doc comment's division of labor against the real
    // registry rather than trusting it. Three arms are seconded by a
    // pinned corpus row *because* a level reaches them; the fourth
    // is pinned here alone *because* none does. A level that grows a
    // `preview` subverb beside a bare form reading no keys flips
    // that, and this is what says so — at which point the arm wants
    // a corpus row and this comment wants a rewrite.
    let mut reached = [false; 4];
    for (_, grammar) in all_levels() {
        let composed = grammar
            .bare
            .as_ref()
            .is_some_and(|b| !b.readable_keys().is_empty());
        let staged = grammar.subverb("preview").is_some();
        reached[usize::from(composed) * 2 + usize::from(staged)] = true;
    }
    assert!(
        reached[3] && reached[2] && reached[0],
        "the three arms `EXEC_CORPUS` seconds must each be reachable: {reached:?}"
    );
    assert!(
        !reached[1],
        "a level now declares `preview` beside a bare form that reads no keys, so the \
         `(false, true)` arm is reachable and owes a pinned corpus row of its own"
    );
}

/// Every `.rs` file under `commands/` that a shipped build
/// compiles: the tree minus the modules a parent declares as
/// `#[cfg(test)] mod <name>;`, which are whole files of test code
/// (`border/tests.rs` is the one in this tree).
///
/// The same distinction `baumhard::util::unwrap_posture` draws for
/// the same reason: a scan that reads test files reports their
/// assertions as if they were shipped code, and a scan that skips
/// live code reports nothing at all.
fn command_sources() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/application/console/commands");
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}"));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    // Whole-file test modules, named by whichever parent declares
    // them. Collected first, because the declaration sits in a
    // different file from the one it excludes.
    let mut excluded: Vec<std::path::PathBuf> = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let dir = path.parent().unwrap_or(&root).to_path_buf();
        let mut gated = false;
        for line in src.lines() {
            let line = line.trim();
            if line == "#[cfg(test)]" {
                gated = true;
                continue;
            }
            if gated {
                if let Some(name) = line
                    .strip_prefix("mod ")
                    .or_else(|| line.strip_prefix("pub mod "))
                    .and_then(|rest| rest.strip_suffix(';'))
                {
                    excluded.push(dir.join(format!("{name}.rs")));
                    excluded.push(dir.join(name).join("mod.rs"));
                }
                gated = false;
            }
        }
    }
    files.retain(|p| !excluded.contains(p));
    files.sort();
    files
}

/// Remove `{…}` format placeholders from `text`, leaving everything the
/// literal spells out itself.
///
/// The input that makes its caller fail: `usage: {} text=<text>` — a
/// placeholder for the head and a hand-written tail. Placeholders do not
/// nest in `format!`, so scanning for the next `}` after each `{` is exact;
/// an unclosed `{` consumes the remainder, which is the conservative
/// direction (it can only make a line look *more* derived, and such a
/// literal would not compile as a format string anyway).
fn strip_format_placeholders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        match rest[open..].find('}') {
            Some(close) => rest = &rest[open + close + 1..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Every `usage:` line a verb prints has its words supplied by
/// [`super::usage`], never typed out in the verb.
///
/// This is the one check here that reads the sources rather than the
/// registry, and it is the only shape that could have caught the
/// defect it was written for. `label` answered its no-arguments case
/// with
///
/// ```text
/// usage: label text="<text>" [position=<start|middle|end>]
/// ```
///
/// — two of the four keys `label`'s grammar declares. Every
/// registry-wide invariant above stayed green, because each of them
/// reads the table and the table was right; what was wrong was a
/// verb that did not ask it. The other three surfaces
/// (`label bogus=1`, `label <TAB>`, `help label`) had said four for
/// as long as the grammar had, so the four disagreed in the shipped
/// branch of the very epic whose acceptance criterion is that they
/// cannot.
///
/// The rule is mechanical: inside `console/commands/`, a string
/// literal may spell `usage:` only when what follows it is a format
/// placeholder — `help`'s `format!("usage: {}", head)`, whose
/// argument comes from [`super::usage::forms`]. Words after the
/// colon mean the verb wrote the line itself.
///
/// What it does **not** catch, stated so the next reader does not
/// over-trust it: a hand-written refusal that never says `usage:`,
/// and a `format!` whose *arguments* are hand-written. It catches
/// the shape all twenty-one no-argument answers in this tree take,
/// which is the shape the twenty-first took for the whole migration.
#[test]
fn test_no_console_verb_hand_writes_its_own_usage_line() {
    let files = command_sources();
    // The scan proves nothing if it read nothing. Twenty-one command
    // modules plus the registry and the shared helpers; the floor is
    // deliberately below that so a new file does not fail the run,
    // and deliberately above zero so a wrong path does.
    assert!(
        files.len() >= 20,
        "the command scan found only {} files — the walk, not the tree, is what broke",
        files.len()
    );

    let mut offenders: Vec<String> = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        for (line_no, content) in string_literals(&live_lines(&src)) {
            for (i, _) in content.match_indices("usage:") {
                let tail = content[i + "usage:".len()..].trim_start();
                // Every word after the colon must come from a format
                // placeholder, not from the literal. Checking only that
                // the tail *starts* with `{` accepts
                // `usage: {} text=<text>`, where the placeholder supplies
                // the head and the literal hand-writes the rest — the shape
                // `border/preview.rs` used before this series. Strip the
                // placeholders and require nothing but whitespace to
                // survive; format placeholders do not nest, so a
                // non-greedy scan is exact.
                let without_placeholders = strip_format_placeholders(tail);
                if !without_placeholders.trim().is_empty() {
                    offenders.push(format!(
                        "{}:{line_no}: hand-written usage line: `usage:{}`",
                        path.display(),
                        &content[i + "usage:".len()..]
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a verb spells out a usage line instead of deriving it from its grammar — \
         answer with `usage::no_arguments_message` / `usage::subverb_usage` instead:\n  {}",
        offenders.join("\n  ")
    );
}

/// The lines of a source file a shipped build compiles: every
/// top-level `#[cfg(test)] mod … { … }` dropped, everything else
/// kept, each surviving line paired with its 1-based number.
///
/// A `#[cfg(test)]` that introduces a `use` rather than a module is
/// *not* a cut point — `commands/mode.rs` opens one at line 34 and
/// then carries four hundred lines of live code, so cutting the file
/// at the first `#[cfg(test)]` would hide most of the verb from the
/// scan while the scan reported success.
///
/// The closing brace of a top-level module is a line that is exactly
/// `}`, which is what lets this be a line scan rather than a brace
/// parse; [`test_the_console_source_scan_reads_what_it_claims_to`]
/// holds that assumption against the tree.
fn live_lines(src: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_end() == "#[cfg(test)]"
            && lines
                .get(i + 1)
                .is_some_and(|l| l.starts_with("mod ") || l.starts_with("pub mod "))
        {
            i += 1;
            while i < lines.len() && lines[i] != "}" {
                i += 1;
            }
            i += 1;
            continue;
        }
        out.push((i + 1, lines[i]));
        i += 1;
    }
    out
}

/// Every string literal in `lines`, paired with the line it opens
/// on. Comments are skipped; a literal that spans lines is followed
/// across them.
///
/// There are no raw strings and no `'"'` char literals under
/// `commands/`, which is what keeps this a three-state scan;
/// [`test_the_console_source_scan_reads_what_it_claims_to`] holds
/// that too, since a raw string would make the scan silently read
/// the wrong thing rather than fail.
fn string_literals(lines: &[(usize, &str)]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut open: Option<(usize, String)> = None;
    for (line_no, line) in lines {
        let mut chars = line.char_indices().peekable();
        while let Some((idx, c)) = chars.next() {
            match &mut open {
                Some((_, body)) => match c {
                    '\\' => {
                        if let Some((_, escaped)) = chars.next() {
                            body.push(escaped);
                        }
                    }
                    '"' => {
                        let done = open.take().expect("just matched Some");
                        out.push(done);
                    }
                    other => body.push(other),
                },
                None => {
                    if c == '/' && line[idx..].starts_with("//") {
                        break;
                    }
                    if c == '"' {
                        open = Some((*line_no, String::new()));
                    }
                }
            }
        }
        // A literal left open at end of line continues on the next
        // one — Rust string literals may span lines, and the console's
        // longer messages are written that way.
        if let Some((_, body)) = &mut open {
            body.push('\n');
        }
    }
    out
}

/// The three assumptions the source scan above rests on, held
/// against the tree it reads rather than asserted in prose.
///
/// A `#[cfg(test)] mod` indented under something else would survive
/// [`live_lines`] and put test assertions in front of the usage
/// scan; a raw string would make [`string_literals`] read past its
/// own end; and a whole-file test module the parent gates with
/// `#[cfg(test)] mod x;` is invisible from inside the file itself —
/// `border/tests.rs` is that file, and the first draft of this scan
/// reported its five `assert_exec_err_contains(…, "usage: border …")`
/// lines as verbs hand-writing their usage. None of the three would
/// announce itself.
#[test]
fn test_the_console_source_scan_reads_what_it_claims_to() {
    let files = command_sources();
    assert!(
        files.len() >= 20,
        "the walk found only {} command sources",
        files.len()
    );
    assert!(
        !files.iter().any(|p| p.ends_with("border/tests.rs")),
        "`border/tests.rs` is a whole-file test module and must not be scanned as verb source"
    );
    let mut declared = 0usize;
    let mut excised = 0usize;
    for path in &files {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        // `r"` / `r#"` only opens a raw string when the `r` starts a
        // token; `"…color "` contains the same two bytes and does
        // not, which is what the first draft of this assertion
        // tripped over. Asked of the *live* lines only: `font.rs`
        // quotes a family name with `r#"…"#` inside its test module,
        // which the scan never reads.
        for (line_no, line) in live_lines(&src) {
            for pat in ["r\"", "r#\""] {
                for (i, _) in line.match_indices(pat) {
                    let before = line[..i].chars().next_back();
                    assert!(
                        before.is_some_and(|c| c.is_alphanumeric() || c == '_'),
                        "{}:{line_no}: a raw string literal would make `string_literals` \
                         read past its own end",
                        path.display()
                    );
                }
            }
        }
        let mut gated = false;
        for line in src.lines() {
            if line.trim() == "#[cfg(test)]" {
                gated = true;
                continue;
            }
            if gated {
                let body = line.trim_start();
                if (body.starts_with("mod ") || body.starts_with("pub mod ")) && body.ends_with('{') {
                    declared += 1;
                    assert!(
                        !line.starts_with(char::is_whitespace),
                        "{}: `#[cfg(test)] {}` is nested, and the excision only matches \
                         a module at column 0",
                        path.display(),
                        body
                    );
                }
                gated = false;
            }
        }
        excised += usize::from(live_lines(&src).len() < src.lines().count());
    }
    assert!(
        declared > 20 && excised > 20,
        "expected an inline test module in most command sources; declared {declared}, \
         excised from {excised} files"
    );
}

/// The alternatives a hint spells out are values its own vocabulary
/// accepts.
///
/// `Word::hint` and `Key::hint` are free text sitting one line above
/// the [`super::Vocabulary`] they describe, and nothing held the two
/// together. `border color` was declared as
///
/// ```text
/// Subverb::bare("color", "per-field", "set border color (#hex|var|preset|reset)")
///     .taking(&[Form::slots(&[Slot::req(free_words("#hex|var(--name)", COLOR_PRESET_WORDS))])])
/// ```
///
/// with `COLOR_PRESETS = ["accent", "edge", "fg", "reset"]` beside
/// it. `border color preset` answers `ERR color: unknown color
/// 'preset'`; `border color accent` succeeds. Both hints named the
/// value the slot refuses and omitted the three it accepts, and the
/// engine then *pinned* them — into `border <TAB>`, `canvas border
/// <TAB>`, `border preview <TAB>` and `help BORDER`.
///
/// What is checkable is narrow and deliberately so: an alternation —
/// a run of words joined by `|` — is a hint quoting a vocabulary, and
/// every alternative in it must be a word the vocabulary declares or
/// a fragment of its placeholder. Prose is not checked, because prose
/// is not a quotation. A leading `key=` names which vocabulary the
/// run belongs to, which is how `section text`'s
/// `(runs=preserve|clear)` is read against the `runs` key rather than
/// against the subverb's own text slot.
///
/// That narrowness shapes the correction as well as the check. The
/// `color` *key*'s hint listed its values with commas —
/// `"#hex, var(--name), preset, or 'reset'"` — which no alternation
/// scan can tell from prose, so it is now spelled
/// `"#hex, var(--name), accent | edge | fg, or 'reset'"`: the three
/// values the vocabulary declares are written as the alternation they
/// are, and the rewrite is what brings them under this test. A hint
/// that quotes a vocabulary in prose is still outside it.
#[test]
fn test_every_hinted_alternation_names_values_its_vocabulary_accepts() {
    let mut offenders: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for (verb, grammar) in all_levels() {
        for key in grammar.keys() {
            for run in alternations(key.hint) {
                checked += 1;
                check_run(grammar, &run, &[key.vocab], &mut offenders, || {
                    format!("{verb}: key `{}` of level `{}`", key.name, grammar.label)
                });
            }
        }
        for subverb in grammar.subverbs() {
            // A subverb's hint may quote either one of its own slots
            // or one of the keys its forms read — `border side`
            // quotes the first, `section text` the second — so the
            // vocabularies it may draw on are the union.
            let mut vocabs: Vec<super::Vocabulary> = Vec::new();
            for form in subverb.forms {
                vocabs.extend(form.slots.iter().map(|s| s.vocab));
            }
            vocabs.extend(
                subverb
                    .readable_keys_for(&[])
                    .iter()
                    .filter_map(|n| grammar.key(n))
                    .map(|k| k.vocab),
            );
            for run in alternations(subverb.hint) {
                checked += 1;
                check_run(grammar, &run, &vocabs, &mut offenders, || {
                    format!("{verb}: subverb `{} {}`", grammar.label, subverb.name)
                });
            }
        }
    }
    // A hint scan that found no alternations would pass by reading
    // nothing. The border and section families alone declare more
    // than a dozen, counted across every level that composes them.
    assert!(
        checked >= 12,
        "the hint scan found only {checked} alternations — the parse, not the registry, is what broke"
    );
    assert!(
        offenders.is_empty(),
        "a hint names a value the vocabulary beside it rejects:\n  {}",
        offenders.join("\n  ")
    );
}

/// Every `a|b|c` run in `hint`, each as its own alternative list.
///
/// Spaces around the pipes are removed first, so the two spellings
/// in the tree — `light | heavy | double` and `top|bottom|left` —
/// read the same; each alternative is then trimmed of the
/// punctuation a sentence wraps it in.
fn alternations(hint: &str) -> Vec<Vec<String>> {
    const EDGE: &[char] = &['(', ')', '[', ']', ',', '.', ';', ':', '\'', '"', '`', '<', '>'];
    let squeezed = hint.split('|').map(str::trim).collect::<Vec<_>>().join("|");
    squeezed
        .split_whitespace()
        .filter(|t| t.contains('|'))
        .map(|t| {
            t.split('|')
                .map(|w| w.trim_matches(EDGE).to_string())
                .filter(|w| !w.is_empty())
                .collect()
        })
        .filter(|run: &Vec<String>| run.len() > 1)
        .collect()
}

/// Hold one alternation against the vocabularies its declaration
/// makes available. A `key=` on the first alternative redirects the
/// whole run to that key's vocabulary, which is the only shape in
/// the tree where a hint quotes a vocabulary that is not its own.
fn check_run(
    grammar: &'static Grammar,
    run: &[String],
    vocabs: &[super::Vocabulary],
    offenders: &mut Vec<String>,
    site: impl Fn() -> String,
) {
    let mut run = run.to_vec();
    let mut vocabs = vocabs.to_vec();
    if let Some((name, first)) = run.first().and_then(|w| w.split_once('=')) {
        if let Some(key) = grammar.key(name) {
            let first = first.to_string();
            run[0] = first;
            vocabs = vec![key.vocab];
        }
    }
    for word in &run {
        if !vocabs.iter().any(|v| admits_hinted_word(v, word)) {
            offenders.push(format!(
                "{}: `{word}` is in the hint and in no vocabulary",
                site()
            ));
        }
    }
}

/// Whether a hint may name `word`: a declared vocabulary entry, or a
/// fragment of the placeholder that stands in for the open half of a
/// [`super::Vocabulary::FreeWords`] / [`super::Vocabulary::Rows`]
/// (`var` for `var(--name)`).
fn admits_hinted_word(vocab: &super::Vocabulary, word: &str) -> bool {
    let (placeholder, words): (&str, &[super::Word]) = match vocab {
        super::Vocabulary::Free { placeholder } => (placeholder, &[]),
        super::Vocabulary::Words(words) => ("", words),
        super::Vocabulary::FreeWords { placeholder, words } => (placeholder, words),
        super::Vocabulary::Rows {
            placeholder,
            sentinels,
            ..
        } => (placeholder, sentinels),
    };
    words.iter().any(|w| w.name.eq_ignore_ascii_case(word))
        || (!word.is_empty()
            && placeholder
                .to_ascii_lowercase()
                .contains(&word.to_ascii_lowercase()))
}

/// A positional narrows a two-form subverb to the shape that
/// declares it, and the keys of the other shape are then refused by
/// name.
///
/// The mechanism `section resize` needed and did not have.
/// `resize` declares the `fill` literal in one form and the
/// `w=`/`h=` pair in the other; the engine read the *union* of a
/// subverb's forms, so `section resize fill w=99 h=99` was accepted
/// and then dropped by a handler that returns on `fill` before it
/// looks at the pairs — `OK section: no change`, identically on
/// origin/main.
///
/// Three cases, because the narrowing has three outcomes and only
/// the first is the fix: a committed positional that one form
/// declares, no positional at all (the union stands, which is what
/// keeps `section resize <TAB>` offering `fill` beside `w=`), and a
/// positional no form admits (the union stands again, so a
/// structural error is reported on its own terms rather than as a
/// key problem).
#[test]
fn test_a_committed_positional_narrows_a_subverb_to_the_form_that_declares_it() {
    use super::{eligible_forms, free, Form, Slot, Subverb, Vocabulary, Word};

    static LITERAL: &[Word] = &[Word::new("fill", "the literal form")];
    static FORMS: &[Form] = &[
        Form::keys(&["w", "h"], &["target"]),
        Form::slots(&[Slot::req(Vocabulary::Words(LITERAL))]).reading(&["target"]),
    ];
    static SIZED: Subverb = Subverb::bare("resize", "geometry", "two shapes").taking(FORMS);

    assert_eq!(SIZED.readable_keys_for(&[]), vec!["w", "h", "target"]);
    assert_eq!(SIZED.readable_keys_for(&["fill"]), vec!["target"]);
    assert_eq!(SIZED.readable_keys_for(&["FILL"]), vec!["target"]);
    assert_eq!(SIZED.readable_keys_for(&["bogus"]), vec!["w", "h", "target"]);

    // The fallback is a fallback, not an accident: no form admits
    // `bogus`, so every form is back in play.
    assert_eq!(eligible_forms(FORMS, &["bogus"]).len(), 2);
    assert_eq!(eligible_forms(FORMS, &["fill"]).len(), 1);

    // An open vocabulary admits whatever is typed, so a form whose
    // slot is free never narrows anything away — the property that
    // keeps `border palette <name> field=…` reading `field` for a
    // palette named `fill`.
    static OPEN: &[Form] = &[Form::slots(&[Slot::req(free("name"))]).reading(&["target"])];
    assert_eq!(eligible_forms(OPEN, &["anything at all"]).len(), 1);
}

/// A repeated key resolves to its **last** occurrence.
///
/// The rule outlived its test. A parser-side test pinned it while
/// `Args::kv` owned it; `Args::kv` was deleted with the hand-rolled
/// grammar, the rule moved intact to [`super::kvs::value`]'s
/// `rfind`, and what replaced that test —
/// [`crate::application::console::parser`]'s
/// `test_args_kvs_keeps_every_occurrence_in_order` — pins the
/// *precondition*, that the iterator yields both occurrences, rather
/// than the resolution between them. So the behavior shipped
/// untested and unpinned.
///
/// Asserted here at its new home, and pinned end to end by the
/// `body glyph=dash glyph=dot` / `body glyph=dot glyph=dash` pair in
/// `EXEC_CORPUS`, which swap answers under a first-wins reader.
#[test]
fn test_a_repeated_key_resolves_to_its_last_occurrence() {
    use super::kvs::{value, Pair};
    use super::{free, Key};

    static GLYPH: Key = Key::new("glyph", "edge body glyph", free("name"));
    static SIZE: Key = Key::new("size", "point size", free("pt"));

    let pairs = vec![
        Pair {
            key: &GLYPH,
            value: "dash",
        },
        Pair {
            key: &SIZE,
            value: "12",
        },
        Pair {
            key: &GLYPH,
            value: "dot",
        },
    ];
    assert_eq!(value(&pairs, "glyph"), Some("dot"));
    // The other key is unaffected by the repetition beside it, and a
    // key that is not on the line is still `None` — the two failures
    // an `rfind` over the wrong predicate would produce.
    assert_eq!(value(&pairs, "size"), Some("12"));
    assert_eq!(value(&pairs, "absent"), None);
    assert_eq!(value(&[], "glyph"), None);
}
