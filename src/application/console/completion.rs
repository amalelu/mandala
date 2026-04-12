//! Tab-completion engine.
//!
//! Dispatches to one of three sources depending on where the cursor
//! sits:
//!
//! 1. Token 0 (command name) — fuzzy-match over [`COMMANDS`], hiding
//!    non-applicable commands.
//! 2. Token N > 0 — hand off to the resolved command's own
//!    [`complete`] fn, which knows the enum/id vocabulary for that
//!    position.
//! 3. Unknown command at token 0 with no suffix — fall through to
//!    an empty result (nothing to complete).

use super::commands::{command_by_name, Command, COMMANDS};
use super::fuzzy::fuzzy_score;
use super::parser::tokenize;
use super::ConsoleContext;

/// One completion candidate. `text` is the token that replaces the
/// partial; `display` is what the popup shows (usually equal to
/// `text`, but can include an arrow or padding); `hint` is the dim
/// right-hand side text (e.g. summary).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub display: String,
    pub hint: Option<String>,
}

/// Immutable snapshot of where the cursor is at the moment Tab was
/// pressed, passed to each command's `complete` fn.
pub struct CompletionState<'a> {
    /// All tokens including the partial one the user is typing.
    pub tokens: &'a [String],
    /// Which token index the cursor is inside.
    pub cursor_token: usize,
    /// The partial text of the token under the cursor.
    pub partial: &'a str,
}

/// Build completion candidates for `input` given a cursor byte
/// offset. Pure function — no GPU, no I/O. The event loop calls this
/// on Tab; unit tests drive it directly.
pub fn complete(input: &str, cursor: usize, ctx: &ConsoleContext) -> Vec<Completion> {
    // Identify the partial token by consulting the substring up to
    // the cursor.
    let cursor = cursor.min(input.len());
    let prefix = &input[..cursor];
    let tokens = tokenize(prefix);
    // If the prefix ends on whitespace (or is empty), the user is
    // starting a fresh token — cursor_token is `tokens.len()`.
    let at_word_boundary = prefix
        .chars()
        .last()
        .map(|c| c.is_whitespace())
        .unwrap_or(true);
    let cursor_token = if at_word_boundary {
        tokens.len()
    } else {
        tokens.len().saturating_sub(1)
    };
    let partial: &str = if at_word_boundary {
        ""
    } else {
        tokens.last().map(|s| s.as_str()).unwrap_or("")
    };

    // Build a stable tokens view that includes the (possibly empty)
    // partial so commands can reason about lookahead tokens too. The
    // view passed to command::complete always has cursor_token valid.
    let tokens_view: Vec<String> = if at_word_boundary {
        let mut v: Vec<String> = tokens.clone();
        v.push(String::new());
        v
    } else {
        tokens.clone()
    };
    let state = CompletionState {
        tokens: &tokens_view,
        cursor_token,
        partial,
    };

    if cursor_token == 0 {
        return complete_command_name(partial, ctx);
    }
    // Token > 0: resolve the first token to a command and defer.
    let first = match tokens.first() {
        Some(t) => t.as_str(),
        None => return Vec::new(),
    };
    let cmd = match command_by_name(first) {
        Some(c) => c,
        None => return Vec::new(),
    };
    (cmd.complete)(&state, ctx)
}

fn complete_command_name(partial: &str, ctx: &ConsoleContext) -> Vec<Completion> {
    // Rank applicable commands by fuzzy score against their name.
    // Non-applicable commands are skipped entirely so the completion
    // popup doesn't offer verbs that would immediately error.
    let mut scored: Vec<(&'static Command, i32)> = COMMANDS
        .iter()
        .filter(|c| (c.applicable)(ctx))
        .filter_map(|c| {
            if partial.is_empty() {
                Some((c, 0))
            } else {
                fuzzy_score(partial, c.name).map(|s| (c, s))
            }
        })
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored
        .into_iter()
        .map(|(c, _)| Completion {
            text: c.name.to_string(),
            display: c.name.to_string(),
            hint: Some(c.summary.to_string()),
        })
        .collect()
}

/// Helper used by per-command `complete` fns: filter a static enum
/// list by prefix and return completions with empty hints.
pub fn enum_completion<S: AsRef<str>>(options: &[S], partial: &str) -> Vec<Completion> {
    let partial_lc = partial.to_ascii_lowercase();
    options
        .iter()
        .filter(|o| {
            o.as_ref()
                .to_ascii_lowercase()
                .starts_with(&partial_lc)
        })
        .map(|o| Completion {
            text: o.as_ref().to_string(),
            display: o.as_ref().to_string(),
            hint: None,
        })
        .collect()
}
