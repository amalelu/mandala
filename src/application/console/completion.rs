// SPDX-License-Identifier: MPL-2.0

//! Contextual completion engine.
//!
//! Dispatches off a small [`CompletionContext`] enum that the engine
//! computes from the input + cursor once; commands then match on the
//! context in their `complete` fn. Prefix match only — no fuzzy
//! scoring.
//!
//! Three contexts:
//!
//! - [`CommandName`](CompletionContext::CommandName): token 0, the
//!   command verb. Engine-owned — commands don't get called at this
//!   position.
//! - [`Token`](CompletionContext::Token): a bare token past the
//!   command name. `index` is the positional slot, counting kv-form
//!   tokens as "not positional". Commands can treat this as either a
//!   positional arg or a prospective kv-key — they get to decide.
//! - [`KvValue`](CompletionContext::KvValue): the cursor sits on the
//!   value side of a `key=value` pair. `key` is the text before the
//!   `=`; `partial` is the text after.
//!
//! The engine is a pure function; the event loop re-runs it on every
//! keystroke that mutates the input buffer, so the popup stays live.

use super::commands::{command_by_name, COMMANDS};
use super::parser::tokenize;
use super::ConsoleContext;

/// One completion candidate. `text` is the token that replaces the
/// partial; `display` is what the popup shows (usually equal to
/// `text`, but can include an arrow or padding); `hint` is the dim
/// right-hand side text (e.g. summary).
///
/// `font_family` pins the shaped face for the candidate's `display`
/// text (not the hint). Set by the font-name completer so each row
/// renders in its own face — `font set A` shows every "A..." family
/// pre-shaped in that family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub display: String,
    pub hint: Option<String>,
    pub font_family: Option<String>,
}

/// Where the cursor is logically sitting. Computed by [`complete`]
/// from the raw input + cursor byte offset; handed to each command's
/// `complete` fn so it can choose the right vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionContext {
    /// Token 0 — picking a command verb. Not forwarded to any
    /// command; the engine handles it directly.
    CommandName,
    /// A bare token past the command name. `index` counts
    /// positionals-so-far (ignoring kv-form tokens) at the cursor
    /// position. Commands treat this as either a positional slot or
    /// as an opportunity to suggest kv-keys.
    Token { index: usize },
    /// Value side of a `key=value` pair. `partial` in the state
    /// holds the substring after `=`.
    KvValue { key: String },
}

/// Snapshot of where the cursor is, passed to each command's
/// `complete` fn. `context` is the primary dispatch switch;
/// [`Self::arg_tokens`] is the verb's own token slice, which
/// `console::spec::descent` walks.
///
/// There is deliberately no raw cursor-token index here, and no
/// per-command completer left to reach for one. An index used to be
/// published beside `tokens`, and reaching for it is what let a
/// `Token { index }` arm — which counts *positionals* — pair with a
/// lookahead that counted *tokens*, so the two disagreed the moment
/// a kv pair sat earlier on the line. The grammar descent is the
/// only reader of token order now.
pub struct CompletionState<'a> {
    pub tokens: &'a [String],
    /// What the user has typed in the current completion slot:
    /// - `CommandName`: the leading verb chars (e.g. `"co"`)
    /// - `Token`: the current bare token (e.g. `"he"`)
    /// - `KvValue`: the text after `=` within the current token
    pub partial: &'a str,
    pub context: CompletionContext,
}

impl CompletionState<'_> {
    /// The verb's own tokens — everything past the command name,
    /// which is the same slice [`super::parser::Args`] is built
    /// from on the execute side. Empty when the state carries no
    /// tokens at all, so a completer that reasons over raw token
    /// order never indexes off the end (`Application::run` reaches
    /// these on every keystroke; `CODE_CONVENTIONS.md` §9 forbids
    /// a panic there).
    pub fn arg_tokens(&self) -> &[String] {
        self.tokens.get(1..).unwrap_or(&[])
    }
}

/// Build completion candidates for `input` given a cursor byte
/// offset. Pure function — no GPU, no I/O.
pub fn complete(input: &str, cursor: usize, ctx: &ConsoleContext) -> Vec<Completion> {
    let cursor = cursor.min(input.len());
    let prefix = &input[..cursor];
    let tokens = tokenize(prefix);
    // If the prefix ends on whitespace (or is empty), the user is
    // starting a fresh token, so the cursor sits one slot past the
    // last complete one.
    let at_word_boundary = prefix.chars().last().map(|c| c.is_whitespace()).unwrap_or(true);
    let cursor_token = if at_word_boundary {
        tokens.len()
    } else {
        tokens.len().saturating_sub(1)
    };
    let raw_partial: String = if at_word_boundary {
        String::new()
    } else {
        tokens.last().cloned().unwrap_or_default()
    };

    // Build a stable tokens view that includes the (possibly empty)
    // partial so commands can reason about lookahead tokens too.
    let tokens_view: Vec<String> = if at_word_boundary {
        let mut v = tokens.clone();
        v.push(String::new());
        v
    } else {
        tokens.clone()
    };

    // Context detection.
    let (context, partial): (CompletionContext, String) = if cursor_token == 0 {
        (CompletionContext::CommandName, raw_partial)
    } else {
        match split_kv_at_cursor(&raw_partial) {
            Some((key, value_partial)) => (CompletionContext::KvValue { key }, value_partial),
            None => {
                // Count positionals before `cursor_token`. A token
                // is kv-form iff it contains `=` and doesn't start
                // with one.
                let index = tokens_view[1..cursor_token]
                    .iter()
                    .filter(|t| !is_kv_token(t))
                    .count();
                (CompletionContext::Token { index }, raw_partial)
            }
        }
    };

    let state = CompletionState {
        tokens: &tokens_view,
        partial: partial.as_str(),
        context,
    };

    if let CompletionContext::CommandName = state.context {
        return complete_command_name(state.partial, ctx);
    }
    // Past token 0: resolve the first token and defer.
    let first = match tokens.first() {
        Some(t) => t.as_str(),
        None => return Vec::new(),
    };
    let cmd = match command_by_name(first) {
        Some(c) => c,
        None => return Vec::new(),
    };
    cmd.completions(&state, ctx)
}

fn complete_command_name(partial: &str, ctx: &ConsoleContext) -> Vec<Completion> {
    let partial_lc = partial.to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|c| (c.applicable)(ctx))
        .filter(|c| c.name.to_ascii_lowercase().starts_with(&partial_lc))
        .map(|c| Completion {
            text: c.name.to_string(),
            display: c.name.to_string(),
            hint: Some(c.summary.to_string()),
            font_family: None,
        })
        .collect()
}

/// Helper used by per-command `complete` fns: filter a static list
/// by prefix and return completions with empty hints.
pub fn prefix_filter<S: AsRef<str>>(options: &[S], partial: &str) -> Vec<Completion> {
    let partial_lc = partial.to_ascii_lowercase();
    options
        .iter()
        .filter(|o| o.as_ref().to_ascii_lowercase().starts_with(&partial_lc))
        .map(|o| Completion {
            text: o.as_ref().to_string(),
            display: o.as_ref().to_string(),
            hint: None,
            font_family: None,
        })
        .collect()
}

use super::parser::is_kv_token;

/// Owned-`String` wrapper around [`super::parser::split_kv`] for
/// cursor-time partial-token splitting. The completion state is
/// built from owned strings (the borrow scope outlives this call),
/// so we `.to_string()` at the boundary rather than threading the
/// borrow through `CompletionContext::KvValue`.
fn split_kv_at_cursor(partial: &str) -> Option<(String, String)> {
    super::parser::split_kv(partial).map(|(k, v)| (k.to_string(), v.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a minimal CompletionContext from a raw input / cursor
    // pair by driving the pure split logic — this isolates the
    // detection from the command-registry lookup.
    fn detect_context(input: &str) -> CompletionContext {
        let cursor = input.len();
        let prefix = &input[..cursor];
        let tokens = tokenize(prefix);
        let at_word_boundary = prefix.chars().last().map(|c| c.is_whitespace()).unwrap_or(true);
        let cursor_token = if at_word_boundary {
            tokens.len()
        } else {
            tokens.len().saturating_sub(1)
        };
        let raw_partial: String = if at_word_boundary {
            String::new()
        } else {
            tokens.last().cloned().unwrap_or_default()
        };
        let tokens_view: Vec<String> = if at_word_boundary {
            let mut v = tokens.clone();
            v.push(String::new());
            v
        } else {
            tokens.clone()
        };
        if cursor_token == 0 {
            return CompletionContext::CommandName;
        }
        match split_kv_at_cursor(&raw_partial) {
            Some((key, _)) => CompletionContext::KvValue { key },
            None => CompletionContext::Token {
                index: tokens_view[1..cursor_token]
                    .iter()
                    .filter(|t| !is_kv_token(t))
                    .count(),
            },
        }
    }

    #[test]
    fn test_context_at_empty_is_command_name() {
        assert_eq!(detect_context(""), CompletionContext::CommandName);
    }

    #[test]
    fn test_context_while_typing_verb_is_command_name() {
        assert_eq!(detect_context("co"), CompletionContext::CommandName);
    }

    #[test]
    fn test_context_after_verb_space_is_token_0() {
        assert_eq!(detect_context("color "), CompletionContext::Token { index: 0 });
    }

    #[test]
    fn test_context_kv_value_is_detected_on_equals() {
        match detect_context("color bg=") {
            CompletionContext::KvValue { key } => assert_eq!(key, "bg"),
            other => panic!("expected KvValue, got {:?}", other),
        }
        match detect_context("color bg=#12") {
            CompletionContext::KvValue { key } => assert_eq!(key, "bg"),
            other => panic!("expected KvValue, got {:?}", other),
        }
    }

    #[test]
    fn test_context_kv_tokens_skipped_in_positional_index() {
        // Two prior kv-form tokens, one positional. Cursor sits on
        // a fresh bare-token slot — positional index is 1 (the one
        // prior positional was counted).
        match detect_context("help bg=#fff extra ") {
            CompletionContext::Token { index } => assert_eq!(index, 1),
            other => panic!("expected Token, got {:?}", other),
        }
    }

    #[test]
    fn test_context_leading_equals_is_positional_not_kv() {
        // Token `=raw` is the parser's escape hatch for a literal
        // value starting with `=` — completion treats it the same
        // way (not a kv-value mid-token).
        match detect_context("color =rawpartial") {
            CompletionContext::Token { .. } => {} // ok
            other => panic!("expected Token, got {:?}", other),
        }
    }

    #[test]
    fn test_prefix_filter_case_insensitive() {
        let opts = ["Anchor", "body", "COLOR"];
        let out = prefix_filter(&opts, "co");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "COLOR");
    }
}
