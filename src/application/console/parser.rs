// SPDX-License-Identifier: MPL-2.0

//! Shell-style tokenizer and command resolver for the console.
//!
//! The surface is small — whitespace splits, `"quoted strings"` keep
//! spaces inside, and `\"` / `\\` escape inside quotes. No variable
//! expansion, no pipes, no `--flag` machinery. `key=value` tokens
//! arrive as a single token and are split post-tokenize by
//! [`Args::kvs`]; this keeps the tokenizer byte-thin.
//!
//! Two entry points:
//!
//! - [`tokenize`] splits `input` into tokens and is what the
//!   completion engine wants.
//! - [`parse`] additionally resolves the first token against
//!   [`crate::application::console::commands::COMMANDS`] and returns
//!   the concrete command + remaining args.

use super::commands::{command_by_name, Command};

/// Split `input` into tokens. Returns an empty `Vec` for empty /
/// whitespace-only input. A trailing unterminated quote is tolerated
/// — the partial token is returned as-is so completion can still
/// reason about the fragment the user is typing.
///
/// # Escape semantics inside quoted strings
///
/// Only two escape sequences are recognised:
///
/// - `\"` → `"`
/// - `\\` → `\`
///
/// **Any other backslash is preserved literally**, including
/// `\n`, `\t`, `\xNN`, etc. The console doesn't grow a printf-
/// style escape grammar — most console commands work on
/// identifiers or hex literals where `\n` is not a meaningful
/// payload, and surfacing a backslash literally is the least-
/// surprising shape for users typing Windows path fragments
/// inside quotes (`save "C:\maps\foo.json"`). Authors who want
/// a literal backslash followed by `t` get exactly that — no
/// confusing tab.
pub fn tokenize(input: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut in_quote = false;
    let mut iter = input.chars().peekable();
    let mut had_any_char = false;
    while let Some(c) = iter.next() {
        if in_quote {
            match c {
                '"' => {
                    in_quote = false;
                }
                '\\' => {
                    if let Some(&next) = iter.peek() {
                        if next == '"' || next == '\\' {
                            buf.push(next);
                            iter.next();
                            continue;
                        }
                    }
                    buf.push(c);
                }
                _ => buf.push(c),
            }
            had_any_char = true;
        } else if c.is_whitespace() {
            if had_any_char {
                tokens.push(std::mem::take(&mut buf));
                had_any_char = false;
            }
        } else if c == '"' {
            in_quote = true;
            had_any_char = true;
        } else {
            buf.push(c);
            had_any_char = true;
        }
    }
    if had_any_char {
        tokens.push(buf);
    }
    tokens
}

/// Outcome of resolving the first token against `COMMANDS`.
pub enum ParseResult {
    /// Empty or whitespace-only input.
    Empty,
    /// No command's name or alias matches the first token.
    Unknown(String),
    /// Successfully resolved.
    Ok {
        cmd: &'static Command,
        args: Vec<String>,
    },
}

/// Tokenize `input`, resolve the first token against `COMMANDS`, and
/// return the parse result. No applicability checks happen here — the
/// caller decides whether to execute.
pub fn parse(input: &str) -> ParseResult {
    let tokens = tokenize(input);
    let Some((head, rest)) = tokens.split_first() else {
        return ParseResult::Empty;
    };
    match command_by_name(head) {
        Some(cmd) => ParseResult::Ok {
            cmd,
            args: rest.to_vec(),
        },
        None => ParseResult::Unknown(head.clone()),
    }
}

/// Thin wrapper over a slice of tokens (everything after the command
/// name). Exposes two views: positional tokens (anything without
/// `=`) and key-value pairs (anything containing `=`).
pub struct Args<'a> {
    tokens: &'a [String],
}

impl<'a> Args<'a> {
    pub fn new(tokens: &'a [String]) -> Self {
        Self { tokens }
    }

    pub fn tokens(&self) -> &'a [String] {
        self.tokens
    }

    /// Positional tokens skip any `key=value` token. A token is "kv"
    /// iff it contains `'='` and does not *start* with `'='` — so
    /// `=foo` is treated as a positional, which is the escape hatch
    /// for a literal value beginning with `=`.
    pub fn positional(&self, idx: usize) -> Option<&str> {
        self.positionals().nth(idx)
    }

    /// Iterator over positional tokens — skips kv tokens.
    pub fn positionals(&self) -> impl Iterator<Item = &str> {
        self.tokens.iter().filter(|t| !is_kv_token(t)).map(|s| s.as_str())
    }

    /// Iterator over `(key, value)` pairs. A kv token splits on the
    /// *first* `=`, so `color=var(--x)` yields `("color", "var(--x)")`.
    pub fn kvs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.tokens.iter().filter_map(|t| split_kv(t))
    }
}

/// A token is a kv iff it contains `=` and the `=` is not the first
/// character.
pub(super) fn is_kv_token(t: &str) -> bool {
    match t.find('=') {
        Some(0) | None => false,
        Some(_) => true,
    }
}

/// Split a `"key=value"` token into `(key, value)`. Returns
/// `None` when no `=` is present, or when `=` is at byte offset 0
/// (a leading-`=` token stays positional — the escape hatch for
/// literal values that happen to start with `=`).
///
/// Borrow-returning so cursor-time completion paths can call this
/// without forcing an allocation; owned-`String` callers
/// (`completion::compute_completion_state`) `.to_string()` at the
/// boundary.
pub(super) fn split_kv(t: &str) -> Option<(&str, &str)> {
    let eq = t.find('=')?;
    if eq == 0 {
        return None;
    }
    Some((&t[..eq], &t[eq + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_empty_returns_empty() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("   \t\n").is_empty());
    }

    #[test]
    fn test_tokenize_splits_on_whitespace() {
        assert_eq!(
            tokenize("color bg=#123 text=accent"),
            vec!["color", "bg=#123", "text=accent"]
        );
    }

    #[test]
    fn test_tokenize_preserves_quoted_spaces() {
        assert_eq!(
            tokenize(r#"label text="hello world""#),
            vec!["label", "text=hello world"]
        );
    }

    #[test]
    fn test_tokenize_handles_escaped_quote() {
        assert_eq!(
            tokenize(r#"say "he said \"hi\"""#),
            vec!["say", r#"he said "hi""#]
        );
    }

    #[test]
    fn test_tokenize_unterminated_quote_returns_partial_token() {
        assert_eq!(tokenize(r#"say "hello"#), vec!["say", "hello"]);
    }

    #[test]
    fn test_tokenize_empty_quote_yields_empty_token() {
        assert_eq!(tokenize(r#"foo "" bar"#), vec!["foo", "", "bar"]);
    }

    #[test]
    fn test_parse_empty_input() {
        assert!(matches!(parse(""), ParseResult::Empty));
        assert!(matches!(parse("   "), ParseResult::Empty));
    }

    #[test]
    fn test_parse_unknown_command() {
        let result = parse("wibble frob");
        assert!(matches!(result, ParseResult::Unknown(ref s) if s == "wibble"));
    }

    #[test]
    fn test_parse_resolves_command_name() {
        let result = parse("help");
        match result {
            ParseResult::Ok { cmd, args } => {
                assert_eq!(cmd.name, "help");
                assert!(args.is_empty());
            }
            _ => panic!("expected Ok"),
        }
    }

    #[test]
    fn test_args_positional_skips_kv_tokens() {
        let toks: Vec<String> = vec![
            "bg=#123".into(),
            "my-id".into(),
            "text=accent".into(),
            "extra".into(),
        ];
        let args = Args::new(&toks);
        assert_eq!(args.positional(0), Some("my-id"));
        assert_eq!(args.positional(1), Some("extra"));
        assert_eq!(args.positional(2), None);
    }

    #[test]
    fn test_args_leading_equals_token_is_positional() {
        // Escape hatch: a value literally starting with '=' isn't
        // parsed as a kv pair.
        let toks: Vec<String> = vec!["=raw".into()];
        let args = Args::new(&toks);
        assert_eq!(args.positional(0), Some("=raw"));
        assert_eq!(args.kvs().count(), 0);
    }

    #[test]
    fn test_args_kvs_iterates_key_value_pairs() {
        let toks: Vec<String> = vec!["bg=#123".into(), "positional".into(), "text=accent".into()];
        let args = Args::new(&toks);
        let pairs: Vec<(&str, &str)> = args.kvs().collect();
        assert_eq!(pairs, vec![("bg", "#123"), ("text", "accent")]);
    }

    #[test]
    fn test_args_kvs_splits_on_first_equals_only() {
        // A value carrying `=` inside (e.g. a data-url or a CSS
        // `var()`) keeps its remaining equals intact.
        let toks: Vec<String> = vec!["color=var(--x)".into()];
        let args = Args::new(&toks);
        assert_eq!(args.kvs().collect::<Vec<_>>(), vec![("color", "var(--x)")]);
    }

    /// A repeated key yields both pairs in line order; picking the
    /// last is the reader's job (`spec::kvs::value` does it, with
    /// the shell intuition that the last one sticks).
    #[test]
    fn test_args_kvs_keeps_every_occurrence_in_order() {
        let toks: Vec<String> = vec!["bg=#111".into(), "bg=#222".into()];
        let args = Args::new(&toks);
        assert_eq!(
            args.kvs().collect::<Vec<_>>(),
            vec![("bg", "#111"), ("bg", "#222")]
        );
    }
}
