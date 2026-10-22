// SPDX-License-Identifier: MPL-2.0

//! Usage forms, search tags and the unknown-subverb listing, all
//! derived from the one [`Grammar`] declaration.
//!
//! Before this module, `Command::usage` and `Command::tags` were
//! `&'static str` literals that `help` printed verbatim and that
//! nothing derived from the verb's key list. A key could be added,
//! offered by the popup on the next keystroke, and stay absent from
//! `help <verb>` forever with the suite green — which is exactly how
//! `font range=` and `color range=` both shipped. Here a key added
//! to a level is documented by the same edit that makes it
//! parseable, because there is only one edit.

use super::{Bare, Form, Grammar, Slot, Subverb, Vocabulary};

/// How a vocabulary reads inside a usage form.
///
/// A one-word closed vocabulary is a *literal* rather than a
/// choice, so it prints bare: `section resize fill`, not
/// `section resize <fill>`. Everything else is bracketed, which is
/// the shape every hand-written usage line in the console already
/// used.
fn render_vocabulary(vocab: &Vocabulary) -> String {
    match vocab {
        Vocabulary::Free { placeholder } => format!("<{}>", placeholder),
        Vocabulary::Words(words) => match words {
            [only] => only.name.to_string(),
            _ => format!("<{}>", words.iter().map(|w| w.name).collect::<Vec<_>>().join("|")),
        },
        Vocabulary::FreeWords { placeholder, words }
        | Vocabulary::Rows {
            placeholder,
            sentinels: words,
            ..
        } => {
            if words.is_empty() {
                format!("<{}>", placeholder)
            } else {
                format!(
                    "<{}|{}>",
                    placeholder,
                    words.iter().map(|w| w.name).collect::<Vec<_>>().join("|")
                )
            }
        }
    }
}

fn render_slot(slot: &Slot) -> String {
    let body = render_vocabulary(&slot.vocab);
    if slot.optional {
        format!("[{}]", body)
    } else {
        body
    }
}

/// `key=<value>`, bracketed when the form spells the key as
/// optional.
fn render_key(grammar: &Grammar, name: &str, optional: bool) -> String {
    let value = match grammar.key(name) {
        Some(key) => render_vocabulary(&key.vocab),
        // A form naming a key the level does not declare is caught
        // by `test_every_form_key_is_declared_by_its_level`; the
        // fallback keeps `help` printing rather than panicking in
        // an interactive path (CODE_CONVENTIONS §9).
        None => "<value>".to_string(),
    };
    if optional {
        format!("[{}={}]", name, value)
    } else {
        format!("{}={}", name, value)
    }
}

/// The widest a spelled-out usage line is allowed to get before its
/// kv tail collapses to `<key>=<value> …`.
///
/// Presentation, not vocabulary: nothing is lost by collapsing,
/// because `help` prints one `keys:` line per key underneath and
/// [`key_lines`] derives that block from the same declaration. What
/// the threshold buys is that a two-key verb keeps the informative
/// line it always had (`zoom [min=<zoom|unset>] [max=<zoom|unset>]`)
/// while `border`'s fifteen-key composed form stops being a
/// 300-column line the console overlay truncates mid-word.
const MAX_SPELLED_FORM: usize = 120;

fn push_form(head: &str, grammar: &Grammar, form: &Form) -> String {
    let mut tail = String::new();
    for slot in form.slots {
        tail.push(' ');
        tail.push_str(&render_slot(slot));
    }
    for name in form.required {
        tail.push(' ');
        tail.push_str(&render_key(grammar, name, false));
    }
    for name in form.optional {
        tail.push(' ');
        tail.push_str(&render_key(grammar, name, true));
    }
    if head.len() + tail.len() > MAX_SPELLED_FORM {
        tail = " <key>=<value> …".to_string();
    }
    format!("{}{}", head, tail)
}

/// One line per kv key the verb declares, at any level: the key
/// with its value vocabulary, then the sentence it is described by.
///
/// This is what makes the collapse above lossless, and it is also
/// the first time `help` has published a verb's key vocabulary at
/// all — `Command::tags` and `Command::usage` were hand-written
/// literals that nothing derived from the key list, so `font
/// range=` and `color range=` were each parseable, named in their
/// verb's own rejection, and documented nowhere.
///
/// Keys are deduplicated by name across levels: `border preview`
/// composes `border`'s whole keyset, and printing it twice would
/// say nothing the first copy did not.
pub fn key_lines(grammar: &'static Grammar) -> Vec<String> {
    let mut seen: Vec<&'static str> = Vec::new();
    let mut out: Vec<String> = Vec::new();
    collect_key_lines(grammar, &mut seen, &mut out);
    out
}

fn collect_key_lines(grammar: &'static Grammar, seen: &mut Vec<&'static str>, out: &mut Vec<String>) {
    for key in grammar.keys() {
        if seen.contains(&key.name) {
            continue;
        }
        seen.push(key.name);
        let spec = format!("{}={}", key.name, render_vocabulary(&key.vocab));
        out.push(if key.hint.is_empty() {
            spec
        } else {
            format!("{} — {}", spec, key.hint)
        });
    }
    for subverb in grammar.subverbs() {
        if let Some(child) = subverb.child {
            collect_key_lines(child, seen, out);
        }
    }
}

/// Every usage line this level publishes, outermost form first and
/// the bare form last.
///
/// One line per (subverb, form) pair, so `section move` prints its
/// delta shape and its absolute shape separately rather than
/// bracketing both onto a line the verb rejects. A subverb that
/// opens a child level contributes that level's lines instead of
/// one of its own — the child carries the full label, so
/// `border preview commit|cancel` reads as one line at the top
/// level.
pub fn forms(grammar: &'static Grammar) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for subverb in grammar.subverbs() {
        match subverb.child {
            Some(child) => out.extend(forms(child)),
            None => out.extend(subverb_forms(grammar, subverb)),
        }
    }
    if let Some(bare) = &grammar.bare {
        out.extend(bare_forms(grammar, bare));
    }
    out
}

/// The one usage line a single [`Form`] prints, under a subverb's
/// head or under the level's own.
///
/// `help` prints every form of every subverb; this is the same
/// rendering asked for one of them, so a rejection that has to name
/// a *particular* shape — `section resize fill w=99`, refused because
/// `w=` belongs to the other one — quotes the same words the help
/// page does.
pub fn form_line(grammar: &'static Grammar, subverb: Option<&'static Subverb>, form: &Form) -> String {
    let head = match subverb {
        Some(s) => format!("{} {}", grammar.label, s.name),
        None => grammar.label.to_string(),
    };
    push_form(&head, grammar, form)
}

fn subverb_forms(grammar: &'static Grammar, subverb: &'static Subverb) -> Vec<String> {
    let head = format!("{} {}", grammar.label, subverb.name);
    if subverb.forms.is_empty() {
        return vec![head];
    }
    subverb
        .forms
        .iter()
        .map(|form| push_form(&head, grammar, form))
        .collect()
}

fn bare_forms(grammar: &'static Grammar, bare: &Bare) -> Vec<String> {
    let head = grammar.label.to_string();
    if bare.forms.is_empty() {
        return vec![head];
    }
    bare.forms
        .iter()
        .map(|form| push_form(&head, grammar, form))
        .collect()
}

/// The search words `help <verb>` publishes: the verb's own name,
/// every subverb name at every depth, every kv key, then the
/// synonyms the command declares for words the grammar does not
/// contain (`wheel` under `color`, `lod` under `zoom`).
///
/// Derived rather than authored, so a key added to a level is
/// searchable by the same edit — the `tags` literal used to be a
/// third hand-maintained copy of the same vocabulary, and the two
/// keys that shipped undocumented were missing from it too.
pub fn tags(grammar: &'static Grammar, synonyms: &'static [&'static str]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut push = |word: &'static str| {
        if !word.is_empty() && !out.contains(&word) {
            out.push(word);
        }
    };
    // The label is the level's own words; the leading verb name is
    // the searchable one.
    if let Some(first) = grammar.label.split_whitespace().next() {
        push(first);
    }
    collect_tags(grammar, &mut push);
    for word in synonyms {
        push(word);
    }
    out
}

fn collect_tags(grammar: &'static Grammar, push: &mut impl FnMut(&'static str)) {
    for subverb in grammar.subverbs() {
        push(subverb.name);
        if let Some(child) = subverb.child {
            collect_tags(child, push);
        }
    }
    for key in grammar.keys() {
        push(key.name);
    }
}

/// The grouped listing of everything a level accepts, under a
/// caller-supplied heading.
///
/// Two callers, one shape: `usage: border` when the verb was handed
/// nothing it can act on, and `border: unknown subverb 'nope'` when
/// the word at the subverb slot names nothing the level declares.
/// Both used to be hand-written, and the two hand-written listings
/// on `border` and on `section` had already come to describe
/// different things at different column widths.
///
/// Groups appear in the order their first member is declared;
/// members within a group appear in declaration order. The label
/// column is as wide as the widest group name at this level, which
/// is what made the hand-written `border` listing (widest:
/// `visibility`) and the hand-written `section` listing (widest:
/// `structure`) sit at different widths — both are reproduced by
/// measuring rather than by two literals.
///
/// The listing prints subverb *names*, not their shapes: it exists
/// to answer "which words are there", and `help <verb>` — one
/// keystroke away, derived from the same table, and complete for
/// the first time — answers "and what does each take".
pub fn listing(grammar: &'static Grammar, head: &str) -> String {
    let mut groups: Vec<(&'static str, Vec<String>)> = Vec::new();
    for subverb in grammar.subverbs() {
        match groups.iter_mut().find(|(g, _)| *g == subverb.group) {
            Some((_, names)) => names.push(subverb.name.to_string()),
            None => groups.push((subverb.group, vec![subverb.name.to_string()])),
        }
    }
    if let Some(bare) = &grammar.bare {
        let mut body = String::new();
        for form in bare.forms {
            for slot in form.slots {
                if !body.is_empty() {
                    body.push(' ');
                }
                body.push_str(&render_slot(slot));
            }
            if form.names().next().is_some() && !body.contains("<key>=<value>") {
                if !body.is_empty() {
                    body.push(' ');
                }
                body.push_str("<key>=<value> …");
            }
        }
        if !body.is_empty() {
            groups.push((bare.group, vec![body]));
        }
    }
    let width = groups.iter().map(|(g, _)| g.len()).max().unwrap_or(0) + 2;
    let mut out = head.to_string();
    for (group, names) in &groups {
        out.push('\n');
        out.push_str(&format!(
            "  {:<width$}{}",
            format!("{}:", group),
            names.join(" | "),
            width = width
        ));
    }
    out
}

/// The listing a level prints when the word at its subverb slot
/// names nothing it declares.
pub fn unknown_subverb_message(grammar: &'static Grammar, typed: &str) -> String {
    listing(
        grammar,
        &format!("{}: unknown subverb '{}'", grammar.label, typed),
    )
}

/// What a level prints when it was handed nothing it can act on.
///
/// A level with subverbs answers with the grouped listing, because
/// "which words are there" is the question a user who typed the bare
/// verb is asking. A level with none — `anchor`, `body`, `spacing` —
/// has no groups to list, so it prints its forms instead, which is
/// the one-line usage those verbs have always shown.
pub fn no_arguments_message(grammar: &'static Grammar) -> String {
    if grammar.subverbs().next().is_none() {
        return format!("usage: {}", forms(grammar).join(" | "));
    }
    listing(grammar, &format!("usage: {}", grammar.label))
}

/// The `usage: …` a level prints when a subverb was handed too
/// little to act on — every shape that subverb accepts, joined the
/// way `help` joins forms, so the rejection and the help page cannot
/// word the shape differently.
///
/// A subverb with two forms prints both: `section move` takes
/// `dx=`/`dy=` *or* `x=`/`y=`, and one bracketed line would document
/// a command it refuses.
pub fn subverb_usage(grammar: &'static Grammar, subverb: &'static Subverb) -> String {
    let lines = subverb_forms(grammar, subverb);
    if lines.is_empty() {
        return format!("usage: {} {}", grammar.label, subverb.name);
    }
    format!("usage: {}", lines.join(" | "))
}

/// The rejection for a positional past everything the matched form
/// declares, so `border padding 12 50` does not silently drop the
/// `50`.
///
/// `full` is the form's own words (`border padding`); the composed
/// alternative and the `preview` suggestion are both read off the
/// level, so a surface that has no `preview` subverb does not
/// suggest one.
///
/// The composed alternative asks whether the bare form reads any
/// *keys*, not merely whether it exists: `open`'s bare form is a
/// path slot and nothing else, and pointing it at
/// ``open <key>=<value> …`` would name a form that has no keys to
/// fill it.
pub fn extra_positional_message(grammar: &'static Grammar, full: &str, extra: &str) -> String {
    let mut out = format!("{}: unexpected extra positional '{}'.", full, extra);
    let composed = grammar
        .bare
        .as_ref()
        .is_some_and(|b| !b.readable_keys().is_empty());
    let staged = grammar.subverb("preview").is_some();
    let label = grammar.label;
    // Four combinations, and the suggestion has to open its own
    // sentence in each. Appending the clauses one after another
    // instead left `open` — which has neither — with the bare
    // period twice, and would have left a level with `preview` but
    // no keys opening on `or`.
    let tail = match (composed, staged) {
        (true, true) => Some(format!(
            "Compose multiple edits via the kv form (`{label} <key>=<value> …`) \
             or stage with `{label} preview …`"
        )),
        (true, false) => Some(format!(
            "Compose multiple edits via the kv form (`{label} <key>=<value> …`)"
        )),
        (false, true) => Some(format!("Stage with `{label} preview …`")),
        (false, false) => None,
    };
    if let Some(tail) = tail {
        out.push(' ');
        out.push_str(&tail);
        out.push('.');
    }
    out
}
