//! Shell-style tokenizer and command resolver for the console.
//!
//! The surface is small — whitespace splits, `"quoted strings"` keep
//! spaces inside, and `--flag=value` / `--flag value` both parse. No
//! variable expansion, no pipes, no escaping beyond `\"` and `\\`
//! inside quotes. Anything more elaborate is out of scope: this is a
//! command console, not a shell.
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
    if tokens.is_empty() {
        return ParseResult::Empty;
    }
    let (head, rest) = tokens.split_first().unwrap();
    match command_by_name(head) {
        Some(cmd) => ParseResult::Ok {
            cmd,
            args: rest.to_vec(),
        },
        None => ParseResult::Unknown(head.clone()),
    }
}

/// Thin wrapper over a slice of tokens (everything after the command
/// name). Lets commands grab positionals and flags without each one
/// reimplementing the same indexing.
pub struct Args<'a> {
    tokens: &'a [String],
}

impl<'a> Args<'a> {
    pub fn new(tokens: &'a [String]) -> Self {
        Self { tokens }
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn tokens(&self) -> &'a [String] {
        self.tokens
    }

    /// Positional args skip any `--flag` / `--flag=value` / `--flag
    /// value` tokens. Useful for commands where flags can appear in
    /// any position.
    pub fn positional(&self, idx: usize) -> Option<&str> {
        let mut seen = 0usize;
        let mut i = 0usize;
        while i < self.tokens.len() {
            let t = &self.tokens[i];
            if t.starts_with("--") {
                if !t.contains('=') && i + 1 < self.tokens.len() {
                    // consume next token as value
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if seen == idx {
                return Some(t);
            }
            seen += 1;
            i += 1;
        }
        None
    }

    /// Bare-flag presence check. Does NOT match `--flag=value` or
    /// `--flag value`.
    pub fn has_flag(&self, name: &str) -> bool {
        let needle = format!("--{}", name);
        self.tokens.iter().any(|t| t == &needle)
    }

    /// Read the value for `--flag=<value>` or `--flag <value>`. Prefers
    /// the `=` form when both are present (so `--scope=self --scope
    /// children` yields `self`).
    pub fn flag_value(&self, name: &str) -> Option<&str> {
        let prefix = format!("--{}=", name);
        let bare = format!("--{}", name);
        for (i, t) in self.tokens.iter().enumerate() {
            if let Some(v) = t.strip_prefix(&prefix) {
                return Some(v);
            }
            if t == &bare {
                if let Some(next) = self.tokens.get(i + 1) {
                    if !next.starts_with("--") {
                        return Some(next.as_str());
                    }
                }
            }
        }
        None
    }
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
        assert_eq!(tokenize("anchor set from auto"), vec!["anchor", "set", "from", "auto"]);
    }

    #[test]
    fn test_tokenize_preserves_quoted_spaces() {
        assert_eq!(
            tokenize(r#"label set "hello world""#),
            vec!["label", "set", "hello world"]
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
    fn test_parse_resolves_command_with_args() {
        let result = parse("anchor set from auto");
        match result {
            ParseResult::Ok { cmd, args } => {
                assert_eq!(cmd.name, "anchor");
                assert_eq!(args, vec!["set", "from", "auto"]);
            }
            _ => panic!("expected Ok"),
        }
    }

    #[test]
    fn test_args_positional_skips_flags() {
        let toks: Vec<String> = vec!["--scope", "self", "my-id".into(), "extra".into()]
            .into_iter()
            .map(Into::into)
            .collect();
        let args = Args::new(&toks);
        assert_eq!(args.positional(0), Some("my-id"));
        assert_eq!(args.positional(1), Some("extra"));
        assert_eq!(args.positional(2), None);
    }

    #[test]
    fn test_args_flag_value_equals_form() {
        let toks: Vec<String> = vec!["--scope=children".into()];
        let args = Args::new(&toks);
        assert_eq!(args.flag_value("scope"), Some("children"));
    }

    #[test]
    fn test_args_flag_value_space_form() {
        let toks: Vec<String> = vec!["--scope".into(), "parent".into()];
        let args = Args::new(&toks);
        assert_eq!(args.flag_value("scope"), Some("parent"));
    }

    #[test]
    fn test_args_flag_value_missing() {
        let toks: Vec<String> = vec!["foo".into()];
        let args = Args::new(&toks);
        assert_eq!(args.flag_value("scope"), None);
    }

    #[test]
    fn test_args_has_flag_bare() {
        let toks: Vec<String> = vec!["--all".into()];
        let args = Args::new(&toks);
        assert!(args.has_flag("all"));
        assert!(!args.has_flag("nope"));
    }
}
