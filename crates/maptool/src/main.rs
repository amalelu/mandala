use baumhard::mindmap::loader::load_from_file;
use baumhard::mindmap::model::MindMap;
use regex::{Regex, RegexBuilder};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

const USAGE: &str = "\
Usage: maptool <command> <map.json> <args...>

Commands:
  show <map.json> <node-id>     Print the text of the node with this ID.
  grep <map.json> <pattern>     Print every line in any node whose text
                                or notes matches the regex <pattern>,
                                one match per line as '<node-id>: <line>'.
                                Literal strings also work (they're valid
                                regexes). Pass -i anywhere before the
                                pattern for case-insensitive matching.
  apply <map.json> <pattern> [-i] [--notes] [--dry-run] -- <cmd> [args...]
                                For each node whose text (or notes with
                                --notes) has a line matching <pattern>,
                                pipe that field on stdin to <cmd> and
                                replace it with the command's stdout.
                                One trailing newline from <cmd> is
                                stripped. text_runs are cleared on
                                nodes whose text changed. Writes the
                                map back in place; --dry-run prints the
                                IDs that would change to stderr and
                                skips the write. Zero matches is an
                                error (exit 1), matching `grep`.";

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
        Err(CliError::Subprocess(msg)) => {
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
    Subprocess(String),
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
        "apply" => {
            let parsed = parse_apply_args(&args[1..])?;
            let regex = build_regex(parsed.pattern, parsed.case_insensitive)
                .map_err(|msg| CliError::Usage(format!("apply: {msg}")))?;
            let mut map = load_map(parsed.map_path)?;
            let ids = select_nodes(&map, &regex, parsed.target_notes);
            if ids.is_empty() {
                return Err(CliError::NotFound(format!(
                    "no nodes matched: {}",
                    parsed.pattern
                )));
            }
            let changed = apply_command(
                &mut map,
                &ids,
                parsed.target_notes,
                parsed.cmd,
                parsed.cmd_args,
            )?;
            if parsed.dry_run {
                eprintln!("dry-run: would modify {} node(s):", changed.len());
                for id in &changed {
                    eprintln!("  {id}");
                }
            } else if !changed.is_empty() {
                save_map(Path::new(parsed.map_path), &map)?;
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

/// Parsed form of the `apply` subcommand's positional arguments.
#[derive(Debug)]
struct ApplyArgs<'a> {
    map_path: &'a str,
    pattern: &'a str,
    case_insensitive: bool,
    target_notes: bool,
    dry_run: bool,
    cmd: &'a str,
    cmd_args: &'a [String],
}

/// Parse the args that follow `apply` on the command line. Flags `-i`,
/// `--notes`, and `--dry-run` are recognised anywhere before the `--`
/// separator. Everything after `--` is the external command and its
/// args, passed through verbatim so users can invoke any program.
fn parse_apply_args(args: &[String]) -> Result<ApplyArgs<'_>, CliError> {
    let mut case_insensitive = false;
    let mut target_notes = false;
    let mut dry_run = false;
    let mut positional: Vec<&str> = Vec::new();
    let mut sep_at: Option<usize> = None;
    for (i, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "--" => {
                sep_at = Some(i);
                break;
            }
            "-i" => case_insensitive = true,
            "--notes" => target_notes = true,
            "--dry-run" => dry_run = true,
            other => positional.push(other),
        }
    }
    let map_path = positional
        .first()
        .copied()
        .ok_or_else(|| CliError::Usage("apply: missing <map.json>".into()))?;
    let pattern = positional
        .get(1)
        .copied()
        .ok_or_else(|| CliError::Usage("apply: missing <pattern>".into()))?;
    let sep_at = sep_at.ok_or_else(|| {
        CliError::Usage("apply: missing `--` separator before command".into())
    })?;
    let tail = &args[sep_at + 1..];
    let cmd = tail
        .first()
        .map(|s| s.as_str())
        .ok_or_else(|| CliError::Usage("apply: missing command after `--`".into()))?;
    let cmd_args: &[String] = &tail[1..];
    Ok(ApplyArgs {
        map_path,
        pattern,
        case_insensitive,
        target_notes,
        dry_run,
        cmd,
        cmd_args,
    })
}

/// Return the sorted IDs of every node whose *target field* has at
/// least one line matching `regex`. Target field is `node.text` by
/// default, or `node.notes` when `target_notes` is true. Sort order
/// matches `grep_nodes`: numeric IDs compared as `u64`, others
/// lexicographic.
fn select_nodes(map: &MindMap, regex: &Regex, target_notes: bool) -> Vec<String> {
    let mut ids: Vec<String> = map
        .nodes
        .values()
        .filter(|n| {
            let target = if target_notes { &n.notes } else { &n.text };
            target.lines().any(|line| regex.is_match(line))
        })
        .map(|n| n.id.clone())
        .collect();
    ids.sort_by(|a, b| match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        _ => a.cmp(b),
    });
    ids
}

/// For each node in `ids`, pipe its target field through `cmd` and
/// replace the field with the command's stdout. When `target_notes`
/// is false and a node's `text` actually changed, that node's
/// `text_runs` are cleared — byte offsets would otherwise point into
/// stale positions. When `target_notes` is true, `text_runs` are left
/// alone (notes don't have runs).
///
/// Returns the list of IDs whose target field was actually modified,
/// preserving the input order. Aborts on the first subprocess failure
/// without touching subsequent nodes — callers that then choose not to
/// save get all-or-nothing semantics.
fn apply_command(
    map: &mut MindMap,
    ids: &[String],
    target_notes: bool,
    cmd: &str,
    cmd_args: &[String],
) -> Result<Vec<String>, CliError> {
    let mut changed: Vec<String> = Vec::new();
    for id in ids {
        let node = map
            .nodes
            .get_mut(id)
            .expect("id came from select_nodes, must exist in map");
        let input = if target_notes {
            node.notes.clone()
        } else {
            node.text.clone()
        };
        let new_value = run_pipe(cmd, cmd_args, &input)?;
        if new_value != input {
            if target_notes {
                node.notes = new_value;
            } else {
                node.text = new_value;
                node.text_runs.clear();
            }
            changed.push(id.clone());
        }
    }
    Ok(changed)
}

/// Spawn `cmd` with `cmd_args`, write `input` to its stdin, close
/// stdin to signal EOF, and return its stdout as a `String`. One
/// trailing newline (`\n` or `\r\n`) is stripped so POSIX text tools
/// that always append a newline don't inflate the node's text on every
/// apply. A non-zero exit status becomes `CliError::Subprocess` with
/// stderr folded into the message.
fn run_pipe(cmd: &str, cmd_args: &[String], input: &str) -> Result<String, CliError> {
    let mut child = Command::new(cmd)
        .args(cmd_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| CliError::Subprocess(format!("failed to spawn `{cmd}`: {e}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| CliError::Subprocess(format!("`{cmd}`: stdin handle missing")))?;
    // If the child exits before reading (or doesn't read stdin at all),
    // write_all returns EPIPE. Swallow that specifically — we want the
    // child's real exit status, not "broken pipe", to surface.
    if let Err(e) = stdin.write_all(input.as_bytes()) {
        if e.kind() != std::io::ErrorKind::BrokenPipe {
            return Err(CliError::Subprocess(format!(
                "`{cmd}`: write stdin: {e}"
            )));
        }
    }
    drop(stdin); // close the pipe so the child sees EOF
    let output = child
        .wait_with_output()
        .map_err(|e| CliError::Subprocess(format!("`{cmd}`: wait: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into());
        return Err(CliError::Subprocess(format!(
            "`{cmd}` exited with status {code}: {}",
            stderr.trim()
        )));
    }
    let mut out = String::from_utf8(output.stdout).map_err(|e| {
        CliError::Subprocess(format!("`{cmd}` produced non-UTF-8 output: {e}"))
    })?;
    if out.ends_with('\n') {
        out.pop();
        if out.ends_with('\r') {
            out.pop();
        }
    }
    Ok(out)
}

/// Serialize `map` back to `path` using pretty JSON. `Io` error on
/// serialisation or write failure.
fn save_map(path: &Path, map: &MindMap) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(map)
        .map_err(|e| CliError::Io(format!("failed to serialise map: {e}")))?;
    fs::write(path, json)
        .map_err(|e| CliError::Io(format!("failed to write {}: {e}", path.display())))
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

    // --- apply: fixture + tmpfile helpers ---------------------------
    //
    // The apply tests use a small hand-crafted map (tests/fixtures/
    // apply_test.mindmap.json) instead of testament, so the assertions
    // can name every node by ID without being coupled to the real map's
    // content. End-to-end tests that actually save the map copy the
    // fixture to a unique tmp path per test so parallel test runs don't
    // stomp on each other.

    fn apply_fixture_path() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/apply_test.mindmap.json");
        p
    }

    fn apply_fixture() -> MindMap {
        load_from_file(&apply_fixture_path()).unwrap()
    }

    fn tmp_map(name: &str) -> PathBuf {
        // pid + nanos keeps parallel test runs distinct.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut dst = std::env::temp_dir();
        dst.push(format!(
            "maptool_apply_{}_{}_{}.mindmap.json",
            name,
            std::process::id(),
            nanos
        ));
        std::fs::copy(apply_fixture_path(), &dst).unwrap();
        dst
    }

    // --- select_nodes -----------------------------------------------

    #[test]
    fn select_nodes_text_field_matches_hello() {
        let map = apply_fixture();
        let ids = select_nodes(&map, &rx("hello", false), false);
        assert_eq!(ids, vec!["n1".to_string(), "n4".to_string()]);
    }

    #[test]
    fn select_nodes_text_field_ignores_notes() {
        let map = apply_fixture();
        // NOTES_TOKEN only appears in n2's notes field, not any text.
        let ids = select_nodes(&map, &rx("NOTES_TOKEN", false), false);
        assert!(ids.is_empty(), "text-target should ignore notes: {ids:?}");
    }

    #[test]
    fn select_nodes_notes_field_matches_only_notes() {
        let map = apply_fixture();
        let ids = select_nodes(&map, &rx("NOTES_TOKEN", false), true);
        assert_eq!(ids, vec!["n2".to_string()]);
    }

    #[test]
    fn select_nodes_case_insensitive() {
        let map = apply_fixture();
        let ids = select_nodes(&map, &rx("HELLO", true), false);
        assert_eq!(ids, vec!["n1".to_string(), "n4".to_string()]);
    }

    #[test]
    fn select_nodes_no_match_empty() {
        let map = apply_fixture();
        assert!(select_nodes(&map, &rx("xyzzy_absent", false), false).is_empty());
    }

    // --- run_pipe ---------------------------------------------------

    #[test]
    fn run_pipe_uppercases_with_tr() {
        let out = run_pipe("tr", &["a-z".into(), "A-Z".into()], "hello world").unwrap();
        assert_eq!(out, "HELLO WORLD");
    }

    #[test]
    fn run_pipe_strips_one_trailing_newline() {
        // `cat; echo` emits the input followed by one extra newline.
        let out = run_pipe(
            "sh",
            &["-c".into(), "cat; echo".into()],
            "abc",
        )
        .unwrap();
        assert_eq!(out, "abc", "exactly one trailing newline should be stripped");
    }

    #[test]
    fn run_pipe_strips_only_one_newline() {
        // Two `echo`s emit two trailing newlines; only one is stripped.
        let out = run_pipe(
            "sh",
            &["-c".into(), "cat; echo; echo".into()],
            "abc",
        )
        .unwrap();
        assert_eq!(out, "abc\n");
    }

    #[test]
    fn run_pipe_preserves_internal_newlines() {
        let out = run_pipe("cat", &[], "one\ntwo\nthree\n").unwrap();
        assert_eq!(out, "one\ntwo\nthree");
    }

    #[test]
    fn run_pipe_nonzero_exit_is_subprocess_error() {
        let err = run_pipe("sh", &["-c".into(), "exit 7".into()], "x").unwrap_err();
        match err {
            CliError::Subprocess(msg) => {
                assert!(msg.contains('7'), "expected exit 7 in message, got: {msg}");
            }
            other => panic!("expected Subprocess, got {other:?}"),
        }
    }

    #[test]
    fn run_pipe_missing_binary_is_subprocess_error() {
        let err = run_pipe("__definitely_not_a_real_binary_xyz__", &[], "x").unwrap_err();
        assert!(matches!(err, CliError::Subprocess(_)));
    }

    // --- apply_command ----------------------------------------------

    #[test]
    fn apply_command_text_updates_and_clears_runs() {
        let mut map = apply_fixture();
        let ids = vec!["n1".to_string(), "n4".to_string()];
        let changed =
            apply_command(&mut map, &ids, false, "tr", &["a-z".into(), "A-Z".into()]).unwrap();
        assert_eq!(changed, vec!["n1".to_string(), "n4".to_string()]);
        assert_eq!(map.nodes["n1"].text, "HELLO WORLD");
        assert!(
            map.nodes["n1"].text_runs.is_empty(),
            "text_runs should be cleared when text changes"
        );
        assert_eq!(map.nodes["n4"].text, "HELLO AGAIN");
        assert!(map.nodes["n4"].text_runs.is_empty());
        // Untouched node keeps its runs.
        assert_eq!(map.nodes["n2"].text, "Alpha\nBeta\nGamma");
        assert_eq!(map.nodes["n2"].text_runs.len(), 1);
    }

    #[test]
    fn apply_command_notes_preserves_text_and_runs() {
        let mut map = apply_fixture();
        let original_text = map.nodes["n2"].text.clone();
        // TextRun doesn't implement PartialEq, so check structural fields.
        let before_len = map.nodes["n2"].text_runs.len();
        let before_start = map.nodes["n2"].text_runs[0].start;
        let before_end = map.nodes["n2"].text_runs[0].end;
        let ids = vec!["n2".to_string()];
        let changed =
            apply_command(&mut map, &ids, true, "tr", &["a-z".into(), "A-Z".into()]).unwrap();
        assert_eq!(changed, vec!["n2".to_string()]);
        assert_eq!(map.nodes["n2"].notes, "SECRET NOTES_TOKEN HERE");
        assert_eq!(map.nodes["n2"].text, original_text, "text untouched");
        assert_eq!(map.nodes["n2"].text_runs.len(), before_len);
        assert_eq!(map.nodes["n2"].text_runs[0].start, before_start);
        assert_eq!(map.nodes["n2"].text_runs[0].end, before_end);
    }

    #[test]
    fn apply_command_idempotent_when_output_equals_input() {
        let mut map = apply_fixture();
        let ids = vec!["n3".to_string()];
        // `cat` returns input verbatim; n3's text has no trailing newline,
        // so strip-one is a no-op and the value is unchanged.
        let changed = apply_command(&mut map, &ids, false, "cat", &[]).unwrap();
        assert!(
            changed.is_empty(),
            "expected no change, got: {changed:?}"
        );
        assert_eq!(map.nodes["n3"].text, "unchanged");
    }

    #[test]
    fn apply_command_subprocess_failure_propagates() {
        let mut map = apply_fixture();
        let ids = vec!["n1".to_string()];
        let result = apply_command(&mut map, &ids, false, "sh", &["-c".into(), "exit 4".into()]);
        assert!(matches!(result, Err(CliError::Subprocess(_))));
    }

    // --- run() dispatch for apply -----------------------------------

    #[test]
    fn run_apply_end_to_end_text() {
        let path = tmp_map("end_to_end_text");
        let args = as_strings(&[
            "apply",
            path.to_str().unwrap(),
            "hello",
            "--",
            "tr",
            "a-z",
            "A-Z",
        ]);
        assert!(run(&args).is_ok());
        let reloaded = load_from_file(&path).unwrap();
        assert_eq!(reloaded.nodes["n1"].text, "HELLO WORLD");
        assert_eq!(reloaded.nodes["n4"].text, "HELLO AGAIN");
        assert!(reloaded.nodes["n1"].text_runs.is_empty());
        assert!(reloaded.nodes["n4"].text_runs.is_empty());
        // Nodes that didn't match keep their content and their runs.
        assert_eq!(reloaded.nodes["n2"].text, "Alpha\nBeta\nGamma");
        assert_eq!(reloaded.nodes["n2"].text_runs.len(), 1);
        assert_eq!(reloaded.nodes["n3"].text, "unchanged");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_apply_end_to_end_notes() {
        let path = tmp_map("end_to_end_notes");
        let args = as_strings(&[
            "apply",
            path.to_str().unwrap(),
            "NOTES_TOKEN",
            "--notes",
            "--",
            "tr",
            "a-z",
            "A-Z",
        ]);
        assert!(run(&args).is_ok());
        let reloaded = load_from_file(&path).unwrap();
        assert_eq!(reloaded.nodes["n2"].notes, "SECRET NOTES_TOKEN HERE");
        assert_eq!(reloaded.nodes["n2"].text, "Alpha\nBeta\nGamma");
        assert_eq!(
            reloaded.nodes["n2"].text_runs.len(),
            1,
            "--notes edits should leave text_runs alone"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_apply_dry_run_does_not_modify_file() {
        let path = tmp_map("dry_run");
        let before = std::fs::read(&path).unwrap();
        let args = as_strings(&[
            "apply",
            path.to_str().unwrap(),
            "hello",
            "--dry-run",
            "--",
            "tr",
            "a-z",
            "A-Z",
        ]);
        assert!(run(&args).is_ok());
        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after, "--dry-run must not write the map");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_apply_no_matches_is_not_found_and_no_write() {
        let path = tmp_map("no_match");
        let before = std::fs::read(&path).unwrap();
        let args = as_strings(&[
            "apply",
            path.to_str().unwrap(),
            "xyzzy_absent_token",
            "--",
            "tr",
            "a-z",
            "A-Z",
        ]);
        match run(&args) {
            Err(CliError::NotFound(_)) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after, "no-match run must not write the map");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_apply_subprocess_failure_leaves_file_unchanged() {
        let path = tmp_map("subprocess_fail");
        let before = std::fs::read(&path).unwrap();
        let args = as_strings(&[
            "apply",
            path.to_str().unwrap(),
            "hello",
            "--",
            "sh",
            "-c",
            "exit 3",
        ]);
        match run(&args) {
            Err(CliError::Subprocess(_)) => {}
            other => panic!("expected Subprocess, got {other:?}"),
        }
        let after = std::fs::read(&path).unwrap();
        assert_eq!(
            before, after,
            "file must be unchanged when any subprocess fails"
        );
        let _ = std::fs::remove_file(&path);
    }

    // --- parse_apply_args -------------------------------------------

    #[test]
    fn parse_apply_args_basic_cmd_with_args() {
        let args = as_strings(&["map.json", "pat", "--", "tr", "a", "b"]);
        let p = parse_apply_args(&args).unwrap();
        assert_eq!(p.map_path, "map.json");
        assert_eq!(p.pattern, "pat");
        assert_eq!(p.cmd, "tr");
        assert_eq!(p.cmd_args, &["a".to_string(), "b".to_string()]);
        assert!(!p.case_insensitive);
        assert!(!p.target_notes);
        assert!(!p.dry_run);
    }

    #[test]
    fn parse_apply_args_flags_scattered_before_separator() {
        let args = as_strings(&[
            "-i", "map.json", "--notes", "--dry-run", "pat", "--", "cmd",
        ]);
        let p = parse_apply_args(&args).unwrap();
        assert_eq!(p.map_path, "map.json");
        assert_eq!(p.pattern, "pat");
        assert!(p.case_insensitive);
        assert!(p.target_notes);
        assert!(p.dry_run);
        assert_eq!(p.cmd, "cmd");
        assert!(p.cmd_args.is_empty());
    }

    #[test]
    fn parse_apply_args_flag_after_separator_is_passed_through() {
        // --dry-run after `--` is part of the user's command, not ours.
        let args = as_strings(&["map.json", "pat", "--", "echo", "--dry-run"]);
        let p = parse_apply_args(&args).unwrap();
        assert!(!p.dry_run, "--dry-run after `--` must not set our flag");
        assert_eq!(p.cmd, "echo");
        assert_eq!(p.cmd_args, &["--dry-run".to_string()]);
    }

    #[test]
    fn parse_apply_args_missing_separator_errors() {
        let args = as_strings(&["map.json", "pat", "tr", "a", "b"]);
        match parse_apply_args(&args) {
            Err(CliError::Usage(msg)) => assert!(msg.contains("--")),
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn parse_apply_args_empty_tail_errors() {
        let args = as_strings(&["map.json", "pat", "--"]);
        match parse_apply_args(&args) {
            Err(CliError::Usage(msg)) => {
                assert!(msg.contains("after `--`"), "got: {msg}")
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn parse_apply_args_missing_map_errors() {
        let args = as_strings(&["--", "cmd"]);
        assert!(matches!(parse_apply_args(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn parse_apply_args_missing_pattern_errors() {
        let args = as_strings(&["map.json", "--", "cmd"]);
        assert!(matches!(parse_apply_args(&args), Err(CliError::Usage(_))));
    }

    #[test]
    fn run_apply_invalid_regex_is_usage_error() {
        let args = as_strings(&[
            "apply",
            "__does_not_exist.json",
            "[unclosed",
            "--",
            "cat",
        ]);
        match run(&args) {
            Err(CliError::Usage(msg)) => assert!(msg.starts_with("apply: invalid regex")),
            other => panic!("expected apply: invalid regex usage error, got {other:?}"),
        }
    }
}
