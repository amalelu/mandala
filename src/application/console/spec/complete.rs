// SPDX-License-Identifier: MPL-2.0

//! Completion, derived from one [`Grammar`] walk.
//!
//! Descend the positional tokens that name subverbs, stop at the
//! first that does not, then answer for the slot the cursor is
//! actually in:
//!
//! - **at the level's subverb slot** — the subverbs it declares
//!   (minus the gated ones when a kv already made the slot kv-form),
//!   then the bare form's first slot vocabulary, then the kv keys;
//! - **inside a matched subverb's slots** — that slot's vocabulary,
//!   plus the kv keys once every *required* slot sits behind the
//!   cursor. This is what keeps `border preset <TAB>` a preset list
//!   while `section resize <TAB>` offers `fill` beside `w=` / `h=`;
//! - **on the value side of `key=`** — that key's vocabulary, and
//!   only if the matched form actually reads the key. `border show
//!   side=<TAB>` answers because `show` declares `side`, and the
//!   same slot on a form that does not declare it stays quiet.
//!
//! No arm here indexes a raw token. The only token-order reader is
//! [`super::descent`], and every arm below is keyed on the
//! positional index the execute path counts.

use crate::application::console::completion::{Completion, CompletionContext, CompletionState};
use crate::application::console::ConsoleContext;

use super::descent::{descend_at, subverb_slot_is_positional, Stop};
use super::{Form, Grammar, Key, Vocabulary, Word};

/// The popup for one console line, from the verb's grammar alone.
pub fn completions(
    grammar: &'static Grammar,
    state: &CompletionState,
    ctx: &ConsoleContext,
) -> Vec<Completion> {
    completions_at(grammar, state, ctx, 0)
}

/// [`completions`] for a level entered past a token the parent verb
/// already consumed — the completion-side twin of
/// [`super::descent::descend_at`], and the reason `section frame …`
/// can read the border grammar while `section` itself still answers
/// its own token-0 popup.
pub fn completions_at(
    grammar: &'static Grammar,
    state: &CompletionState,
    ctx: &ConsoleContext,
    start_slot: usize,
) -> Vec<Completion> {
    let tokens = state.arg_tokens();
    let descent = descend_at(grammar, tokens, start_slot);
    let level = descent.level;
    match &state.context {
        // Token 0 of the line is the verb name; the engine answers
        // it without consulting any grammar.
        CompletionContext::CommandName => Vec::new(),
        CompletionContext::KvValue { key } => match readable_key(&descent, tokens, key) {
            Some(k) => vocabulary_rows(&k.vocab, ctx, state.partial),
            None => Vec::new(),
        },
        CompletionContext::Token { index } if *index == descent.slot => {
            let positional_form = subverb_slot_is_positional(tokens, descent.slot);
            let mut out = subverb_rows(level, state.partial, positional_form);
            if let Some(bare) = &level.bare {
                out.extend(form_rows(level, bare.forms, 0, &[], ctx, state.partial));
            }
            out
        }
        CompletionContext::Token { index } => match descent.stop {
            Stop::Matched(subverb) => {
                let i = index.saturating_sub(descent.slot + 1);
                let committed = committed_before(&descent, tokens, i);
                form_rows(level, subverb.forms, i, &committed, ctx, state.partial)
            }
            // `Bare` past the bare form's own slots is the kv tail:
            // the keys stay on offer, which is what makes
            // `border preset=heavy <TAB>` list the rest of them.
            Stop::Bare => match &level.bare {
                Some(bare) => {
                    let committed = committed_before(&descent, tokens, bare.slot_count());
                    key_rows(level, &bare.readable_keys_for(&committed), state.partial)
                }
                None => Vec::new(),
            },
            // The word at the subverb slot names nothing this level
            // declares, or a kv already made the slot kv-form. The
            // line will be refused; offering its tail a vocabulary
            // would be promising a completion the verb cannot honor.
            Stop::Unknown | Stop::KvForm => Vec::new(),
        },
    }
}

/// The positionals already committed to the matched form's first `i`
/// slots — the same narrowing the parse loop applies, asked of the
/// slots strictly *before* the cursor so the one under it is still
/// free to become anything the popup offers.
///
/// The completion state carries no `Args`, so the slot walk is done
/// against the raw token slice here rather than through
/// `Descent::committed_slots`; both count positionals from
/// `descent.slot`, which is the invariant that keeps a `Token`
/// arm and the execute path looking at the same words.
fn committed_before<'a>(descent: &super::Descent, tokens: &'a [String], i: usize) -> Vec<&'a str> {
    let base = match descent.stop {
        Stop::Matched(_) => descent.slot + 1,
        _ => descent.slot,
    };
    (0..i)
        .map_while(|n| {
            tokens
                .iter()
                .filter(|t| !crate::application::console::parser::is_kv_token(t))
                .nth(base + n)
                .map(String::as_str)
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// The key under the cursor, but only when the shape being typed
/// reads it. A key the level declares and this form does not is not
/// offered a value vocabulary, for the same reason the parse loop
/// rejects it by name.
fn readable_key(descent: &super::Descent, tokens: &[String], key: &str) -> Option<&'static Key> {
    let readable = match descent.stop {
        Stop::Matched(subverb) => {
            subverb.readable_keys_for(&committed_before(descent, tokens, subverb.slot_count()))
        }
        Stop::Bare => {
            let bare = descent.level.bare.as_ref()?;
            bare.readable_keys_for(&committed_before(descent, tokens, bare.slot_count()))
        }
        Stop::Unknown | Stop::KvForm => return None,
    };
    if !readable.contains(&key) {
        return None;
    }
    descent.level.key(key)
}

/// What every shape of one subverb (or of a level's bare form)
/// offers at positional slot `i`: each form's own vocabulary for
/// that slot, then that form's kv keys once its *required* slots
/// sit behind the cursor.
///
/// Two shapes answering the same slot is what makes
/// `section resize <TAB>` offer `fill` beside `w=` / `h=`: one form
/// declares the literal, the other declares the pair, and the
/// cursor is legal in both.
fn form_rows(
    grammar: &'static Grammar,
    forms: &'static [Form],
    i: usize,
    committed: &[&str],
    ctx: &ConsoleContext,
    partial: &str,
) -> Vec<Completion> {
    let mut out = Vec::new();
    let mut keys: Vec<&'static str> = Vec::new();
    let push = |name: &'static str, keys: &mut Vec<&'static str>| {
        if !keys.contains(&name) {
            keys.push(name);
        }
    };
    // Only the shapes the positionals behind the cursor still admit:
    // `section resize fill <TAB>` is the `fill` form, so the other
    // form's `w=` / `h=` are withheld here exactly as the parse loop
    // refuses them.
    let eligible = super::eligible_forms(forms, committed);
    for form in &eligible {
        if let Some(slot) = form.slots.get(i) {
            out.extend(vocabulary_rows(&slot.vocab, ctx, partial));
        }
    }
    // Required keys of every eligible form first, then the
    // optional ones — the same order `help` prints and the
    // rejection quotes.
    for form in eligible.iter().filter(|f| f.required_slots_behind(i)) {
        for name in form.required {
            push(name, &mut keys);
        }
    }
    for form in eligible.iter().filter(|f| f.required_slots_behind(i)) {
        for name in form.optional {
            push(name, &mut keys);
        }
    }
    out.extend(key_rows(grammar, &keys, partial));
    out
}

/// One row per subverb whose name starts with `partial`,
/// case-insensitively — subverb names are matched that way on the
/// execute side, so `border PRE<TAB>` must reach the arm
/// `border PRESET heavy` runs.
///
/// `positional_form` is the discriminator's answer for this slot.
/// When it is false a kv already sits ahead of the subverb, so the
/// level will refuse its gated subverbs by name — the popup
/// withholds exactly those and keeps the rest, which the level
/// still honors there.
pub fn subverb_rows(grammar: &'static Grammar, partial: &str, positional_form: bool) -> Vec<Completion> {
    let partial_lc = partial.to_ascii_lowercase();
    grammar
        .subverbs()
        .filter(|s| positional_form || !s.gated)
        .filter(|s| s.name.starts_with(&partial_lc))
        .map(|s| row(s.name, s.hint))
        .collect()
}

/// One `key=` row per readable key whose name starts with
/// `partial`. Kv keys filter **case-sensitively** — a key is a
/// field name, not a word the user picks, and `border TOP=x` is
/// `unknown key 'TOP'` on the execute side
/// (`commands/mod.rs` § Casing).
fn key_rows(grammar: &'static Grammar, names: &[&'static str], partial: &str) -> Vec<Completion> {
    names
        .iter()
        .filter(|n| n.starts_with(partial))
        .filter_map(|n| grammar.key(n))
        .map(|k| {
            let text = format!("{}=", k.name);
            Completion {
                display: text.clone(),
                text,
                hint: (!k.hint.is_empty()).then(|| k.hint.to_string()),
                font_family: None,
            }
        })
        .collect()
}

/// The rows one vocabulary offers for `partial`.
///
/// Closed words and sentinels filter case-insensitively, matching
/// the parsers that read them (`eq_ignore_ascii_case` on every
/// sentinel in the console). Document-derived rows own their own
/// filtering, because only they know how their values compare — a
/// palette name is stored verbatim and found case-insensitively.
pub fn vocabulary_rows(vocab: &Vocabulary, ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    match vocab {
        Vocabulary::Free { .. } => Vec::new(),
        Vocabulary::Words(words) | Vocabulary::FreeWords { words, .. } => word_rows(words, partial),
        Vocabulary::Rows { rows, sentinels, .. } => {
            let mut out = rows(ctx, partial);
            out.extend(word_rows(sentinels, partial));
            out
        }
    }
}

fn word_rows(words: &'static [Word], partial: &str) -> Vec<Completion> {
    let partial_lc = partial.to_ascii_lowercase();
    words
        .iter()
        .filter(|w| w.name.to_ascii_lowercase().starts_with(&partial_lc))
        .map(|w| row(w.name, w.hint))
        .collect()
}

fn row(name: &str, hint: &str) -> Completion {
    Completion {
        text: name.to_string(),
        display: name.to_string(),
        hint: (!hint.is_empty()).then(|| hint.to_string()),
        font_family: None,
    }
}
