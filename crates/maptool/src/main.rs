use baumhard::mindmap::loader::load_from_file;
use baumhard::mindmap::model::MindMap;
use regex::{Regex, RegexBuilder};
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: maptool <command> <map.json> <args...>

Commands:
  show <map.json> <node-id>     Print the text of the node with this ID.
  grep <map.json> <pattern>     Print every line in any node whose text
                                or notes matches the regex <pattern>,
                                one match per line as '<node-id>: <line>'.
                                Literal strings also work (they're valid
                                regexes). Pass -i anywhere before the
                                pattern for case-insensitive matching.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(msg)) => {
            eprintln!("{msg}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::NotFound(msg)) => {
            eprintln!("{msg}");
            ExitCode::from(1)
        }
        Err(CliError::Io(msg)) => {
            eprintln!("{msg}");
            ExitCode::from(1)
        }
    }
}

/// Deviation from `CODE_CONVENTIONS.md §4` ("no custom error types"):
/// a CLI binary genuinely needs to map distinct failure modes to
/// distinct exit codes. The app-crate rule assumes an interactive/GPU
/// posture where panicking at startup and logging at runtime is fine;
/// that doesn't translate to a tool that's supposed to be scriptable.
/// This enum is kept deliberately tiny — three string variants, no
/// `impl Error`, no `From` chains, no `thiserror` — so it stays a
/// dispatch table for exit codes rather than a growing taxonomy.
#[derive(Debug)]
enum CliError {
    Usage(String),
    NotFound(String),
    Io(String),
}

fn run(args: &[String]) -> Result<(), CliError> {
    let cmd = args
        .first()
        .ok_or_else(|| CliError::Usage("missing command".into()))?;
    match cmd.as_str() {
        "show" => {
            let map_path = args
                .get(1)
                .ok_or_else(|| CliError::Usage("show: missing <map.json>".into()))?;
            let node_id = args
                .get(2)
                .ok_or_else(|| CliError::Usage("show: missing <node-id>".into()))?;
            let map = load_map(map_path)?;
            let text = show_node(&map, node_id)
                .ok_or_else(|| CliError::NotFound(format!("node not found: {node_id}")))?;
            println!("{text}");
            Ok(())
        }
        "grep" => {
            let parsed = parse_grep_args(&args[1..])?;
            let regex = build_regex(parsed.pattern, parsed.case_insensitive)
                .map_err(|msg| CliError::Usage(format!("grep: {msg}")))?;
            let map = load_map(parsed.map_path)?;
            let matches = grep_nodes(&map, &regex);
            if matches.is_empty() {
                return Err(CliError::NotFound(format!(
                    "no matches for: {}",
                    parsed.pattern
                )));
            }
            for (id, line) in matches {
                println!("{id}: {line}");
            }
            Ok(())
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(CliError::Usage(format!("unknown command: {other}"))),
    }
}

fn load_map(path: &str) -> Result<MindMap, CliError> {
    load_from_file(Path::new(path)).map_err(CliError::Io)
}

/// Return the node's text, or None if no node has that ID.
fn show_node<'a>(map: &'a MindMap, node_id: &str) -> Option<&'a str> {
    map.nodes.get(node_id).map(|n| n.text.as_str())
}

/// Parsed form of the `grep` subcommand's positional arguments.
/// Borrowed from the caller's `&[String]` slice — no allocations.
struct GrepArgs<'a> {
    map_path: &'a str,
    pattern: &'a str,
    case_insensitive: bool,
}

/// Parse the args that follow `grep` on the command line. `-i` is
/// recognised anywhere in the arg list (not just immediately after
/// `grep`), and anything that isn't `-i` is treated as a positional
/// in its declared order. Users who legitimately need to match a
/// literal `-i` can escape it in the regex (e.g. `\-i`).
fn parse_grep_args(args: &[String]) -> Result<GrepArgs<'_>, CliError> {
    let mut case_insensitive = false;
    let mut positional: Vec<&str> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-i" => case_insensitive = true,
            other => positional.push(other),
        }
    }
    let map_path = positional
        .first()
        .copied()
        .ok_or_else(|| CliError::Usage("grep: missing <map.json>".into()))?;
    let pattern = positional
        .get(1)
        .copied()
        .ok_or_else(|| CliError::Usage("grep: missing <pattern>".into()))?;
    Ok(GrepArgs {
        map_path,
        pattern,
        case_insensitive,
    })
}

/// Compile a user-supplied pattern into a regex. Returns a plain
/// message on failure so the caller can prefix it with a subcommand
/// name (`grep: invalid regex ...`) without this helper knowing
/// which command invoked it.
fn build_regex(pattern: &str, case_insensitive: bool) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| format!("invalid regex {pattern:?}: {e}"))
}

/// Return every `(id, line)` pair where `line` is a line of a node's
/// `text` or `notes` that matches `regex`. A single node can produce
/// several entries if more than one of its lines matches (grep-style).
///
/// Results are sorted by node ID. IDs that parse as `u64` are
/// compared numerically (so `"97982720"` sorts before `"352207208"`
/// even though lexicographically it wouldn't); IDs that don't parse
/// fall back to lexicographic order. The sort is stable, so within a
/// node lines keep their natural order: `text` lines first, in
/// order, then `notes` lines, in order.
fn grep_nodes<'a>(map: &'a MindMap, regex: &Regex) -> Vec<(&'a str, &'a str)> {
    let mut out: Vec<(&'a str, &'a str)> = Vec::new();
    for node in map.nodes.values() {
        for line in node.text.lines() {
            if regex.is_match(line) {
                out.push((node.id.as_str(), line));
            }
        }
        for line in node.notes.lines() {
            if regex.is_match(line) {
                out.push((node.id.as_str(), line));
            }
        }
    }
    out.sort_by(|(a, _), (b, _)| match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        _ => a.cmp(b),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn testament() -> MindMap {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop(); // crates/maptool -> crates
        p.pop(); // crates -> root
        p.push("maps/testament.mindmap.json");
        load_from_file(&p).unwrap()
    }

    fn rx(pattern: &str, case_insensitive: bool) -> Regex {
        build_regex(pattern, case_insensitive).unwrap()
    }

    // --- show -------------------------------------------------------

    #[test]
    fn show_returns_text_for_known_id() {
        let map = testament();
        assert_eq!(show_node(&map, "348068464"), Some("Lord God"));
    }

    #[test]
    fn show_returns_none_for_unknown_id() {
        let map = testament();
        assert!(show_node(&map, "does-not-exist").is_none());
    }

    // --- grep / grep_nodes ------------------------------------------

    #[test]
    fn grep_finds_literal_pattern() {
        let map = testament();
        let hits = grep_nodes(&map, &rx("Lord God", false));
        assert!(hits.iter().any(|(id, _)| *id == "348068464"));
    }

    #[test]
    fn grep_case_insensitive_matches() {
        let map = testament();
        let insen = grep_nodes(&map, &rx("lord god", true));
        assert!(insen.iter().any(|(id, _)| *id == "348068464"));
    }

    #[test]
    fn grep_empty_on_no_match() {
        let map = testament();
        assert!(grep_nodes(&map, &rx("xyzzy-no-such-token", false)).is_empty());
    }

    #[test]
    fn grep_regex_metacharacters_match() {
        let map = testament();
        // "." is a wildcard, "L.rd God" matches "Lord God".
        let hits = grep_nodes(&map, &rx("L.rd God", false));
        assert!(hits.iter().any(|(id, _)| *id == "348068464"));
    }

    #[test]
    fn grep_regex_character_class_matches() {
        let map = testament();
        // Character class: matches either "Lord" or "lord".
        let hits = grep_nodes(&map, &rx("[Ll]ord God", false));
        assert!(hits.iter().any(|(id, _)| *id == "348068464"));
    }

    #[test]
    fn grep_regex_anchor_matches() {
        let map = testament();
        // "^Lord God" anchors on the start of a line (the root node
        // text has "Lord God" as its first and only line).
        let hits = grep_nodes(&map, &rx("^Lord God", false));
        assert!(hits.iter().any(|(id, _)| *id == "348068464"));
    }

    #[test]
    fn grep_invalid_regex_message() {
        // Unclosed bracket is a syntax error; build_regex returns a
        // bare message without the "grep:" prefix (that's added by
        // the caller in the grep subcommand).
        let err = build_regex("[unclosed", false).unwrap_err();
        assert!(err.contains("invalid regex"), "got: {err}");
        assert!(!err.starts_with("grep:"), "build_regex must not hardcode subcommand prefix");
    }

    #[test]
    fn grep_searches_notes_field() {
        // Inject a unique sentinel into one node's notes. No other
        // node in testament contains this token, and it isn't in
        // any node's text — so finding it proves notes are searched.
        let mut map = testament();
        map.nodes
            .get_mut("348068464")
            .unwrap()
            .notes = "SENTINEL_ZXCVBNM_12345".into();

        let hits = grep_nodes(&map, &rx("SENTINEL_ZXCVBNM_12345", false));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "348068464");
        assert!(hits[0].1.contains("SENTINEL_ZXCVBNM_12345"));
    }

    #[test]
    fn grep_returns_text_lines_before_notes_lines() {
        let mut map = testament();
        let node = map.nodes.get_mut("348068464").unwrap();
        node.text = "MARK_A\nMARK_B".into();
        node.notes = "MARK_C".into();

        let hits = grep_nodes(&map, &rx("^MARK_", false));
        let just_this: Vec<&str> = hits
            .iter()
            .filter(|(id, _)| *id == "348068464")
            .map(|(_, line)| *line)
            .collect();
        assert_eq!(just_this, vec!["MARK_A", "MARK_B", "MARK_C"]);
    }

    // --- parse_grep_args --------------------------------------------

    fn as_strings(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_grep_args_i_first() {
        let args = as_strings(&["-i", "map.json", "pat"]);
        let p = parse_grep_args(&args).unwrap();
        assert_eq!(p.map_path, "map.json");
        assert_eq!(p.pattern, "pat");
        assert!(p.case_insensitive);
    }

    #[test]
    fn parse_grep_args_i_after_map_path() {
        // The position bug from the review: -i between map and pattern.
        let args = as_strings(&["map.json", "-i", "pat"]);
        let p = parse_grep_args(&args).unwrap();
        assert_eq!(p.map_path, "map.json");
        assert_eq!(p.pattern, "pat");
        assert!(p.case_insensitive);
    }

    #[test]
    fn parse_grep_args_i_after_pattern() {
        let args = as_strings(&["map.json", "pat", "-i"]);
        let p = parse_grep_args(&args).unwrap();
        assert_eq!(p.map_path, "map.json");
        assert_eq!(p.pattern, "pat");
        assert!(p.case_insensitive);
    }

    #[test]
    fn parse_grep_args_no_i_flag() {
        let args = as_strings(&["map.json", "pat"]);
        let p = parse_grep_args(&args).unwrap();
        assert!(!p.case_insensitive);
    }

    #[test]
    fn parse_grep_args_missing_map_errors() {
        let args: Vec<String> = vec![];
        assert!(matches!(parse_grep_args(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn parse_grep_args_missing_pattern_errors() {
        let args = as_strings(&["map.json"]);
        assert!(matches!(parse_grep_args(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn parse_grep_args_only_flag_is_missing_map() {
        let args = as_strings(&["-i"]);
        // `-i` is consumed; no positional map path remains.
        assert!(matches!(parse_grep_args(&args), Err(CliError::Usage(_))));
    }

    // --- run() dispatch ---------------------------------------------

    #[test]
    fn run_no_command_is_usage_error() {
        let args: Vec<String> = vec![];
        assert!(matches!(run(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn run_unknown_command_is_usage_error() {
        let args = as_strings(&["foobar"]);
        assert!(matches!(run(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn run_show_missing_map_is_usage_error() {
        let args = as_strings(&["show"]);
        assert!(matches!(run(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn run_show_missing_node_id_is_usage_error() {
        // Note: uses a bogus map path — parser short-circuits before
        // load, so no I/O hits disk.
        let args = as_strings(&["show", "__does_not_exist.json"]);
        assert!(matches!(run(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn run_grep_missing_pattern_is_usage_error() {
        let args = as_strings(&["grep", "__does_not_exist.json"]);
        assert!(matches!(run(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn run_grep_invalid_regex_is_usage_error() {
        let args = as_strings(&["grep", "__does_not_exist.json", "[unclosed"]);
        match run(&args) {
            Err(CliError::Usage(msg)) => assert!(msg.starts_with("grep: invalid regex")),
            other => panic!("expected grep: invalid regex usage error, got {other:?}"),
        }
    }

    #[test]
    fn run_help_succeeds() {
        for flag in ["-h", "--help", "help"] {
            let args = as_strings(&[flag]);
            assert!(run(&args).is_ok(), "{flag} should succeed");
        }
    }
}
